#!/usr/bin/env bash

set -euo pipefail

readonly OFFICIAL_CDN_BASE_URL="https://cdn.chat2db-ai.com"

ossutil_bin=""
aliyun_bin=""
oss_config=""
plan_directory=""
snapshot_file=""
promotion_active=false
promotion_complete=false
publication_committed=false
publication_attempted=false
journal_object=""
journal_reused=false

die() {
  echo "error: $*" >&2
  exit 1
}

reject_control() {
  local value="$1" label="$2"
  case "$value" in
    *$'\t'*|*$'\n'*|*$'\r'*) die "$label must not contain tabs or line breaks" ;;
  esac
}

validate_object_name() {
  local value="$1" part
  reject_control "$value" "object name"
  case "$value" in
    ""|/*|*/|*//*|*\\*) die "invalid object name: $value" ;;
  esac
  IFS='/' read -r -a object_parts <<< "$value"
  for part in "${object_parts[@]}"; do
    [[ "$part" =~ ^[0-9A-Za-z][0-9A-Za-z._+-]*$ ]] || die "invalid object name: $value"
    case "$part" in .|..) die "invalid object name: $value" ;; esac
  done
}

validate_official_url() {
  local value="$1" object_name
  reject_control "$value" "CDN URL"
  case "$value" in
    "$OFFICIAL_CDN_BASE_URL"/*) ;;
    *) die "CDN URL is not under the official HTTPS origin: $value" ;;
  esac
  object_name="${value#"$OFFICIAL_CDN_BASE_URL"/}"
  validate_object_name "$object_name"
}

validate_sha256() {
  local value="$1"
  [[ "$value" =~ ^[0-9a-f]{64}$ ]] || die "invalid SHA-256 digest: $value"
}

local_sha256() {
  sha256sum "$1" | awk '{print $1}'
}

remote_uri() {
  printf 'oss://%s/%s' "$ALIYUN_OSS_BUCKET" "$1"
}

download_object() {
  local destination="$1" output_file="$2"
  "$ossutil_bin" -c "$oss_config" cp -f "$(remote_uri "$destination")" "$output_file"
}

# Returns 0 for present, 1 for absent, and 2 when OSS state cannot be determined.
remote_state() {
  local destination="$1" output
  if output="$("$ossutil_bin" -c "$oss_config" stat "$(remote_uri "$destination")" 2>&1)"; then
    return 0
  fi
  if grep -Eq 'StatusCode=404|NoSuchKey|ObjectNotExist' <<< "$output"; then
    return 1
  fi
  echo "error: cannot determine whether object exists: $destination" >&2
  echo "$output" >&2
  return 2
}

readback_object() {
  local destination="$1" expected_sha="$2" output_file="$3" actual_sha
  rm -f -- "$output_file"
  if ! download_object "$destination" "$output_file"; then
    echo "error: cannot read back OSS object: $destination" >&2
    return 1
  fi
  actual_sha="$(local_sha256 "$output_file")"
  if [ "$actual_sha" != "$expected_sha" ]; then
    echo "error: OSS readback checksum mismatch for $destination" >&2
    return 1
  fi
}

put_immutable_create_only() {
  local source="$1" destination="$2"
  local content_md5 request_date signature encoded_destination object_url
  content_md5="$(python3 - "$source" <<'PY'
import base64
import hashlib
import sys

digest = hashlib.md5(usedforsecurity=False)
with open(sys.argv[1], "rb") as source:
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(chunk)
print(base64.b64encode(digest.digest()).decode("ascii"))
PY
)"
  request_date="$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S GMT')"
  signature="$(python3 - "$ALIYUN_OSS_BUCKET" "$destination" "$content_md5" "$request_date" <<'PY'
import base64
import hashlib
import hmac
import os
import sys

bucket, destination, content_md5, request_date = sys.argv[1:]
canonical_resource = f"/{bucket}/{destination}"
string_to_sign = (
    f"PUT\n{content_md5}\napplication/octet-stream\n{request_date}\n"
    f"x-oss-forbid-overwrite:true\n{canonical_resource}"
)
signature = hmac.new(
    os.environ["ALIYUN_OSS_ACCESS_KEY_SECRET"].encode("utf-8"),
    string_to_sign.encode("utf-8"),
    hashlib.sha1,
).digest()
print(base64.b64encode(signature).decode("ascii"))
PY
)"
  encoded_destination="$(python3 - "$destination" <<'PY'
import sys
import urllib.parse

print(urllib.parse.quote(sys.argv[1], safe="/-_.~"))
PY
)"
  object_url="https://$ALIYUN_OSS_BUCKET.${ALIYUN_OSS_ENDPOINT#https://}/$encoded_destination"
  curl --fail --silent --show-error \
    --proto '=https' \
    --request PUT \
    --upload-file "$source" \
    --header "Authorization: OSS $ALIYUN_OSS_ACCESS_KEY_ID:$signature" \
    --header "Content-MD5: $content_md5" \
    --header 'Content-Type: application/octet-stream' \
    --header "Date: $request_date" \
    --header 'x-oss-forbid-overwrite:true' \
    "$object_url"
}

ensure_create_only_object() {
  local source="$1" destination="$2" expected_sha="$3" label="$4"
  local upload_output readback
  readback="$plan_directory/create-only-$(printf '%s' "$destination" | sha256sum | awk '{print $1}')"
  if remote_state "$destination"; then
    readback_object "$destination" "$expected_sha" "$readback" \
      || die "refusing to replace $label: $destination"
    echo "Already published $label $destination"
    return 0
  else
    case "$?" in
      1) ;;
      *) die "cannot safely stage $label: $destination" ;;
    esac
  fi

  echo "Uploading $label $destination"
  if ! upload_output="$(put_immutable_create_only "$source" "$destination" 2>&1)"; then
    if remote_state "$destination" \
      && readback_object "$destination" "$expected_sha" "$readback"; then
      echo "Concurrent publisher created identical $label $destination"
      return 0
    fi
    echo "$upload_output" >&2
    die "failed to create $label without overwrite: $destination"
  fi
  readback_object "$destination" "$expected_sha" "$readback" \
    || die "$label failed OSS readback: $destination"
}

validate_upload_plan() {
  local file="$1" expected_count="$2" source destination expected_sha
  local source_real artifacts_real count=0
  declare -A destinations=()
  artifacts_real="$(realpath -e "$PUBLISH_ARTIFACT_DIRECTORY")"
  while IFS=$'\t' read -r source destination expected_sha; do
    reject_control "$source" "upload source"
    validate_object_name "$destination"
    validate_sha256 "$expected_sha"
    if [ ! -f "$source" ] || [ -L "$source" ]; then
      die "upload source is not a regular file: $source"
    fi
    source_real="$(realpath -e "$source")"
    case "$source_real" in
      "$artifacts_real"/*) ;;
      *) die "upload source escapes artifact directory: $source" ;;
    esac
    [ "$(local_sha256 "$source_real")" = "$expected_sha" ] \
      || die "upload source checksum changed after plan creation: $source"
    [ -z "${destinations[$destination]+present}" ] || die "duplicate upload destination: $destination"
    destinations["$destination"]=1
    count=$((count + 1))
  done < "$file"
  [ "$count" -eq "$expected_count" ] \
    || die "unexpected upload plan size in $file: expected $expected_count, found $count"
}

validate_verify_plan() {
  local file="$1" expected_count="$2" url expected_sha name count=0
  declare -A names=()
  while IFS=$'\t' read -r url expected_sha name; do
    validate_official_url "$url"
    validate_sha256 "$expected_sha"
    validate_object_name "$name"
    [ "$url" = "$OFFICIAL_CDN_BASE_URL/$name" ] || die "verification URL/name mismatch: $name"
    [ -z "${names[$name]+present}" ] || die "duplicate verification object: $name"
    names["$name"]=1
    count=$((count + 1))
  done < "$file"
  [ "$count" -eq "$expected_count" ] \
    || die "unexpected verification plan size in $file: expected $expected_count, found $count"
}

validate_refresh_plan() {
  local file="$1" expected_count="$2" url count=0
  declare -A urls=()
  while IFS= read -r url; do
    validate_official_url "$url"
    [ -z "${urls[$url]+present}" ] || die "duplicate refresh URL: $url"
    urls["$url"]=1
    count=$((count + 1))
  done < "$file"
  [ "$count" -eq "$expected_count" ] \
    || die "unexpected refresh plan size in $file: expected $expected_count, found $count"
}

validate_plan() {
  validate_upload_plan "$plan_directory/immutable-uploads.tsv" 19
  validate_verify_plan "$plan_directory/immutable-verify.tsv" 19
  validate_upload_plan "$plan_directory/promotion-uploads.tsv" 9
  validate_refresh_plan "$plan_directory/promotion-refresh.txt" 9
  validate_verify_plan "$plan_directory/promotion-verify.tsv" 9
}

stage_immutable_objects() {
  local source destination expected_sha
  while IFS=$'\t' read -r source destination expected_sha; do
    ensure_create_only_object "$source" "$destination" "$expected_sha" "immutable object"
  done < "$plan_directory/immutable-uploads.tsv"
}

verify_url() {
  local url="$1" expected_sha="$2" name="$3"
  local download actual_sha attempt
  validate_official_url "$url"
  validate_sha256 "$expected_sha"
  validate_object_name "$name"
  download="$plan_directory/cdn-$(printf '%s' "$name" | sha256sum | awk '{print $1}')"
  for attempt in 1 2 3 4 5 6; do
    if curl --fail --silent --show-error \
      --proto '=https' \
      --connect-timeout 10 \
      --max-time 180 \
      --header "Referer: $OFFICIAL_CDN_BASE_URL/" \
      --header 'Cache-Control: no-cache' \
      --output "$download" \
      "$url"; then
      actual_sha="$(local_sha256 "$download")"
      if [ "$actual_sha" = "$expected_sha" ]; then
        echo "Verified $name through the CDN"
        return 0
      fi
    fi
    if [ "$attempt" -lt 6 ]; then
      sleep 10
    fi
  done
  echo "error: CDN readback checksum did not match for $name" >&2
  return 1
}

verify_plan() {
  local file="$1" url expected_sha name
  while IFS=$'\t' read -r url expected_sha name; do
    verify_url "$url" "$expected_sha" "$name" || return 1
  done < "$file"
}

refresh_and_wait() {
  local url="$1" response task_id status deadline
  validate_official_url "$url"
  echo "Refreshing $url"
  if ! response="$("$aliyun_bin" cdn RefreshObjectCaches --ObjectType File --ObjectPath "$url")"; then
    echo "error: CDN refresh request failed for $url" >&2
    return 1
  fi
  task_id="$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("RefreshTaskId", ""))' <<< "$response")"
  if [ -z "$task_id" ]; then
    echo "error: CDN refresh did not return RefreshTaskId" >&2
    return 1
  fi

  deadline=$((SECONDS + 300))
  while :; do
    if ! response="$("$aliyun_bin" cdn DescribeRefreshTasks --TaskId "$task_id")"; then
      echo "error: cannot read CDN refresh task: $task_id" >&2
      return 1
    fi
    status="$(python3 -c '
import json, sys

def statuses(value):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == "Status" and isinstance(child, str):
                yield child
            else:
                yield from statuses(child)
    elif isinstance(value, list):
        for child in value:
            yield from statuses(child)

values = list(statuses(json.load(sys.stdin)))
if any(value.lower() == "failed" for value in values):
    print("failed")
elif values and all(value.lower() == "complete" for value in values):
    print("complete")
else:
    print("pending")
' <<< "$response")"
    case "$status" in
      complete) return 0 ;;
      failed)
        echo "error: CDN refresh task failed: $task_id" >&2
        return 1
        ;;
    esac
    if [ "$SECONDS" -ge "$deadline" ]; then
      echo "error: timed out waiting for CDN refresh task: $task_id" >&2
      return 1
    fi
    sleep 10
  done
}

refresh_plan() {
  local file="$1" url
  while IFS= read -r url; do
    refresh_and_wait "$url" || return 1
  done < "$file"
}

journal_backup_object() {
  local version="$1" destination="$2" destination_hash
  destination_hash="$(printf '%s' "$destination" | sha256sum | awk '{print $1}')"
  printf 'release-journal/chat2db-cli/%s/backups/%s' "$version" "$destination_hash"
}

validate_durable_journal() {
  local file="$1" version="$2"
  local destination state backup_object old_sha extra expected_backup count=0
  local expected_file="$plan_directory/promotion-destinations.txt"
  local seen_file="$plan_directory/journal-destinations.txt"

  : > "$expected_file"
  while IFS=$'\t' read -r _ destination _; do
    validate_object_name "$destination"
    printf '%s\n' "$destination" >> "$expected_file"
  done < "$plan_directory/promotion-uploads.tsv"
  [ "$(wc -l < "$expected_file" | tr -d '[:space:]')" -eq 9 ] \
    || die "promotion plan is incomplete"
  [ "$(sort -u "$expected_file" | wc -l | tr -d '[:space:]')" -eq 9 ] \
    || die "promotion plan contains duplicate destinations"

  : > "$seen_file"
  while IFS=$'\t' read -r destination state backup_object old_sha extra; do
    validate_object_name "$destination"
    grep -Fxq -- "$destination" "$expected_file" \
      || die "journal contains an unexpected destination: $destination"
    ! grep -Fxq -- "$destination" "$seen_file" \
      || die "journal contains a duplicate destination: $destination"
    [ -z "${extra:-}" ] || die "journal row has too many fields: $destination"
    case "$state" in
      present)
        expected_backup="$(journal_backup_object "$version" "$destination")"
        [ "$backup_object" = "$expected_backup" ] \
          || die "journal backup path is invalid: $destination"
        validate_object_name "$backup_object"
        validate_sha256 "$old_sha"
        ;;
      absent)
        if [ "$backup_object" != - ] || [ "$old_sha" != - ]; then
          die "absent journal entry has backup data: $destination"
        fi
        ;;
      *) die "journal contains an invalid state for $destination: $state" ;;
    esac
    printf '%s\n' "$destination" >> "$seen_file"
    count=$((count + 1))
  done < "$file"

  [ "$count" -eq 9 ] \
    || die "durable promotion journal is incomplete"
}

load_durable_journal() {
  local version="$1" durable_file="$plan_directory/promotion-journal.tsv"
  local destination state backup_object old_sha backup count=0

  if ! download_object "$journal_object" "$durable_file"; then
    die "cannot read durable promotion journal: $journal_object"
  fi
  validate_durable_journal "$durable_file" "$version"

  : > "$snapshot_file"
  while IFS=$'\t' read -r destination state backup_object old_sha; do
    count=$((count + 1))
    if [ "$state" = present ]; then
      backup="$plan_directory/promotion-backup-$count"
      readback_object "$backup_object" "$old_sha" "$backup" \
        || die "durable promotion backup is invalid: $destination"
    else
      backup="-"
    fi
    printf '%s\t%s\t%s\t%s\n' "$destination" "$state" "$backup" "$old_sha" >> "$snapshot_file"
  done < "$durable_file"
  [ "$count" -eq 9 ] || die "loaded promotion snapshot is incomplete"
}

create_durable_journal() {
  local version="$1" durable_file="$plan_directory/promotion-journal-new.tsv"
  local destination backup backup_object old_sha state count=0 journal_sha

  : > "$snapshot_file"
  : > "$durable_file"
  while IFS=$'\t' read -r _ destination _; do
    count=$((count + 1))
    backup="$plan_directory/promotion-backup-$count"
    if remote_state "$destination"; then
      if ! download_object "$destination" "$backup"; then
        die "cannot snapshot promotion object: $destination"
      fi
      old_sha="$(local_sha256 "$backup")"
      backup_object="$(journal_backup_object "$version" "$destination")"
      ensure_create_only_object "$backup" "$backup_object" "$old_sha" "promotion backup"
      state="present"
    else
      case "$?" in
        1)
          old_sha="-"
          backup="-"
          backup_object="-"
          state="absent"
          ;;
        *) die "cannot snapshot promotion object state: $destination" ;;
      esac
    fi
    printf '%s\t%s\t%s\t%s\n' "$destination" "$state" "$backup" "$old_sha" >> "$snapshot_file"
    printf '%s\t%s\t%s\t%s\n' \
      "$destination" "$state" "$backup_object" "$old_sha" >> "$durable_file"
  done < "$plan_directory/promotion-uploads.tsv"
  [ "$count" -eq 9 ] || die "promotion snapshot is incomplete"
  validate_durable_journal "$durable_file" "$version"
  journal_sha="$(local_sha256 "$durable_file")"
  ensure_create_only_object "$durable_file" "$journal_object" "$journal_sha" \
    "promotion journal"
}

prepare_durable_journal() {
  local version="$1"
  journal_object="release-journal/chat2db-cli/$version/promotion-snapshot.tsv"
  if remote_state "$journal_object"; then
    journal_reused=true
    load_durable_journal "$version"
    echo "Loaded durable promotion journal $journal_object"
  else
    case "$?" in
      1)
        journal_reused=false
        create_durable_journal "$version"
        echo "Created durable promotion journal $journal_object"
        ;;
      *) die "cannot determine durable promotion journal state: $journal_object" ;;
    esac
  fi
}

apply_promotion_uploads() {
  local source destination expected_sha readback
  while IFS=$'\t' read -r source destination expected_sha; do
    echo "Promoting $destination"
    if ! "$ossutil_bin" -c "$oss_config" cp -f "$source" "$(remote_uri "$destination")"; then
      echo "error: failed to promote $destination" >&2
      return 1
    fi
    readback="$plan_directory/promoted-$(printf '%s' "$destination" | sha256sum | awk '{print $1}')"
    readback_object "$destination" "$expected_sha" "$readback" || return 1
  done < "$plan_directory/promotion-uploads.tsv"
}

verify_url_absent() {
  local url="$1" attempt http_code
  validate_official_url "$url"
  for attempt in 1 2 3 4 5 6; do
    http_code="$(curl --silent --show-error \
      --proto '=https' \
      --connect-timeout 10 \
      --max-time 180 \
      --header "Referer: $OFFICIAL_CDN_BASE_URL/" \
      --header 'Cache-Control: no-cache' \
      --output /dev/null \
      --write-out '%{http_code}' \
      "$url" || true)"
    if [ "$http_code" = 404 ]; then
      return 0
    fi
    if [ "$attempt" -lt 6 ]; then
      sleep 10
    fi
  done
  echo "error: rolled-back CDN object is still visible: $url" >&2
  return 1
}

rollback_promotion() {
  local destination state backup old_sha url rollback_failed=0 state_result
  echo "Restoring all previous mutable objects from the durable promotion journal" >&2
  while IFS=$'\t' read -r destination state backup old_sha; do
    case "$state" in
      present)
        if ! "$ossutil_bin" -c "$oss_config" cp -f "$backup" "$(remote_uri "$destination")" \
          || ! readback_object "$destination" "$old_sha" "$plan_directory/rollback-readback"; then
          echo "error: failed to restore $destination" >&2
          rollback_failed=1
        fi
        ;;
      absent)
        if ! "$ossutil_bin" -c "$oss_config" rm -f "$(remote_uri "$destination")"; then
          echo "error: failed to remove newly-created promotion object $destination" >&2
          rollback_failed=1
        else
          if remote_state "$destination"; then
            state_result=0
          else
            state_result=$?
          fi
          if [ "$state_result" -ne 1 ]; then
            echo "error: newly-created promotion object still exists: $destination" >&2
            rollback_failed=1
          fi
        fi
        ;;
      *)
        echo "error: invalid promotion snapshot state for $destination: $state" >&2
        rollback_failed=1
        ;;
    esac
  done < "$snapshot_file"

  while IFS= read -r url; do
    refresh_and_wait "$url" || rollback_failed=1
  done < "$plan_directory/promotion-refresh.txt"

  while IFS=$'\t' read -r destination state backup old_sha; do
    url="$OFFICIAL_CDN_BASE_URL/$destination"
    if [ "$state" = present ]; then
      verify_url "$url" "$old_sha" "$destination" || rollback_failed=1
    elif [ "$state" = absent ]; then
      verify_url_absent "$url" || rollback_failed=1
    fi
  done < "$snapshot_file"
  return "$rollback_failed"
}

write_release_asset_names() {
  local directory="$1" output_file="$2"
  python3 - "$directory" "$output_file" <<'PY'
import pathlib
import sys

directory = pathlib.Path(sys.argv[1])
entries = list(directory.iterdir())
invalid = [entry.name for entry in entries if entry.is_symlink() or not entry.is_file()]
if invalid:
    raise SystemExit(f"release directory contains non-file entries: {invalid}")
names = sorted(entry.name for entry in entries)
pathlib.Path(sys.argv[2]).write_text("".join(f"{name}\n" for name in names), encoding="utf-8")
PY
}

github_release_state() {
  local attempt is_draft
  for attempt in 1 2 3; do
    if is_draft="$(
      gh release view "$RELEASE_TAG" --repo "$GH_REPO" --json isDraft --jq .isDraft 2>/dev/null
    )"; then
      case "$is_draft" in
        true) printf 'draft\n'; return 0 ;;
        false) printf 'public\n'; return 0 ;;
      esac
    fi
    if [ "$attempt" -lt 3 ]; then
      sleep "$attempt"
    fi
  done
  return 1
}

verify_github_release_assets() {
  local github_assets="$plan_directory/github-assets"
  local expected_names="$plan_directory/expected-release-assets.txt"
  local actual_names="$plan_directory/actual-release-assets.txt"
  local asset_name

  rm -rf -- "$github_assets"
  mkdir -p -- "$github_assets"
  if ! gh release download "$RELEASE_TAG" --repo "$GH_REPO" --dir "$github_assets"; then
    echo "error: cannot download draft GitHub Release assets" >&2
    return 1
  fi
  if ! cmp "$PUBLISH_ARTIFACT_DIRECTORY/SHA256SUMS" "$github_assets/SHA256SUMS"; then
    echo "error: draft GitHub Release checksum manifest differs from the canonical bundle" >&2
    return 1
  fi
  if ! (cd "$github_assets" && sha256sum --check SHA256SUMS); then
    echo "error: draft GitHub Release asset checksum verification failed" >&2
    return 1
  fi

  if ! write_release_asset_names "$PUBLISH_ARTIFACT_DIRECTORY" "$expected_names" \
    || ! write_release_asset_names "$github_assets" "$actual_names"; then
    echo "error: cannot enumerate GitHub Release assets" >&2
    return 1
  fi
  if ! diff -u "$expected_names" "$actual_names"; then
    echo "error: draft GitHub Release assets do not exactly match the canonical bundle" >&2
    return 1
  fi
  while IFS= read -r asset_name; do
    if ! cmp "$PUBLISH_ARTIFACT_DIRECTORY/$asset_name" "$github_assets/$asset_name"; then
      echo "error: draft GitHub Release asset differs from the canonical bundle: $asset_name" >&2
      return 1
    fi
  done < "$expected_names"
}

verify_and_publish_github_release() {
  local state
  if ! state="$(github_release_state)"; then
    echo "error: cannot read GitHub Release $RELEASE_TAG" >&2
    return 1
  fi
  if [ "$state" != draft ]; then
    echo "error: GitHub Release $RELEASE_TAG is not a draft" >&2
    return 1
  fi
  verify_github_release_assets || return 1

  publication_attempted=true
  if gh release edit "$RELEASE_TAG" --repo "$GH_REPO" --draft=false --latest; then
    publication_committed=true
    return 0
  fi

  # Resolve a lost response after a successful GitHub mutation before deciding
  # whether CDN promotion must be rolled back.
  if state="$(github_release_state)" && [ "$state" = public ]; then
    publication_committed=true
    echo "warning: GitHub publish returned an error, but the Release is public" >&2
    return 0
  fi
  echo "error: failed to publish GitHub Release $RELEASE_TAG" >&2
  return 1
}

cleanup_and_maybe_rollback() {
  local status=$? rollback_status=0 state="" should_rollback=false
  trap - EXIT INT TERM
  if [ "$promotion_active" = true ] && [ "$promotion_complete" != true ]; then
    if [ "$publication_committed" = true ]; then
      echo "warning: Release is public; preserving CDN promotion for durable reconciliation" >&2
    elif [ "$publication_attempted" = true ]; then
      if state="$(github_release_state)" && [ "$state" = public ]; then
        publication_committed=true
        echo "warning: Release became public; preserving CDN promotion for durable reconciliation" >&2
      else
        echo "error: GitHub publication was attempted but is not final; deferring rollback to a rerun" >&2
        status=1
      fi
    elif state="$(github_release_state)"; then
      if [ "$state" = public ]; then
        publication_committed=true
        echo "warning: Release became public; preserving CDN promotion for durable reconciliation" >&2
      else
        should_rollback=true
      fi
    else
      echo "error: cannot determine GitHub publication state; deferring rollback to a rerun" >&2
      status=1
    fi
    if [ "$should_rollback" = true ]; then
      set +e
      rollback_promotion
      rollback_status=$?
      set -e
      if [ "$rollback_status" -ne 0 ]; then
        echo "error: promotion rollback was incomplete; inspect the errors above" >&2
        status=1
      fi
    fi
  fi
  [ -z "$oss_config" ] || rm -f -- "$oss_config"
  exit "$status"
}

validate_environment() {
  local variable_name value
  local required_environment=(
    ALIYUN_OSS_ACCESS_KEY_ID
    ALIYUN_OSS_ACCESS_KEY_SECRET
    ALIYUN_OSS_ENDPOINT
    ALIYUN_OSS_BUCKET
    GITHUB_ACTION_PATH
    PUBLISH_ARTIFACT_DIRECTORY
    PUBLISH_MODE
    RUNNER_TEMP
  )
  case "${PUBLISH_MODE:-}" in
    cdn-only) ;;
    release) required_environment+=(GH_REPO GH_TOKEN RELEASE_TAG) ;;
    *) die "PUBLISH_MODE must be cdn-only or release" ;;
  esac
  for variable_name in "${required_environment[@]}"; do
    value="${!variable_name:-}"
    [ -n "$value" ] || die "required environment variable is empty: $variable_name"
    reject_control "$value" "$variable_name"
  done
  [[ "$ALIYUN_OSS_BUCKET" =~ ^[a-z0-9][a-z0-9-]{1,61}[a-z0-9]$ ]] \
    || die "ALIYUN_OSS_BUCKET is not a valid bucket name"
  [[ "$ALIYUN_OSS_ENDPOINT" =~ ^https://[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?(:[0-9]+)?$ ]] \
    || die "ALIYUN_OSS_ENDPOINT must be an HTTPS origin without a path"
  [[ "$ALIYUN_OSS_ENDPOINT" != *..* ]] || die "ALIYUN_OSS_ENDPOINT contains an invalid hostname"
  if [ "$PUBLISH_MODE" = release ]; then
    [[ "$GH_REPO" =~ ^[0-9A-Za-z_.-]+/[0-9A-Za-z_.-]+$ ]] \
      || die "GH_REPO must be an owner/repository name"
    [[ "$RELEASE_TAG" =~ ^v[0-9A-Za-z][0-9A-Za-z.+-]*$ ]] \
      || die "RELEASE_TAG is invalid"
  fi
  if [ ! -d "$PUBLISH_ARTIFACT_DIRECTORY" ] || [ -L "$PUBLISH_ARTIFACT_DIRECTORY" ]; then
    die "artifact directory must be a real directory"
  fi
  [ -d "$GITHUB_ACTION_PATH" ] || die "GITHUB_ACTION_PATH is not a directory"
}

configure_clients() {
  local oss_endpoint="${ALIYUN_OSS_ENDPOINT#https://}"
  ossutil_bin="$RUNNER_TEMP/chat2db-cdn-tools/ossutil"
  aliyun_bin="$RUNNER_TEMP/chat2db-cdn-tools/aliyun"
  if [ ! -x "$ossutil_bin" ] || [ ! -x "$aliyun_bin" ]; then
    die "pinned Alibaba Cloud clients were not installed"
  fi

  oss_config="$RUNNER_TEMP/chat2db-ossutil.config"
  umask 077
  {
    echo '[Credentials]'
    echo 'language=EN'
    printf 'endpoint=%s\n' "$oss_endpoint"
    printf 'accessKeyID=%s\n' "$ALIYUN_OSS_ACCESS_KEY_ID"
    printf 'accessKeySecret=%s\n' "$ALIYUN_OSS_ACCESS_KEY_SECRET"
  } > "$oss_config"

  export ALIBABA_CLOUD_ACCESS_KEY_ID="$ALIYUN_OSS_ACCESS_KEY_ID"
  export ALIBABA_CLOUD_ACCESS_KEY_SECRET="$ALIYUN_OSS_ACCESS_KEY_SECRET"
  export ALIBABA_CLOUD_IGNORE_PROFILE=TRUE
  export ALIBABA_CLOUD_REGION_ID=cn-hangzhou
}

main() {
  local version state
  command -v cmp >/dev/null
  command -v curl >/dev/null
  command -v diff >/dev/null
  command -v gh >/dev/null
  command -v python3 >/dev/null
  command -v realpath >/dev/null
  command -v sha256sum >/dev/null
  validate_environment

  plan_directory="$RUNNER_TEMP/chat2db-cdn-plan"
  snapshot_file="$plan_directory/promotion-snapshot.tsv"
  rm -rf -- "$plan_directory"
  mkdir -p -- "$plan_directory"
  trap cleanup_and_maybe_rollback EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  version="$(python3 "$GITHUB_ACTION_PATH/build-plan.py" \
    --artifact-directory "$PUBLISH_ARTIFACT_DIRECTORY" \
    --cdn-base-url "$OFFICIAL_CDN_BASE_URL" \
    --output-directory "$plan_directory")"
  [ -n "$version" ] || die "publishing plan did not return a version"
  validate_plan
  configure_clients

  echo "Staging complete Chat2DB CLI and Skill release $version"
  stage_immutable_objects
  verify_plan "$plan_directory/immutable-verify.tsv" \
    || die "immutable CDN verification failed"

  if [ "$PUBLISH_MODE" = cdn-only ]; then
    echo "Published and verified immutable Chat2DB CDN objects for $version"
    return 0
  fi

  if ! state="$(github_release_state)"; then
    die "cannot determine GitHub publication state before CDN promotion"
  fi
  prepare_durable_journal "$version"

  if [ "$state" = public ]; then
    publication_committed=true
    verify_github_release_assets \
      || die "public GitHub Release assets do not match the canonical bundle"
    promotion_active=true
    apply_promotion_uploads || die "public Release CDN reconciliation failed"
    refresh_plan "$plan_directory/promotion-refresh.txt" \
      || die "public Release CDN refresh failed"
    verify_plan "$plan_directory/promotion-verify.tsv" \
      || die "public Release CDN verification failed"
    promotion_complete=true
    promotion_active=false
    echo "Reconciled complete Chat2DB CDN promotion with public GitHub release $version"
    return 0
  fi

  if [ "$journal_reused" = true ]; then
    promotion_active=true
    rollback_promotion || die "durable promotion recovery failed"
    promotion_active=false
  fi
  python3 "$GITHUB_ACTION_PATH/validate-latest.py" \
    --snapshot "$snapshot_file" \
    --release-version "$version" \
    || die "current latest manifests failed the promotion gate"
  promotion_active=true
  apply_promotion_uploads || die "promotion upload failed"
  refresh_plan "$plan_directory/promotion-refresh.txt" || die "promotion refresh failed"
  verify_plan "$plan_directory/promotion-verify.tsv" || die "promotion verification failed"
  verify_and_publish_github_release || die "GitHub Release publication failed"
  promotion_complete=true
  promotion_active=false
  echo "Published and verified complete Chat2DB CDN and GitHub release $version"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
