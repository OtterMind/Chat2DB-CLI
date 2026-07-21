#!/usr/bin/env bash
# package-cli.sh — produce a clean chat2db CLI release archive + manifest stub.
#
# Output (under dist/, ignored by git):
#   dist/chat2db-<version>-<os>-<arch>.tar.gz   (Unix CLI and license notices)
#   dist/chat2db-<version>-<os>-<arch>.zip      (Windows CLI and license notices)
#   dist/manifest-<version>-<os>-<arch>.json    (BinaryManifest stub; URL has __CDN_BASE__ placeholder)
#
# Hygiene: the release archive MUST contain exactly the CLI binary, LICENSE,
# and THIRD_PARTY_LICENSES.txt at the archive root. Installers validate this
# fixed shape before extracting the binary.
#
# Usage:
#   bash scripts/package-cli.sh                 (host os/arch)
#   bash scripts/package-cli.sh --target X      (cargo --target X for cross-compile)
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"
export COPYFILE_DISABLE=1

# Reproducible-build timestamp. Zero is a stable default and is represented as
# 1980-01-01 in ZIP files because the format cannot encode earlier dates.
SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
export SOURCE_DATE_EPOCH

PYTHON_BIN="${PYTHON_BIN:-}"
if [ -z "$PYTHON_BIN" ]; then
  if command -v python3 >/dev/null 2>&1; then
    PYTHON_BIN=python3
  elif command -v python >/dev/null 2>&1; then
    PYTHON_BIN=python
  else
    echo "error: python3 or python is required" >&2
    exit 1
  fi
fi
CARGO_BIN="${CARGO_BIN:-cargo}"

for legal_file in LICENSE THIRD_PARTY_LICENSES.txt; do
  if [ ! -s "$legal_file" ]; then
    echo "error: required legal file is missing or empty: $legal_file" >&2
    exit 1
  fi
done

CARGO_TARGET=""
while [ $# -gt 0 ]; do
  case "$1" in
    --target)
      [ $# -ge 2 ] || { echo "error: --target requires a value" >&2; exit 1; }
      CARGO_TARGET="$2"; shift 2 ;;
    --target=*) CARGO_TARGET="${1#--target=}"; shift ;;
    *) echo "error: unknown argument: $1" >&2; exit 1 ;;
  esac
done

# --- read version from Cargo.toml -------------------------------------------
VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -n1)"
if [ -z "$VERSION" ]; then
  echo "error: could not parse 'version' from Cargo.toml" >&2
  exit 1
fi

# --- resolve os/arch --------------------------------------------------------
if [ -n "$CARGO_TARGET" ]; then
  case "$CARGO_TARGET" in
    aarch64-apple-darwin)        OS="darwin"; ARCH="arm64" ;;
    x86_64-apple-darwin)         OS="darwin"; ARCH="x86_64" ;;
    aarch64-unknown-linux-gnu)   OS="linux";  ARCH="arm64" ;;
    x86_64-unknown-linux-gnu)    OS="linux";  ARCH="x86_64" ;;
    aarch64-pc-windows-msvc)     OS="windows"; ARCH="arm64" ;;
    x86_64-pc-windows-msvc)      OS="windows"; ARCH="x86_64" ;;
    x86_64-pc-windows-gnu)       OS="windows"; ARCH="x86_64" ;;
    *) echo "error: unsupported --target: $CARGO_TARGET" >&2; exit 1 ;;
  esac
  if [ "$OS" = "windows" ]; then
    BINARY_PATH="target/$CARGO_TARGET/release/chat2db.exe"
  else
    BINARY_PATH="target/$CARGO_TARGET/release/chat2db"
  fi
  CARGO_ARGS=(--release --target "$CARGO_TARGET")
else
  uname_s="$(uname -s)"
  uname_m="$(uname -m)"
  case "$uname_s" in
    Darwin) OS="darwin" ;;
    Linux)  OS="linux" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *) echo "error: unsupported host OS: $uname_s" >&2; exit 1 ;;
  esac
  case "$uname_m" in
    arm64|aarch64) ARCH="arm64" ;;
    x86_64)        ARCH="x86_64" ;;
    *) echo "error: unsupported host arch: $uname_m" >&2; exit 1 ;;
  esac
  if [ "$OS" = "windows" ]; then
    BINARY_PATH="target/release/chat2db.exe"
  else
    BINARY_PATH="target/release/chat2db"
  fi
  CARGO_ARGS=(--release)
fi

if [ "$OS" = "windows" ]; then
  ARCHIVE_NAME="chat2db-$VERSION-$OS-$ARCH.zip"
  ARCHIVE_TYPE="zip"
  BINARY_NAME="chat2db.exe"
else
  ARCHIVE_NAME="chat2db-$VERSION-$OS-$ARCH.tar.gz"
  ARCHIVE_TYPE="tarball"
  BINARY_NAME="chat2db"
fi
MANIFEST_NAME="manifest-$VERSION-$OS-$ARCH.json"

echo "building chat2db $VERSION for $OS/$ARCH..."
"$CARGO_BIN" build "${CARGO_ARGS[@]}"
if [ ! -f "$BINARY_PATH" ]; then
  echo "error: expected binary not found at $BINARY_PATH" >&2
  exit 1
fi

