#!/usr/bin/env bash
# Standalone Chat2DB CLI Skill bundle downloader (curl|bash).
#
# Downloads the Skill bundle (sha256-verified against the published manifest)
# into ./chat2db-cli/ under the current working directory. This script is a
# safer replacement for `git clone` — it does NOT install the bundle into any
# AI client's skill discovery path. After it finishes, place the directory
# wherever you want it to take effect, e.g.:
#
#   mv chat2db-cli ~/.claude/skills/             # user-level (Claude Code)
#   mv chat2db-cli ~/.codex/skills/              # user-level (Codex)
#   chat2db skill install --client claude-code   # install via the CLI/CDN
#
# Independent of the chat2db CLI: the binary does not need to be installed
# for this script to work.
#
# Environment variables:
#   CHAT2DB_SKILL_VERSION    pin a release (default: latest)
#   CHAT2DB_SKILL_MANIFEST_BASE  test-only Release manifest base override
#   CHAT2DB_SKILL_CDN_BASE   CDN base URL (default: https://cdn.chat2db-ai.com)
#   CHAT2DB_SKILL_TARGET     destination directory (default: $PWD/chat2db-cli)
#   CHAT2DB_SKILL_FORCE=1    overwrite an existing target directory
#
# Flags (alternative to env vars):
#   --target <dir>    same as CHAT2DB_SKILL_TARGET
#   --force           same as CHAT2DB_SKILL_FORCE=1
#   -h, --help        show usage
#
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: install-skill.sh [options]

Download the Chat2DB CLI Skill bundle from the official CDN, verify sha256,
and write the clean runtime bundle to a target directory.

Options:
  --target <dir>      Destination directory (default: ./chat2db-cli).
  --force             Replace an existing chat2db-cli skill bundle.
  -h, --help          Show this help.

Environment:
  CHAT2DB_SKILL_VERSION    Pin a release (default: latest).
  CHAT2DB_SKILL_MANIFEST_BASE  Test-only Release manifest base override.
  CHAT2DB_SKILL_CDN_BASE   CDN base URL (default: https://cdn.chat2db-ai.com).
  CHAT2DB_SKILL_TARGET     Destination directory (default: $PWD/chat2db-cli).
  CHAT2DB_SKILL_FORCE=1    Replace an existing target directory.

Examples:
  curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-skill.sh | bash
  curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-skill.sh | bash -s -- --target "$HOME/.codex/skills/chat2db-cli" --force
  curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-skill.sh | bash -s -- --target "$HOME/.claude/skills/chat2db-cli" --force
EOF
}

VERSION="${CHAT2DB_SKILL_VERSION:-latest}"
MANIFEST_BASE="${CHAT2DB_SKILL_MANIFEST_BASE:-https://github.com/OtterMind/Chat2DB-CLI/releases}"
CDN_BASE="${CHAT2DB_SKILL_CDN_BASE:-https://cdn.chat2db-ai.com}"
MANIFEST_BASE="${MANIFEST_BASE%/}"
CDN_BASE="${CDN_BASE%/}"
CDN_REFERER="$CDN_BASE/"
TARGET_DIR="${CHAT2DB_SKILL_TARGET:-$PWD/chat2db-cli}"
FORCE="${CHAT2DB_SKILL_FORCE:-0}"

while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --force) FORCE=1; shift ;;
    --target)
      [ $# -ge 2 ] || { echo "error: --target requires a value" >&2; exit 1; }
      TARGET_DIR="$2"; shift 2 ;;
    --target=*) TARGET_DIR="${1#--target=}"; shift ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

# Canonicalise the parent so HOME aliases and path traversal cannot bypass
# replacement guards. The final component itself must never be a symlink.
while [ "$TARGET_DIR" != "/" ] && [ "${TARGET_DIR%/}" != "$TARGET_DIR" ]; do
  TARGET_DIR="${TARGET_DIR%/}"
