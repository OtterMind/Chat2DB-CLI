#!/bin/sh
# install-cli.sh - canonical Chat2DB CLI release-binary installer (curl|sh).
#
# Supported targets:
#   darwin/arm64   darwin/x86_64   linux/x86_64   linux/arm64
#
# Environment variables:
#   CHAT2DB_CLI_VERSION            pin a release (default: latest)
#   CHAT2DB_CLI_MANIFEST_BASE      test-only Release manifest base override
#   CHAT2DB_CLI_CDN_BASE           CDN base URL (default: https://cdn.chat2db-ai.com)
#   CHAT2DB_CLI_BIN_DIR            install destination dir (default: $HOME/.local/bin)
#   CHAT2DB_CLI_FORCE=1            overwrite a non-Chat2DB binary at the destination
#   CHAT2DB_CLI_NO_MODIFY_PATH=1   skip shell rc PATH modification
#   CHAT2DB_CLI_NO_PATH_WARN=1     legacy: suppress PATH warning and skip rc PATH modification
#   CHAT2DB_CLI_NO_COMPLETIONS=1   skip auto-installing shell completions
#
# Positional flags (alternative to env vars):
#   --force             same as CHAT2DB_CLI_FORCE=1
#   --no-modify-path    same as CHAT2DB_CLI_NO_MODIFY_PATH=1
#
set -eu

if [ -z "${HOME:-}" ] && [ -z "${CHAT2DB_CLI_BIN_DIR:-}" ]; then
  echo "error: HOME is not set; set CHAT2DB_CLI_BIN_DIR to choose an install directory" >&2
  exit 1
fi

VERSION="${CHAT2DB_CLI_VERSION:-latest}"
MANIFEST_BASE="${CHAT2DB_CLI_MANIFEST_BASE:-https://github.com/OtterMind/Chat2DB-CLI/releases}"
CDN_BASE="${CHAT2DB_CLI_CDN_BASE:-https://cdn.chat2db-ai.com}"
MANIFEST_BASE="${MANIFEST_BASE%/}"
CDN_BASE="${CDN_BASE%/}"
CDN_REFERER="$CDN_BASE/"
BIN_DIR="${CHAT2DB_CLI_BIN_DIR:-$HOME/.local/bin}"
BIN_PATH="$BIN_DIR/chat2db"
FORCE="${CHAT2DB_CLI_FORCE:-0}"
NO_MODIFY_PATH="${CHAT2DB_CLI_NO_MODIFY_PATH:-0}"
NO_PATH_WARN="${CHAT2DB_CLI_NO_PATH_WARN:-0}"

for arg in "$@"; do
  case "$arg" in
    --force) FORCE=1 ;;
    --no-modify-path) NO_MODIFY_PATH=1 ;;
    *) echo "error: unknown argument: $arg" >&2; exit 1 ;;
  esac
done

# --- detect os/arch -----------------------------------------------------------
uname_s="$(uname -s)"
uname_m="$(uname -m)"
case "$uname_s" in
  Darwin)
    OS="darwin"
    case "$uname_m" in
      arm64|aarch64) ARCH="arm64" ;;
      x86_64) ARCH="x86_64" ;;
      *) echo "error: unsupported macOS arch: $uname_m" >&2; exit 1 ;;
    esac
    ;;
  Linux)
    OS="linux"
    case "$uname_m" in
      x86_64) ARCH="x86_64" ;;
      aarch64|arm64) ARCH="arm64" ;;
      *) echo "error: unsupported Linux arch: $uname_m" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "error: unsupported OS: $uname_s" >&2
    exit 1
    ;;
esac

# --- validate release coordinates -------------------------------------------
is_semver() {
  printf '%s\n' "$1" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
}

