#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
EXPECTED_LATEST = {
    "download/chat2db-cli/latest/darwin/arm64/manifest.json": "version",
    "download/chat2db-cli/latest/darwin/x86_64/manifest.json": "version",
    "download/chat2db-cli/latest/linux/arm64/manifest.json": "version",
    "download/chat2db-cli/latest/linux/x86_64/manifest.json": "version",
    "download/chat2db-cli/latest/windows/x86_64/manifest.json": "version",
    "download/chat2db-skill/latest/manifest.json": "skill_version",
}


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def parse_semver(value: str) -> tuple[tuple[int, int, int], list[str] | None]:
    match = SEMVER.fullmatch(value)
    if not match:
        fail(f"invalid semantic version: {value}")
    prerelease = match.group(4)
    return tuple(int(part) for part in match.group(1, 2, 3)), (
        prerelease.split(".") if prerelease else None
    )


def compare_identifiers(left: list[str], right: list[str]) -> int:
    for left_item, right_item in zip(left, right):
        if left_item == right_item:
            continue
        left_numeric = left_item.isdigit()
        right_numeric = right_item.isdigit()
        if left_numeric and right_numeric:
            return (int(left_item) > int(right_item)) - (int(left_item) < int(right_item))
        if left_numeric != right_numeric:
            return -1 if left_numeric else 1
        return (left_item > right_item) - (left_item < right_item)
    return (len(left) > len(right)) - (len(left) < len(right))


def compare_semver(left: str, right: str) -> int:
    left_core, left_pre = parse_semver(left)
    right_core, right_pre = parse_semver(right)
    if left_core != right_core:
        return (left_core > right_core) - (left_core < right_core)
    if left_pre is None or right_pre is None:
        if left_pre is right_pre:
            return 0
        return 1 if left_pre is None else -1
    return compare_identifiers(left_pre, right_pre)


def validate_latest_snapshot(snapshot: Path, release_version: str) -> str | None:
    parse_semver(release_version)
    latest: dict[str, tuple[str, Path]] = {}
    try:
        lines = snapshot.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        fail(f"cannot read promotion snapshot: {error}")
    for line in lines:
        if any(character in line for character in ("\t\t", "\r", "\n")):
            fail("invalid promotion snapshot field")
        fields = line.split("\t")
        if len(fields) != 4:
            fail("invalid promotion snapshot row")
        destination, state, backup, _ = fields
        if destination not in EXPECTED_LATEST:
            continue
        if destination in latest or state not in ("present", "absent"):
            fail(f"invalid latest manifest snapshot: {destination}")
        latest[destination] = (state, Path(backup))

    if set(latest) != set(EXPECTED_LATEST):
        fail("promotion snapshot does not contain all six latest manifests")
    states = {state for state, _ in latest.values()}
    if states == {"absent"}:
        return None

    versions = set()
    for destination, (state, backup) in latest.items():
        if state == "absent":
            continue
        try:
            manifest = json.loads(backup.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            fail(f"cannot read current latest manifest {destination}: {error}")
        if not isinstance(manifest, dict):
            fail(f"current latest manifest is not an object: {destination}")
        version = manifest.get(EXPECTED_LATEST[destination])
        if not isinstance(version, str):
            fail(f"current latest manifest has no version: {destination}")
        parse_semver(version)
        versions.add(version)

    if len(versions) != 1:
        fail(f"present latest manifests have split versions: {sorted(versions)}")
    current_version = versions.pop()
    if compare_semver(release_version, current_version) <= 0:
        fail(
            f"release version {release_version} must be strictly newer than current latest "
            f"{current_version}"
        )
    return current_version


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--snapshot", required=True, type=Path)
    parser.add_argument("--release-version", required=True)
    args = parser.parse_args()
    current = validate_latest_snapshot(args.snapshot, args.release_version)
    print("first release" if current is None else f"advancing from {current}")


if __name__ == "__main__":
    main()