done
case "$TARGET_DIR" in
  /*) ;;
  *) TARGET_DIR="$PWD/$TARGET_DIR" ;;
esac
target_name="$(basename "$TARGET_DIR")"
case "$target_name" in
  ''|/|.|..)
    echo "error: unsafe target directory: $TARGET_DIR" >&2
    exit 1
    ;;
esac
if [ -L "$TARGET_DIR" ]; then
  echo "error: refusing symlink target: $TARGET_DIR" >&2
  exit 1
fi
TARGET_PARENT="$(dirname "$TARGET_DIR")"
mkdir -p "$TARGET_PARENT"
TARGET_PARENT="$(cd -P "$TARGET_PARENT" && pwd -P)"
if [ "$TARGET_PARENT" = "/" ]; then
  TARGET_DIR="/$target_name"
else
  TARGET_DIR="$TARGET_PARENT/$target_name"
fi
if [ "$TARGET_DIR" = "/" ]; then
  echo "error: unsafe target directory: $TARGET_DIR" >&2
  exit 1
fi
if [ -n "${HOME:-}" ] && [ -d "$HOME" ]; then
  home_canonical="$(cd -P "$HOME" && pwd -P)"
  if [ "$TARGET_DIR" = "$home_canonical" ]; then
    echo "error: refusing to replace HOME: $TARGET_DIR" >&2
    exit 1
  fi
fi

# --- validate release coordinates ------------------------------------------
is_semver() {
  printf '%s\n' "$1" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
}

validate_url_base() {
  local base="$1" name="$2" require_origin="$3" scheme rest authority path host port
  case "$base" in
    https://*) scheme="https"; rest="${base#https://}" ;;
    http://*)  scheme="http";  rest="${base#http://}" ;;
    *) echo "error: $name must use https" >&2; return 1 ;;
  esac
  case "$rest" in
    ''|*'?'*|*'#'*|*'@'*|*\\*|*'%'*|*[!A-Za-z0-9._~:/-]*)
      echo "error: $name is not a canonical URL base: $base" >&2
      return 1
      ;;
  esac

  authority="${rest%%/*}"
  if [ "$authority" = "$rest" ]; then path=""; else path="/${rest#*/}"; fi
  case "$authority" in
    ''|*:*:*) echo "error: $name has an invalid authority: $base" >&2; return 1 ;;
    *:*)
      host="${authority%%:*}"
      port="${authority#*:}"
      case "$port" in ''|0|0[0-9]*|*[!0-9]*|??????*) echo "error: $name has an invalid port: $base" >&2; return 1 ;; esac
      if [ "$port" -lt 1 ] || [ "$port" -gt 65535 ] || \
         { [ "$scheme" = "https" ] && [ "$port" -eq 443 ]; } || \
         { [ "$scheme" = "http" ] && [ "$port" -eq 80 ]; }; then
        echo "error: $name has a non-canonical port: $base" >&2
        return 1
      fi
      ;;
    *) host="$authority" ;;
  esac
  case "$host" in
    ''|*[A-Z]*|.*|*.|*..*|-*|*-|*[!a-z0-9.-]*)
      echo "error: $name has a non-canonical host: $base" >&2
      return 1
      ;;
  esac
  if [ "$scheme" = "http" ]; then
    case "$host" in 127.0.0.1|localhost) ;; *) echo "error: $name permits http only for loopback tests" >&2; return 1 ;; esac
  fi
  case "$path" in
    *//*|*/./*|*/../*|*/.|*/..|*[!A-Za-z0-9._~/-]*)
      echo "error: $name has a non-canonical path: $base" >&2
      return 1
      ;;
  esac
  if [ "$require_origin" = "1" ] && [ -n "$path" ]; then
    echo "error: $name must be an origin without a path: $base" >&2
    return 1
  fi
}

if [ "$VERSION" != "latest" ] && ! is_semver "$VERSION"; then
  echo "error: invalid release version: $VERSION" >&2
  exit 1
fi
validate_url_base "$MANIFEST_BASE" "CHAT2DB_SKILL_MANIFEST_BASE" 0
validate_url_base "$CDN_BASE" "CHAT2DB_SKILL_CDN_BASE" 1

if [ "$VERSION" = "latest" ]; then
  MANIFEST_URL="$MANIFEST_BASE/latest/download/manifest-skill.json"
else
  MANIFEST_URL="$MANIFEST_BASE/download/v$VERSION/manifest-skill.json"
fi
MANIFEST_SCHEME="${MANIFEST_URL%%://*}"
CDN_SCHEME="${CDN_BASE%%://*}"

# --- http helpers (mirror install-cli.sh) -----------------------------------
have_curl=0; have_wget=0
command -v curl >/dev/null 2>&1 && have_curl=1
command -v wget >/dev/null 2>&1 && have_wget=1
if [ "$have_curl" -eq 0 ] && [ "$have_wget" -eq 0 ]; then
  echo "error: neither curl nor wget is installed; cannot fetch release artifacts" >&2
  exit 1