validate_url_base() {
  _base="$1"
  _name="$2"
  _require_origin="$3"

  case "$_base" in
    https://*) _scheme="https"; _rest="${_base#https://}" ;;
    http://*)  _scheme="http";  _rest="${_base#http://}" ;;
    *) echo "error: $_name must use https" >&2; return 1 ;;
  esac
  case "$_rest" in
    ''|*'?'*|*'#'*|*'@'*|*\\*|*'%'*|*[!A-Za-z0-9._~:/-]*)
      echo "error: $_name is not a canonical URL base: $_base" >&2
      return 1
      ;;
  esac

  _authority="${_rest%%/*}"
  if [ "$_authority" = "$_rest" ]; then _path=""; else _path="/${_rest#*/}"; fi
  case "$_authority" in
    ''|*:*:*) echo "error: $_name has an invalid authority: $_base" >&2; return 1 ;;
    *:*)
      _host="${_authority%%:*}"
      _port="${_authority#*:}"
      case "$_port" in ''|0|0[0-9]*|*[!0-9]*|??????*) echo "error: $_name has an invalid port: $_base" >&2; return 1 ;; esac
      if [ "$_port" -lt 1 ] || [ "$_port" -gt 65535 ] || \
         { [ "$_scheme" = "https" ] && [ "$_port" -eq 443 ]; } || \
         { [ "$_scheme" = "http" ] && [ "$_port" -eq 80 ]; }; then
        echo "error: $_name has a non-canonical port: $_base" >&2
        return 1
      fi
      ;;
    *) _host="$_authority" ;;
  esac
  case "$_host" in
    ''|*[A-Z]*|.*|*.|*..*|-*|*-|*[!a-z0-9.-]*)
      echo "error: $_name has a non-canonical host: $_base" >&2
      return 1
      ;;
  esac
  if [ "$_scheme" = "http" ]; then
    case "$_host" in 127.0.0.1|localhost) ;; *) echo "error: $_name permits http only for loopback tests" >&2; return 1 ;; esac
  fi
  case "$_path" in
    *//*|*/./*|*/../*|*/.|*/..|*[!A-Za-z0-9._~/-]*)
      echo "error: $_name has a non-canonical path: $_base" >&2
      return 1
      ;;
  esac
  if [ "$_require_origin" = "1" ] && [ -n "$_path" ]; then
    echo "error: $_name must be an origin without a path: $_base" >&2
    return 1
  fi
}

if [ "$VERSION" != "latest" ] && ! is_semver "$VERSION"; then
  echo "error: invalid release version: $VERSION" >&2
  exit 1
fi
validate_url_base "$MANIFEST_BASE" "CHAT2DB_CLI_MANIFEST_BASE" 0
validate_url_base "$CDN_BASE" "CHAT2DB_CLI_CDN_BASE" 1

manifest_name="manifest-cli-$OS-$ARCH.json"
if [ "$VERSION" = "latest" ]; then
  MANIFEST_URL="$MANIFEST_BASE/latest/download/$manifest_name"
else
  MANIFEST_URL="$MANIFEST_BASE/download/v$VERSION/$manifest_name"
fi
MANIFEST_SCHEME="${MANIFEST_URL%%://*}"
CDN_SCHEME="${CDN_BASE%%://*}"

# --- http helpers -------------------------------------------------------------
have_curl=0
have_wget=0
command -v curl >/dev/null 2>&1 && have_curl=1
command -v wget >/dev/null 2>&1 && have_wget=1
if [ "$have_curl" -eq 0 ] && [ "$have_wget" -eq 0 ]; then
  echo "error: neither curl nor wget is installed; cannot fetch release artifacts" >&2
  exit 1
fi

http_get_manifest() {
  _url="$1"
  if [ "$have_curl" -eq 1 ]; then
    curl --fail --silent --show-error --location --retry 3 --retry-delay 2 \
      --max-redirs 5 --proto "=$MANIFEST_SCHEME" --proto-redir "=$MANIFEST_SCHEME" "$_url"
  else
    wget --tries=3 --waitretry=2 --max-redirect=5 -qO- "$_url"
  fi
}

