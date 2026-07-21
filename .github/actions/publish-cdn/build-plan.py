#!/usr/bin/env python3

import argparse
import hashlib
import json
import re
import stat
import tarfile
import urllib.parse
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any


OFFICIAL_CDN_BASE_URL = "https://cdn.chat2db-ai.com"
SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)
SHA256_HEX = re.compile(r"^[0-9a-f]{64}$")
SAFE_ASSET_NAME = re.compile(r"^[0-9A-Za-z][0-9A-Za-z._+-]*$")
SAFE_OBJECT_PART = re.compile(r"^[0-9A-Za-z][0-9A-Za-z._+-]*$")
CLI_TARGETS = (
    ("darwin", "arm64", "tar.gz", "tarball"),
    ("darwin", "x86_64", "tar.gz", "tarball"),
    ("linux", "arm64", "tar.gz", "tarball"),
    ("linux", "x86_64", "tar.gz", "tarball"),
    ("windows", "x86_64", "zip", "zip"),
)


def fail(message: str) -> None:
    raise SystemExit(f"error: {message}")


def reject_control(value: str, label: str) -> None:
    if any(character in value for character in ("\t", "\n", "\r")):
        fail(f"{label} must not contain tabs or line breaks")


def validate_object_name(value: str, label: str = "object name") -> str:
    reject_control(value, label)
    if "\\" in value or value.startswith("/") or value.endswith("/") or "//" in value:
        fail(f"invalid {label}: {value}")
    raw_parts = value.split("/")
    parts = PurePosixPath(value).parts
    if (
        not parts
        or tuple(raw_parts) != parts
        or any(part in ("", ".", "..") or not SAFE_OBJECT_PART.fullmatch(part) for part in parts)
    ):
        fail(f"invalid {label}: {value}")
    return value


def validate_cdn_base(value: str) -> str:
    reject_control(value, "CDN base URL")
    parsed = urllib.parse.urlsplit(value)
    if (
        value != OFFICIAL_CDN_BASE_URL
        or parsed.scheme != "https"
        or parsed.netloc != "cdn.chat2db-ai.com"
        or parsed.path
        or parsed.query
        or parsed.fragment
        or parsed.username is not None
        or parsed.password is not None
    ):
        fail(f"CDN base URL must be exactly {OFFICIAL_CDN_BASE_URL}")
    return value


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_regular_file(path: Path, artifacts: Path) -> Path:
    reject_control(path.name, "release asset name")
    if not SAFE_ASSET_NAME.fullmatch(path.name):
        fail(f"unsafe release asset name: {path.name}")
    try:
        mode = path.lstat().st_mode
    except OSError as error:
        fail(f"cannot stat release asset {path}: {error}")
    if not stat.S_ISREG(mode):
        fail(f"release asset must be a regular file: {path}")
    resolved = path.resolve()
    try:
        resolved.relative_to(artifacts)
    except ValueError:
        fail(f"release asset escapes artifact directory: {path}")
    if resolved.stat().st_size == 0:
        fail(f"release asset must not be empty: {path}")
    return resolved


def reject_controls_in_json(value: Any, path: Path) -> None:
    if isinstance(value, str):
        reject_control(value, f"string in {path.name}")
    elif isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str):
                fail(f"JSON object key must be a string in {path}")
            reject_control(key, f"key in {path.name}")
            reject_controls_in_json(child, path)
    elif isinstance(value, list):
        for child in value:
            reject_controls_in_json(child, path)


def load_json(path: Path, artifacts: Path) -> dict:
    require_regular_file(path, artifacts)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"cannot read JSON file {path}: {error}")
    if not isinstance(value, dict):
        fail(f"JSON root must be an object: {path}")
    reject_controls_in_json(value, path)
    return value