fi

http_get_manifest() {
  local url="$1"
  if [ "$have_curl" -eq 1 ]; then
    curl --fail --silent --show-error --location --retry 3 --retry-delay 2 \
      --max-redirs 5 --proto "=$MANIFEST_SCHEME" --proto-redir "=$MANIFEST_SCHEME" "$url"
  else
    wget --tries=3 --waitretry=2 --max-redirect=5 -qO- "$url"
  fi
}

http_download() {
  local url="$1" dest="$2"
  if [ "$have_curl" -eq 1 ]; then
    curl --fail --silent --show-error --location --retry 3 --retry-delay 2 \
      --max-redirs 5 --proto "=$CDN_SCHEME" --proto-redir "=$CDN_SCHEME" \
      --header "Referer: $CDN_REFERER" -o "$dest" "$url"
  else
    wget --tries=3 --waitretry=2 --max-redirect=5 --header="Referer: $CDN_REFERER" -qO "$dest" "$url"
  fi
}

# --- fetch + parse manifest --------------------------------------------------
echo "fetching skill manifest: $MANIFEST_URL"
manifest_json="$(http_get_manifest "$MANIFEST_URL")" || {
  echo "error: failed to fetch manifest at $MANIFEST_URL" >&2
  exit 1
}

# The Release workflow emits one string field per line. Requiring one exact
# match rejects ambiguous or duplicate fields without adding jq as a dependency.
manifest_field() {
  local field="$1" values count
  values="$(printf '%s\n' "$manifest_json" | sed -n "s/^[[:space:]]*\"$field\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\"[[:space:]]*,\{0,1\}[[:space:]]*$/\\1/p")"
  count="$(printf '%s\n' "$values" | sed '/^$/d' | wc -l | tr -d '[:space:]')"
  if [ "$count" != "1" ]; then
    echo "error: manifest must contain exactly one string field '$field'" >&2
    return 1
  fi
  case "$values" in *\\*) echo "error: manifest field '$field' must not use escapes" >&2; return 1 ;; esac
  printf '%s' "$values"
}

skill_version="$(manifest_field skill_version)" || exit 1
bundle_path="$(manifest_field bundle_path)" || exit 1
artifact_url="$(manifest_field url)" || exit 1
sha256_hex="$(manifest_field sha256)" || exit 1
artifact_type="$(manifest_field artifact_type)" || exit 1

if ! is_semver "$skill_version" || \
   { [ "$VERSION" != "latest" ] && [ "$skill_version" != "$VERSION" ]; }; then
  echo "error: manifest version '$skill_version' does not match requested release '$VERSION'" >&2
  exit 1
fi
if [ "$bundle_path" != "chat2db-cli" ]; then
  echo "error: manifest bundle_path must be 'chat2db-cli'" >&2
  exit 1
fi
case "$sha256_hex" in
  ???????????????????????????????????????????????????????????????? ) ;;
  *) echo "error: manifest sha256 must contain 64 lowercase hexadecimal characters" >&2; exit 1 ;;
esac
case "$sha256_hex" in *[!0-9a-f]*) echo "error: manifest sha256 must contain 64 lowercase hexadecimal characters" >&2; exit 1 ;; esac
if [ "$artifact_type" != "skill_zip" ]; then
  echo "error: manifest artifact_type must be 'skill_zip'" >&2
  exit 1
fi

archive_name="chat2db-skill-$skill_version.zip"
expected_artifact_url="$CDN_BASE/download/chat2db-skill/$skill_version/$archive_name"
if [ "$artifact_url" != "$expected_artifact_url" ]; then
  echo "error: unofficial_artifact_url: $artifact_url" >&2
  echo "  expected: $expected_artifact_url" >&2
  exit 1
fi

