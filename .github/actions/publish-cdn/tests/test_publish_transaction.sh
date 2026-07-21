#!/usr/bin/env bash
# shellcheck disable=SC2016 # Assertions intentionally match literal shell source.

set -euo pipefail

action_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repository_root="$(cd "$action_root/../../.." && pwd)"
publish="$action_root/publish.sh"
planner="$action_root/build-plan.py"
action="$action_root/action.yml"
release_workflow="$repository_root/.github/workflows/release.yml"
quality_workflow="$repository_root/.github/workflows/quality.yml"

fail() {
  echo "error: $*" >&2
  exit 1
}

assert_contains() {
  local file="$1" text="$2"
  grep -Fq -- "$text" "$file" || fail "$file does not contain: $text"
}

assert_not_contains() {
  local file="$1" text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file unexpectedly contains: $text"
  fi
}

assert_order() {
  local file="$1"
  shift
  local text line previous=0
  for text in "$@"; do
    line="$(grep -nF -- "$text" "$file" | tail -n 1 | cut -d: -f1)"
    [ -n "$line" ] || fail "$file does not contain ordered step: $text"
    [ "$line" -gt "$previous" ] || fail "steps are out of order at: $text"
    previous="$line"
  done
}

bash -n "$publish"
python3 -m py_compile "$planner"

# Immutable writes are create-only, accept races only after readback, and are
# CDN-verified before any mutable promotion starts.
assert_contains "$publish" "--header 'x-oss-forbid-overwrite:true'"
assert_contains "$publish" 'put_immutable_create_only "$source" "$destination"'
assert_contains "$publish" 'readback_object "$destination" "$expected_sha" "$readback"'
assert_contains "$publish" 'validate_upload_plan "$plan_directory/immutable-uploads.tsv" 19'
assert_contains "$publish" 'validate_verify_plan "$plan_directory/immutable-verify.tsv" 19'
assert_order "$publish" \
  '  stage_immutable_objects' \
  '  verify_plan "$plan_directory/immutable-verify.tsv"' \
  '  if [ "$PUBLISH_MODE" = cdn-only ]; then' \
  '  prepare_durable_journal "$version"' \
  '  python3 "$GITHUB_ACTION_PATH/validate-latest.py"' \
  '  promotion_active=true' \
  '  apply_promotion_uploads' \
  '  refresh_plan "$plan_directory/promotion-refresh.txt"' \
  '  verify_plan "$plan_directory/promotion-verify.tsv"' \
  '  verify_and_publish_github_release' \
  '  promotion_complete=true'

# The EXIT trap remains armed for every promotion write/refresh/verify failure.
assert_contains "$publish" 'trap cleanup_and_maybe_rollback EXIT'
assert_contains "$publish" 'if [ "$promotion_active" = true ] && [ "$promotion_complete" != true ]; then'
assert_contains "$publish" 'rollback_promotion'
assert_contains "$publish" 'release-journal/chat2db-cli/$version/promotion-snapshot.tsv'
assert_contains "$publish" 'ensure_create_only_object "$durable_file" "$journal_object" "$journal_sha"'
assert_contains "$publish" 'Loaded durable promotion journal $journal_object'

# Rollback has both branches required to restore the exact previous mutable
# state, then refreshes and verifies that restored state through the CDN.
assert_contains "$publish" 'cp -f "$backup" "$(remote_uri "$destination")"'
assert_contains "$publish" 'rm -f "$(remote_uri "$destination")"'
assert_contains "$publish" 'refresh_and_wait "$url" || rollback_failed=1'
assert_contains "$publish" 'verify_url "$url" "$old_sha" "$destination" || rollback_failed=1'
assert_contains "$publish" 'verify_url_absent "$url" || rollback_failed=1'

