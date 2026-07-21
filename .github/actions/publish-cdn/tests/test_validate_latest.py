#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[1] / "validate-latest.py"
SPEC = importlib.util.spec_from_file_location("publish_cdn_validate_latest", MODULE_PATH)
assert SPEC and SPEC.loader
VALIDATE_LATEST = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATE_LATEST)


class ValidateLatestTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary_directory.name)
        self.snapshot = self.root / "snapshot.tsv"

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def write_snapshot(self, versions: dict[str, str | None]) -> None:
        rows = []
        for index, (destination, field) in enumerate(
            VALIDATE_LATEST.EXPECTED_LATEST.items(), start=1
        ):
            version = versions[destination]
            if version is None:
                rows.append(f"{destination}\tabsent\t-\t-\n")
                continue
            backup = self.root / f"backup-{index}.json"
            backup.write_text(json.dumps({field: version}) + "\n")
            rows.append(f"{destination}\tpresent\t{backup}\t{'0' * 64}\n")
        self.snapshot.write_text("".join(rows))

    def all_versions(self, version: str | None) -> dict[str, str | None]:
        return {destination: version for destination in VALIDATE_LATEST.EXPECTED_LATEST}

    def test_allows_first_release_only_when_all_six_are_absent(self) -> None:
        self.write_snapshot(self.all_versions(None))
        self.assertIsNone(
            VALIDATE_LATEST.validate_latest_snapshot(self.snapshot, "0.2.0")
        )

    def test_allows_strict_upgrade_from_one_consistent_version(self) -> None:
        self.write_snapshot(self.all_versions("0.1.9"))
        self.assertEqual(
            "0.1.9", VALIDATE_LATEST.validate_latest_snapshot(self.snapshot, "0.2.0")
        )

    def test_allows_consistent_partial_platform_expansion(self) -> None:
        versions = self.all_versions("0.1.0")
        versions[next(iter(versions))] = None
        self.write_snapshot(versions)
        self.assertEqual(
            "0.1.0", VALIDATE_LATEST.validate_latest_snapshot(self.snapshot, "0.2.0")
        )

    def test_rejects_partial_expansion_with_split_versions(self) -> None:
        versions = self.all_versions("0.1.0")
        destinations = list(versions)
        versions[destinations[0]] = None
        versions[destinations[1]] = "0.1.1"
        self.write_snapshot(versions)
        with self.assertRaises(SystemExit):
            VALIDATE_LATEST.validate_latest_snapshot(self.snapshot, "0.2.0")

    def test_rejects_split_equal_or_newer_current_versions(self) -> None:
        split = self.all_versions("0.1.0")
        split[next(iter(split))] = "0.1.1"
        self.write_snapshot(split)
        with self.assertRaises(SystemExit):
            VALIDATE_LATEST.validate_latest_snapshot(self.snapshot, "0.2.0")

        for current in ("0.2.0", "0.3.0"):
            self.write_snapshot(self.all_versions(current))
            with self.subTest(current=current), self.assertRaises(SystemExit):
                VALIDATE_LATEST.validate_latest_snapshot(self.snapshot, "0.2.0")

    def test_semver_prerelease_precedence(self) -> None:
        self.assertLess(
            VALIDATE_LATEST.compare_semver("1.0.0-rc.1", "1.0.0"), 0
        )
        self.assertGreater(
            VALIDATE_LATEST.compare_semver("1.0.0-rc.10", "1.0.0-rc.2"), 0
        )
        self.assertEqual(
            VALIDATE_LATEST.compare_semver("1.0.0+build.2", "1.0.0+build.1"), 0
        )


if __name__ == "__main__":
    unittest.main()