def validate_cli_archive(path: Path, archive_type: str) -> None:
    try:
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
    except (OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        fail(f"cannot inspect CLI archive {path}: {error}")
    if names != expected or invalid_type or empty_entry:
        fail(f"unexpected CLI archive entries in {path}: {names}")


def validate_skill_archive(path: Path) -> None:
    try:
        with zipfile.ZipFile(path) as archive:
            entries = archive.infolist()
            names = [entry.filename for entry in entries]
    except (OSError, zipfile.BadZipFile) as error:
        fail(f"cannot inspect Skill archive {path}: {error}")
    if any(
        entry.is_dir()
        or entry.file_size == 0
        or stat.S_IFMT(entry.external_attr >> 16) == stat.S_IFLNK
        for entry in entries
    ):
        fail(f"Skill archive may contain only regular files: {path}")
    for name in names:
        validate_object_name(name, "Skill archive entry")
    if len(names) != len(set(names)):
        fail(f"Skill archive contains duplicate entries: {path}")
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


def validate_checksum_file(path: Path, artifacts: Path, expected_names: set[str]) -> None:
    require_regular_file(path, artifacts)
    try:
        lines = path.read_text(encoding="ascii").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        fail(f"cannot read SHA256SUMS: {error}")
    if not lines:
        fail("SHA256SUMS must not be empty")

    checksums: dict[str, str] = {}
    for line in lines:
        reject_control(line, "SHA256SUMS entry")
        match = re.fullmatch(r"([0-9a-f]{64})  ([0-9A-Za-z][0-9A-Za-z._+-]*)", line)
        if not match:
            fail(f"invalid SHA256SUMS entry: {line!r}")
        digest, name = match.groups()
        validate_object_name(name, "SHA256SUMS asset name")
        if name in checksums:
            fail(f"duplicate SHA256SUMS entry: {name}")
        checksums[name] = digest

    actual_names = set(checksums)
    if actual_names != expected_names:
        missing = sorted(expected_names - actual_names)
        unexpected = sorted(actual_names - expected_names)
        fail(f"SHA256SUMS asset set mismatch; missing={missing}, unexpected={unexpected}")
    for name, expected_digest in checksums.items():
        actual_digest = sha256(require_regular_file(artifacts / name, artifacts))
        if actual_digest != expected_digest:
            fail(f"SHA256SUMS digest mismatch for {name}")


def add_upload(
    section: dict,
    source: Path,
    destination: str,
    cdn_base: str,
    *,
    refresh: bool = False,
) -> None:
    destination = validate_object_name(destination)
    source_digest = sha256(source)
    url = f"{cdn_base}/{destination}"
    reject_control(str(source), "upload source")
    reject_control(url, "verification URL")
    section["uploads"].append(
        {"source": str(source), "destination": destination, "sha256": source_digest}
    )
    section["verify"].append({"url": url, "sha256": source_digest, "name": destination})
    if refresh:
        section["refresh"].append(url)


def validate_manifest_artifact(
    manifest: dict,
    manifest_path: Path,
    *,
    version_field: str,
    version: str,
    expected_url: str,
    expected_type: str,
    archive_path: Path,
) -> None:
    artifact = manifest.get("artifact")
    if not isinstance(artifact, dict):
        fail(f"manifest artifact must be an object: {manifest_path}")
    digest = artifact.get("sha256")
    if not isinstance(digest, str) or not SHA256_HEX.fullmatch(digest):
        fail(f"manifest artifact SHA-256 is invalid: {manifest_path}")
    if (
        manifest.get(version_field) != version
        or artifact.get("url") != expected_url
        or digest != sha256(archive_path)
        or artifact.get("artifact_type") != expected_type
    ):
        fail(f"manifest does not match its archive: {manifest_path}")


def discover_version(artifacts: Path) -> str:
    versions = set()
    for path in artifacts.iterdir():
        reject_control(path.name, "release asset name")
        match = re.fullmatch(r"chat2db-(.+)-darwin-arm64\.tar\.gz", path.name)
        if match:
            versions.add(match.group(1))
    if len(versions) != 1:
        fail(f"expected exactly one release version, found: {sorted(versions)}")
    version = versions.pop()
    if not SEMVER.fullmatch(version):
        fail(f"invalid release version: {version}")
    return version


def build_release_plan(artifacts_argument: Path, cdn_base_argument: str) -> dict:
    reject_control(str(artifacts_argument), "artifact directory")
    if not artifacts_argument.is_dir():
        fail(f"artifact directory does not exist: {artifacts_argument}")
    if artifacts_argument.is_symlink():
        fail(f"artifact directory must not be a symbolic link: {artifacts_argument}")
    artifacts = artifacts_argument.resolve()
    cdn_base = validate_cdn_base(cdn_base_argument)
    version = discover_version(artifacts)

    immutable = {"uploads": [], "verify": []}
    promotion = {"uploads": [], "refresh": [], "verify": []}
    expected_assets: set[str] = set()

    for legal_name in ("LICENSE", "THIRD_PARTY_LICENSES.txt"):
        legal_path = require_regular_file(artifacts / legal_name, artifacts)
        expected_assets.add(legal_name)
        add_upload(
            immutable,
            legal_path,
            f"download/chat2db-cli/{version}/{legal_name}",
            cdn_base,
        )

    for os_name, architecture, extension, artifact_type in CLI_TARGETS:
        archive_name = f"chat2db-{version}-{os_name}-{architecture}.{extension}"
        manifest_name = f"manifest-{version}-{os_name}-{architecture}.json"
        alias_name = f"manifest-cli-{os_name}-{architecture}.json"
        archive_path = require_regular_file(artifacts / archive_name, artifacts)
        manifest_path = require_regular_file(artifacts / manifest_name, artifacts)
        alias_path = require_regular_file(artifacts / alias_name, artifacts)
        expected_assets.update((archive_name, manifest_name, alias_name))

        validate_cli_archive(archive_path, artifact_type)
        manifest = load_json(manifest_path, artifacts)
        destination_prefix = f"download/chat2db-cli/{version}/{os_name}/{architecture}"
        expected_url = f"{cdn_base}/{destination_prefix}/{archive_name}"
        validate_manifest_artifact(
            manifest,
            manifest_path,
            version_field="version",
            version=version,
            expected_url=expected_url,
            expected_type=artifact_type,
            archive_path=archive_path,
        )
        if alias_path.read_bytes() != manifest_path.read_bytes():
            fail(f"generic CLI manifest alias differs from {manifest_name}: {alias_name}")

        add_upload(immutable, archive_path, f"{destination_prefix}/{archive_name}", cdn_base)
        add_upload(immutable, manifest_path, f"{destination_prefix}/manifest.json", cdn_base)
        add_upload(
            promotion,
            manifest_path,
            f"download/chat2db-cli/latest/{os_name}/{architecture}/manifest.json",
            cdn_base,
            refresh=True,
        )

    skill_archive_name = f"chat2db-skill-{version}.zip"
    skill_manifest_name = f"manifest-{version}.json"
    skill_alias_name = "manifest-skill.json"
    skill_archive = require_regular_file(artifacts / skill_archive_name, artifacts)
    skill_manifest_path = require_regular_file(artifacts / skill_manifest_name, artifacts)
    skill_alias_path = require_regular_file(artifacts / skill_alias_name, artifacts)
    expected_assets.update((skill_archive_name, skill_manifest_name, skill_alias_name))
    validate_skill_archive(skill_archive)
    skill_manifest = load_json(skill_manifest_path, artifacts)
    skill_prefix = f"download/chat2db-skill/{version}"
    validate_manifest_artifact(
        skill_manifest,
        skill_manifest_path,
        version_field="skill_version",
        version=version,
        expected_url=f"{cdn_base}/{skill_prefix}/{skill_archive_name}",
        expected_type="skill_zip",
        archive_path=skill_archive,
    )
    if skill_alias_path.read_bytes() != skill_manifest_path.read_bytes():
        fail(f"generic Skill manifest alias differs from {skill_manifest_name}")
    add_upload(immutable, skill_archive, f"{skill_prefix}/{skill_archive_name}", cdn_base)
    add_upload(immutable, skill_manifest_path, f"{skill_prefix}/manifest.json", cdn_base)
    add_upload(
        promotion,
        skill_manifest_path,
        "download/chat2db-skill/latest/manifest.json",
        cdn_base,
        refresh=True,
    )

    installers = (
        (f"install-cli-{version}.sh", "install-cli.sh", "cli.sh"),
        (f"install-cli-{version}.ps1", "install-cli.ps1", "cli.ps1"),
        (f"install-skill-{version}.sh", "install-skill.sh", "skill.sh"),
    )
    for asset_name, alias_name, public_name in installers:
        source = require_regular_file(artifacts / asset_name, artifacts)
        alias = require_regular_file(artifacts / alias_name, artifacts)
        expected_assets.update((asset_name, alias_name))
        if alias.read_bytes() != source.read_bytes():
            fail(f"generic installer alias differs from {asset_name}: {alias_name}")
        add_upload(immutable, source, f"install/{version}/{public_name}", cdn_base)
        add_upload(promotion, source, f"install/{public_name}", cdn_base, refresh=True)

    sbom_name = "chat2db-cli.cdx.json"
    sbom_path = require_regular_file(artifacts / sbom_name, artifacts)
    expected_assets.add(sbom_name)
    sbom = load_json(sbom_path, artifacts)
    metadata = sbom.get("metadata")
    component = metadata.get("component") if isinstance(metadata, dict) else None
    if (
        sbom.get("bomFormat") != "CycloneDX"
        or not isinstance(component, dict)
        or component.get("version") != version
    ):
        fail(f"SBOM does not describe Chat2DB CLI {version}: {sbom_path}")
    add_upload(
        immutable,
        sbom_path,
        f"download/chat2db-cli/{version}/{sbom_name}",
        cdn_base,
    )

    checksum_name = "SHA256SUMS"
    checksum_path = require_regular_file(artifacts / checksum_name, artifacts)
    validate_checksum_file(checksum_path, artifacts, expected_assets)
    add_upload(
        immutable,
        checksum_path,
        f"download/chat2db-cli/{version}/{checksum_name}",
        cdn_base,
    )

    provenance_name = f"chat2db-cli-{version}.intoto.jsonl"
    provenance_path = require_regular_file(artifacts / provenance_name, artifacts)
    try:
        provenance_records = [
            json.loads(line)
            for line in provenance_path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        if not provenance_records or any(
            not isinstance(record, dict) for record in provenance_records
        ):
            fail(f"invalid provenance bundle: {provenance_path}")
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid provenance bundle {provenance_path}: {error}")

    actual_assets = {path.name for path in artifacts.iterdir()}
    allowed_assets = expected_assets | {checksum_name, provenance_name}
    if actual_assets != allowed_assets:
        missing = sorted(allowed_assets - actual_assets)
        unexpected = sorted(actual_assets - allowed_assets)
        fail(f"release bundle asset set mismatch; missing={missing}, unexpected={unexpected}")

    return {
        "version": version,
        "cdn_base_url": cdn_base,
        "immutable": immutable,
        "promotion": promotion,
    }


def write_tsv(path: Path, rows: list[tuple[str, ...]]) -> None:
    for row in rows:
        for value in row:
            reject_control(value, f"field written to {path.name}")
    path.write_text("".join("\t".join(row) + "\n" for row in rows), encoding="utf-8")


def write_plan_files(plan: dict, output_argument: Path) -> None:
    reject_control(str(output_argument), "output directory")
    if output_argument.exists() and output_argument.is_symlink():
        fail(f"output directory must not be a symbolic link: {output_argument}")
    output_argument.mkdir(parents=True, exist_ok=True)
    output = output_argument.resolve()
    (output / "plan.json").write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")

    immutable = plan["immutable"]
    promotion = plan["promotion"]
    write_tsv(
        output / "immutable-uploads.tsv",
        [
            (item["source"], item["destination"], item["sha256"])
            for item in immutable["uploads"]
        ],
    )
    write_tsv(
        output / "immutable-verify.tsv",
        [(item["url"], item["sha256"], item["name"]) for item in immutable["verify"]],
    )
    write_tsv(
        output / "promotion-uploads.tsv",
        [
            (item["source"], item["destination"], item["sha256"])
            for item in promotion["uploads"]
        ],
    )
    (output / "promotion-refresh.txt").write_text(
        "".join(url + "\n" for url in promotion["refresh"]), encoding="utf-8"
    )
    write_tsv(
        output / "promotion-verify.tsv",
        [(item["url"], item["sha256"], item["name"]) for item in promotion["verify"]],
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact-directory", type=Path, required=True)
    parser.add_argument("--cdn-base-url", required=True)
    parser.add_argument("--output-directory", type=Path, required=True)
    args = parser.parse_args()

    plan = build_release_plan(args.artifact_directory, args.cdn_base_url)
    write_plan_files(plan, args.output_directory)
    print(plan["version"])


if __name__ == "__main__":
    main()