# Draft asset verification and GitHub publication remain inside the same EXIT
# trap as mutable CDN promotion. Any pre-publication failure restores aliases.
assert_contains "$publish" 'gh release download "$RELEASE_TAG" --repo "$GH_REPO"'
assert_contains "$publish" 'gh release edit "$RELEASE_TAG" --repo "$GH_REPO" --draft=false --latest'
assert_contains "$publish" 'cmp "$PUBLISH_ARTIFACT_DIRECTORY/$asset_name" "$github_assets/$asset_name"'
assert_contains "$publish" 'verify_and_publish_github_release || die "GitHub Release publication failed"'
assert_contains "$publish" 'Release became public; preserving CDN promotion for durable reconciliation'
assert_contains "$release_workflow" 'gh release delete-asset "$RELEASE_TAG" "$asset_name" --yes'
assert_contains "$release_workflow" 'GH_TOKEN: ${{ github.token }}'
assert_not_contains "$release_workflow" '  publish-release:'

# The action consumes one assembled bundle and cannot reach back into checkout
# scripts or split CLI and Skill into separate transactions.
assert_not_contains "$planner" 'GITHUB_WORKSPACE'
assert_not_contains "$planner" 'repository / "scripts"'
assert_not_contains "$action" 'product:'
assert_contains "$action" 'PUBLISH_ARTIFACT_DIRECTORY: ${{ inputs.artifact-directory }}'
assert_contains "$action" 'PUBLISH_MODE: ${{ inputs.mode }}'
assert_contains "$release_workflow" 'mode: cdn-only'
assert_contains "$release_workflow" 'mode: release'
assert_contains "$release_workflow" "if: \${{ needs.resolve.outputs.mode == 'cdn-only' }}"
assert_contains "$release_workflow" "if: \${{ needs.resolve.outputs.mode == 'release' }}"
assert_contains "$release_workflow" 'FEISHU_WEBHOOK_URL: ${{ secrets.FEISHU_RELEASE_NOTIFY_WEBHOOK }}'
assert_contains "$release_workflow" 'CHAT2DB_CDN_BASE_URL: https://cdn.chat2db-ai.com'
assert_not_contains "$release_workflow" '  push:'
assert_contains "$release_workflow" 'run-name: Chat2DB CLI ${{ inputs.mode }} v${{ inputs.version }}'
assert_contains "$release_workflow" 'SOURCE_REF: ${{ github.ref }}'
assert_contains "$release_workflow" 'if [ "$SOURCE_REF" != refs/heads/main ]; then'
assert_contains "$release_workflow" 'git show-ref --verify --quiet "refs/tags/$RELEASE_TAG"'
assert_contains "$release_workflow" 'test "$(git rev-list -n 1 "$RELEASE_TAG")" = "$GITHUB_SHA"'
assert_contains "$release_workflow" 'gh api "repos/$GH_REPO/git/tags/$tag_object_sha"'
assert_contains "$release_workflow" 'python3 .github/scripts/validate-release-tag.py'
assert_contains "$release_workflow" "gh release list \\"
assert_contains "$release_workflow" '> "$existing_tags_file"'
assert_contains "$release_workflow" 'mapfile -t existing_tags < "$existing_tags_file"'
assert_not_contains "$release_workflow" 'mapfile -t existing_tags < <('
assert_contains "$release_workflow" "if: \${{ always() && needs.validate.result == 'success' }}"
assert_contains "$release_workflow" 'ref: ${{ github.sha }}'
assert_not_contains "$release_workflow" '${{ vars.CHAT2DB_CDN_BASE_URL }}'
assert_not_contains "$quality_workflow" 'gitleaks/gitleaks-action@'
assert_contains "$quality_workflow" 'GITLEAKS_VERSION: 8.30.1'
assert_contains "$quality_workflow" 'GITLEAKS_SHA256: 551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb'
assert_contains "$quality_workflow" 'gitleaks git --redact --verbose --no-banner --log-opts=--all .'
[ "$(grep -c '^    continue-on-error: true$' "$release_workflow")" -eq 2 ] \
  || fail "both Feishu notification jobs must be non-blocking"

# Exercise exact draft-asset matching and GitHub publication failure handling
# without network or provider credentials.
# shellcheck source=.github/actions/publish-cdn/publish.sh
source "$publish"
transaction_test_root="$(mktemp -d)"
trap 'rm -rf -- "$transaction_test_root"' EXIT
PUBLISH_ARTIFACT_DIRECTORY="$transaction_test_root/release-assets"
plan_directory="$transaction_test_root/plan"
snapshot_file="$plan_directory/promotion-snapshot.tsv"
RELEASE_TAG="v0.2.0"
GH_REPO="OtterMind/Chat2DB-CLI"
mkdir -p "$PUBLISH_ARTIFACT_DIRECTORY" "$plan_directory"
printf 'artifact\n' > "$PUBLISH_ARTIFACT_DIRECTORY/chat2db-test.tar.gz"
printf 'provenance\n' > "$PUBLISH_ARTIFACT_DIRECTORY/chat2db-cli-0.2.0.intoto.jsonl"
(
  cd "$PUBLISH_ARTIFACT_DIRECTORY"
  sha256sum chat2db-test.tar.gz > SHA256SUMS
)

