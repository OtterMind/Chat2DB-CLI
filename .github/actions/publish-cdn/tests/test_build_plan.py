#!/usr/bin/env python3

import hashlib
import importlib.util
import io
import json
import stat
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[1] / "build-plan.py"
SPEC = importlib.util.spec_from_file_location("publish_cdn_build_plan", MODULE_PATH)
assert SPEC and SPEC.loader
BUILD_PLAN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD_PLAN)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class BuildPlanTest(unittest.TestCase):
    version = "0.2.0"

    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.artifacts = Path(self.temporary_directory.name) / "assets"
        self.output = Path(self.temporary_directory.name) / "plan"
        self.artifacts.mkdir()
        self._build_fixture()

    def tearDown(self) -> None:
        self.temporary_directory.cleanup()

    def _write_cli_archive(self, path: Path, archive_type: str) -> None:
        payload = f"binary:{path.name}".encode()
        contents = (
            ("LICENSE", b"Apache License fixture", 0o644),
            ("THIRD_PARTY_LICENSES.txt", b"Third-party license fixture", 0o644),
            ("chat2db.exe" if archive_type == "zip" else "chat2db", payload, 0o755),
        )
        if archive_type == "zip":
            with zipfile.ZipFile(path, "w") as archive:
                for name, entry_contents, mode in contents:
                    entry = zipfile.ZipInfo(name)
                    entry.external_attr = (stat.S_IFREG | mode) << 16
                    archive.writestr(entry, entry_contents)
        else:
            with tarfile.open(path, "w:gz") as archive:
                for name, entry_contents, mode in contents:
                    entry = tarfile.TarInfo(name)
                    entry.mode = mode
                    entry.size = len(entry_contents)
                    archive.addfile(entry, io.BytesIO(entry_contents))

    def _build_fixture(self) -> None:
        for os_name, architecture, extension, artifact_type in BUILD_PLAN.CLI_TARGETS:
            archive_name = f"chat2db-{self.version}-{os_name}-{architecture}.{extension}"
            archive_path = self.artifacts / archive_name
            self._write_cli_archive(archive_path, artifact_type)
            manifest = {
                "version": self.version,
                "artifact": {
                    "url": (
                        f"{BUILD_PLAN.OFFICIAL_CDN_BASE_URL}/download/chat2db-cli/"
                        f"{self.version}/{os_name}/{architecture}/{archive_name}"
                    ),
                    "sha256": digest(archive_path),
                    "artifact_type": artifact_type,
                },
            }
            manifest_bytes = (json.dumps(manifest, indent=2) + "\n").encode()
            (self.artifacts / f"manifest-{self.version}-{os_name}-{architecture}.json").write_bytes(
                manifest_bytes
            )
            (self.artifacts / f"manifest-cli-{os_name}-{architecture}.json").write_bytes(
                manifest_bytes
            )

        skill_archive_name = f"chat2db-skill-{self.version}.zip"
        skill_archive = self.artifacts / skill_archive_name
        with zipfile.ZipFile(skill_archive, "w") as archive:
            for name, contents in (
                ("LICENSE", "Apache License fixture\n"),
                ("THIRD_PARTY_LICENSES.txt", "Third-party license fixture\n"),
                ("chat2db-cli/SKILL.md", "---\nname: chat2db-cli\n---\n"),
            ):
                entry = zipfile.ZipInfo(name)
                entry.external_attr = (stat.S_IFREG | 0o644) << 16
                archive.writestr(entry, contents)
        skill_manifest = {
            "skill_version": self.version,
            "artifact": {
                "url": (
                    f"{BUILD_PLAN.OFFICIAL_CDN_BASE_URL}/download/chat2db-skill/"
                    f"{self.version}/{skill_archive_name}"
                ),
                "sha256": digest(skill_archive),
                "artifact_type": "skill_zip",
            },
        }
        skill_manifest_bytes = (json.dumps(skill_manifest, indent=2) + "\n").encode()
        (self.artifacts / f"manifest-{self.version}.json").write_bytes(skill_manifest_bytes)
        (self.artifacts / "manifest-skill.json").write_bytes(skill_manifest_bytes)

        for installer_name, alias_name in (
            (f"install-cli-{self.version}.sh", "install-cli.sh"),
            (f"install-cli-{self.version}.ps1", "install-cli.ps1"),
            (f"install-skill-{self.version}.sh", "install-skill.sh"),
        ):
            contents = f"installer:{installer_name}\n"
            (self.artifacts / installer_name).write_text(contents)
            (self.artifacts / alias_name).write_text(contents)

        (self.artifacts / "LICENSE").write_text("Apache License fixture\n")
        (self.artifacts / "THIRD_PARTY_LICENSES.txt").write_text(
            "Third-party license fixture\n"
        )

        (self.artifacts / "chat2db-cli.cdx.json").write_text(
            json.dumps(
                {
                    "bomFormat": "CycloneDX",
                    "specVersion": "1.5",
                    "metadata": {"component": {"name": "chat2db", "version": self.version}},
                }
            )
            + "\n"
        )
        self._rewrite_checksums()
        (self.artifacts / f"chat2db-cli-{self.version}.intoto.jsonl").write_text(
            '{"mediaType":"application/vnd.in-toto+json"}\n'
        )

    def _rewrite_checksums(self) -> None:
        checksum_path = self.artifacts / "SHA256SUMS"
        checksum_path.unlink(missing_ok=True)
        checksum_path.write_text(
            "".join(
                f"{digest(path)}  {path.name}\n"
                for path in sorted(self.artifacts.iterdir(), key=lambda item: item.name)
                if not path.name.endswith(".intoto.jsonl")
            )
        )

    def test_builds_one_complete_transaction_plan(self) -> None:
        plan = BUILD_PLAN.build_release_plan(
            self.artifacts, BUILD_PLAN.OFFICIAL_CDN_BASE_URL
        )

        self.assertEqual(self.version, plan["version"])
        self.assertEqual(19, len(plan["immutable"]["uploads"]))
        self.assertEqual(19, len(plan["immutable"]["verify"]))
        self.assertEqual(9, len(plan["promotion"]["uploads"]))
        self.assertEqual(9, len(plan["promotion"]["refresh"]))
        self.assertEqual(9, len(plan["promotion"]["verify"]))
        immutable_destinations = {
            item["destination"] for item in plan["immutable"]["uploads"]
        }
        self.assertIn(f"install/{self.version}/cli.sh", immutable_destinations)
        self.assertIn(f"install/{self.version}/cli.ps1", immutable_destinations)
        self.assertIn(f"install/{self.version}/skill.sh", immutable_destinations)
        self.assertIn(
            f"download/chat2db-cli/{self.version}/LICENSE", immutable_destinations
        )
        self.assertIn(
            f"download/chat2db-cli/{self.version}/THIRD_PARTY_LICENSES.txt",
            immutable_destinations,
        )
        promotion_destinations = {
            item["destination"] for item in plan["promotion"]["uploads"]
        }
        self.assertEqual(
            {
                "download/chat2db-cli/latest/darwin/arm64/manifest.json",
                "download/chat2db-cli/latest/darwin/x86_64/manifest.json",
                "download/chat2db-cli/latest/linux/arm64/manifest.json",
                "download/chat2db-cli/latest/linux/x86_64/manifest.json",
                "download/chat2db-cli/latest/windows/x86_64/manifest.json",
                "download/chat2db-skill/latest/manifest.json",
                "install/cli.sh",
                "install/cli.ps1",
                "install/skill.sh",
            },
            promotion_destinations,
        )

        BUILD_PLAN.write_plan_files(plan, self.output)
        self.assertEqual(19, len((self.output / "immutable-uploads.tsv").read_text().splitlines()))
        self.assertEqual(9, len((self.output / "promotion-uploads.tsv").read_text().splitlines()))

    def test_rejects_non_official_or_non_https_cdn(self) -> None:
        for url in (
            "http://cdn.chat2db-ai.com",
            "https://example.com",
            "https://cdn.chat2db-ai.com/path",
            "https://cdn.chat2db-ai.com\t",
        ):
            with self.subTest(url=url), self.assertRaises(SystemExit):
                BUILD_PLAN.build_release_plan(self.artifacts, url)

    def test_rejects_checksum_mismatch(self) -> None:
        installer = self.artifacts / f"install-cli-{self.version}.sh"
        installer.write_text("tampered\n")
        with self.assertRaises(SystemExit):
            BUILD_PLAN.build_release_plan(self.artifacts, BUILD_PLAN.OFFICIAL_CDN_BASE_URL)

    def test_rejects_incomplete_or_extra_bundle(self) -> None:
        (self.artifacts / "unexpected.txt").write_text("unexpected\n")
        self._rewrite_checksums()
        with self.assertRaises(SystemExit):
            BUILD_PLAN.build_release_plan(self.artifacts, BUILD_PLAN.OFFICIAL_CDN_BASE_URL)

    def test_rejects_different_generic_manifest_alias(self) -> None:
        alias = self.artifacts / "manifest-cli-linux-x86_64.json"
        alias.write_text("{}\n")
        self._rewrite_checksums()
        with self.assertRaises(SystemExit):
            BUILD_PLAN.build_release_plan(self.artifacts, BUILD_PLAN.OFFICIAL_CDN_BASE_URL)

    def test_rejects_different_generic_installer_alias(self) -> None:
        (self.artifacts / "install-cli.sh").write_text("different\n")
        self._rewrite_checksums()
        with self.assertRaises(SystemExit):
            BUILD_PLAN.build_release_plan(self.artifacts, BUILD_PLAN.OFFICIAL_CDN_BASE_URL)

    def test_rejects_control_characters_and_path_traversal(self) -> None:
        for value in (
            "../latest/manifest.json",
            "a/../../b",
            "a/./b",
            "a/%2e%2e/b",
            "a?query",
            "a\t/b",
            "a\n/b",
        ):
            with self.subTest(value=value), self.assertRaises(SystemExit):
                BUILD_PLAN.validate_object_name(value)
        (self.artifacts / "bad\tasset").write_text("bad\n")
        with self.assertRaises(SystemExit):
            BUILD_PLAN.build_release_plan(self.artifacts, BUILD_PLAN.OFFICIAL_CDN_BASE_URL)


if __name__ == "__main__":
    unittest.main()
