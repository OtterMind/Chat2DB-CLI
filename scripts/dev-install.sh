#!/usr/bin/env bash
# Build and transactionally install the local CLI and Skill for development.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: dev-install.sh [options]

Build the current checkout and install both the CLI and its bundled Skill.

Options:
  --client <name>     Install the Skill for codex or claude-code (default: codex).
  --skill-target <d>  Override the Skill target directory.
  --bin-dir <dir>     Override the CLI binary directory.
  --no-skill          Install only the CLI.
  --force             Replace non-Chat2DB CLI/Skill targets. Symlinks and Git targets stay protected.
  -h, --help          Show this help.

Environment:
  CHAT2DB_CLI_BIN_DIR        CLI binary directory (default: $HOME/.local/bin).
  CHAT2DB_SKILL_DEV_CLIENT   Default Skill client (codex or claude-code).
  CHAT2DB_SKILL_DEV_TARGET   Override the Skill target directory.
  CHAT2DB_DEV_INSTALL_SKILL  Set to 0 to skip the Skill installation.
  CHAT2DB_DEV_INSTALL_FORCE  Set to 1 to replace non-Chat2DB targets.
EOF
}

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
CLIENT="${CHAT2DB_SKILL_DEV_CLIENT:-codex}"
SKILL_TARGET="${CHAT2DB_SKILL_DEV_TARGET:-}"
INSTALL_SKILL="${CHAT2DB_DEV_INSTALL_SKILL:-1}"
FORCE="${CHAT2DB_DEV_INSTALL_FORCE:-0}"
BIN_DIR="${CHAT2DB_CLI_BIN_DIR:-${HOME:-}/.local/bin}"

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
  echo "error: HOME is not set; pass --bin-dir to choose an install directory" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1 && [ -f "${HOME:-}/.cargo/env" ]; then
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found; install Rust via https://rustup.rs/" >&2
  exit 1
fi

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) BINARY_NAME="chat2db.exe" ;;
  *) BINARY_NAME="chat2db" ;;
esac
BUILT="$ROOT_DIR/target/release/$BINARY_NAME"

strip_trailing_slashes() {
  local path="$1"
  while [ "$path" != "/" ] && [ "${path%/}" != "$path" ]; do
    path="${path%/}"
  done
  printf '%s' "$path"
}

