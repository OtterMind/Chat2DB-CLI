#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON_BIN="${PYTHON_BIN:-python3}"
FIXED_EPOCH=1700000000

WORK_DIR="$(mktemp -d)"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

FIXTURE_ROOT="$WORK_DIR/repository"
mkdir -p "$FIXTURE_ROOT/scripts"
cp "$REPO_ROOT/Cargo.toml" "$FIXTURE_ROOT/Cargo.toml"
cp "$REPO_ROOT/scripts/package-cli.sh" "$FIXTURE_ROOT/scripts/package-cli.sh"
printf 'Apache License test fixture\n' > "$FIXTURE_ROOT/LICENSE"
printf 'Third-party notices test fixture\n' > "$FIXTURE_ROOT/THIRD_PARTY_LICENSES.txt"

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

package_twice() {
  local target="$1"
  local binary_name="$2"
  local archive_name="$3"
  local binary_path="$FIXTURE_ROOT/target/$target/release/$binary_name"

  mkdir -p "$(dirname "$binary_path")"
  printf 'fixed chat2db test binary\n' > "$binary_path"

  PYTHON_BIN="$PYTHON_BIN" \
    CARGO_BIN=true \
    SOURCE_DATE_EPOCH="$FIXED_EPOCH" \
    bash "$FIXTURE_ROOT/scripts/package-cli.sh" --target "$target" >/dev/null
  cp "$FIXTURE_ROOT/dist/$archive_name" "$WORK_DIR/first-$archive_name"

  # The source timestamp must not affect the archive.
  touch "$binary_path"
  PYTHON_BIN="$PYTHON_BIN" \
    CARGO_BIN=true \
    SOURCE_DATE_EPOCH="$FIXED_EPOCH" \
    bash "$FIXTURE_ROOT/scripts/package-cli.sh" --target "$target" >/dev/null

  local first_sha second_sha
  first_sha="$(sha256_file "$WORK_DIR/first-$archive_name")"
  second_sha="$(sha256_file "$FIXTURE_ROOT/dist/$archive_name")"
  test "$first_sha" = "$second_sha"
  printf '%s  %s\n' "$second_sha" "$archive_name"
}

VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$REPO_ROOT/Cargo.toml" | head -n1)"
LINUX_ARCHIVE="chat2db-$VERSION-linux-x86_64.tar.gz"
WINDOWS_ARCHIVE="chat2db-$VERSION-windows-x86_64.zip"

package_twice x86_64-unknown-linux-gnu chat2db "$LINUX_ARCHIVE"
package_twice x86_64-pc-windows-msvc chat2db.exe "$WINDOWS_ARCHIVE"

"$PYTHON_BIN" - \
  "$FIXTURE_ROOT/dist/$LINUX_ARCHIVE" \
  "$FIXTURE_ROOT/dist/$WINDOWS_ARCHIVE" \
  "$FIXED_EPOCH" <<'PY'
import datetime
import gzip
import stat
import struct
import sys
import tarfile
import zipfile

tar_path, zip_path, epoch_text = sys.argv[1:]
epoch = int(epoch_text)
expected_contents = {
    "LICENSE": b"Apache License test fixture\n",
    "THIRD_PARTY_LICENSES.txt": b"Third-party notices test fixture\n",
    "chat2db": b"fixed chat2db test binary\n",
    "chat2db.exe": b"fixed chat2db test binary\n",
}

with open(tar_path, "rb") as handle:
    header = handle.read(10)
assert header[:2] == b"\x1f\x8b"
assert struct.unpack("<I", header[4:8])[0] == epoch

with tarfile.open(tar_path, "r:gz") as archive:
    members = archive.getmembers()
    assert [member.name for member in members] == [
        "LICENSE",
        "THIRD_PARTY_LICENSES.txt",
        "chat2db",
    ]
    for member in members:
        assert member.uid == member.gid == 0
        assert member.uname == member.gname == ""
        assert member.mode == (0o755 if member.name == "chat2db" else 0o644)
        assert member.mtime == epoch
        assert archive.extractfile(member).read() == expected_contents[member.name]

expected_zip_time = datetime.datetime.fromtimestamp(
    epoch,
    datetime.timezone.utc,
).timetuple()[:6]
expected_zip_time = (*expected_zip_time[:5], expected_zip_time[5] - expected_zip_time[5] % 2)
with zipfile.ZipFile(zip_path) as archive:
    infos = archive.infolist()
    assert [info.filename for info in infos] == [
        "LICENSE",
        "THIRD_PARTY_LICENSES.txt",
        "chat2db.exe",
    ]
    for info in infos:
        assert info.date_time == expected_zip_time
        assert info.create_system == 3
        mode = 0o755 if info.filename == "chat2db.exe" else 0o644
        assert info.external_attr >> 16 == stat.S_IFREG | mode
        assert info.internal_attr == 0
        assert info.extra == b""
        assert info.comment == b""
        assert archive.read(info) == expected_contents[info.filename]
PY

echo "package-cli reproducibility and metadata checks passed"