http_download() {
  _url="$1"
  _dest="$2"
  if [ "$have_curl" -eq 1 ]; then
    curl --fail --silent --show-error --location --retry 3 --retry-delay 2 \
      --max-redirs 5 --proto "=$CDN_SCHEME" --proto-redir "=$CDN_SCHEME" \
      --header "Referer: $CDN_REFERER" -o "$_dest" "$_url"
  else
    wget --tries=3 --waitretry=2 --max-redirect=5 --header="Referer: $CDN_REFERER" -qO "$_dest" "$_url"
  fi
}

# --- fetch + parse manifest ---------------------------------------------------
echo "fetching manifest: $MANIFEST_URL"
manifest_json="$(http_get_manifest "$MANIFEST_URL")" || {
  echo "error: failed to fetch manifest at $MANIFEST_URL" >&2
  exit 1
}

# The Release workflow emits one string field per line. Requiring one exact
# match rejects ambiguous or duplicate fields without adding jq as a dependency.
manifest_field() {
  _field="$1"
  _values="$(printf '%s\n' "$manifest_json" | sed -n "s/^[[:space:]]*\"$_field\"[[:space:]]*:[[:space:]]*\"\([^\"]*\)\"[[:space:]]*,\{0,1\}[[:space:]]*$/\\1/p")"
  _count="$(printf '%s\n' "$_values" | sed '/^$/d' | wc -l | tr -d '[:space:]')"
  if [ "$_count" != "1" ]; then
    echo "error: manifest must contain exactly one string field '$_field'" >&2
    return 1
  fi
  case "$_values" in *\\*) echo "error: manifest field '$_field' must not use escapes" >&2; return 1 ;; esac
  printf '%s' "$_values"
}

manifest_version="$(manifest_field version)" || exit 1
artifact_url="$(manifest_field url)" || exit 1
sha256_hex="$(manifest_field sha256)" || exit 1
artifact_type="$(manifest_field artifact_type)" || exit 1

if ! is_semver "$manifest_version" || \
   { [ "$VERSION" != "latest" ] && [ "$manifest_version" != "$VERSION" ]; }; then
  echo "error: manifest version '$manifest_version' does not match requested release '$VERSION'" >&2
  exit 1
fi
case "$sha256_hex" in
  ???????????????????????????????????????????????????????????????? ) ;;
  *) echo "error: manifest sha256 must contain 64 lowercase hexadecimal characters" >&2; exit 1 ;;
esac
case "$sha256_hex" in *[!0-9a-f]*) echo "error: manifest sha256 must contain 64 lowercase hexadecimal characters" >&2; exit 1 ;; esac
if [ "$artifact_type" != "tarball" ]; then
  echo "error: manifest artifact_type '$artifact_type' is invalid for $OS/$ARCH" >&2
  exit 1
fi

archive_name="chat2db-$manifest_version-$OS-$ARCH.tar.gz"
expected_artifact_url="$CDN_BASE/download/chat2db-cli/$manifest_version/$OS/$ARCH/$archive_name"
if [ "$artifact_url" != "$expected_artifact_url" ]; then
  echo "error: unofficial_artifact_url: $artifact_url" >&2
  echo "  expected: $expected_artifact_url" >&2
  exit 1
fi