# CDN-only mode must validate without GitHub publication credentials, while the
# release mode must refuse to start without them.
PUBLISH_MODE="cdn-only"
ALIYUN_OSS_ACCESS_KEY_ID="test-key"
ALIYUN_OSS_ACCESS_KEY_SECRET="test-secret"
ALIYUN_OSS_ENDPOINT="https://oss-cn-hangzhou.aliyuncs.com"
ALIYUN_OSS_BUCKET="chat2db-test"
GITHUB_ACTION_PATH="$action_root"
RUNNER_TEMP="$transaction_test_root/runner"
unset GH_REPO GH_TOKEN RELEASE_TAG
validate_environment
if (PUBLISH_MODE="release"; validate_environment) >/dev/null 2>&1; then
  fail "release mode accepted missing GitHub publication credentials"
fi
PUBLISH_MODE="release"
GH_REPO="OtterMind/Chat2DB-CLI"
export GH_TOKEN="test-token"
RELEASE_TAG="v0.2.0"

# Durable journals must describe every promotion destination exactly once and
# bind each present entry to the version-specific create-only backup key.
mkdir -p "$plan_directory"
: > "$plan_directory/promotion-uploads.tsv"
journal_fixture="$transaction_test_root/promotion-journal.tsv"
: > "$journal_fixture"
for index in 1 2 3 4 5 6 7 8 9; do
  destination="latest/object-$index"
  printf '/tmp/source\t%s\t%s\n' "$destination" "$(printf '0%.0s' {1..64})" \
    >> "$plan_directory/promotion-uploads.tsv"
  if [ "$index" -eq 9 ]; then
    printf '%s\tabsent\t-\t-\n' "$destination" >> "$journal_fixture"
  else
    backup_object="$(journal_backup_object 0.2.0 "$destination")"
    printf '%s\tpresent\t%s\t%s\n' \
      "$destination" "$backup_object" "$(printf '1%.0s' {1..64})" >> "$journal_fixture"
  fi
done
validate_durable_journal "$journal_fixture" 0.2.0
cp "$journal_fixture" "$transaction_test_root/invalid-journal.tsv"
sed -i.bak '1s/[0-9a-f]\{64\}$/invalid/' "$transaction_test_root/invalid-journal.tsv"
if (validate_durable_journal "$transaction_test_root/invalid-journal.tsv" 0.2.0) >/dev/null 2>&1; then
  fail "durable journal accepted an invalid backup digest"
fi

# Reconstruct the local rollback snapshot from a durable journal and verified
# backup object, which is the path used after a runner crash.
recovery_journal="$transaction_test_root/recovery-journal.tsv"
backup_payload="$transaction_test_root/old-alias"
printf 'old alias\n' > "$backup_payload"
backup_sha="$(local_sha256 "$backup_payload")"
: > "$recovery_journal"
for index in 1 2 3 4 5 6 7 8 9; do
  destination="latest/object-$index"
  if [ "$index" -eq 1 ]; then
    recovery_backup_object="$(journal_backup_object 0.2.0 "$destination")"
    printf '%s\tpresent\t%s\t%s\n' \
      "$destination" "$recovery_backup_object" "$backup_sha" >> "$recovery_journal"
  else
    printf '%s\tabsent\t-\t-\n' "$destination" >> "$recovery_journal"
  fi
done
download_object() {
  case "$1" in
    "$journal_object") cp "$recovery_journal" "$2" ;;
    "$recovery_backup_object") cp "$backup_payload" "$2" ;;
    *) return 1 ;;
  esac
}
remote_state() {
  [ "$1" = "release-journal/chat2db-cli/0.2.0/promotion-snapshot.tsv" ]
}
journal_reused=false
prepare_durable_journal 0.2.0
[ "$journal_reused" = true ] || fail "existing durable journal was not selected for recovery"
[ "$(wc -l < "$snapshot_file" | tr -d '[:space:]')" -eq 9 ] \
  || fail "durable journal did not reconstruct all promotion entries"
