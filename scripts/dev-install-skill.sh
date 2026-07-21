#!/usr/bin/env bash
# Install the repository Skill bundle into a local agent client for development.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: dev-install-skill.sh [options]

Install the Skill from this checkout into a local agent client.

Options:
  --client <name>  Install for codex or claude-code (default: codex).
  --target <dir>   Override the client Skill target directory.
  --force          Replace an unowned target. Git and symlink targets remain protected.
  -h, --help       Show this help.

Environment:
  CHAT2DB_SKILL_DEV_CLIENT  Default client (codex or claude-code).
  CHAT2DB_SKILL_DEV_TARGET  Override the Skill target directory.
  CHAT2DB_SKILL_DEV_FORCE=1 Replace an unowned target.
EOF
}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=dev-install-common.sh
# shellcheck disable=SC1091
. "$REPO_ROOT/scripts/dev-install-common.sh"
SKILL_ROOT="$REPO_ROOT/skill"
CLIENT="${CHAT2DB_SKILL_DEV_CLIENT:-codex}"
TARGET_DIR="${CHAT2DB_SKILL_DEV_TARGET:-}"
FORCE="${CHAT2DB_SKILL_DEV_FORCE:-0}"
CHECK_ONLY=0
RECOVER_ONLY=0
PRINT_TARGET=0

while [ $# -gt 0 ]; do
  case "$1" in
    --client)
      [ $# -ge 2 ] || { echo "error: --client requires a value" >&2; exit 1; }
      CLIENT="$2"
      shift 2
      ;;
    --client=*) CLIENT="${1#--client=}"; shift ;;
    --target)
      [ $# -ge 2 ] || { echo "error: --target requires a value" >&2; exit 1; }
      TARGET_DIR="$2"
      shift 2
      ;;
    --target=*) TARGET_DIR="${1#--target=}"; shift ;;
    --force) FORCE=1; shift ;;
    --check-only) CHECK_ONLY=1; shift ;;
    --recover-only) RECOVER_ONLY=1; shift ;;
    --print-target) PRINT_TARGET=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

case "$CLIENT" in
  codex|claude-code) ;;
  *) echo "error: --client must be codex or claude-code" >&2; exit 1 ;;
esac
case "$FORCE" in
  0|1) ;;
  *) echo "error: CHAT2DB_SKILL_DEV_FORCE must be 0 or 1" >&2; exit 1 ;;
esac
if [ -z "${HOME:-}" ] && [ -z "$TARGET_DIR" ]; then
  echo "error: HOME is not set; pass --target to choose an install directory" >&2
  exit 1
fi
if [ -z "$TARGET_DIR" ]; then
  case "$CLIENT" in
    codex) TARGET_DIR="$HOME/.codex/skills/chat2db-cli" ;;
    claude-code) TARGET_DIR="$HOME/.claude/skills/chat2db-cli" ;;
  esac
fi

strip_trailing_slashes() {
  local path="$1"
  while [ "$path" != "/" ] && [ "${path%/}" != "$path" ]; do
    path="${path%/}"
  done
  printf '%s' "$path"
}

canonicalize_target() {
  local raw name parent physical_parent
  case "$1" in
    *$'\n'*) echo "error: target path must not contain a newline" >&2; return 1 ;;
  esac
  raw="$(strip_trailing_slashes "$1")"
  raw="$(dev_absolute_input_path "$raw")"
  name="$(basename "$raw")"
  case "$name" in
    ''|/|.|..) echo "error: unsafe target directory: $1" >&2; return 1 ;;
  esac
  if [ -L "$raw" ]; then
    echo "error: refusing symlink target: $raw" >&2
    return 1
  fi
  parent="$(dirname "$raw")"
  mkdir -p "$parent"
  physical_parent="$(cd -P "$parent" && pwd -P)"
  if [ "$physical_parent" = "/" ]; then
    printf '/%s' "$name"
  else
    printf '%s/%s' "$physical_parent" "$name"
  fi
}

TARGET_DIR="$(canonicalize_target "$TARGET_DIR")"
TARGET_PARENT="$(dirname "$TARGET_DIR")"
TARGET_NAME="$(basename "$TARGET_DIR")"
if [ "$TARGET_DIR" = "/" ]; then
  echo "error: unsafe target directory: $TARGET_DIR" >&2
  exit 1
