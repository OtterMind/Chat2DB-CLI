#!/usr/bin/env python3

import argparse
import hashlib
import json
import re
import shutil
import stat
import tarfile
import urllib.parse
import zipfile
from pathlib import Path


SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_cdn_base(value: str) -> str:
    value = value.rstrip("/")
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme != "https" or not parsed.netloc or parsed.path or parsed.query or parsed.fragment:
        fail("CDN base URL must be an HTTPS origin without a path, query, or fragment")
    return value


def load_manifest(path: Path, cdn_base: str) -> dict:
    data = json.loads(path.read_text(encoding="utf-8"))
    artifact = data.get("artifact")
    if not isinstance(artifact, dict) or not isinstance(artifact.get("url"), str):
        fail(f"invalid artifact object in {path}")
    if "__CDN_BASE__" not in artifact["url"]:
        fail(f"manifest must contain the __CDN_BASE__ placeholder: {path}")
    artifact["url"] = artifact["url"].replace("__CDN_BASE__", cdn_base)
    if "__CDN_BASE__" in json.dumps(data):
        fail(f"unresolved CDN placeholder in {path}")
    return data


def validate_cli_archive(path: Path, archive_type: str) -> None:
    if archive_type == "zip":
        with zipfile.ZipFile(path) as archive:
            entries = archive.infolist()
            names = [entry.filename for entry in entries]
            invalid_type = any(
                entry.is_dir() or stat.S_IFMT(entry.external_attr >> 16) == stat.S_IFLNK
                for entry in entries
            )
        empty_entry = any(entry.file_size == 0 for entry in entries)
        expected = ["LICENSE", "THIRD_PARTY_LICENSES.txt", "chat2db.exe"]
    else:
        with tarfile.open(path, "r:gz") as archive:
            entries = archive.getmembers()
            names = [member.name for member in entries]
            invalid_type = any(not member.isfile() for member in entries)
        empty_entry = any(member.size == 0 for member in entries)
        expected = ["LICENSE", "THIRD_PARTY_LICENSES.txt", "chat2db"]
    if names != expected or invalid_type or empty_entry:
        fail(f"unexpected CLI archive entries in {path}: {names}")


def validate_skill_archive(path: Path) -> None:
    with zipfile.ZipFile(path) as archive:
        entries = archive.infolist()
        names = [entry.filename for entry in entries]
    if any(
        entry.is_dir()
        or entry.file_size == 0
        or stat.S_IFMT(entry.external_attr >> 16) == stat.S_IFLNK
        for entry in entries
    ):
        fail(f"Skill archive may contain only regular files: {path}")
    required = {"LICENSE", "THIRD_PARTY_LICENSES.txt", "chat2db-cli/SKILL.md"}
    if not required.issubset(names):
        fail(f"Skill archive is missing required files: {path}")
    unexpected = [
        name
        for name in names
        if name not in required
        and not re.fullmatch(r"chat2db-cli/references/[^/]+\.md", name)
    ]
    if unexpected:
        fail(f"unexpected Skill archive entries in {path}: {unexpected}")


def copy_manifest_and_artifact(
    manifest_path: Path,
    data: dict,
    artifact_path: Path,
    output_directory: Path,
    checksum_name: str,
    manifest_alias: str,
) -> None:
    output_directory.mkdir(parents=True, exist_ok=True)
    output_artifact = output_directory / artifact_path.name
    output_manifest = output_directory / manifest_path.name
    output_manifest_alias = output_directory / manifest_alias
    shutil.copy2(artifact_path, output_artifact)
    manifest_contents = json.dumps(data, indent=2) + "\n"
    output_manifest.write_text(manifest_contents, encoding="utf-8")
    output_manifest_alias.write_text(manifest_contents, encoding="utf-8")

    checksum_path = output_directory / checksum_name
    checksum_path.write_text(
        f"{sha256(output_artifact)}  {output_artifact.name}\n"
        f"{sha256(output_manifest)}  {output_manifest.name}\n"
        f"{sha256(output_manifest_alias)}  {output_manifest_alias.name}\n",
        encoding="utf-8",
    )


