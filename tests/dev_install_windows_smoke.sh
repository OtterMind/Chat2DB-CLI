#!/usr/bin/env bash
set -euo pipefail

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  *) echo "error: this smoke test requires Windows Git Bash, MSYS2, or Cygwin" >&2; exit 1 ;;
esac

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
TEST_ROOT="$(mktemp -d)"
ORIGINAL_HOME="${HOME:?HOME must be set}"
TEST_CARGO_HOME="${CARGO_HOME:-$ORIGINAL_HOME/.cargo}"
TEST_RUSTUP_HOME="${RUSTUP_HOME:-$ORIGINAL_HOME/.rustup}"
cleanup() {
  local status=$?
  trap - EXIT
  if [ "$status" -ne 0 ]; then
    for log in "$TEST_ROOT"/*.log; do
      [ -f "$log" ] || continue
      sed -n '1,200p' "$log" >&2
    done
  fi
  rm -rf "$TEST_ROOT"
  exit "$status"
}
trap cleanup EXIT

HOME_DIR="$TEST_ROOT/home"
BIN_DIR="$TEST_ROOT/bin"
SKILL_DIR="$TEST_ROOT/skills/chat2db-cli"
mkdir -p "$HOME_DIR" "$(dirname "$SKILL_DIR")"

BIN_DIR_WINDOWS="$(cygpath -w "$BIN_DIR")"
SKILL_DIR_WINDOWS="$(cygpath -w "$SKILL_DIR")"
EXPECTED_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n 1)"

run_install() {
  local log="$1"
  (
    export HOME="$HOME_DIR" CARGO_HOME="$TEST_CARGO_HOME" RUSTUP_HOME="$TEST_RUSTUP_HOME"
    cd "$ROOT_DIR"
    bash scripts/dev-install.sh \
      --bin-dir "$BIN_DIR_WINDOWS" \
      --skill-target "$SKILL_DIR_WINDOWS"
  ) > "$log" 2>&1
}

run_install "$TEST_ROOT/first.log"

CLI="$BIN_DIR/chat2db.exe"
CLI_PROVENANCE="$BIN_DIR/.chat2db.exe.chat2db-dev-install.provenance"
SKILL_PROVENANCE="$(dirname "$SKILL_DIR")/.chat2db-cli.chat2db-dev-install.provenance"
[ -f "$CLI" ] && [ -x "$CLI" ]
[ -f "$CLI_PROVENANCE" ]
[ -f "$SKILL_DIR/SKILL.md" ]
[ -f "$SKILL_PROVENANCE" ]
case "$("$CLI" version)" in
  "chat2db $EXPECTED_VERSION"*) ;;
  *) echo "error: installed Windows CLI has the wrong identity" >&2; exit 1 ;;
esac
[ ! -e "$BIN_DIR/.chat2db-dev-install.tx" ]
[ ! -e "$BIN_DIR/.chat2db-dev-install.tx.committed" ]
[ ! -e "$(dirname "$SKILL_DIR")/.chat2db-cli.chat2db-dev-install.tx" ]
[ ! -e "$(dirname "$SKILL_DIR")/.chat2db-cli.chat2db-dev-install.tx.committed" ]

run_install "$TEST_ROOT/second.log"
if ! grep -F "CLI is up to date" "$TEST_ROOT/second.log" >/dev/null || \
   ! grep -F "Skill is up to date" "$TEST_ROOT/second.log" >/dev/null; then
  echo "error: repeated Windows development installation was not idempotent" >&2
  exit 1
fi

echo "Windows development installer smoke test passed"
