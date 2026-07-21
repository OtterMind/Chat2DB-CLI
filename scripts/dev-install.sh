#!/usr/bin/env bash
# Build and transactionally install the local CLI and Skill for development.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: dev-install.sh [options]

Build the current checkout and install the CLI and its bundled Skill as one transaction.

Options:
  --client <name>     Install the Skill for codex or claude-code (default: codex).
  --skill-target <d>  Override the Skill target directory.
  --bin-dir <dir>     Override the CLI binary directory.
  --no-skill          Install only the CLI.
  --force             Replace unowned CLI/Skill targets. Symlinks and Git targets stay protected.
  -h, --help          Show this help.

Environment:
  CHAT2DB_CLI_BIN_DIR        CLI binary directory (default: $HOME/.local/bin).
  CHAT2DB_SKILL_DEV_CLIENT   Default Skill client (codex or claude-code).
  CHAT2DB_SKILL_DEV_TARGET   Override the Skill target directory.
  CHAT2DB_DEV_INSTALL_SKILL  Set to 0 to skip the Skill installation.
  CHAT2DB_DEV_INSTALL_FORCE  Set to 1 to replace unowned targets.
EOF
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=dev-install-common.sh
# shellcheck disable=SC1091
. "$ROOT_DIR/scripts/dev-install-common.sh"
CLIENT="${CHAT2DB_SKILL_DEV_CLIENT:-codex}"
SKILL_TARGET="${CHAT2DB_SKILL_DEV_TARGET:-}"
INSTALL_SKILL="${CHAT2DB_DEV_INSTALL_SKILL:-1}"
FORCE="${CHAT2DB_DEV_INSTALL_FORCE:-0}"
BIN_DIR="${CHAT2DB_CLI_BIN_DIR:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --client)
      [ $# -ge 2 ] || { echo "error: --client requires a value" >&2; exit 1; }
      CLIENT="$2"
      shift 2
      ;;
    --client=*) CLIENT="${1#--client=}"; shift ;;
    --skill-target)
      [ $# -ge 2 ] || { echo "error: --skill-target requires a value" >&2; exit 1; }
      SKILL_TARGET="$2"
      shift 2
      ;;
    --skill-target=*) SKILL_TARGET="${1#--skill-target=}"; shift ;;
    --bin-dir)
      [ $# -ge 2 ] || { echo "error: --bin-dir requires a value" >&2; exit 1; }
      BIN_DIR="$2"
      shift 2
      ;;
    --bin-dir=*) BIN_DIR="${1#--bin-dir=}"; shift ;;
    --no-skill) INSTALL_SKILL=0; shift ;;
    --force) FORCE=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

case "$CLIENT" in codex|claude-code) ;; *) echo "error: --client must be codex or claude-code" >&2; exit 1 ;; esac
case "$INSTALL_SKILL" in 0|1) ;; *) echo "error: CHAT2DB_DEV_INSTALL_SKILL must be 0 or 1" >&2; exit 1 ;; esac
case "$FORCE" in 0|1) ;; *) echo "error: CHAT2DB_DEV_INSTALL_FORCE must be 0 or 1" >&2; exit 1 ;; esac

if [ -z "$BIN_DIR" ]; then
  if [ -z "${HOME:-}" ]; then
    echo "error: HOME is not set; pass --bin-dir to choose an install directory" >&2
    exit 1
  fi
  BIN_DIR="$HOME/.local/bin"
fi
if [ "$INSTALL_SKILL" = "1" ] && [ -z "$SKILL_TARGET" ] && [ -z "${HOME:-}" ]; then
  echo "error: HOME is not set; pass --skill-target or use --no-skill" >&2
  exit 1
fi

strip_trailing_slashes() {
  local path="$1"
  while [ "$path" != "/" ] && [ "${path%/}" != "$path" ]; do
    path="${path%/}"
  done
  printf '%s' "$path"
}