# --- download + sha256 verify -----------------------------------------------
TMPDIR_STAGE="$(mktemp -d)"
transaction_pending=""
transaction_backup=""
transaction_replacing=0
cleanup() {
  local status=$?
  trap - EXIT HUP INT TERM
  if [ "$transaction_replacing" = "1" ] && [ -n "$transaction_backup" ] && [ -e "$transaction_backup" ]; then
    if rm -rf "$TARGET_DIR" && mv "$transaction_backup" "$TARGET_DIR"; then
      transaction_replacing=0
    else
      echo "error: failed to restore the previous Skill; backup preserved at $transaction_backup" >&2
      status=1
    fi
  fi
  if [ -n "$transaction_pending" ]; then
    rm -rf "$transaction_pending"
  fi
  if [ "$transaction_replacing" = "0" ] && [ -n "$transaction_backup" ]; then
    rm -rf "$transaction_backup"
  fi
  rm -rf "$TMPDIR_STAGE"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

zipfile="$TMPDIR_STAGE/$archive_name"
echo "downloading: $artifact_url"
http_download "$artifact_url" "$zipfile"

if command -v shasum >/dev/null 2>&1; then
  actual_sha="$(shasum -a 256 "$zipfile" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  actual_sha="$(sha256sum "$zipfile" | awk '{print $1}')"
else
  echo "error: neither shasum nor sha256sum is available; cannot verify download" >&2
  exit 1
fi

if [ "$actual_sha" != "$sha256_hex" ]; then
  echo "error: checksum_mismatch" >&2
  echo "  expected: $sha256_hex" >&2
  echo "  actual:   $actual_sha" >&2
  exit 1
fi

# --- extract -----------------------------------------------------------------
if ! command -v unzip >/dev/null 2>&1; then
  echo "error: unzip is not installed; cannot extract skill bundle" >&2
  exit 1
fi
extract_dir="$TMPDIR_STAGE/extract"
mkdir -p "$extract_dir"

# Accept only the files produced by scripts/package-skill.sh. Validate before
# extraction so a compromised archive cannot write outside the staging area.
while IFS= read -r entry; do
  case "$entry" in
    /*|*..*|*\\*)
      echo "error: skill_bundle_corrupt: unsafe zip entry: $entry" >&2
      exit 1
      ;;
    "$bundle_path/"|"$bundle_path/references/")
      ;;
    "$bundle_path/SKILL.md")
      ;;
    "$bundle_path/references/"*.md)
      reference_name="${entry#"$bundle_path/references/"}"
      case "$reference_name" in
        ''|*/*)
          echo "error: skill_bundle_corrupt: unexpected zip entry: $entry" >&2
          exit 1
          ;;
      esac
      ;;
    "LICENSE"|"THIRD_PARTY_LICENSES.txt")
      ;;
    *)
      echo "error: skill_bundle_corrupt: unexpected zip entry: $entry" >&2
      exit 1
      ;;
  esac
done < <(unzip -Z1 "$zipfile")

unzip -q "$zipfile" -d "$extract_dir"

bundle_root="$extract_dir/$bundle_path"
if [ ! -s "$extract_dir/LICENSE" ] || [ ! -s "$extract_dir/THIRD_PARTY_LICENSES.txt" ]; then
  echo "error: skill_bundle_corrupt: zip is missing non-empty license notices" >&2
  exit 1
fi
if [ ! -d "$bundle_root" ] || [ ! -s "$bundle_root/SKILL.md" ]; then
  echo "error: skill_bundle_corrupt: zip missing $bundle_path/SKILL.md" >&2
  exit 1
fi

# Sanity: bundle's frontmatter `name:` must be chat2db-cli. Otherwise we'd
# silently drop something unrelated under our canonical directory name.
staged_name="$(sed -n 's/^[[:space:]]*name:[[:space:]]*\(.*\)/\1/p' "$bundle_root/SKILL.md" | head -n1 | tr -d '"' | tr -d "'" | tr -d '[:space:]')"
if [ "$staged_name" != "chat2db-cli" ]; then
  echo "error: skill_bundle_corrupt: SKILL.md frontmatter name is '$staged_name', expected 'chat2db-cli'" >&2
  exit 1
fi
staged_version="$(sed -n 's/^[[:space:]]*version:[[:space:]]*\(.*\)/\1/p' "$bundle_root/SKILL.md" | head -n1 | tr -d '"' | tr -d "'" | tr -d '[:space:]')"
if [ "$staged_version" != "$skill_version" ]; then
  echo "error: skill_bundle_corrupt: SKILL.md version '$staged_version' does not match manifest '$skill_version'" >&2
  exit 1
fi

# --- classify existing target -----------------------------------------------
state="empty"
if [ -L "$TARGET_DIR" ]; then
  state="symlink"
