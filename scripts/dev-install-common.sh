#!/usr/bin/env bash
# Shared safety primitives for the Bash development installers.

DEV_LOCK_TOKEN=""
DEV_LOCK_CANDIDATE=""
DEV_LOCK_GUARD=""
DEV_LOCK_GUARD_CANDIDATE=""
DEV_LOCK_GUARD_TOKEN=""
DEV_LOCK_GUARD_HELD=0

dev_sha_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "error: shasum or sha256sum is required" >&2
    return 1
  fi
}

dev_sha_stream() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  else
    sha256sum | awk '{print $1}'
  fi
}

dev_directory_hash() {
  (
    cd "$1" || exit 1
    find . -type f 2>/dev/null \
      | sed 's#^\./##' \
      | LC_ALL=C sort \
      | while IFS= read -r file; do
          printf '%s  %s\n' "$(dev_sha_file "$file")" "$file"
        done \
      | dev_sha_stream
  )
}

dev_path_hash() {
  if [ -f "$1" ] && [ ! -L "$1" ]; then
    printf 'file:%s' "$(dev_sha_file "$1")"
  elif [ -d "$1" ] && [ ! -L "$1" ]; then
    printf 'dir:%s' "$(dev_directory_hash "$1")"
  else
    return 1
  fi
}

dev_metadata_value() {
  local file="$1" key="$2"
  sed -n "s/^${key}=//p" "$file" | head -n 1
}

dev_path_has_ancestor() {
  local ancestor="$2" current="$1"
  [ -e "$ancestor" ] || [ -L "$ancestor" ] || return 1
  while :; do
    if { [ -e "$current" ] || [ -L "$current" ]; } && [ "$current" -ef "$ancestor" ]; then
      return 0
    fi
    [ "$current" = "/" ] && break
    current="$(dirname "$current")"
  done
  return 1
}

dev_paths_overlap() {
  dev_path_has_ancestor "$1" "$2" || dev_path_has_ancestor "$2" "$1"
}

dev_path_has_git_ancestor() {
  local current="$1"
  while :; do
    if [ -e "$current/.git" ] || [ -L "$current/.git" ]; then
      return 0
    fi
    [ "$current" = "/" ] && break
    current="$(dirname "$current")"
  done
  return 1
}

