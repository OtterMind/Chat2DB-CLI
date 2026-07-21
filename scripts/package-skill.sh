#!/usr/bin/env bash
# Build the Chat2DB CLI Skill zip and manifest without publishing them.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKILL_ROOT="$REPO_ROOT/skill"
DIST_DIR="${CHAT2DB_DIST_DIR:-$REPO_ROOT/dist}"

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

if [ ! -f "$SKILL_ROOT/SKILL.md" ]; then
  echo "error: skill/SKILL.md not found" >&2
  exit 1
fi
for legal_file in LICENSE THIRD_PARTY_LICENSES.txt; do
  if [ ! -s "$REPO_ROOT/$legal_file" ]; then
    echo "error: required legal file is missing or empty: $legal_file" >&2
    exit 1
  fi
done

frontmatter_value() {
  local field="$1"
  sed -n "s/^[[:space:]]*$field:[[:space:]]*\\(.*\\)/\\1/p" "$SKILL_ROOT/SKILL.md" \
    | head -n1 \
    | tr -d '"' \
    | tr -d "'" \
    | sed 's/^[[:space:]]*//;s/[[:space:]]*$//'
}

VERSION="$(frontmatter_value version)"
COMPATIBLE_CLI="$(frontmatter_value compatible-cli-versions)"
CARGO_VERSION="$(
  sed -n '/^\[package\]/,/^\[/s/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
    "$REPO_ROOT/Cargo.toml" | head -n1
)"

case "$VERSION" in
  ''|*[!0-9A-Za-z.+-]*)
    echo "error: invalid or missing Skill version: $VERSION" >&2
    exit 1
    ;;
esac
if [ -z "$COMPATIBLE_CLI" ]; then
  echo "error: SKILL.md is missing compatible-cli-versions" >&2
  exit 1
fi
if [ "$VERSION" != "$CARGO_VERSION" ]; then
  echo "error: Skill version $VERSION does not match Cargo version $CARGO_VERSION" >&2
  exit 1
fi

BUNDLE_DIR_NAME="chat2db-cli"
ZIP_NAME="chat2db-skill-$VERSION.zip"
MANIFEST_NAME="manifest-$VERSION.json"

STAGE="$(mktemp -d)"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

BUNDLE_ROOT="$STAGE/$BUNDLE_DIR_NAME"
mkdir -p "$BUNDLE_ROOT/references"
cp "$REPO_ROOT/LICENSE" "$STAGE/LICENSE"
cp "$REPO_ROOT/THIRD_PARTY_LICENSES.txt" "$STAGE/THIRD_PARTY_LICENSES.txt"
cp "$SKILL_ROOT/SKILL.md" "$BUNDLE_ROOT/SKILL.md"
for file in "$SKILL_ROOT"/references/*.md; do
  [ -f "$file" ] || continue
  cp "$file" "$BUNDLE_ROOT/references/"
done

mkdir -p "$DIST_DIR"
rm -f "$DIST_DIR/$ZIP_NAME"
"$PYTHON_BIN" - "$STAGE" "$BUNDLE_DIR_NAME" "$DIST_DIR/$ZIP_NAME" <<'PY'
import os
import stat
import sys
import zipfile

stage, bundle_name, destination = sys.argv[1:]
with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for root, directories, files in os.walk(stage):
        directories.sort()
        for filename in sorted(files):
            source = os.path.join(root, filename)
            relative = os.path.relpath(source, stage).replace(os.sep, "/")
            entry = zipfile.ZipInfo(relative, date_time=(1980, 1, 1, 0, 0, 0))
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = (stat.S_IFREG | 0o644) << 16
            with open(source, "rb") as handle:
                archive.writestr(entry, handle.read())
PY

if command -v shasum >/dev/null 2>&1; then
  SHA256="$(shasum -a 256 "$DIST_DIR/$ZIP_NAME" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  SHA256="$(sha256sum "$DIST_DIR/$ZIP_NAME" | awk '{print $1}')"
else
  echo "error: shasum or sha256sum is required" >&2
  exit 1
fi

cat > "$DIST_DIR/$MANIFEST_NAME" <<EOF
{
  "skill_version": "$VERSION",
  "compatible_cli_versions": "$COMPATIBLE_CLI",
  "bundle_path": "$BUNDLE_DIR_NAME",
  "artifact": {
    "url": "__CDN_BASE__/download/chat2db-skill/$VERSION/$ZIP_NAME",
    "sha256": "$SHA256",
    "artifact_type": "skill_zip"
  }
}
EOF

echo "packaged: $DIST_DIR/$ZIP_NAME"
echo "manifest: $DIST_DIR/$MANIFEST_NAME"
echo "sha256: $SHA256"
