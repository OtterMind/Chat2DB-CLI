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
  --force          Replace a non-Chat2DB target. Git and symlink targets remain protected.
  -h, --help       Show this help.

Environment:
  CHAT2DB_SKILL_DEV_CLIENT  Default client (codex or claude-code).
  CHAT2DB_SKILL_DEV_TARGET  Override the Skill target directory.
  CHAT2DB_SKILL_DEV_FORCE=1 Replace a non-Chat2DB target.
EOF
}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SKILL_ROOT="$REPO_ROOT/skill"
CLIENT="${CHAT2DB_SKILL_DEV_CLIENT:-codex}"
TARGET_DIR="${CHAT2DB_SKILL_DEV_TARGET:-}"
FORCE="${CHAT2DB_SKILL_DEV_FORCE:-0}"

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
  raw="$(strip_trailing_slashes "$1")"
  case "$raw" in
    /*) ;;
    *) raw="$PWD/$raw" ;;
  esac
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
  if [ "$TARGET_DIR" = "$HOME_CANONICAL" ]; then
    echo "error: refusing to replace HOME: $TARGET_DIR" >&2
    exit 1
  fi
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

sha_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    echo "error: shasum or sha256sum is required" >&2
    return 1
  fi
}

sha_stream() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 | awk '{print $1}'
  else
    sha256sum | awk '{print $1}'
  fi
}

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
          printf '%s  %s\n' "$(sha_file "$file")" "$file"
        done \
      | sha_stream
  )
}

frontmatter_value() {
  local file="$1" key="$2"
  sed -n "s/^[[:space:]]*$key:[[:space:]]*[\"']\{0,1\}\([^\"']*\)[\"']\{0,1\}[[:space:]]*$/\\1/p" "$file" \
    | head -n 1 \
    | tr -d '[:space:]'
}

CARGO_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n 1)"
SKILL_VERSION="$(frontmatter_value "$SKILL_ROOT/SKILL.md" version)"
SKILL_NAME="$(frontmatter_value "$SKILL_ROOT/SKILL.md" name)"
if [ -z "$CARGO_VERSION" ] || [ "$SKILL_NAME" != "chat2db-cli" ] || [ "$SKILL_VERSION" != "$CARGO_VERSION" ]; then
  echo "error: Cargo and Skill identities must match (CLI='$CARGO_VERSION', Skill='$SKILL_NAME@$SKILL_VERSION')" >&2
  exit 1
fi

target_state() {
  local path="$1" link
  if [ -L "$path" ]; then
    printf 'symlink'
  elif [ ! -e "$path" ]; then
    printf 'empty'
  elif [ -e "$path/.git" ] || [ -L "$path/.git" ]; then
    printf 'git'
  elif [ ! -d "$path" ] || [ ! -f "$path/SKILL.md" ] || [ -L "$path/SKILL.md" ]; then
    printf 'foreign'
  else
    link="$(find "$path" -type l -print -quit)"
    if [ -n "$link" ]; then
      printf 'foreign'
    elif [ "$(frontmatter_value "$path/SKILL.md" name)" = "chat2db-cli" ]; then
      printf 'ours'
    else
      printf 'foreign'
    fi
  fi
}

LOCK_DIR="$TARGET_PARENT/.${TARGET_NAME}.chat2db-dev-install.lock"
TX_DIR="$TARGET_PARENT/.${TARGET_NAME}.chat2db-dev-install.tx"
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
    rm -rf "$TARGET_DIR"
    if mv "$TX_DIR/backup" "$TARGET_DIR"; then
      TRANSACTION_REPLACING=0
    else
      echo "error: failed to restore the previous Skill; recovery data preserved at $TX_DIR" >&2
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
      case "$owner_pid" in
        ''|*[!0-9]*) owner_pid="" ;;
      esac
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
      echo "error: timed out waiting for Skill installation lock: $LOCK_DIR" >&2
      return 1
    fi
    sleep 0.1
  done
  printf '%s\n' 'chat2db-dev-install' > "$LOCK_DIR/owner"
  printf '%s\n' "$$" > "$LOCK_DIR/pid"
  LOCK_HELD=1
}

recover_transaction() {
  local recorded_target state expected_hash
  [ -e "$TX_DIR" ] || [ -L "$TX_DIR" ] || return 0
  if [ -L "$TX_DIR" ] || [ ! -d "$TX_DIR" ] || [ ! -f "$TX_DIR/target" ]; then
    echo "error: unrecognized Skill transaction path; refusing to remove it: $TX_DIR" >&2
    return 1
  fi
  recorded_target="$(cat "$TX_DIR/target")"
  if [ "$recorded_target" != "$TARGET_DIR" ]; then
    echo "error: Skill transaction target mismatch at $TX_DIR" >&2
    return 1
  fi
  if [ -e "$TX_DIR/backup" ] || [ -L "$TX_DIR/backup" ]; then
    if [ ! -e "$TARGET_DIR" ] && [ ! -L "$TARGET_DIR" ]; then
      mv "$TX_DIR/backup" "$TARGET_DIR"
      echo "recovered previous Skill: $TARGET_DIR"
    else
      state="$(target_state "$TARGET_DIR")"
      if [ "$state" != "ours" ] || [ ! -f "$TX_DIR/expected-hash" ]; then
        echo "error: cannot reconcile interrupted Skill install; recovery data preserved at $TX_DIR" >&2
        return 1
      fi
      expected_hash="$(cat "$TX_DIR/expected-hash")"
      if [ "$(bundle_hash "$TARGET_DIR")" != "$expected_hash" ]; then
        echo "error: interrupted Skill target hash mismatch; recovery data preserved at $TX_DIR" >&2
        return 1
      fi
      rm -rf "$TX_DIR/backup"
    fi
  fi
  rm -rf "$TX_DIR/pending"
  rm -f "$TX_DIR/target" "$TX_DIR/expected-hash"
  rmdir "$TX_DIR"
}

acquire_lock
recover_transaction

STATE="$(target_state "$TARGET_DIR")"
case "$STATE" in
  empty) ;;
  ours) ;;
  git)
    echo "error: refusing to overwrite git-managed target: $TARGET_DIR" >&2
    exit 1
    ;;
  symlink)
    echo "error: refusing symlink target: $TARGET_DIR" >&2
    exit 1
    ;;
  foreign)
    if [ "$FORCE" != "1" ]; then
      echo "error: target is not the Chat2DB CLI Skill: $TARGET_DIR; pass --force to replace it" >&2
      exit 1
    fi
    ;;
esac

SOURCE_HASH="$(bundle_hash "$SKILL_ROOT" source)"
if [ "$STATE" = "ours" ] && [ "$SOURCE_HASH" = "$(bundle_hash "$TARGET_DIR")" ]; then
  echo "Skill is up to date: $TARGET_DIR ($SOURCE_HASH)"
  exit 0
fi

mkdir "$TX_DIR"
printf '%s\n' "$TARGET_DIR" > "$TX_DIR/target"
printf '%s\n' "$SOURCE_HASH" > "$TX_DIR/expected-hash"
mkdir -p "$TX_DIR/pending/references"
cp "$SKILL_ROOT/SKILL.md" "$TX_DIR/pending/SKILL.md"
for file in "$SKILL_ROOT"/references/*.md; do
  [ -f "$file" ] || continue
  cp "$file" "$TX_DIR/pending/references/"
done

if [ "$(frontmatter_value "$TX_DIR/pending/SKILL.md" name)" != "chat2db-cli" ] || \
   [ "$(frontmatter_value "$TX_DIR/pending/SKILL.md" version)" != "$CARGO_VERSION" ] || \
   [ "$(bundle_hash "$TX_DIR/pending")" != "$SOURCE_HASH" ]; then
  echo "error: staged Skill failed its identity or integrity check" >&2
  exit 1
fi

if [ "$STATE" != "empty" ]; then
  mv "$TARGET_DIR" "$TX_DIR/backup"
  TRANSACTION_REPLACING=1
fi
if ! mv "$TX_DIR/pending" "$TARGET_DIR"; then
  echo "error: failed to activate the local Skill" >&2
  exit 1
fi

if [ "$TRANSACTION_REPLACING" = "1" ]; then
  rm -rf "$TX_DIR/backup"
  TRANSACTION_REPLACING=0
fi
rm -f "$TX_DIR/target" "$TX_DIR/expected-hash"
rmdir "$TX_DIR"

echo "installed Skill for $CLIENT: $TARGET_DIR ($SOURCE_HASH)"
