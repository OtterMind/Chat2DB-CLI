#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="${CHAT2DB_CLI_BIN_DIR:-"$HOME/.local/bin"}"
BIN_PATH="$BIN_DIR/chat2db"
WITH_SKILLS="${CHAT2DB_CLI_WITH_SKILLS:-0}"

for arg in "$@"; do
  case "$arg" in
    --with-skills) WITH_SKILLS=1 ;;
    *) echo "error: unknown argument: $arg" >&2; exit 1 ;;
  esac
done

if [ ! -e "$BIN_PATH" ]; then
  echo "already absent: $BIN_PATH"
else
  if [ ! -f "$BIN_PATH" ]; then
    echo "error: refusing to remove non-file path: $BIN_PATH" >&2
    exit 1
  fi

  VERSION_OUTPUT="$("$BIN_PATH" version 2>/dev/null || true)"
  case "$VERSION_OUTPUT" in
    chat2db\ *) ;;
    *)
      echo "error: refusing to remove $BIN_PATH because it does not look like Chat2DB CLI" >&2
      exit 1
      ;;
  esac

  rm -f "$BIN_PATH"
  echo "removed: $BIN_PATH"
fi

if [ "$WITH_SKILLS" = "1" ]; then
  SKILLS_DIR="$HOME/.claude/skills/chat2db-cli"
  SKILL_MD="$SKILLS_DIR/SKILL.md"

  if [ ! -d "$SKILLS_DIR" ]; then
    echo "skills: nothing to remove (not present at $SKILLS_DIR)"
  elif [ -d "$SKILLS_DIR/.git" ]; then
    echo "warning: $SKILLS_DIR contains a .git directory; refusing to delete (looks like a working clone)" >&2
  elif [ ! -f "$SKILL_MD" ]; then
    echo "warning: $SKILL_MD missing; refusing to delete $SKILLS_DIR" >&2
  elif ! grep -E '^name:[[:space:]]*"?chat2db-cli"?[[:space:]]*$' "$SKILL_MD" >/dev/null 2>&1; then
    echo "warning: $SKILL_MD frontmatter does not declare name: chat2db-cli; refusing to delete $SKILLS_DIR" >&2
  else
    rm -rf "$SKILLS_DIR"
    echo "removed: $SKILLS_DIR"
  fi
fi