canonicalize_directory() {
  local raw
  case "$1" in
    *$'\n'*) echo "error: install path must not contain a newline" >&2; return 1 ;;
  esac
  raw="$(strip_trailing_slashes "$1")"
  raw="$(dev_absolute_input_path "$raw")"
  if [ -L "$raw" ]; then
    echo "error: refusing symlink binary directory: $raw" >&2
    return 1
  fi
  mkdir -p "$raw"
  (cd -P "$raw" && pwd -P)
}

BIN_DIR="$(canonicalize_directory "$BIN_DIR")"
if [ "$BIN_DIR" = "/" ]; then
  echo "error: refusing to install the CLI directly into /" >&2
  exit 1
fi
if dev_paths_overlap "$BIN_DIR" "$ROOT_DIR"; then
  echo "error: refusing a CLI binary directory that overlaps the source checkout: $BIN_DIR" >&2
  exit 1
fi
if dev_path_has_git_ancestor "$BIN_DIR"; then
  echo "error: refusing a CLI binary directory inside a Git worktree: $BIN_DIR" >&2
  exit 1
fi
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) BINARY_NAME="chat2db.exe" ;;
  *) BINARY_NAME="chat2db" ;;
esac
TARGET="$BIN_DIR/$BINARY_NAME"
PROVENANCE_FILE="$BIN_DIR/.${BINARY_NAME}.chat2db-dev-install.provenance"
CLI_LOCK_FILE="$BIN_DIR/.chat2db-dev-install.lock"
TX_DIR="$BIN_DIR/.chat2db-dev-install.tx"
TX_COMMITTED="$TX_DIR.committed"
TX_BUILD="$TX_DIR.build.$$"
BUILD_DIR="$ROOT_DIR/target/dev-install"
BUILD_LOCK_FILE="$BUILD_DIR/.chat2db-dev-build.lock"
BUILD_LOCK_TOKEN=""
BUILT=""

SKILL_ARGS=(--client "$CLIENT")
if [ -n "$SKILL_TARGET" ]; then
  SKILL_ARGS+=(--target "$SKILL_TARGET")
fi
if [ "$FORCE" = "1" ]; then
  SKILL_ARGS+=(--force)
fi
if [ "$INSTALL_SKILL" = "1" ]; then
  SKILL_TARGET="$(bash "$ROOT_DIR/scripts/dev-install-skill.sh" "${SKILL_ARGS[@]}" --print-target)"
  SKILL_PARENT="$(dirname "$SKILL_TARGET")"
  SKILL_NAME="$(basename "$SKILL_TARGET")"
  SKILL_PROVENANCE="$SKILL_PARENT/.${SKILL_NAME}.chat2db-dev-install.provenance"
  SKILL_LOCK_FILE="$SKILL_PARENT/.${SKILL_NAME}.chat2db-dev-install.lock"
  if dev_paths_overlap "$SKILL_TARGET" "$BIN_DIR"; then
    echo "error: Skill target must not overlap the CLI binary directory" >&2
    exit 1
  fi
else
  SKILL_PARENT=""
  SKILL_NAME=""
  SKILL_PROVENANCE=""
  SKILL_LOCK_FILE=""
fi

if [ -L "$TARGET" ] || [ -L "$PROVENANCE_FILE" ]; then
  echo "error: refusing symlink CLI target or provenance: $TARGET" >&2
  exit 1