loaded_backup="$(awk -F '\t' 'NR == 1 { print $3 }' "$snapshot_file")"
cmp "$backup_payload" "$loaded_backup" \
  || fail "durable journal did not restore the verified backup payload"

mock_github_mode="stale"
mock_release_state="draft"
gh() {
  case "$1:$2" in
    release:view)
      [ "$mock_release_state" = draft ] && printf 'true\n' || printf 'false\n'
      ;;
    release:download)
      cp "$PUBLISH_ARTIFACT_DIRECTORY"/* "$plan_directory/github-assets/"
      if [ "$mock_github_mode" = stale ]; then
        printf 'stale\n' > "$plan_directory/github-assets/old-asset.txt"
      elif [ "$mock_github_mode" = tampered-provenance ]; then
        printf 'tampered\n' > "$plan_directory/github-assets/chat2db-cli-0.2.0.intoto.jsonl"
      fi
      ;;
    release:edit)
      if [ "$mock_github_mode" = success ]; then
        mock_release_state="public"
        return 0
      fi
      if [ "$mock_github_mode" = applied-error ]; then
        mock_release_state="public"
      fi
      return 1
      ;;
    *) return 1 ;;
  esac
}

if verify_and_publish_github_release >/dev/null 2>&1; then
  fail "stale draft asset was accepted"
fi
mock_github_mode="tampered-provenance"
if verify_and_publish_github_release >/dev/null 2>&1; then
  fail "tampered draft provenance was accepted"
fi
mock_github_mode="publish-failure"
if verify_and_publish_github_release >/dev/null 2>&1; then
  fail "GitHub publication failure was accepted while the Release remained draft"
fi
mock_github_mode="success"
publication_committed=false
verify_and_publish_github_release \
  || fail "valid draft assets could not be published"
[ "$publication_committed" = true ] || fail "successful publication was not committed"

# A failed client response after GitHub applied the publication must resolve to
# public and commit the CDN promotion instead of rolling it back.
mock_github_mode="applied-error"
mock_release_state="draft"
publication_committed=false
verify_and_publish_github_release \
  || fail "public Release was not reconciled after a lost publication response"
[ "$publication_committed" = true ] || fail "lost response did not commit public Release state"

# The EXIT handler rolls back only while GitHub still reports a Draft. Public or
# unknown state preserves the aliases for the next durable reconciliation run.
rollback_marker="$transaction_test_root/rollback-called"
mock_cleanup_state="public"
github_release_state() {
  [ "$mock_cleanup_state" = unknown ] && return 1
  printf '%s\n' "$mock_cleanup_state"
}
rollback_promotion() {
  : > "$rollback_marker"
  return 0
}
promotion_active=true
promotion_complete=false
publication_committed=false
publication_attempted=false
oss_config=""
rm -f "$rollback_marker"
(cleanup_and_maybe_rollback)
[ ! -e "$rollback_marker" ] || fail "public Release triggered CDN rollback"

mock_cleanup_state="draft"
rm -f "$rollback_marker"
(cleanup_and_maybe_rollback)
[ -e "$rollback_marker" ] || fail "Draft Release did not trigger CDN rollback"

mock_cleanup_state="draft"
publication_attempted=true
rm -f "$rollback_marker"
if (cleanup_and_maybe_rollback) >/dev/null 2>&1; then
  fail "uncertain publication attempt was treated as successful cleanup"
fi
[ ! -e "$rollback_marker" ] \
  || fail "uncertain publication attempt triggered an unsafe CDN rollback"

mock_cleanup_state="unknown"
publication_attempted=false
rm -f "$rollback_marker"
if (cleanup_and_maybe_rollback) >/dev/null 2>&1; then
  fail "unknown GitHub state was treated as a successful cleanup"
fi
[ ! -e "$rollback_marker" ] || fail "unknown GitHub state triggered unsafe CDN rollback"

echo "publish transaction structure verified without OSS access"