# --- download to tempdir + verify --------------------------------------------
TMPDIR_STAGE="$(mktemp -d)"
install_pending=""
install_backup=""
install_replacing=0
cleanup() {
  _status=$?
  trap - EXIT HUP INT TERM
  if [ "$install_replacing" = "1" ] && [ -n "$install_backup" ] && [ -e "$install_backup" ]; then
    rm -f "$BIN_PATH"
    if mv "$install_backup" "$BIN_PATH"; then
      install_replacing=0
    else
      echo "error: failed to restore the previous CLI; backup preserved at $install_backup" >&2
      _status=1
    fi
  fi
  if [ -n "$install_pending" ]; then
    rm -f "$install_pending"
  fi
  if [ "$install_replacing" = "0" ] && [ -n "$install_backup" ]; then
    rm -f "$install_backup"
  fi
  rm -rf "$TMPDIR_STAGE"
  exit "$_status"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

tarball="$TMPDIR_STAGE/$archive_name"
echo "downloading: $artifact_url"
http_download "$artifact_url" "$tarball"

if command -v shasum >/dev/null 2>&1; then
  actual_sha="$(shasum -a 256 "$tarball" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  actual_sha="$(sha256sum "$tarball" | awk '{print $1}')"
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

# --- extract ------------------------------------------------------------------
archive_entries="$(tar -tzf "$tarball")" || {
  echo "error: release archive is not a valid gzip-compressed tar archive" >&2
  exit 1
}
expected_archive_entries="$(printf '%s\n' LICENSE THIRD_PARTY_LICENSES.txt chat2db)"
if [ "$archive_entries" != "$expected_archive_entries" ]; then
  echo "error: release archive must contain only chat2db, LICENSE, and THIRD_PARTY_LICENSES.txt at the root" >&2
  exit 1
fi
tar -xzf "$tarball" -C "$TMPDIR_STAGE"
staged="$TMPDIR_STAGE/chat2db"
if [ ! -f "$staged" ] || [ -L "$staged" ]; then
  echo "error: tarball did not contain a 'chat2db' binary at the root" >&2
  exit 1
fi
chmod 0755 "$staged"

# --- refuse to overwrite a non-Chat2DB binary ---------------------------------
mkdir -p "$BIN_DIR"
if [ -d "$BIN_PATH" ]; then
  echo "error: $BIN_PATH is a directory and cannot be replaced" >&2
  exit 1
fi
if [ -e "$BIN_PATH" ] || [ -L "$BIN_PATH" ]; then
  existing_version="$("$BIN_PATH" version 2>/dev/null || true)"
  case "$existing_version" in
    chat2db\ *) ;;
    *)
      if [ "$FORCE" != "1" ]; then
        echo "error: $BIN_PATH exists and is not a Chat2DB binary; pass --force or set CHAT2DB_CLI_FORCE=1 to overwrite" >&2
        exit 1
      fi
      ;;
  esac
fi

# --- transactionally install -------------------------------------------------
install_pending="$(mktemp "$BIN_DIR/.chat2db.pending.XXXXXX")"
install -m 0755 "$staged" "$install_pending"
if ! "$install_pending" version >/dev/null 2>&1; then
  echo "error: downloaded CLI failed its pre-installation version check" >&2
  exit 1
fi

if [ -e "$BIN_PATH" ] || [ -L "$BIN_PATH" ]; then
  install_backup="$install_pending.backup"
  if [ -e "$install_backup" ] || [ -L "$install_backup" ]; then
    echo "error: temporary backup path already exists: $install_backup" >&2
    exit 1
  fi
  install_replacing=1
  mv "$BIN_PATH" "$install_backup"
fi

if ! mv "$install_pending" "$BIN_PATH"; then
  echo "error: failed to activate the downloaded CLI" >&2
  exit 1
fi
install_pending=""

if [ "$install_replacing" = "1" ]; then
  rm -f "$install_backup"
  install_backup=""
  install_replacing=0
fi

# --- PATH check + shell rc wiring --------------------------------------------
PATH_MARKER_BEGIN="# >>> chat2db PATH >>>"
PATH_MARKER_END="# <<< chat2db PATH <<<"

escape_double_quoted() {
  printf '%s' "$1" | sed "s/\\\\/\\\\\\\\/g; s/\"/\\\\\"/g; s/\\\$/\\\\\\\$/g; s/\`/\\\\\`/g"
}