dev_absolute_input_path() {
  local raw="$1"
  case "$raw" in
    /*) printf '%s' "$raw" ;;
    [A-Za-z]:[\\/]*)
      if ! command -v cygpath >/dev/null 2>&1; then
        echo "error: Windows drive paths require cygpath" >&2
        return 1
      fi
      cygpath -u "$raw"
      ;;
    *) printf '%s/%s' "$PWD" "$raw" ;;
  esac
}

dev_valid_provenance_header() {
  local file="$1" artifact="$2" target="$3" expected_version="${4:-}" recorded_hash recorded_version
  [ -f "$file" ] && [ ! -L "$file" ] || return 1
  [ "$(wc -l < "$file" | tr -d '[:space:]')" = "5" ] || return 1
  [ "$(dev_metadata_value "$file" owner)" = "chat2db-dev-install" ] || return 1
  [ "$(dev_metadata_value "$file" artifact)" = "$artifact" ] || return 1
  [ "$(dev_metadata_value "$file" target)" = "$target" ] || return 1
  recorded_hash="$(dev_metadata_value "$file" sha256)"
  case "$recorded_hash" in ''|*[!0-9a-f]*) return 1 ;; esac
  [ "${#recorded_hash}" -eq 64 ] || return 1
  recorded_version="$(dev_metadata_value "$file" version)"
  [ -n "$recorded_version" ] || return 1
  [ -z "$expected_version" ] || [ "$recorded_version" = "$expected_version" ]
}

dev_write_provenance() {
  local output="$1" artifact="$2" target="$3" hash="$4" version="$5"
  cat > "$output" <<EOF
owner=chat2db-dev-install
artifact=$artifact
target=$target
sha256=$hash
version=$version
EOF
}

dev_lock_snapshot_value() {
  local snapshot="$1" key="$2"
  printf '%s\n' "$snapshot" | sed -n "s/^${key}=//p" | head -n 1
}

dev_valid_lock_snapshot() {
  local snapshot="$1" owner pid token
  [ "$(printf '%s\n' "$snapshot" | wc -l | tr -d '[:space:]')" = "3" ] || return 1
  owner="$(dev_lock_snapshot_value "$snapshot" owner)"
  pid="$(dev_lock_snapshot_value "$snapshot" pid)"
  token="$(dev_lock_snapshot_value "$snapshot" token)"
  [ "$owner" = "chat2db-dev-install" ] || return 1
  case "$pid" in ''|*[!0-9]*) return 1 ;; esac
  [ -n "$token" ]
}

dev_acquire_lock() {
  local lock_file="$1" label="$2" attempts=0 max_attempts sleep_seconds
  local snapshot owner_pid acquired=0 guard_incomplete_attempts=0 guard_grace_attempts
  max_attempts="${CHAT2DB_DEV_INSTALL_LOCK_ATTEMPTS:-600}"
  sleep_seconds="${CHAT2DB_DEV_INSTALL_LOCK_SLEEP:-0.1}"
  guard_grace_attempts="${CHAT2DB_DEV_INSTALL_GUARD_GRACE_ATTEMPTS:-20}"
  case "$max_attempts" in ''|*[!0-9]*) echo "error: invalid lock attempt count" >&2; return 1 ;; esac
  case "$guard_grace_attempts" in ''|*[!0-9]*) echo "error: invalid guard grace count" >&2; return 1 ;; esac

  DEV_LOCK_TOKEN="$label:$$:${RANDOM:-0}:$(date +%s)"
  DEV_LOCK_CANDIDATE="$lock_file.candidate.$$.$label"
  DEV_LOCK_GUARD="$lock_file.guard"
  DEV_LOCK_GUARD_TOKEN="guard:$label:$$:${RANDOM:-0}:$(date +%s)"
  DEV_LOCK_GUARD_CANDIDATE="$DEV_LOCK_GUARD.candidate.$$.$label"
  umask 077
  printf 'owner=chat2db-dev-install\npid=%s\ntoken=%s\n' "$$" "$DEV_LOCK_TOKEN" > "$DEV_LOCK_CANDIDATE"
  printf 'owner=chat2db-dev-install\npid=%s\ntoken=%s\n' "$$" "$DEV_LOCK_GUARD_TOKEN" > "$DEV_LOCK_GUARD_CANDIDATE"
  while [ "$acquired" = "0" ]; do
    if mkdir "$DEV_LOCK_GUARD" 2>/dev/null; then
      DEV_LOCK_GUARD_HELD=1
      if [ "${CHAT2DB_TEST_CRASH_AT:-}" = "lock-guard-acquired" ]; then
        kill -KILL "$$"
      fi
      ln "$DEV_LOCK_GUARD_CANDIDATE" "$DEV_LOCK_GUARD/metadata"
      if [ "${CHAT2DB_TEST_CRASH_AT:-}" = "lock-guard-published" ]; then
        kill -KILL "$$"
      fi
      if ln "$DEV_LOCK_CANDIDATE" "$lock_file" 2>/dev/null; then
        acquired=1
      elif [ -f "$lock_file" ] && [ ! -L "$lock_file" ]; then
        snapshot="$(cat "$lock_file" 2>/dev/null || true)"
        if dev_valid_lock_snapshot "$snapshot"; then
          owner_pid="$(dev_lock_snapshot_value "$snapshot" pid)"
          if ! kill -0 "$owner_pid" 2>/dev/null; then
            rm -f "$lock_file"
            if ln "$DEV_LOCK_CANDIDATE" "$lock_file" 2>/dev/null; then
              acquired=1
            fi
          fi
        fi
      fi
      rm -f "$DEV_LOCK_GUARD/metadata"
      rmdir "$DEV_LOCK_GUARD"
      DEV_LOCK_GUARD_HELD=0
      guard_incomplete_attempts=0
    elif [ -d "$DEV_LOCK_GUARD" ] && [ ! -L "$DEV_LOCK_GUARD" ]; then
      if [ -f "$DEV_LOCK_GUARD/metadata" ] && [ ! -L "$DEV_LOCK_GUARD/metadata" ]; then
        snapshot="$(cat "$DEV_LOCK_GUARD/metadata" 2>/dev/null || true)"
        if dev_valid_lock_snapshot "$snapshot"; then
          owner_pid="$(dev_lock_snapshot_value "$snapshot" pid)"
          if ! kill -0 "$owner_pid" 2>/dev/null && \
             [ "$(dev_metadata_value "$DEV_LOCK_GUARD/metadata" token 2>/dev/null || true)" = "$(dev_lock_snapshot_value "$snapshot" token)" ]; then
            rm -f "$DEV_LOCK_GUARD/metadata"
            rmdir "$DEV_LOCK_GUARD" 2>/dev/null || true
            guard_incomplete_attempts=0
            continue
          fi
        fi
        guard_incomplete_attempts=0
      elif [ -z "$(find "$DEV_LOCK_GUARD" -mindepth 1 -print -quit 2>/dev/null)" ]; then
        guard_incomplete_attempts=$((guard_incomplete_attempts + 1))
        if [ "$guard_incomplete_attempts" -ge "$guard_grace_attempts" ] && rmdir "$DEV_LOCK_GUARD" 2>/dev/null; then
          guard_incomplete_attempts=0
          continue
        fi
      else
        guard_incomplete_attempts=0
      fi
    fi
    [ "$acquired" = "1" ] && break
    attempts=$((attempts + 1))
    if [ "$attempts" -ge "$max_attempts" ]; then
      rm -f "$DEV_LOCK_CANDIDATE"
      rm -f "$DEV_LOCK_GUARD_CANDIDATE"
      DEV_LOCK_CANDIDATE=""
      DEV_LOCK_GUARD_CANDIDATE=""
      echo "error: timed out waiting for $label installation lock: $lock_file" >&2
      return 1
    fi
    sleep "$sleep_seconds"
  done
  rm -f "$DEV_LOCK_CANDIDATE"
  rm -f "$DEV_LOCK_GUARD_CANDIDATE"
  DEV_LOCK_CANDIDATE=""
  DEV_LOCK_GUARD_CANDIDATE=""
  if [ "${CHAT2DB_TEST_HOLD_LOCK:-}" = "$label" ] && [ -n "${CHAT2DB_TEST_HOLD_LOCK_SECONDS:-}" ]; then
    sleep "$CHAT2DB_TEST_HOLD_LOCK_SECONDS"
  fi
}

dev_release_lock_guard() {
  if [ "$DEV_LOCK_GUARD_HELD" = "1" ] && [ -n "$DEV_LOCK_GUARD" ]; then
    if [ -f "$DEV_LOCK_GUARD/metadata" ] && [ ! -L "$DEV_LOCK_GUARD/metadata" ] && \
       [ "$(dev_metadata_value "$DEV_LOCK_GUARD/metadata" token 2>/dev/null || true)" = "$DEV_LOCK_GUARD_TOKEN" ]; then
      rm -f "$DEV_LOCK_GUARD/metadata"
    fi
    rmdir "$DEV_LOCK_GUARD" 2>/dev/null || true
    DEV_LOCK_GUARD_HELD=0
  fi
}

dev_release_lock() {
  local lock_file="$1" token="$2" current_token
  [ -n "$token" ] || return 0
  if [ -f "$lock_file" ] && [ ! -L "$lock_file" ]; then
    current_token="$(dev_metadata_value "$lock_file" token 2>/dev/null || true)"
    if [ "$current_token" = "$token" ]; then
      rm -f "$lock_file"
    fi
  fi
}

dev_copy_path() {
  if [ -d "$1" ] && [ ! -L "$1" ]; then
    cp -R "$1" "$2"
  else
    cp -p "$1" "$2"
  fi
}

dev_prepare_restore_workspace() {
  local workspace="$1" journal="$2" build_path
  if [ -e "$workspace" ] || [ -L "$workspace" ]; then
    if ! { [ -d "$workspace" ] && [ ! -L "$workspace" ] && \
      [ -f "$workspace/metadata" ] && [ ! -L "$workspace/metadata" ] && \
      [ "$(dev_metadata_value "$workspace/metadata" owner)" = "chat2db-dev-install" ] && \
      [ "$(dev_metadata_value "$workspace/metadata" journal)" = "$journal" ]; }; then
      echo "error: unrecognized recovery workspace; refusing to remove it: $workspace" >&2
      return 1
    fi
    return 0
  fi

  build_path="$workspace.build.$$"
  rm -rf "$build_path"
  mkdir "$build_path"
  printf 'owner=chat2db-dev-install\njournal=%s\n' "$journal" > "$build_path/metadata"
  if ! mv "$build_path" "$workspace"; then
    rm -rf "$build_path"
    return 1
  fi
}

dev_remove_restore_workspace() {
  local workspace="$1" journal="$2" retired
  [ -e "$workspace" ] || [ -L "$workspace" ] || return 0
  dev_prepare_restore_workspace "$workspace" "$journal" || return 1
  retired="$workspace.retired.$$.$(date +%s).${RANDOM:-0}"
  if [ -e "$retired" ] || [ -L "$retired" ]; then
    echo "error: recovery workspace retirement path already exists: $retired" >&2
    return 1
  fi
  mv "$workspace" "$retired"
  if [ "${CHAT2DB_TEST_CRASH_AT:-}" = "restore-workspace-retired" ]; then
    kill -KILL "$$"
  fi
  if ! rm -rf "$retired"; then
    echo "warn: retired recovery workspace remains at $retired" >&2
  fi
}

dev_restore_component() {
  local target="$1" state="$2" backup="$3" original_hash="$4" expected_hash="$5"
  local label="$6" journal="$7" workspace="$8" component="$9" current_hash restore_path quarantine_path
  dev_prepare_restore_workspace "$workspace" "$journal" || return 1
  restore_path="$workspace/$component.pending"
  quarantine_path="$workspace/$component.quarantine"

  case "$state" in
    empty|present) ;;
    *) echo "error: invalid $label transaction state at $journal" >&2; return 1 ;;
  esac

  if [ "$state" = "present" ]; then
    if [ ! -e "$backup" ] || [ -L "$backup" ]; then
      echo "error: $label recovery backup is missing at $journal" >&2
      return 1
    fi
    [ "$(dev_path_hash "$backup")" = "$original_hash" ] || {
      echo "error: $label recovery backup hash mismatch at $journal" >&2
      return 1
    }
  fi

  if [ -e "$target" ] || [ -L "$target" ]; then
    current_hash="$(dev_path_hash "$target" 2>/dev/null || true)"
    if [ "$state" = "present" ] && [ "$current_hash" = "$original_hash" ]; then
      rm -rf "$restore_path" "$quarantine_path"
      return 0
    fi
    if [ "$current_hash" != "$expected_hash" ]; then
      echo "error: interrupted $label changed; recovery data preserved at $journal" >&2
      return 1
    fi
    if [ -e "$quarantine_path" ] || [ -L "$quarantine_path" ]; then
      echo "error: $label recovery quarantine already exists: $quarantine_path" >&2
      return 1
    fi
    if ! mv "$target" "$quarantine_path"; then
      echo "error: failed to quarantine $label; recovery data preserved at $journal" >&2
      return 1
    fi
    if [ "${CHAT2DB_TEST_CRASH_AT:-}" = "restore-component-quarantined" ]; then
      kill -KILL "$$"
    fi
  elif [ -e "$quarantine_path" ] || [ -L "$quarantine_path" ]; then
    if [ "$state" = "present" ] && \
       [ "$(dev_path_hash "$quarantine_path" 2>/dev/null || true)" != "$expected_hash" ]; then
      echo "error: $label recovery quarantine hash mismatch at $journal" >&2
      return 1
    fi
  fi

  case "$state" in
    empty)
      rm -rf "$restore_path" "$quarantine_path"
      ;;
    present)
      if [ -e "$restore_path" ] || [ -L "$restore_path" ]; then
        if [ "$(dev_path_hash "$restore_path" 2>/dev/null || true)" != "$original_hash" ]; then
          rm -rf "$restore_path"
        fi
      fi
      if [ ! -e "$restore_path" ] && [ ! -L "$restore_path" ]; then
        dev_copy_path "$backup" "$restore_path" || return 1
      fi
      if ! mv "$restore_path" "$target"; then
        echo "error: failed to restore $label; recovery data preserved at $journal" >&2
        return 1
      fi
      if [ "${CHAT2DB_TEST_CRASH_AT:-}" = "restore-component-activated" ]; then
        kill -KILL "$$"
      fi
      [ "$(dev_path_hash "$target")" = "$original_hash" ] || {
        echo "error: restored $label failed its integrity check; recovery data preserved at $journal" >&2
        return 1
      }
      rm -rf "$quarantine_path"
      ;;
  esac
}