def stage_cli(input_directory: Path, output_directory: Path, cdn_base: str) -> None:
    manifests = sorted(input_directory.glob("manifest-*.json"))
    if len(manifests) != 1:
        fail(f"expected exactly one CLI manifest, found {len(manifests)}")
    manifest_path = manifests[0]
    data = load_manifest(manifest_path, cdn_base)
    version = data.get("version")
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        fail(f"invalid CLI version in {manifest_path}: {version}")

    artifact_url = data["artifact"]["url"]
    parsed_path = urllib.parse.urlsplit(artifact_url).path.strip("/").split("/")
    if len(parsed_path) != 6 or parsed_path[:2] != ["download", "chat2db-cli"]:
        fail(f"invalid CLI artifact URL in {manifest_path}: {artifact_url}")
    _, _, url_version, os_name, architecture, archive_name = parsed_path
    if url_version != version:
        fail(f"CLI artifact URL version does not match manifest version: {artifact_url}")
    if (os_name, architecture) not in {
        ("darwin", "arm64"),
        ("darwin", "x86_64"),
        ("linux", "arm64"),
        ("linux", "x86_64"),
        ("windows", "x86_64"),
    }:
        fail(f"unsupported CLI target in {manifest_path}: {os_name}/{architecture}")

    expected_archive = f"chat2db-{version}-{os_name}-{architecture}"
    expected_type = "zip" if os_name == "windows" else "tarball"
    expected_archive += ".zip" if os_name == "windows" else ".tar.gz"
    expected_url = (
        f"{cdn_base}/download/chat2db-cli/{version}/{os_name}/{architecture}/{expected_archive}"
    )
    if (
        artifact_url != expected_url
        or archive_name != expected_archive
        or data["artifact"].get("artifact_type") != expected_type
    ):
        fail(f"CLI artifact metadata does not match target in {manifest_path}")
    artifact_path = input_directory / archive_name
    if not artifact_path.is_file():
        fail(f"missing CLI archive: {artifact_path}")
    if sha256(artifact_path) != data["artifact"].get("sha256"):
        fail(f"CLI archive checksum does not match manifest: {artifact_path}")
    validate_cli_archive(artifact_path, expected_type)

    expected_manifest_name = f"manifest-{version}-{os_name}-{architecture}.json"
    if manifest_path.name != expected_manifest_name:
        fail(f"unexpected CLI manifest filename: {manifest_path.name}")
    copy_manifest_and_artifact(
        manifest_path,
        data,
        artifact_path,
        output_directory,
        f"checksums-{version}-{os_name}-{architecture}.txt",
        f"manifest-cli-{os_name}-{architecture}.json",
    )


def stage_skill(input_directory: Path, output_directory: Path, cdn_base: str) -> None:
    archives = sorted(input_directory.glob("chat2db-skill-*.zip"))
    if len(archives) != 1:
        fail(f"expected exactly one Skill archive, found {len(archives)}")
    archive_path = archives[0]
    version = archive_path.name.removeprefix("chat2db-skill-").removesuffix(".zip")
    if not SEMVER.fullmatch(version):
        fail(f"invalid Skill version in archive filename: {archive_path.name}")

    manifest_path = input_directory / f"manifest-{version}.json"
    if not manifest_path.is_file():
        fail(f"missing Skill manifest: {manifest_path}")
    data = load_manifest(manifest_path, cdn_base)
    artifact_url = data["artifact"]["url"]
    expected_url = f"{cdn_base}/download/chat2db-skill/{version}/{archive_path.name}"
    if artifact_url != expected_url:
        fail(f"unexpected Skill artifact URL in {manifest_path}: {artifact_url}")
    if data.get("skill_version") != version or data["artifact"].get("artifact_type") != "skill_zip":
        fail(f"Skill artifact metadata does not match version in {manifest_path}")
    if sha256(archive_path) != data["artifact"].get("sha256"):
        fail(f"Skill archive checksum does not match manifest: {archive_path}")
    validate_skill_archive(archive_path)
    copy_manifest_and_artifact(
        manifest_path,
        data,
        archive_path,
        output_directory,
        f"checksums-{version}.txt",
        "manifest-skill.json",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--product", required=True, choices=("cli", "skill"))
    parser.add_argument("--input-directory", required=True, type=Path)
    parser.add_argument("--output-directory", required=True, type=Path)
    parser.add_argument("--cdn-base-url", required=True)
    args = parser.parse_args()

    if not args.input_directory.is_dir():
        fail(f"input directory does not exist: {args.input_directory}")
    cdn_base = validate_cdn_base(args.cdn_base_url)
    if args.product == "cli":
        stage_cli(args.input_directory, args.output_directory, cdn_base)
    else:
        stage_skill(args.input_directory, args.output_directory, cdn_base)


if __name__ == "__main__":
    main()