path_rc_value() {
  if [ -n "${HOME:-}" ] && [ "$BIN_DIR" = "$HOME/.local/bin" ]; then
    printf '\044HOME/.local/bin'
  else
    escape_double_quoted "$BIN_DIR"
  fi
}

path_export_line() {
  printf 'export PATH="%s:\044PATH"' "$(path_rc_value)"
}

fish_path_line() {
  printf 'fish_add_path "%s"' "$(path_rc_value)"
}

append_path_block() {
  _rc_file="$1"
  _path_line="$2"

  if [ -f "$_rc_file" ] && grep -F "$PATH_MARKER_BEGIN" "$_rc_file" >/dev/null 2>&1; then
    return 1
  fi

  _rc_parent="$(dirname "$_rc_file")"
  mkdir -p "$_rc_parent"
  if [ -f "$_rc_file" ] && [ -s "$_rc_file" ]; then
    _last_char="$(tail -c 1 "$_rc_file" 2>/dev/null || true)"
    if [ -n "$_last_char" ]; then
      printf '\n' >> "$_rc_file"
    fi
  fi

  {
    printf '%s\n' "$PATH_MARKER_BEGIN"
    printf '%s\n' "$_path_line"
    printf '%s\n' "$PATH_MARKER_END"
  } >> "$_rc_file"
}

print_path_hint() {
  _shell_name="$1"
  case "$_shell_name" in
    fish) _manual_line="$(fish_path_line)" ;;
    *) _manual_line="$(path_export_line)" ;;
  esac
  echo "path: open a new terminal, or run:"
  echo "  $_manual_line"
}

exported_shell() {
  env | sed -n 's/^SHELL=//p' | head -n 1
}

configure_path() {
  case ":${PATH:-}:" in
    *":$BIN_DIR:"*) return 0 ;;
  esac

  _shell_name=""
  _shell_env="$(exported_shell)"
  if [ -n "$_shell_env" ]; then
    _shell_name="$(basename "$_shell_env")"
  fi

  if [ "$NO_PATH_WARN" = "1" ]; then
    return 0
  fi

  if [ "$NO_MODIFY_PATH" = "1" ]; then
    echo "path: $BIN_DIR is not in PATH; shell rc not modified"
    print_path_hint "$_shell_name"
    return 0
  fi

  if [ -z "${HOME:-}" ]; then
    echo "path: $BIN_DIR is not in PATH; HOME is not set, so shell rc was not modified"
    print_path_hint "$_shell_name"
    return 0
  fi

  case "$_shell_name" in
    zsh)
      _rc_file="$HOME/.zshrc"
      _path_line="$(path_export_line)"
      ;;
    bash)
      _rc_file="$HOME/.bashrc"
      _path_line="$(path_export_line)"
      ;;
    fish)
      _rc_file="$HOME/.config/fish/config.fish"
      _path_line="$(fish_path_line)"
      ;;
    *)
      echo "path: $BIN_DIR is not in PATH; add it manually"
      print_path_hint ""
      return 0
      ;;
  esac

  if append_path_block "$_rc_file" "$_path_line"; then
    echo "path: added $BIN_DIR to $_rc_file"
  else
    echo "path: already configured in $_rc_file"
  fi
  print_path_hint "$_shell_name"
}

"$BIN_PATH" version
echo "installed: $BIN_PATH"
configure_path

# --- shell completions (best-effort, same pattern as dev-install.sh) ---------
# Failures are non-fatal so a broken rc never blocks installing the binary.
completion_shell="$(exported_shell)"
if [ "${CHAT2DB_CLI_NO_COMPLETIONS:-0}" != "1" ] && [ -n "$completion_shell" ]; then
  case "$(basename "$completion_shell")" in
    zsh|bash|fish)
      completion_shell_name="$(basename "$completion_shell")"
      if "$BIN_PATH" completions --install >/dev/null 2>&1; then
        echo "completions: installed for $completion_shell_name"
      fi
      ;;
  esac
fi