fi
if [ -n "${HOME:-}" ] && [ -d "$HOME" ]; then
  HOME_CANONICAL="$(cd -P "$HOME" && pwd -P)"
  if { [ -e "$TARGET_DIR" ] || [ -L "$TARGET_DIR" ]; } && [ "$TARGET_DIR" -ef "$HOME_CANONICAL" ]; then
    echo "error: refusing to replace HOME: $TARGET_DIR" >&2
    exit 1
  fi
fi

git_managed_ancestor() {
  local nested_marker
  dev_path_has_git_ancestor "$TARGET_DIR" && return 0
  if [ -d "$TARGET_DIR" ]; then
    if ! nested_marker="$(find "$TARGET_DIR" -name .git -print -quit 2>/dev/null)"; then
      return 0
    fi
    [ -z "$nested_marker" ] || return 0
  fi
  return 1
}

if dev_paths_overlap "$TARGET_DIR" "$REPO_ROOT"; then
  echo "error: refusing a Skill target that overlaps the source checkout: $TARGET_DIR" >&2
  exit 1
fi
if git_managed_ancestor; then
  echo "error: refusing a Skill target inside a Git worktree: $TARGET_DIR" >&2
  exit 1
fi
if [ "$PRINT_TARGET" = "1" ]; then
  printf '%s\n' "$TARGET_DIR"
  exit 0
fi

PROVENANCE_FILE="$TARGET_PARENT/.${TARGET_NAME}.chat2db-dev-install.provenance"
LOCK_FILE="$TARGET_PARENT/.${TARGET_NAME}.chat2db-dev-install.lock"
TX_DIR="$TARGET_PARENT/.${TARGET_NAME}.chat2db-dev-install.tx"
TX_COMMITTED="$TX_DIR.committed"
TX_BUILD="$TX_DIR.build.$$"
LOCK_TOKEN=""
LOCK_OWNED=0
RETIRED_TRANSACTION=""
TX_ACTIVE=0
PRESERVE_TX=0

bundle_hash() {
  local directory="$1" scope="${2:-installed}"
  (
    cd "$directory"
    if [ "$scope" = "source" ]; then
      find SKILL.md references -type f 2>/dev/null
    else
      find . -type f 2>/dev/null | sed 's#^\./##'
    fi \
      | LC_ALL=C sort \
      | while IFS= read -r file; do
          printf '%s  %s\n' "$(dev_sha_file "$file")" "$file"
        done \
      | dev_sha_stream
  )
}

frontmatter_value() {
  local file="$1" key="$2"
  sed -n "s/^[[:space:]]*$key:[[:space:]]*[\"']\{0,1\}\([^\"']*\)[\"']\{0,1\}[[:space:]]*$/\\1/p" "$file" \
    | head -n 1 \
    | tr -d '[:space:]'
}

valid_provenance() {
  local recorded_hash
  dev_valid_provenance_header "$PROVENANCE_FILE" skill "$TARGET_DIR" "$CARGO_VERSION" || return 1
  recorded_hash="$(dev_metadata_value "$PROVENANCE_FILE" sha256)"
  [ -d "$TARGET_DIR" ] && [ ! -L "$TARGET_DIR" ] || return 1
  [ "$(bundle_hash "$TARGET_DIR")" = "$recorded_hash" ]
}

target_state() {
  local link
  if [ -L "$TARGET_DIR" ] || [ -L "$PROVENANCE_FILE" ]; then
    printf 'symlink'
  elif [ ! -e "$TARGET_DIR" ]; then
    if [ -e "$PROVENANCE_FILE" ]; then printf 'foreign'; else printf 'empty'; fi
  elif [ -e "$TARGET_DIR/.git" ] || [ -L "$TARGET_DIR/.git" ]; then
    printf 'git'
  elif [ ! -d "$TARGET_DIR" ]; then
    printf 'foreign'
  else
    link="$(find "$TARGET_DIR" -type l -print -quit)"
    if [ -n "$link" ]; then
      printf 'symlink'
    elif valid_provenance; then
      printf 'ours'
    else
      printf 'foreign'
    fi
  fi
}