elif [ -e "$TARGET_DIR" ]; then
  if [ ! -d "$TARGET_DIR" ]; then
    state="foreign"
  elif [ -e "$TARGET_DIR/.git" ] || [ -L "$TARGET_DIR/.git" ]; then
    state="git"
  elif [ ! -f "$TARGET_DIR/SKILL.md" ]; then
    state="foreign"
  else
    existing_name="$(sed -n 's/^name:[[:space:]]*\(.*\)/\1/p' "$TARGET_DIR/SKILL.md" | head -n1 | tr -d '"' | tr -d "'" | tr -d '[:space:]')"
    if [ "$existing_name" = "chat2db-cli" ]; then
      state="ours"
    else
      state="foreign"
    fi
  fi
fi

case "$state" in
  empty) ;;
  git)
    echo "error: skill_target_managed_externally: $TARGET_DIR contains .git" >&2
    echo "  this looks like a git checkout; run 'git -C $TARGET_DIR pull' to update it instead" >&2
    echo "  --force will not override this protection" >&2
    exit 1 ;;
  symlink)
    echo "error: skill_target_managed_externally: refusing symlink target $TARGET_DIR" >&2
    echo "  --force will not override this protection" >&2
    exit 1 ;;
  ours)
    if [ "$FORCE" != "1" ]; then
      echo "error: skill_already_installed: $TARGET_DIR" >&2
      echo "  pass --force or set CHAT2DB_SKILL_FORCE=1 to overwrite" >&2
      exit 1
    fi ;;
  foreign)
    if [ "$FORCE" != "1" ]; then
      echo "error: skill_target_foreign: $TARGET_DIR exists and is not the chat2db-cli skill" >&2
      echo "  pass --force or set CHAT2DB_SKILL_FORCE=1 to replace it, or pass --target <dir> to write somewhere else" >&2
      exit 1
    fi ;;
esac

# --- transactionally replace target -----------------------------------------
mkdir -p "$TARGET_PARENT"
transaction_pending="$(mktemp -d "$TARGET_PARENT/.chat2db-cli.pending.XXXXXX")"
rmdir "$transaction_pending"
mv "$bundle_root" "$transaction_pending"

if [ ! -f "$transaction_pending/SKILL.md" ]; then
  echo "error: failed to stage the Skill in its destination filesystem" >&2
  exit 1
fi
pending_name="$(sed -n 's/^[[:space:]]*name:[[:space:]]*\(.*\)/\1/p' "$transaction_pending/SKILL.md" | head -n1 | tr -d '"' | tr -d "'" | tr -d '[:space:]')"
pending_version="$(sed -n 's/^[[:space:]]*version:[[:space:]]*\(.*\)/\1/p' "$transaction_pending/SKILL.md" | head -n1 | tr -d '"' | tr -d "'" | tr -d '[:space:]')"
if [ "$pending_name" != "chat2db-cli" ] || [ "$pending_version" != "$skill_version" ]; then
  echo "error: staged Skill failed its pre-installation identity check" >&2
  exit 1
fi

if [ "$state" = "ours" ] || [ "$state" = "foreign" ]; then
  transaction_backup="$transaction_pending.backup"
  if [ -e "$transaction_backup" ]; then
    echo "error: temporary backup path already exists: $transaction_backup" >&2
    exit 1
  fi
  transaction_replacing=1
  mv "$TARGET_DIR" "$transaction_backup"
fi

if ! mv "$transaction_pending" "$TARGET_DIR"; then
  echo "error: failed to activate the downloaded Skill" >&2
  exit 1
fi
transaction_pending=""

if [ "$transaction_replacing" = "1" ]; then
  rm -rf "$transaction_backup"
  transaction_backup=""
  transaction_replacing=0
fi

if [ "$state" = "ours" ]; then
  echo "updated: $TARGET_DIR"
else
  echo "downloaded: $TARGET_DIR"
fi
echo

codex_target="${HOME:-}/.codex/skills/chat2db-cli"
claude_target="${HOME:-}/.claude/skills/chat2db-cli"
case "$TARGET_DIR" in
  "$codex_target")
    echo "installed for Codex: $TARGET_DIR" ;;
  "$claude_target")
    echo "installed for Claude Code: $TARGET_DIR" ;;
  *)
    echo "next: place the bundle where you want it active, e.g."
    echo "  mv \"$TARGET_DIR\" ~/.claude/skills/           # Claude Code"
    echo "  mv \"$TARGET_DIR\" ~/.codex/skills/            # Codex"
    echo "  chat2db skill install --client codex          # via the CLI/CDN" ;;
esac
