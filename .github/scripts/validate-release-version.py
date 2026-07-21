#!/usr/bin/env python3

import json
import re
import subprocess
import sys
from pathlib import Path


STABLE_TAG = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def skill_version(path: Path) -> str:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("version:"):
            return line.partition(":")[2].strip().strip("\"'")
    fail(f"missing version field in {path}")


def stable_version(tag: str) -> tuple[int, int, int]:
    match = STABLE_TAG.fullmatch(tag)
    if match is None:
        fail(f"stable release tag must match vX.Y.Z exactly: {tag}")
    return tuple(int(part) for part in match.groups())


def main() -> None:
    if len(sys.argv) < 2:
        fail("usage: validate-release-version.py <vX.Y.Z> [existing-vX.Y.Z ...]")

    tag = sys.argv[1]
    release_version = stable_version(tag)
    tag_version = tag[1:]
    existing_versions = [stable_version(existing) for existing in sys.argv[2:]]
    if existing_versions and release_version <= max(existing_versions):
        fail(
            f"release {tag} must be newer than the current stable release "
            f"v{'.'.join(str(part) for part in max(existing_versions))}"
        )

    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    root_package = next(
        (package for package in metadata["packages"] if package["name"] == "chat2db"),
        None,
    )
    if root_package is None:
        fail("cargo metadata did not contain the chat2db package")
    cargo_version = root_package["version"]
    bundled_skill_version = skill_version(Path("skill/SKILL.md"))

    versions = {
        "tag": tag_version,
        "Cargo.toml": cargo_version,
        "skill/SKILL.md": bundled_skill_version,
    }
    if len(set(versions.values())) != 1:
        details = ", ".join(f"{source}={version}" for source, version in versions.items())
        fail(f"release versions do not match: {details}")

    print(f"validated release version {tag_version}")


if __name__ == "__main__":
    main()