fi
if [ -d "$TARGET" ]; then
  echo "error: CLI target is a directory: $TARGET" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1 && [ -n "${HOME:-}" ] && [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found; install Rust via https://rustup.rs/" >&2
  exit 1
fi

source_skill_hash() {
  (
    cd "$ROOT_DIR/skill"
    find SKILL.md references -type f 2>/dev/null \
      | LC_ALL=C sort \
      | while IFS= read -r file; do
          printf '%s  %s\n' "$(dev_sha_file "$file")" "$file"
        done \
      | dev_sha_stream
  )
}

valid_cli_provenance() {
  local recorded_hash
  dev_valid_provenance_header "$PROVENANCE_FILE" cli "$TARGET" "$CARGO_VERSION" || return 1
  recorded_hash="$(dev_metadata_value "$PROVENANCE_FILE" sha256)"
  [ -f "$TARGET" ] && [ -x "$TARGET" ] && [ ! -L "$TARGET" ] || return 1
  [ "$(dev_sha_file "$TARGET")" = "$recorded_hash" ]
}

cli_target_state() {
  if [ -L "$TARGET" ] || [ -L "$PROVENANCE_FILE" ]; then
    printf 'symlink'
  elif [ ! -e "$TARGET" ]; then
    if [ -e "$PROVENANCE_FILE" ]; then printf 'foreign'; else printf 'empty'; fi
  elif [ -d "$TARGET" ]; then
    printf 'directory'
  elif valid_cli_provenance; then
    printf 'ours'
  else
    printf 'foreign'
  fi
}

CLI_LOCK_TOKEN=""
SKILL_LOCK_TOKEN=""
RECOVERY_SKILL_LOCK_FILE=""
RECOVERY_SKILL_LOCK_TOKEN=""
RETIRED_TRANSACTION=""
TX_ACTIVE=0
PRESERVE_TX=0

skill_command() {
  local mode="$1"
  shift
  if [ -n "$mode" ]; then
    CHAT2DB_DEV_INSTALL_BORROWED_LOCK="$SKILL_LOCK_TOKEN" \
      bash "$ROOT_DIR/scripts/dev-install-skill.sh" "${SKILL_ARGS[@]}" "$mode" "$@"
  else
    CHAT2DB_DEV_INSTALL_BORROWED_LOCK="$SKILL_LOCK_TOKEN" \
      bash "$ROOT_DIR/scripts/dev-install-skill.sh" "${SKILL_ARGS[@]}" "$@"
  fi
}

tx_value() {
  [ -f "$1/$2" ] && cat "$1/$2"
}

valid_transaction_header() {
  local directory="$1" skill_enabled skill_target skill_parent skill_name
  local skill_provenance skill_restore_workspace
  [ -d "$directory" ] && [ ! -L "$directory" ] || return 1
  [ "$(tx_value "$directory" format)" = "chat2db-dev-combined-v2" ] || return 1
  [ "$(tx_value "$directory" cli-target)" = "$TARGET" ] || return 1
  [ "$(tx_value "$directory" cli-provenance)" = "$PROVENANCE_FILE" ] || return 1
  skill_enabled="$(tx_value "$directory" skill-enabled)"
  case "$skill_enabled" in
    0) ;;
    1)
      skill_target="$(tx_value "$directory" skill-target)"
      case "$skill_target" in
        /*) ;;
        *) return 1 ;;
      esac
      case "$skill_target" in *$'\n'*) return 1 ;; esac
      skill_parent="$(dirname "$skill_target")"
      skill_name="$(basename "$skill_target")"
      case "$skill_name" in ''|/|.|..) return 1 ;; esac
      skill_provenance="$(tx_value "$directory" skill-provenance)"
      [ "$skill_provenance" = "$skill_parent/.${skill_name}.chat2db-dev-install.provenance" ] || return 1
      skill_restore_workspace="$(tx_value "$directory" skill-restore-workspace)"
      case "$skill_restore_workspace" in
        "$skill_parent/.${skill_name}.chat2db-restore."*) ;;
        *) return 1 ;;
      esac
      ;;
    *) return 1 ;;
  esac
}

load_transaction_skill_context() {
  local directory="$1"
  TX_SKILL_ENABLED="$(tx_value "$directory" skill-enabled)"
  if [ "$TX_SKILL_ENABLED" = "1" ]; then
    TX_SKILL_TARGET="$(tx_value "$directory" skill-target)"
    TX_SKILL_PARENT="$(dirname "$TX_SKILL_TARGET")"
    TX_SKILL_NAME="$(basename "$TX_SKILL_TARGET")"
    TX_SKILL_PROVENANCE="$(tx_value "$directory" skill-provenance)"
    TX_SKILL_RESTORE_WORKSPACE="$(tx_value "$directory" skill-restore-workspace)"
  else
    TX_SKILL_TARGET=""
    TX_SKILL_PARENT=""
    TX_SKILL_NAME=""
    TX_SKILL_PROVENANCE=""
    TX_SKILL_RESTORE_WORKSPACE=""
  fi
}

acquire_transaction_skill_lock() {
  local directory="$1" expected_lock
  RECOVERY_SKILL_LOCK_FILE=""
  RECOVERY_SKILL_LOCK_TOKEN=""
  valid_transaction_header "$directory" || return 1
  load_transaction_skill_context "$directory"
  [ "$TX_SKILL_ENABLED" = "1" ] || return 0
  expected_lock="$TX_SKILL_PARENT/.${TX_SKILL_NAME}.chat2db-dev-install.lock"
  dev_acquire_lock "$expected_lock" Skill-recovery || return 1
  RECOVERY_SKILL_LOCK_FILE="$expected_lock"
  RECOVERY_SKILL_LOCK_TOKEN="$DEV_LOCK_TOKEN"
  if ! valid_transaction_header "$directory"; then
    dev_release_lock "$RECOVERY_SKILL_LOCK_FILE" "$RECOVERY_SKILL_LOCK_TOKEN"
    RECOVERY_SKILL_LOCK_FILE=""
    RECOVERY_SKILL_LOCK_TOKEN=""
    return 1
  fi
  load_transaction_skill_context "$directory"
  if [ "$TX_SKILL_PARENT/.${TX_SKILL_NAME}.chat2db-dev-install.lock" != "$expected_lock" ]; then
    dev_release_lock "$RECOVERY_SKILL_LOCK_FILE" "$RECOVERY_SKILL_LOCK_TOKEN"
    RECOVERY_SKILL_LOCK_FILE=""
    RECOVERY_SKILL_LOCK_TOKEN=""
    return 1
  fi
}

release_transaction_skill_lock() {
  dev_release_lock "$RECOVERY_SKILL_LOCK_FILE" "$RECOVERY_SKILL_LOCK_TOKEN"
  RECOVERY_SKILL_LOCK_FILE=""
  RECOVERY_SKILL_LOCK_TOKEN=""
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
    echo "warn: retired development journal remains at $retired" >&2
  fi
}

rollback_transaction() {
  local skill_enabled skill_target skill_provenance skill_restore_workspace
  valid_transaction_header "$TX_DIR" || {
    PRESERVE_TX=1
    echo "error: unrecognized combined transaction; refusing to remove it: $TX_DIR" >&2
    return 1
  }
  load_transaction_skill_context "$TX_DIR"
  skill_enabled="$TX_SKILL_ENABLED"
  skill_target="$TX_SKILL_TARGET"
  skill_provenance="$TX_SKILL_PROVENANCE"
  skill_restore_workspace="$TX_SKILL_RESTORE_WORKSPACE"
  if [ "$skill_enabled" = "1" ]; then
    dev_restore_component \
      "$skill_target" \
      "$(tx_value "$TX_DIR" skill-original-state)" \
      "$TX_DIR/skill-backup" \
      "$(tx_value "$TX_DIR" skill-original-hash)" \
      "dir:$(tx_value "$TX_DIR" expected-skill-hash)" \
      "Skill target" "$TX_DIR" "$skill_restore_workspace" target || { PRESERVE_TX=1; return 1; }
    dev_restore_component \
      "$skill_provenance" \
      "$(tx_value "$TX_DIR" skill-provenance-original-state)" \
      "$TX_DIR/skill-provenance-backup" \
      "$(tx_value "$TX_DIR" skill-provenance-original-hash)" \
      "file:$(tx_value "$TX_DIR" expected-skill-provenance-hash)" \
      "Skill provenance" "$TX_DIR" "$skill_restore_workspace" provenance || { PRESERVE_TX=1; return 1; }
    dev_remove_restore_workspace "$skill_restore_workspace" "$TX_DIR" || { PRESERVE_TX=1; return 1; }
  fi
  dev_restore_component \
    "$TARGET" \
    "$(tx_value "$TX_DIR" cli-original-state)" \
    "$TX_DIR/cli-backup" \
    "$(tx_value "$TX_DIR" cli-original-hash)" \
    "file:$(tx_value "$TX_DIR" expected-cli-hash)" \
    "CLI target" "$TX_DIR" "$TX_DIR/recovery-cli" target || { PRESERVE_TX=1; return 1; }
  dev_restore_component \
    "$PROVENANCE_FILE" \
    "$(tx_value "$TX_DIR" cli-provenance-original-state)" \
    "$TX_DIR/cli-provenance-backup" \
    "$(tx_value "$TX_DIR" cli-provenance-original-hash)" \
    "file:$(tx_value "$TX_DIR" expected-cli-provenance-hash)" \
    "CLI provenance" "$TX_DIR" "$TX_DIR/recovery-cli" provenance || { PRESERVE_TX=1; return 1; }
  retire_transaction "$TX_DIR" || { PRESERVE_TX=1; return 1; }
  TX_ACTIVE=0
  test_hook combined-rollback-retired
  remove_retired_transaction "$RETIRED_TRANSACTION"
}

finalize_committed_transaction() {
  local skill_enabled skill_target skill_provenance
  valid_transaction_header "$TX_COMMITTED" || {
    PRESERVE_TX=1
    echo "error: unrecognized committed transaction: $TX_COMMITTED" >&2
    return 1
  }
  load_transaction_skill_context "$TX_COMMITTED"
  skill_enabled="$TX_SKILL_ENABLED"
  skill_target="$TX_SKILL_TARGET"
  skill_provenance="$TX_SKILL_PROVENANCE"
  if [ ! -f "$TARGET" ] || [ "$(dev_sha_file "$TARGET")" != "$(tx_value "$TX_COMMITTED" expected-cli-hash)" ] || \
     [ ! -f "$PROVENANCE_FILE" ] || [ "$(dev_sha_file "$PROVENANCE_FILE")" != "$(tx_value "$TX_COMMITTED" expected-cli-provenance-hash)" ]; then
    PRESERVE_TX=1
    echo "error: committed CLI transaction does not match its targets: $TX_COMMITTED" >&2
    return 1
  fi
  if [ "$skill_enabled" = "1" ]; then
    if [ ! -d "$skill_target" ] || [ "$(dev_directory_hash "$skill_target")" != "$(tx_value "$TX_COMMITTED" expected-skill-hash)" ] || \
       [ ! -f "$skill_provenance" ] || [ "$(dev_sha_file "$skill_provenance")" != "$(tx_value "$TX_COMMITTED" expected-skill-provenance-hash)" ]; then
      PRESERVE_TX=1
      echo "error: committed Skill transaction does not match its targets: $TX_COMMITTED" >&2
      return 1
    fi
  fi
  retire_transaction "$TX_COMMITTED" || { PRESERVE_TX=1; return 1; }
  test_hook combined-committed-retired
  remove_retired_transaction "$RETIRED_TRANSACTION"
}

recover_transactions() {
  if [ -e "$TX_COMMITTED" ] || [ -L "$TX_COMMITTED" ]; then
    if ! acquire_transaction_skill_lock "$TX_COMMITTED"; then
      PRESERVE_TX=1
      echo "error: cannot lock the Skill recorded by $TX_COMMITTED" >&2
      return 1
    fi
    if ! finalize_committed_transaction; then
      release_transaction_skill_lock
      return 1
    fi
    release_transaction_skill_lock
  fi
  if [ -e "$TX_DIR" ] || [ -L "$TX_DIR" ]; then
    if ! acquire_transaction_skill_lock "$TX_DIR"; then
      PRESERVE_TX=1
      echo "error: cannot lock the Skill recorded by $TX_DIR" >&2
      return 1
    fi
    TX_ACTIVE=1
    if ! rollback_transaction; then
      release_transaction_skill_lock
      return 1
    fi
    release_transaction_skill_lock
    echo "recovered previous CLI and Skill transaction"
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
  if [ "$status" -ne 0 ]; then
    if [ "$INSTALL_SKILL" = "1" ] && [ -n "$SKILL_LOCK_TOKEN" ]; then
      if ! skill_command --recover-only; then
        PRESERVE_TX=1
        status=1
      fi
    fi
    if [ "$TX_ACTIVE" = "1" ] && [ "$PRESERVE_TX" = "0" ]; then
      if ! rollback_transaction; then
        status=1
      fi
    fi
  fi
  rm -rf "$TX_BUILD"
  if [ -n "$BUILT" ]; then
    rm -f "$BUILT"
  fi
  [ -n "$DEV_LOCK_CANDIDATE" ] && rm -f "$DEV_LOCK_CANDIDATE"
  [ -n "$DEV_LOCK_GUARD_CANDIDATE" ] && rm -f "$DEV_LOCK_GUARD_CANDIDATE"
  dev_release_lock_guard
  dev_release_lock "$BUILD_LOCK_FILE" "$BUILD_LOCK_TOKEN"
  dev_release_lock "$RECOVERY_SKILL_LOCK_FILE" "$RECOVERY_SKILL_LOCK_TOKEN"
  if [ "$INSTALL_SKILL" = "1" ]; then
    dev_release_lock "$SKILL_LOCK_FILE" "$SKILL_LOCK_TOKEN"
  fi
  dev_release_lock "$CLI_LOCK_FILE" "$CLI_LOCK_TOKEN"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

dev_acquire_lock "$CLI_LOCK_FILE" CLI
CLI_LOCK_TOKEN="$DEV_LOCK_TOKEN"
recover_transactions
if [ "$INSTALL_SKILL" = "1" ]; then
  dev_acquire_lock "$SKILL_LOCK_FILE" Skill
  SKILL_LOCK_TOKEN="$DEV_LOCK_TOKEN"
  skill_command --recover-only
  skill_command --check-only
fi

echo "==> cargo build --release --locked"
mkdir -p "$BUILD_DIR"
dev_acquire_lock "$BUILD_LOCK_FILE" Build
BUILD_LOCK_TOKEN="$DEV_LOCK_TOKEN"
BUILD_OUTPUT="$BUILD_DIR/release/$BINARY_NAME"
BUILT="$BUILD_DIR/.chat2db-dev-built.$$"
rm -f "$BUILD_OUTPUT" "$BUILT"
(cd "$ROOT_DIR" && CARGO_TARGET_DIR=target/dev-install cargo build --release --locked)
if [ ! -f "$BUILD_OUTPUT" ] || [ -L "$BUILD_OUTPUT" ] || [ ! -x "$BUILD_OUTPUT" ]; then
  echo "error: cargo built no executable at $BUILD_OUTPUT" >&2
  exit 1
fi
CARGO_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1)"
BUILT_VERSION="$("$BUILD_OUTPUT" version 2>/dev/null || true)"
case "$BUILT_VERSION" in
  "chat2db $CARGO_VERSION"*) ;;
  *) echo "error: built CLI identity mismatch: '$BUILT_VERSION' (expected chat2db $CARGO_VERSION)" >&2; exit 1 ;;
esac
install -m 0755 "$BUILD_OUTPUT" "$BUILT"
BUILT_HASH="$(dev_sha_file "$BUILT")"
dev_release_lock "$BUILD_LOCK_FILE" "$BUILD_LOCK_TOKEN"
BUILD_LOCK_TOKEN=""

CLI_STATE="$(cli_target_state)"
case "$CLI_STATE" in
  empty|ours) ;;
  symlink) echo "error: refusing symlink CLI target or provenance: $TARGET" >&2; exit 1 ;;
  directory) echo "error: CLI target is a directory: $TARGET" >&2; exit 1 ;;
  foreign)
    if [ "$FORCE" != "1" ]; then
      echo "error: CLI target is not owned by the Chat2DB development installer: $TARGET; pass --force to replace it" >&2
      exit 1
    fi
    ;;
esac

SKILL_SOURCE_HASH=""
SKILL_RESTORE_WORKSPACE=""
if [ "$INSTALL_SKILL" = "1" ]; then
  SKILL_SOURCE_HASH="$(source_skill_hash)"
  SKILL_RESTORE_WORKSPACE="$SKILL_PARENT/.${SKILL_NAME}.chat2db-restore.$$.$(date +%s).${RANDOM:-0}"
  if [ -e "$SKILL_RESTORE_WORKSPACE" ] || [ -L "$SKILL_RESTORE_WORKSPACE" ]; then
    echo "error: recovery workspace already exists: $SKILL_RESTORE_WORKSPACE" >&2
    exit 1
  fi
fi

mkdir "$TX_BUILD"
printf '%s\n' 'chat2db-dev-combined-v2' > "$TX_BUILD/format"
printf '%s\n' "$TARGET" > "$TX_BUILD/cli-target"
printf '%s\n' "$PROVENANCE_FILE" > "$TX_BUILD/cli-provenance"
printf '%s\n' "$INSTALL_SKILL" > "$TX_BUILD/skill-enabled"
printf '%s\n' "$BUILT_HASH" > "$TX_BUILD/expected-cli-hash"
install -m 0755 "$BUILT" "$TX_BUILD/cli-pending"
dev_write_provenance "$TX_BUILD/cli-pending-provenance" cli "$TARGET" "$BUILT_HASH" "$CARGO_VERSION"
printf '%s\n' "$(dev_sha_file "$TX_BUILD/cli-pending-provenance")" > "$TX_BUILD/expected-cli-provenance-hash"

if [ -e "$TARGET" ]; then
  printf '%s\n' 'present' > "$TX_BUILD/cli-original-state"
  printf '%s\n' "$(dev_path_hash "$TARGET")" > "$TX_BUILD/cli-original-hash"
  dev_copy_path "$TARGET" "$TX_BUILD/cli-backup"
else
  printf '%s\n' 'empty' > "$TX_BUILD/cli-original-state"
  : > "$TX_BUILD/cli-original-hash"
fi
if [ -e "$PROVENANCE_FILE" ]; then
  [ -f "$PROVENANCE_FILE" ] && [ ! -L "$PROVENANCE_FILE" ] || {
    echo "error: refusing unsafe CLI provenance: $PROVENANCE_FILE" >&2
    exit 1
  }
  printf '%s\n' 'present' > "$TX_BUILD/cli-provenance-original-state"
  printf '%s\n' "$(dev_path_hash "$PROVENANCE_FILE")" > "$TX_BUILD/cli-provenance-original-hash"
  dev_copy_path "$PROVENANCE_FILE" "$TX_BUILD/cli-provenance-backup"
else
  printf '%s\n' 'empty' > "$TX_BUILD/cli-provenance-original-state"
  : > "$TX_BUILD/cli-provenance-original-hash"
fi

if [ "$INSTALL_SKILL" = "1" ]; then
  printf '%s\n' "$SKILL_TARGET" > "$TX_BUILD/skill-target"
  printf '%s\n' "$SKILL_PROVENANCE" > "$TX_BUILD/skill-provenance"
  printf '%s\n' "$SKILL_RESTORE_WORKSPACE" > "$TX_BUILD/skill-restore-workspace"
  printf '%s\n' "$SKILL_SOURCE_HASH" > "$TX_BUILD/expected-skill-hash"
  dev_write_provenance "$TX_BUILD/expected-skill-provenance" skill "$SKILL_TARGET" "$SKILL_SOURCE_HASH" "$CARGO_VERSION"
  printf '%s\n' "$(dev_sha_file "$TX_BUILD/expected-skill-provenance")" > "$TX_BUILD/expected-skill-provenance-hash"
  if [ -e "$SKILL_TARGET" ]; then
    printf '%s\n' 'present' > "$TX_BUILD/skill-original-state"
    printf '%s\n' "$(dev_path_hash "$SKILL_TARGET")" > "$TX_BUILD/skill-original-hash"
    dev_copy_path "$SKILL_TARGET" "$TX_BUILD/skill-backup"
  else
    printf '%s\n' 'empty' > "$TX_BUILD/skill-original-state"
    : > "$TX_BUILD/skill-original-hash"
  fi
  if [ -e "$SKILL_PROVENANCE" ]; then
    [ -f "$SKILL_PROVENANCE" ] && [ ! -L "$SKILL_PROVENANCE" ] || {
      echo "error: refusing unsafe Skill provenance: $SKILL_PROVENANCE" >&2
      exit 1
    }
    printf '%s\n' 'present' > "$TX_BUILD/skill-provenance-original-state"
    printf '%s\n' "$(dev_path_hash "$SKILL_PROVENANCE")" > "$TX_BUILD/skill-provenance-original-hash"
    dev_copy_path "$SKILL_PROVENANCE" "$TX_BUILD/skill-provenance-backup"
  else
    printf '%s\n' 'empty' > "$TX_BUILD/skill-provenance-original-state"
    : > "$TX_BUILD/skill-provenance-original-hash"
  fi
fi

if [ "$(dev_sha_file "$TX_BUILD/cli-pending")" != "$BUILT_HASH" ]; then
  echo "error: staged CLI failed its integrity check" >&2
  exit 1
fi
mv "$TX_BUILD" "$TX_DIR"
TX_ACTIVE=1
test_hook combined-prepared

CLI_CHANGED=1
if [ "$CLI_STATE" = "ours" ] && [ "$(dev_sha_file "$TARGET")" = "$BUILT_HASH" ]; then
  CLI_CHANGED=0
  echo "==> CLI is up to date: $TARGET ($BUILT_HASH)"
else
  if [ -e "$TARGET" ] || [ -L "$TARGET" ]; then
    mv "$TARGET" "$TX_DIR/cli-target-quarantine"
    test_hook combined-cli-target-quarantined
  fi
  if [ -e "$PROVENANCE_FILE" ] || [ -L "$PROVENANCE_FILE" ]; then
    mv "$PROVENANCE_FILE" "$TX_DIR/cli-provenance-quarantine"
    test_hook combined-cli-provenance-quarantined
  fi
  mv "$TX_DIR/cli-pending" "$TARGET"
  mv "$TX_DIR/cli-pending-provenance" "$PROVENANCE_FILE"
  if [ "$(dev_sha_file "$TARGET")" != "$BUILT_HASH" ] || ! valid_cli_provenance; then
    echo "error: activated CLI failed its integrity check" >&2
    exit 1
  fi
  echo "==> installed CLI: $TARGET"
fi
test_hook combined-cli-activated

if [ "$INSTALL_SKILL" = "1" ]; then
  skill_command ""
  if [ "$(dev_directory_hash "$SKILL_TARGET")" != "$SKILL_SOURCE_HASH" ] || \
     [ "$(dev_sha_file "$SKILL_PROVENANCE")" != "$(tx_value "$TX_DIR" expected-skill-provenance-hash)" ]; then
    echo "error: installed Skill does not match the combined transaction" >&2
    exit 1
  fi
fi
test_hook combined-both-activated

mv "$TX_DIR" "$TX_COMMITTED"
TX_ACTIVE=0
retire_transaction "$TX_COMMITTED"
test_hook combined-committed-retired
remove_retired_transaction "$RETIRED_TRANSACTION"

if [ "$CLI_CHANGED" = "1" ] && [ -n "${SHELL:-}" ]; then
  case "$(basename "$SHELL")" in
    zsh|bash|fish)
      if "$TARGET" completions --install >/dev/null 2>&1; then
        echo "completions: refreshed for $(basename "$SHELL")"
      fi
      ;;
  esac
fi
case ":${PATH:-}:" in
  *":$BIN_DIR:"*) ;;
  *) echo "warn: $BIN_DIR is not on \$PATH; add it to your shell profile" >&2 ;;
esac

if [ "$INSTALL_SKILL" = "1" ]; then
  dev_release_lock "$SKILL_LOCK_FILE" "$SKILL_LOCK_TOKEN"
  SKILL_LOCK_TOKEN=""
fi
dev_release_lock "$CLI_LOCK_FILE" "$CLI_LOCK_TOKEN"
CLI_LOCK_TOKEN=""
trap - EXIT HUP INT TERM
rm -f "$BUILT"