acquire_lock() {
  local borrowed="${CHAT2DB_DEV_INSTALL_BORROWED_LOCK:-}" snapshot

  if [ -n "$borrowed" ]; then
    if [ ! -f "$LOCK_FILE" ] || [ -L "$LOCK_FILE" ]; then
      echo "error: parent Skill installation lock is missing: $LOCK_FILE" >&2
      return 1
    fi
    snapshot="$(cat "$LOCK_FILE")"
    dev_valid_lock_snapshot "$snapshot" || {
      echo "error: parent Skill installation lock is malformed: $LOCK_FILE" >&2
      return 1
    }
    [ "$(dev_lock_snapshot_value "$snapshot" token)" = "$borrowed" ] || {
      echo "error: parent Skill installation lock token mismatch" >&2
      return 1
    }
    [ "$(dev_lock_snapshot_value "$snapshot" pid)" = "$PPID" ] || {
      echo "error: parent Skill installation lock owner mismatch" >&2
      return 1
    }
    LOCK_TOKEN="$borrowed"
    return 0
  fi
  dev_acquire_lock "$LOCK_FILE" Skill
  LOCK_TOKEN="$DEV_LOCK_TOKEN"
  LOCK_OWNED=1
}

release_lock() {
  if [ "$LOCK_OWNED" = "1" ]; then
    dev_release_lock "$LOCK_FILE" "$LOCK_TOKEN"
  fi
  LOCK_OWNED=0
}

tx_value() {
  [ -f "$1/$2" ] && cat "$1/$2"
}

valid_transaction_header() {
  local directory="$1"
  [ -d "$directory" ] && [ ! -L "$directory" ] || return 1
  [ "$(tx_value "$directory" format)" = "chat2db-dev-skill-v2" ] || return 1
  [ "$(tx_value "$directory" target)" = "$TARGET_DIR" ] || return 1
  [ "$(tx_value "$directory" provenance)" = "$PROVENANCE_FILE" ]
}

retire_transaction() {
  local directory="$1"
  RETIRED_TRANSACTION="$directory.retired.$$.$(date +%s).${RANDOM:-0}"
  if [ -e "$RETIRED_TRANSACTION" ] || [ -L "$RETIRED_TRANSACTION" ]; then
    echo "error: transaction retirement path already exists: $RETIRED_TRANSACTION" >&2
    return 1
  fi
  mv "$directory" "$RETIRED_TRANSACTION"
}

remove_retired_transaction() {
  local retired="$1"
  if ! rm -rf "$retired"; then
    echo "warn: retired Skill journal remains at $retired" >&2
  fi
}

rollback_transaction() {
  valid_transaction_header "$TX_DIR" || {
    PRESERVE_TX=1
    echo "error: unrecognized Skill transaction; refusing to remove it: $TX_DIR" >&2
    return 1
  }
  if ! dev_restore_component \
      "$TARGET_DIR" \
      "$(tx_value "$TX_DIR" target-original-state)" \
      "$TX_DIR/target-backup" \
      "$(tx_value "$TX_DIR" target-original-hash)" \
      "dir:$(tx_value "$TX_DIR" expected-hash)" \
      "Skill target" "$TX_DIR" "$TX_DIR/recovery" target || \
     ! dev_restore_component \
      "$PROVENANCE_FILE" \
      "$(tx_value "$TX_DIR" provenance-original-state)" \
      "$TX_DIR/provenance-backup" \
      "$(tx_value "$TX_DIR" provenance-original-hash)" \
      "file:$(tx_value "$TX_DIR" expected-provenance-hash)" \
      "Skill provenance" "$TX_DIR" "$TX_DIR/recovery" provenance; then
    PRESERVE_TX=1
    return 1
  fi
  retire_transaction "$TX_DIR" || { PRESERVE_TX=1; return 1; }
  TX_ACTIVE=0
  test_hook skill-rollback-retired
  remove_retired_transaction "$RETIRED_TRANSACTION"
}