canonicalize_directory() {
  local raw
  raw="$(strip_trailing_slashes "$1")"
  case "$raw" in
    /*) ;;
    *) raw="$PWD/$raw" ;;
  esac
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
TARGET="$BIN_DIR/$BINARY_NAME"
if [ -L "$TARGET" ]; then
  echo "error: refusing symlink CLI target: $TARGET" >&2
  exit 1
fi

sha_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "error: shasum or sha256sum is required" >&2
    return 1
  fi
}

echo "==> cargo build --release --locked"
(cd "$ROOT_DIR" && cargo build --release --locked)
if [ ! -f "$BUILT" ] || [ -L "$BUILT" ] || [ ! -x "$BUILT" ]; then
  echo "error: cargo built no executable at $BUILT" >&2
  exit 1
fi

CARGO_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1)"
BUILT_VERSION="$("$BUILT" version 2>/dev/null || true)"
case "$BUILT_VERSION" in
  "chat2db $CARGO_VERSION"*) ;;
  *) echo "error: built CLI identity mismatch: '$BUILT_VERSION' (expected chat2db $CARGO_VERSION)" >&2; exit 1 ;;
esac
BUILT_HASH="$(sha_of "$BUILT")"

LOCK_DIR="$BIN_DIR/.chat2db-dev-install.lock"
TX_DIR="$BIN_DIR/.chat2db-dev-install.tx"
LOCK_HELD=0
TRANSACTION_REPLACING=0

release_lock() {
  if [ "$LOCK_HELD" = "1" ]; then
    rm -f "$LOCK_DIR/pid" "$LOCK_DIR/owner"
    rmdir "$LOCK_DIR" 2>/dev/null || true
    LOCK_HELD=0
  fi
}

cleanup() {
  local status=$?
  trap - EXIT HUP INT TERM
  if [ "$TRANSACTION_REPLACING" = "1" ] && [ -e "$TX_DIR/backup" ]; then
    rm -f "$TARGET"
    if mv "$TX_DIR/backup" "$TARGET"; then
      TRANSACTION_REPLACING=0
    else
      echo "error: failed to restore the previous CLI; recovery data preserved at $TX_DIR" >&2
      status=1
    fi
  fi
  if [ "$TRANSACTION_REPLACING" = "0" ] && [ -d "$TX_DIR" ]; then
    rm -rf "$TX_DIR"
  fi
  release_lock
  exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

acquire_lock() {
  local attempts=0 owner_pid=""
  while ! mkdir "$LOCK_DIR" 2>/dev/null; do
    if [ -f "$LOCK_DIR/owner" ] && [ "$(cat "$LOCK_DIR/owner" 2>/dev/null || true)" = "chat2db-dev-install" ]; then
      owner_pid="$(cat "$LOCK_DIR/pid" 2>/dev/null || true)"
      case "$owner_pid" in ''|*[!0-9]*) owner_pid="" ;; esac
      if [ -z "$owner_pid" ] || ! kill -0 "$owner_pid" 2>/dev/null; then
        rm -f "$LOCK_DIR/pid" "$LOCK_DIR/owner"
        rmdir "$LOCK_DIR" 2>/dev/null || true
        continue
      fi
    elif [ "$attempts" -ge 10 ] && rmdir "$LOCK_DIR" 2>/dev/null; then
      continue
    fi
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 600 ]; then
      echo "error: timed out waiting for CLI installation lock: $LOCK_DIR" >&2
      return 1
    fi
    sleep 0.1
  done
  printf '%s\n' 'chat2db-dev-install' > "$LOCK_DIR/owner"
  printf '%s\n' "$$" > "$LOCK_DIR/pid"
  LOCK_HELD=1
}

is_chat2db_cli() {
  local output
  [ -f "$1" ] && [ -x "$1" ] && [ ! -L "$1" ] || return 1
  output="$("$1" version 2>/dev/null || true)"
  case "$output" in chat2db\ *) return 0 ;; *) return 1 ;; esac
}

recover_transaction() {
  local recorded_target expected_hash
  [ -e "$TX_DIR" ] || [ -L "$TX_DIR" ] || return 0
  if [ -L "$TX_DIR" ] || [ ! -d "$TX_DIR" ] || [ ! -f "$TX_DIR/target" ]; then
    echo "error: unrecognized CLI transaction path; refusing to remove it: $TX_DIR" >&2
    return 1
  fi
  recorded_target="$(cat "$TX_DIR/target")"
  if [ "$recorded_target" != "$TARGET" ]; then
    echo "error: CLI transaction target mismatch at $TX_DIR" >&2
    return 1
  fi
  if [ -e "$TX_DIR/backup" ] || [ -L "$TX_DIR/backup" ]; then
    if [ ! -e "$TARGET" ] && [ ! -L "$TARGET" ]; then
      mv "$TX_DIR/backup" "$TARGET"
      echo "recovered previous CLI: $TARGET"
    elif [ -f "$TX_DIR/expected-hash" ]; then
      expected_hash="$(cat "$TX_DIR/expected-hash")"
      if ! is_chat2db_cli "$TARGET" || [ "$(sha_of "$TARGET")" != "$expected_hash" ]; then
        echo "error: cannot reconcile interrupted CLI install; recovery data preserved at $TX_DIR" >&2
        return 1
      fi
      rm -f "$TX_DIR/backup"
    else
      echo "error: cannot reconcile interrupted CLI install; recovery data preserved at $TX_DIR" >&2
      return 1
    fi
  fi
  rm -f "$TX_DIR/pending" "$TX_DIR/target" "$TX_DIR/expected-hash"
  rmdir "$TX_DIR"
}

acquire_lock
recover_transaction

if [ -d "$TARGET" ]; then
  echo "error: CLI target is a directory: $TARGET" >&2
  exit 1
fi
if [ -e "$TARGET" ] && ! is_chat2db_cli "$TARGET" && [ "$FORCE" != "1" ]; then
  echo "error: CLI target is not Chat2DB: $TARGET; pass --force to replace it" >&2
  exit 1
fi

CLI_CHANGED=1
if [ -f "$TARGET" ] && [ "$BUILT_HASH" = "$(sha_of "$TARGET")" ]; then
  CLI_CHANGED=0
  echo "==> CLI is up to date: $TARGET ($(sha_of "$TARGET"))"
else
  mkdir "$TX_DIR"
  printf '%s\n' "$TARGET" > "$TX_DIR/target"
  printf '%s\n' "$BUILT_HASH" > "$TX_DIR/expected-hash"
  install -m 0755 "$BUILT" "$TX_DIR/pending"
  if ! is_chat2db_cli "$TX_DIR/pending"; then
    echo "error: staged CLI failed its identity check" >&2
    exit 1
  fi
  if [ -e "$TARGET" ]; then
    mv "$TARGET" "$TX_DIR/backup"
    TRANSACTION_REPLACING=1
  fi
  if ! mv "$TX_DIR/pending" "$TARGET"; then
    echo "error: failed to activate the local CLI" >&2
    exit 1
  fi
  if [ "$TRANSACTION_REPLACING" = "1" ]; then
    rm -f "$TX_DIR/backup"
    TRANSACTION_REPLACING=0
  fi
  rm -f "$TX_DIR/target" "$TX_DIR/expected-hash"
  rmdir "$TX_DIR"
  echo "==> installed CLI: $TARGET"
  "$TARGET" version
fi

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

release_lock
trap - EXIT HUP INT TERM

if [ "$INSTALL_SKILL" = "1" ]; then
  SKILL_ARGS=(--client "$CLIENT")
  if [ -n "$SKILL_TARGET" ]; then
    SKILL_ARGS+=(--target "$SKILL_TARGET")
  fi
  if [ "$FORCE" = "1" ]; then
    SKILL_ARGS+=(--force)
  fi
  bash "$ROOT_DIR/scripts/dev-install-skill.sh" "${SKILL_ARGS[@]}"
fi