mkdir -p dist
rm -f "dist/$ARCHIVE_NAME"
"$PYTHON_BIN" - \
  "$BINARY_PATH" \
  "$REPO_ROOT/dist/$ARCHIVE_NAME" \
  "$BINARY_NAME" \
  "$ARCHIVE_TYPE" \
  "$SOURCE_DATE_EPOCH" \
  "$REPO_ROOT/LICENSE" \
  "$REPO_ROOT/THIRD_PARTY_LICENSES.txt" <<'PY'
import datetime
import gzip
import io
import stat
import sys
import tarfile
import zipfile

source, destination, binary_name, archive_type, epoch_text, license_path, notices_path = sys.argv[1:]

try:
    epoch = int(epoch_text, 10)
except ValueError:
    raise SystemExit("error: SOURCE_DATE_EPOCH must be a non-negative integer")
if epoch < 0 or epoch > 0xFFFFFFFF:
    raise SystemExit("error: SOURCE_DATE_EPOCH must be between 0 and 4294967295")

with open(source, "rb") as handle:
    entries = [(binary_name, handle.read(), 0o755)]
for path, name in ((license_path, "LICENSE"), (notices_path, "THIRD_PARTY_LICENSES.txt")):
    with open(path, "rb") as handle:
        entries.append((name, handle.read(), 0o644))

entries.sort(key=lambda entry: entry[0])
if archive_type == "tarball":
    with open(destination, "wb") as output:
        with gzip.GzipFile(
            filename="",
            mode="wb",
            compresslevel=9,
            fileobj=output,
            mtime=epoch,
        ) as compressed:
            with tarfile.open(
                fileobj=compressed,
                mode="w",
                format=tarfile.USTAR_FORMAT,
            ) as archive:
                for name, contents, mode in entries:
                    entry = tarfile.TarInfo(name)
                    entry.size = len(contents)
                    entry.mode = mode
                    entry.uid = 0
                    entry.gid = 0
                    entry.uname = ""
                    entry.gname = ""
                    entry.mtime = epoch
                    archive.addfile(entry, io.BytesIO(contents))
elif archive_type == "zip":
    zip_epoch = max(epoch, 315532800)  # 1980-01-01T00:00:00Z
    zip_epoch = min(zip_epoch, 0xFFFFFFFF)
    zip_time = datetime.datetime.fromtimestamp(
        zip_epoch,
        datetime.timezone.utc,
    ).timetuple()[:6]
    # ZIP timestamps have a two-second resolution.
    zip_time = (*zip_time[:5], zip_time[5] - zip_time[5] % 2)
    with zipfile.ZipFile(
        destination,
        "w",
        compression=zipfile.ZIP_DEFLATED,
        compresslevel=9,
    ) as archive:
        for name, contents, mode in entries:
            entry = zipfile.ZipInfo(name, date_time=zip_time)
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.create_system = 3
            entry.external_attr = (stat.S_IFREG | mode) << 16
            entry.internal_attr = 0
            entry.extra = b""
            entry.comment = b""
            archive.writestr(entry, contents, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)
else:
    raise SystemExit(f"error: unsupported archive type: {archive_type}")
PY

archive_contents="$("$PYTHON_BIN" - "$REPO_ROOT/dist/$ARCHIVE_NAME" "$ARCHIVE_TYPE" <<'PY'
import sys
import tarfile
import zipfile

path, archive_type = sys.argv[1], sys.argv[2]
if archive_type == "zip":
    with zipfile.ZipFile(path) as archive:
        for name in archive.namelist():
            print(name)
else:
    with tarfile.open(path, "r:gz") as archive:
        for member in archive.getmembers():
            print(member.name)
PY
)"
expected_archive_contents="$(printf '%s\n' LICENSE THIRD_PARTY_LICENSES.txt "$BINARY_NAME")"
if [ "$archive_contents" != "$expected_archive_contents" ]; then
  echo "error: non-sanitized archive contents:" >&2
  printf '%s\n' "$archive_contents" >&2
  exit 1
fi

# --- compute sha256 ---------------------------------------------------------
if command -v shasum >/dev/null 2>&1; then
  SHA256="$(shasum -a 256 "dist/$ARCHIVE_NAME" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  SHA256="$(sha256sum "dist/$ARCHIVE_NAME" | awk '{print $1}')"
else
  echo "error: neither shasum nor sha256sum available" >&2
  exit 1
fi

# --- write manifest stub ----------------------------------------------------
cat > "dist/$MANIFEST_NAME" <<EOF
{
  "version": "$VERSION",
  "artifact": {
    "url": "__CDN_BASE__/download/chat2db-cli/$VERSION/$OS/$ARCH/$ARCHIVE_NAME",
    "sha256": "$SHA256",
    "artifact_type": "$ARCHIVE_TYPE"
  }
}
EOF

echo "packaged: dist/$ARCHIVE_NAME"
echo "  sha256:  $SHA256"
echo "  size:    $(wc -c < "dist/$ARCHIVE_NAME") bytes"
echo "manifest stub: dist/$MANIFEST_NAME"
echo
echo "contents:"
if [ "$ARCHIVE_TYPE" = "zip" ]; then
  "$PYTHON_BIN" - "$REPO_ROOT/dist/$ARCHIVE_NAME" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as archive:
    print("\n".join(archive.namelist()))
PY
else
  tar -tzf "dist/$ARCHIVE_NAME"
fi