finalize_committed_transaction() {
  local expected_hash expected_provenance_hash
  valid_transaction_header "$TX_COMMITTED" || {
    PRESERVE_TX=1
    echo "error: unrecognized committed Skill transaction: $TX_COMMITTED" >&2
    return 1
  }
  expected_hash="$(tx_value "$TX_COMMITTED" expected-hash)"
  expected_provenance_hash="$(tx_value "$TX_COMMITTED" expected-provenance-hash)"
  if [ ! -d "$TARGET_DIR" ] || [ "$(bundle_hash "$TARGET_DIR")" != "$expected_hash" ] || \
     [ ! -f "$PROVENANCE_FILE" ] || [ "$(dev_sha_file "$PROVENANCE_FILE")" != "$expected_provenance_hash" ]; then
    PRESERVE_TX=1
    echo "error: committed Skill transaction does not match its targets: $TX_COMMITTED" >&2
    return 1
  fi
  retire_transaction "$TX_COMMITTED" || { PRESERVE_TX=1; return 1; }
  test_hook skill-committed-retired
  remove_retired_transaction "$RETIRED_TRANSACTION"
}

recover_transactions() {
  if [ -e "$TX_COMMITTED" ] || [ -L "$TX_COMMITTED" ]; then
    finalize_committed_transaction || return 1
  fi
  if [ -e "$TX_DIR" ] || [ -L "$TX_DIR" ]; then
    TX_ACTIVE=1
    rollback_transaction || return 1
    echo "recovered previous Skill: $TARGET_DIR"
  fi
}

test_hook() {
  local phase="$1"
  if [ "${CHAT2DB_TEST_CRASH_AT:-}" = "$phase" ]; then
    kill -KILL "$$"
  fi
  if [ "${CHAT2DB_TEST_FAIL_AT:-}" = "$phase" ]; then
    echo "error: injected development install failure at $phase" >&2
    return 73
  fi
  if [ "${CHAT2DB_TEST_INTERRUPT_AT:-}" = "$phase" ]; then
    kill -TERM "$$"
    sleep 1
  fi
}

cleanup() {
  local status=$?
  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] && [ "$TX_ACTIVE" = "1" ] && [ "$PRESERVE_TX" = "0" ]; then
    if ! rollback_transaction; then
      status=1
    fi
  fi
  rm -rf "$TX_BUILD"
  [ -n "$DEV_LOCK_CANDIDATE" ] && rm -f "$DEV_LOCK_CANDIDATE"
  [ -n "$DEV_LOCK_GUARD_CANDIDATE" ] && rm -f "$DEV_LOCK_GUARD_CANDIDATE"
  dev_release_lock_guard
  release_lock
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

acquire_lock
recover_transactions
if [ "$RECOVER_ONLY" = "1" ]; then
  exit 0
fi

if [ ! -f "$SKILL_ROOT/SKILL.md" ] || [ -L "$SKILL_ROOT/SKILL.md" ]; then
  echo "error: $SKILL_ROOT/SKILL.md is missing or is a symlink" >&2
  exit 1
fi
SOURCE_LINK="$(find "$SKILL_ROOT" -type l -print -quit)"
if [ -n "$SOURCE_LINK" ]; then
  echo "error: source Skill must not contain symlinks: $SOURCE_LINK" >&2
  exit 1
fi

CARGO_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n 1)"
SKILL_VERSION="$(frontmatter_value "$SKILL_ROOT/SKILL.md" version)"
SKILL_NAME="$(frontmatter_value "$SKILL_ROOT/SKILL.md" name)"
if [ -z "$CARGO_VERSION" ] || [ "$SKILL_NAME" != "chat2db-cli" ] || [ "$SKILL_VERSION" != "$CARGO_VERSION" ]; then
  echo "error: Cargo and Skill identities must match (CLI='$CARGO_VERSION', Skill='$SKILL_NAME@$SKILL_VERSION')" >&2
  exit 1
fi

STATE="$(target_state)"
case "$STATE" in
  empty|ours) ;;
  git)
    echo "error: refusing to overwrite git-managed target: $TARGET_DIR" >&2
    exit 1
    ;;
  symlink)
    echo "error: refusing symlink target or provenance: $TARGET_DIR" >&2
    exit 1
    ;;
  foreign)
    if [ "$FORCE" != "1" ]; then
      echo "error: target is not owned by the Chat2DB development installer: $TARGET_DIR; pass --force to replace it" >&2
      exit 1
    fi
    ;;
esac
if [ "$CHECK_ONLY" = "1" ]; then
  echo "Skill preflight passed: $TARGET_DIR"
  exit 0
fi

SOURCE_HASH="$(bundle_hash "$SKILL_ROOT" source)"
if [ "$STATE" = "ours" ] && [ "$SOURCE_HASH" = "$(bundle_hash "$TARGET_DIR")" ]; then
  echo "Skill is up to date: $TARGET_DIR ($SOURCE_HASH)"
  exit 0
fi

mkdir "$TX_BUILD"
printf '%s\n' 'chat2db-dev-skill-v2' > "$TX_BUILD/format"
printf '%s\n' "$TARGET_DIR" > "$TX_BUILD/target"
printf '%s\n' "$PROVENANCE_FILE" > "$TX_BUILD/provenance"
printf '%s\n' "$SOURCE_HASH" > "$TX_BUILD/expected-hash"
mkdir -p "$TX_BUILD/pending/references"
cp "$SKILL_ROOT/SKILL.md" "$TX_BUILD/pending/SKILL.md"
for file in "$SKILL_ROOT"/references/*.md; do
  [ -f "$file" ] || continue
  cp "$file" "$TX_BUILD/pending/references/"
done
dev_write_provenance "$TX_BUILD/pending-provenance" skill "$TARGET_DIR" "$SOURCE_HASH" "$CARGO_VERSION"
printf '%s\n' "$(dev_sha_file "$TX_BUILD/pending-provenance")" > "$TX_BUILD/expected-provenance-hash"

if [ "$STATE" = "empty" ]; then
  printf '%s\n' 'empty' > "$TX_BUILD/target-original-state"
  : > "$TX_BUILD/target-original-hash"
else
  printf '%s\n' 'present' > "$TX_BUILD/target-original-state"
  printf '%s\n' "$(dev_path_hash "$TARGET_DIR")" > "$TX_BUILD/target-original-hash"
  dev_copy_path "$TARGET_DIR" "$TX_BUILD/target-backup"
fi
if [ -e "$PROVENANCE_FILE" ] || [ -L "$PROVENANCE_FILE" ]; then
  if [ ! -f "$PROVENANCE_FILE" ] || [ -L "$PROVENANCE_FILE" ]; then
    echo "error: refusing unsafe Skill provenance: $PROVENANCE_FILE" >&2
    exit 1
  fi
  printf '%s\n' 'present' > "$TX_BUILD/provenance-original-state"
  printf '%s\n' "$(dev_path_hash "$PROVENANCE_FILE")" > "$TX_BUILD/provenance-original-hash"
  cp -p "$PROVENANCE_FILE" "$TX_BUILD/provenance-backup"
else
  printf '%s\n' 'empty' > "$TX_BUILD/provenance-original-state"
  : > "$TX_BUILD/provenance-original-hash"
fi

if [ "$(frontmatter_value "$TX_BUILD/pending/SKILL.md" name)" != "chat2db-cli" ] || \
   [ "$(frontmatter_value "$TX_BUILD/pending/SKILL.md" version)" != "$CARGO_VERSION" ] || \
   [ "$(bundle_hash "$TX_BUILD/pending")" != "$SOURCE_HASH" ]; then
  echo "error: staged Skill failed its identity or integrity check" >&2
  exit 1
fi
mv "$TX_BUILD" "$TX_DIR"
TX_ACTIVE=1
test_hook skill-prepared

if [ -e "$TARGET_DIR" ] || [ -L "$TARGET_DIR" ]; then
  mv "$TARGET_DIR" "$TX_DIR/target-quarantine"
  test_hook skill-target-quarantined
fi
if [ -e "$PROVENANCE_FILE" ] || [ -L "$PROVENANCE_FILE" ]; then
  mv "$PROVENANCE_FILE" "$TX_DIR/provenance-quarantine"
  test_hook skill-provenance-quarantined
fi
mv "$TX_DIR/pending" "$TARGET_DIR"
mv "$TX_DIR/pending-provenance" "$PROVENANCE_FILE"
if [ "$(bundle_hash "$TARGET_DIR")" != "$SOURCE_HASH" ] || ! valid_provenance; then
  echo "error: activated Skill failed its integrity check" >&2
  exit 1
fi
test_hook skill-activated

mv "$TX_DIR" "$TX_COMMITTED"
TX_ACTIVE=0
retire_transaction "$TX_COMMITTED"
test_hook skill-committed-retired
remove_retired_transaction "$RETIRED_TRANSACTION"
echo "installed Skill for $CLIENT: $TARGET_DIR ($SOURCE_HASH)"
