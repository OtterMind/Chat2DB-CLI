use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256, Sha512};
use url::Url;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::platform::HostOs;

pub const OFFICIAL_CDN_HOST: &str = "cdn.chat2db-ai.com";
const OFFICIAL_RELEASE_BASE: &str =
    "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download";

/// Manifest for a `chat2db` CLI release binary archive.
///
/// Schema: `{ version, artifact: { url, sha256, artifact_type? } }`. The
/// artifact URL is constrained to the official CDN and a release archive suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryManifest {
    pub version: String,
    pub artifact: Artifact,
}

/// Manifest for a Chat2DB Agent Skill bundle zip.
///
/// Schema: `{ skill_version, compatible_cli_versions, bundle_path, artifact:
/// { url, sha256, artifact_type } }`. `compatible_cli_versions` is a semver
/// range expressed as `VersionReq` syntax. Release coordinates are canonical:
/// `bundle_path` is `chat2db-cli`, `artifact_type` is `skill_zip`, and the CDN
/// path and filename contain the exact canonical `skill_version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillManifest {
    pub skill_version: String,
    pub compatible_cli_versions: String,
    /// Canonical top-level directory inside the zip (`chat2db-cli`).
    pub bundle_path: String,
    pub artifact: Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub url: String,
    pub sha256: String,
    pub artifact_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopUpdateArtifact {
    pub edition: Edition,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub url: String,
    pub size: u64,
    pub checksum: DesktopArtifactChecksum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopArtifactChecksum {
    Sha256(String),
    Sha512(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopArtifactRegistry {
    artifacts: Vec<DesktopUpdateArtifact>,
}

#[derive(Debug, Deserialize)]
struct RawArtifact {
    url: String,
    sha256: String,
    artifact_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawBinaryManifest {
    version: String,
    artifact: RawArtifact,
}

#[derive(Debug, Deserialize)]
struct RawSkillManifest {
    skill_version: String,
    compatible_cli_versions: String,
    bundle_path: String,
    artifact: RawArtifact,
}

#[derive(Debug, Deserialize)]
struct RawDesktopArtifactRegistry {
    schema_version: u32,
    artifacts: Vec<RawDesktopArtifact>,
}

#[derive(Debug, Deserialize)]
struct RawDesktopArtifact {
    edition: Edition,
    os: String,
    arch: String,
    version: String,
    url: String,
    size: u64,
    checksum: RawDesktopArtifactChecksum,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "algorithm", content = "value", rename_all = "lowercase")]
enum RawDesktopArtifactChecksum {
    Sha256(String),
    Sha512(String),
}

const ALLOWED_BINARY_SUFFIXES: &[&str] = &[".tar.gz", ".tgz", ".zip"];
const ALLOWED_SKILL_SUFFIXES: &[&str] = &[".zip"];
pub(crate) const OFFICIAL_SKILL_BUNDLE_PATH: &str = "chat2db-cli";
const OFFICIAL_SKILL_ARTIFACT_TYPE: &str = "skill_zip";

impl BinaryManifest {
    pub fn parse(bytes: &[u8], os: &str, arch: &str) -> Result<Self> {
        let raw: RawBinaryManifest = serde_json::from_slice(bytes)?;
        let manifest = Self {
            version: raw.version,
            artifact: Artifact {
                url: raw.artifact.url,
                sha256: raw.artifact.sha256.to_ascii_lowercase(),
                artifact_type: raw.artifact.artifact_type,
            },
        };
        manifest.validate(os, arch)?;
        Ok(manifest)
    }

    fn validate(&self, os: &str, arch: &str) -> Result<()> {
        validate_official_artifact_url(&self.artifact.url, ALLOWED_BINARY_SUFFIXES)?;
        validate_sha256(&self.artifact.sha256)?;
        validate_release_version(&self.version, "version")?;

        let (suffix, artifact_type) = match (os, arch) {
            ("darwin" | "linux", "arm64" | "x86_64") => ("tar.gz", "tarball"),
            ("windows", "x86_64") => ("zip", "zip"),
            _ => {
                return Err(Chat2DbError::InvalidReleaseManifest(format!(
                    "unsupported CLI target: {os}/{arch}"
                )));
            }
        };
        let archive_name = format!("chat2db-{}-{os}-{arch}.{suffix}", self.version);
        let expected_url = format!(
            "https://{OFFICIAL_CDN_HOST}/download/chat2db-cli/{}/{os}/{arch}/{archive_name}",
            self.version
        );
        if self.artifact.url != expected_url
            || self.artifact.artifact_type.as_deref() != Some(artifact_type)
        {
            return Err(Chat2DbError::InvalidReleaseManifest(format!(
                "CLI artifact metadata does not match target {os}/{arch}"
            )));
        }
        Ok(())
    }
}

impl SkillManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw: RawSkillManifest = serde_json::from_slice(bytes)?;
        let manifest = Self {
            skill_version: raw.skill_version,
            compatible_cli_versions: raw.compatible_cli_versions,
            bundle_path: raw.bundle_path,
            artifact: Artifact {
                url: raw.artifact.url,
                sha256: raw.artifact.sha256.to_ascii_lowercase(),
                artifact_type: raw.artifact.artifact_type,
            },
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        validate_official_artifact_url(&self.artifact.url, ALLOWED_SKILL_SUFFIXES)?;
        validate_sha256(&self.artifact.sha256)?;
        validate_release_version(&self.skill_version, "skill_version")?;
        if self.bundle_path != OFFICIAL_SKILL_BUNDLE_PATH {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: format!(
                    "manifest bundle_path is {:?}, expected {:?}",
                    self.bundle_path, OFFICIAL_SKILL_BUNDLE_PATH
                ),
            });
        }
        let archive_name = format!("chat2db-skill-{}.zip", self.skill_version);
        let expected_url = format!(
            "https://{OFFICIAL_CDN_HOST}/download/chat2db-skill/{}/{archive_name}",
            self.skill_version
        );
        if self.artifact.url != expected_url
            || self.artifact.artifact_type.as_deref() != Some(OFFICIAL_SKILL_ARTIFACT_TYPE)
        {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: "manifest Skill artifact metadata does not match skill_version".to_string(),
            });
        }
        semver::VersionReq::parse(&self.compatible_cli_versions).map_err(|_| {
            Chat2DbError::SkillBundleCorrupt {
                reason: format!(
                    "manifest compatible_cli_versions is not a valid semver range: {}",
                    self.compatible_cli_versions
                ),
            }
        })?;
        Ok(())
    }

    /// Return Ok(()) if `cli_version` satisfies `compatible_cli_versions`.
    pub fn check_cli_compatibility(&self, cli_version: &str) -> Result<()> {
        let req = semver::VersionReq::parse(&self.compatible_cli_versions).map_err(|_| {
            Chat2DbError::SkillBundleCorrupt {
                reason: format!(
                    "manifest compatible_cli_versions is not a valid semver range: {}",
                    self.compatible_cli_versions
                ),
            }
        })?;
        let parsed =
            semver::Version::parse(cli_version).map_err(|_| Chat2DbError::SkillBundleCorrupt {
                reason: format!("running CLI version is not valid semver: {cli_version}"),
            })?;
        if req.matches(&parsed) {
            Ok(())
        } else {
            Err(Chat2DbError::SkillVersionIncompatible {
                required: self.compatible_cli_versions.clone(),
                current: cli_version.to_string(),
            })
        }
    }
}

impl DesktopArtifactRegistry {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw: RawDesktopArtifactRegistry = serde_json::from_slice(bytes)?;
        if raw.schema_version != 1 {
            return Err(Chat2DbError::InvalidReleaseManifest(
                "desktop artifact registry schema_version must be 1".to_string(),
            ));
        }
        if raw.artifacts.is_empty() {
            return Err(Chat2DbError::InvalidReleaseManifest(
                "desktop artifact registry must not be empty".to_string(),
            ));
        }

        let mut targets = HashSet::new();
        let mut artifacts = Vec::with_capacity(raw.artifacts.len());
        for raw_artifact in raw.artifacts {
            let os = desktop_host_os(&raw_artifact.os)?;
            if os == HostOs::Windows && raw_artifact.arch != "x86_64" {
                return Err(Chat2DbError::InvalidReleaseManifest(
                    "Windows desktop registry entries must use x86_64".to_string(),
                ));
            }
            let arch = normalize_desktop_arch(os, &raw_artifact.arch)?;
            let target = (
                raw_artifact.edition,
                raw_artifact.os.clone(),
                arch.to_string(),
            );
            if !targets.insert(target) {
                return Err(Chat2DbError::InvalidReleaseManifest(format!(
                    "desktop artifact target is duplicated: {}/{}/{}",
                    raw_artifact.edition, raw_artifact.os, arch
                )));
            }
            validate_release_version(&raw_artifact.version, "version")?;
            if raw_artifact.size == 0 {
                return Err(Chat2DbError::InvalidReleaseManifest(format!(
                    "desktop artifact size must be greater than zero: {}/{}/{}",
                    raw_artifact.edition, raw_artifact.os, arch
                )));
            }

            let expected_name =
                raw_artifact
                    .edition
                    .desktop_artifact_name(&raw_artifact.version, os, arch)?;
            let expected_url = expected_desktop_artifact_url(
                raw_artifact.edition,
                os,
                arch,
                &raw_artifact.version,
                &expected_name,
            )?;
            if raw_artifact.url != expected_url {
                return Err(Chat2DbError::InvalidReleaseManifest(format!(
                    "desktop artifact URL does not match {}/{}/{}",
                    raw_artifact.edition, raw_artifact.os, arch
                )));
            }

            let checksum = validate_desktop_checksum(raw_artifact.edition, raw_artifact.checksum)?;
            artifacts.push(DesktopUpdateArtifact {
                edition: raw_artifact.edition,
                os: raw_artifact.os,
                arch: arch.to_string(),
                version: raw_artifact.version,
                url: raw_artifact.url,
                size: raw_artifact.size,
                checksum,
            });
        }
        Ok(Self { artifacts })
    }

    pub fn artifact(
        &self,
        edition: Edition,
        os: HostOs,
        arch: &str,
    ) -> Result<&DesktopUpdateArtifact> {
        let os_label = desktop_os_label(os)?;
        let arch = normalize_desktop_arch(os, arch)?;
        let mut matches = self.artifacts.iter().filter(|artifact| {
            artifact.edition == edition && artifact.os == os_label && artifact.arch == arch
        });
        let Some(file) = matches.next() else {
            return Err(Chat2DbError::InvalidReleaseManifest(format!(
                "desktop artifact is missing: {edition}/{os_label}/{arch}"
            )));
        };
        if matches.next().is_some() {
            return Err(Chat2DbError::InvalidReleaseManifest(format!(
                "desktop artifact is duplicated: {edition}/{os_label}/{arch}"
            )));
        }
        Ok(file)
    }
}

pub fn embedded_desktop_artifacts() -> &'static [u8] {
    include_bytes!("../desktop-artifacts.json")
}

fn desktop_host_os(os: &str) -> Result<HostOs> {
    match os {
        "darwin" => Ok(HostOs::Macos),
        "windows" => Ok(HostOs::Windows),
        other => Err(Chat2DbError::InvalidReleaseManifest(format!(
            "unsupported desktop artifact OS: {other}"
        ))),
    }
}

fn desktop_os_label(os: HostOs) -> Result<&'static str> {
    match os {
        HostOs::Macos => Ok("darwin"),
        HostOs::Windows => Ok("windows"),
        HostOs::Linux | HostOs::Other => Err(Chat2DbError::UnsupportedPlatform),
    }
}

fn normalize_desktop_arch(os: HostOs, arch: &str) -> Result<&str> {
    match (os, arch) {
        (HostOs::Macos, "arm64" | "x86_64") => Ok(arch),
        (HostOs::Windows, "arm64" | "x86_64") => Ok("x86_64"),
        (_, other) => Err(Chat2DbError::UnsupportedArch(other.to_string())),
    }
}

fn expected_desktop_artifact_url(
    edition: Edition,
    os: HostOs,
    arch: &str,
    version: &str,
    name: &str,
) -> Result<String> {
    match edition {
        Edition::Community => Ok(format!(
            "https://github.com/OtterMind/Chat2DB/releases/download/v{version}/{name}"
        )),
        Edition::Pro | Edition::Local => {
            let channel = if edition == Edition::Pro {
                "download"
            } else {
                "offline"
            };
            let target = match os {
                HostOs::Macos => {
                    let architecture = if arch == "arm64" { "arm64" } else { "x64" };
                    format!("mac/{architecture}")
                }
                HostOs::Windows => "windows".to_string(),
                HostOs::Linux | HostOs::Other => {
                    return Err(Chat2DbError::UnsupportedPlatform);
                }
            };
            Ok(format!(
                "https://{OFFICIAL_CDN_HOST}/{channel}/latest/{target}/{name}"
            ))
        }
    }
}

fn validate_desktop_checksum(
    edition: Edition,
    checksum: RawDesktopArtifactChecksum,
) -> Result<DesktopArtifactChecksum> {
    match (edition, checksum) {
        (Edition::Community, RawDesktopArtifactChecksum::Sha256(value)) => {
            let value = value.to_ascii_lowercase();
            validate_sha256(&value)?;
            Ok(DesktopArtifactChecksum::Sha256(value))
        }
        (Edition::Pro | Edition::Local, RawDesktopArtifactChecksum::Sha512(value)) => {
            let decoded = STANDARD.decode(&value).map_err(|_| {
                Chat2DbError::InvalidReleaseManifest(
                    "desktop artifact sha512 is not valid base64".to_string(),
                )
            })?;
            if decoded.len() != 64 {
                return Err(Chat2DbError::InvalidReleaseManifest(
                    "desktop artifact sha512 must decode to 64 bytes".to_string(),
                ));
            }
            Ok(DesktopArtifactChecksum::Sha512(value))
        }
        _ => Err(Chat2DbError::InvalidReleaseManifest(
            "desktop artifact checksum algorithm does not match the edition".to_string(),
        )),
    }
}

/// GitHub Release URL for the independently hosted CLI release manifest.
pub fn binary_manifest_url(os: &str, arch: &str) -> String {
    format!("{OFFICIAL_RELEASE_BASE}/manifest-cli-{os}-{arch}.json")
}

/// GitHub Release URL for the independently hosted Skill release manifest.
pub fn skill_manifest_url() -> String {
    format!("{OFFICIAL_RELEASE_BASE}/manifest-skill.json")
}

pub fn validate_official_artifact_url(url: &str, allowed_suffixes: &[&str]) -> Result<()> {
    let parsed = Url::parse(url)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(OFFICIAL_CDN_HOST)
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Chat2DbError::UnofficialArtifactUrl);
    }
    let path = parsed.path();
    if !allowed_suffixes.iter().any(|suffix| path.ends_with(suffix)) {
        return Err(Chat2DbError::UnsupportedArtifact);
    }
    Ok(())
}

fn validate_release_version(version: &str, field: &str) -> Result<()> {
    let parsed = semver::Version::parse(version).map_err(|_| {
        Chat2DbError::InvalidReleaseManifest(format!(
            "{field} must be canonical SemVer without a v prefix: {version}"
        ))
    })?;
    if parsed.to_string() != version {
        return Err(Chat2DbError::InvalidReleaseManifest(format!(
            "{field} must be canonical SemVer without a v prefix: {version}"
        )));
    }
    Ok(())
}

pub fn validate_sha256(sha256: &str) -> Result<()> {
    if sha256.len() == 64 && sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(Chat2DbError::InvalidSha256)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Chat2DbError::ChecksumMismatch {
            expected: expected.to_ascii_lowercase(),
            actual,
        })
    }
}

pub fn verify_file_sha256(path: &Path, expected: &str) -> Result<()> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(Chat2DbError::ChecksumMismatch {
            expected: expected.to_ascii_lowercase(),
            actual,
        })
    }
}

pub fn verify_file_sha512(path: &Path, expected_base64: &str) -> Result<()> {
    let expected = STANDARD.decode(expected_base64).map_err(|_| {
        Chat2DbError::InvalidReleaseManifest(
            "desktop update artifact sha512 is not valid base64".to_string(),
        )
    })?;
    if expected.len() != 64 {
        return Err(Chat2DbError::InvalidReleaseManifest(
            "desktop update artifact sha512 must decode to 64 bytes".to_string(),
        ));
    }
    let mut file = File::open(path)?;
    let mut hasher = Sha512::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hasher.finalize();
    if actual.as_slice() != expected.as_slice() {
        return Err(Chat2DbError::ChecksumMismatch {
            expected: expected_base64.to_string(),
            actual: STANDARD.encode(actual),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn builds_binary_manifest_url() {
        assert_eq!(
            binary_manifest_url("darwin", "arm64"),
            "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-cli-darwin-arm64.json"
        );
    }

    #[test]
    fn builds_skill_manifest_url() {
        assert_eq!(
            skill_manifest_url(),
            "https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/manifest-skill.json"
        );
    }

    #[test]
    fn parses_binary_manifest() {
        let body = br#"{
            "version": "0.1.1",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/0.1.1/darwin/arm64/chat2db-0.1.1-darwin-arm64.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "tarball"
            }
        }"#;
        let manifest = BinaryManifest::parse(body, "darwin", "arm64").unwrap();
        assert_eq!(manifest.version, "0.1.1");
        assert!(manifest.artifact.url.ends_with(".tar.gz"));
    }

    #[test]
    fn parses_binary_zip_manifest() {
        let body = br#"{
            "version": "0.1.1",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/0.1.1/windows/x86_64/chat2db-0.1.1-windows-x86_64.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "zip"
            }
        }"#;
        let manifest = BinaryManifest::parse(body, "windows", "x86_64").unwrap();
        assert!(manifest.artifact.url.ends_with(".zip"));
    }

    #[test]
    fn binary_manifest_rejects_third_party_host() {
        let body = br#"{
            "version": "0.1.1",
            "artifact": {
                "url": "https://example.com/chat2db.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }
        }"#;
        let err = BinaryManifest::parse(body, "darwin", "arm64").unwrap_err();
        assert!(matches!(err, Chat2DbError::UnofficialArtifactUrl));
    }

    #[test]
    fn binary_manifest_rejects_wrong_target() {
        let body = br#"{
            "version": "0.1.1",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/0.1.1/linux/x86_64/chat2db-0.1.1-linux-x86_64.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "tarball"
            }
        }"#;
        let err = BinaryManifest::parse(body, "darwin", "arm64").unwrap_err();
        assert!(matches!(err, Chat2DbError::InvalidReleaseManifest(_)));
    }

    #[test]
    fn binary_manifest_rejects_mismatched_version_and_artifact_type() {
        let wrong_version = br#"{
            "version": "0.1.1",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/0.1.2/darwin/arm64/chat2db-0.1.2-darwin-arm64.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "tarball"
            }
        }"#;
        assert!(matches!(
            BinaryManifest::parse(wrong_version, "darwin", "arm64").unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));

        let wrong_type = br#"{
            "version": "0.1.1",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/0.1.1/darwin/arm64/chat2db-0.1.1-darwin-arm64.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "zip"
            }
        }"#;
        assert!(matches!(
            BinaryManifest::parse(wrong_type, "darwin", "arm64").unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));
    }

    #[test]
    fn binary_manifest_requires_canonical_version() {
        let body = br#"{
            "version": "v0.1.1",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/v0.1.1/darwin/arm64/chat2db-v0.1.1-darwin-arm64.tar.gz",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "tarball"
            }
        }"#;
        assert!(matches!(
            BinaryManifest::parse(body, "darwin", "arm64").unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));
    }

    #[test]
    fn parses_skill_manifest() {
        let body = br#"{
            "skill_version": "0.1.1",
            "compatible_cli_versions": ">=0.1, <0.2",
            "bundle_path": "chat2db-cli",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.1/chat2db-skill-0.1.1.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "skill_zip"
            }
        }"#;
        let manifest = SkillManifest::parse(body).unwrap();
        assert_eq!(manifest.skill_version, "0.1.1");
        assert_eq!(manifest.bundle_path, "chat2db-cli");
        manifest.check_cli_compatibility("0.1.0").unwrap();
        let err = manifest.check_cli_compatibility("0.2.0").unwrap_err();
        assert!(matches!(err, Chat2DbError::SkillVersionIncompatible { .. }));
    }

    #[test]
    fn skill_manifest_requires_canonical_release_coordinates() {
        let noncanonical_version = br#"{
            "skill_version": "v0.1.1",
            "compatible_cli_versions": ">=0.1, <0.2",
            "bundle_path": "chat2db-cli",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/v0.1.1/chat2db-skill-v0.1.1.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "skill_zip"
            }
        }"#;
        assert!(matches!(
            SkillManifest::parse(noncanonical_version).unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));

        let wrong_bundle_path = br#"{
            "skill_version": "0.1.1",
            "compatible_cli_versions": ">=0.1, <0.2",
            "bundle_path": "bundle",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.1/chat2db-skill-0.1.1.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "skill_zip"
            }
        }"#;
        assert!(matches!(
            SkillManifest::parse(wrong_bundle_path).unwrap_err(),
            Chat2DbError::SkillBundleCorrupt { .. }
        ));

        let wrong_artifact_type = br#"{
            "skill_version": "0.1.1",
            "compatible_cli_versions": ">=0.1, <0.2",
            "bundle_path": "chat2db-cli",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.1/chat2db-skill-0.1.1.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "zip"
            }
        }"#;
        assert!(matches!(
            SkillManifest::parse(wrong_artifact_type).unwrap_err(),
            Chat2DbError::SkillBundleCorrupt { .. }
        ));

        let mismatched_url_version = br#"{
            "skill_version": "0.1.1",
            "compatible_cli_versions": ">=0.1, <0.2",
            "bundle_path": "chat2db-cli",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.2/chat2db-skill-0.1.2.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "skill_zip"
            }
        }"#;
        assert!(matches!(
            SkillManifest::parse(mismatched_url_version).unwrap_err(),
            Chat2DbError::SkillBundleCorrupt { .. }
        ));
    }

    #[test]
    fn skill_manifest_rejects_traversal_in_bundle_path() {
        let body = br#"{
            "skill_version": "0.1.0",
            "compatible_cli_versions": ">=0.1",
            "bundle_path": "../etc",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.0/chat2db-skill-0.1.0.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "skill_zip"
            }
        }"#;
        let err = SkillManifest::parse(body).unwrap_err();
        assert!(matches!(err, Chat2DbError::SkillBundleCorrupt { .. }));
    }

    #[test]
    fn skill_manifest_rejects_invalid_semver_range() {
        let body = br#"{
            "skill_version": "0.1.0",
            "compatible_cli_versions": "not-a-range",
            "bundle_path": "chat2db-cli",
            "artifact": {
                "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/0.1.0/chat2db-skill-0.1.0.zip",
                "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "artifact_type": "skill_zip"
            }
        }"#;
        let err = SkillManifest::parse(body).unwrap_err();
        assert!(matches!(err, Chat2DbError::SkillBundleCorrupt { .. }));
    }

    #[test]
    fn embedded_desktop_registry_covers_every_supported_target() {
        let registry = DesktopArtifactRegistry::parse(embedded_desktop_artifacts()).unwrap();
        assert_eq!(registry.artifacts.len(), 9);

        for edition in Edition::ALL {
            for (os, arch) in [
                (HostOs::Macos, "arm64"),
                (HostOs::Macos, "x86_64"),
                (HostOs::Windows, "x86_64"),
            ] {
                let artifact = registry.artifact(edition, os, arch).unwrap();
                assert!(artifact.size > 0);
            }
        }

        let community = registry
            .artifact(Edition::Community, HostOs::Macos, "arm64")
            .unwrap();
        assert!(
            community
                .url
                .starts_with("https://github.com/OtterMind/Chat2DB/releases/download/v5.3.0/")
        );
        assert!(matches!(
            community.checksum,
            DesktopArtifactChecksum::Sha256(_)
        ));

        let pro = registry
            .artifact(Edition::Pro, HostOs::Windows, "arm64")
            .unwrap();
        assert_eq!(pro.arch, "x86_64");
        assert!(matches!(pro.checksum, DesktopArtifactChecksum::Sha512(_)));
    }

    #[test]
    fn desktop_registry_rejects_unofficial_urls_and_weak_checksum_selection() {
        let unofficial = br#"{
          "schema_version": 1,
          "artifacts": [{
            "edition": "community",
            "os": "windows",
            "arch": "x86_64",
            "version": "5.3.0",
            "url": "https://example.com/Chat2DB-Community-5.3.0.msi",
            "size": 123,
            "checksum": {
              "algorithm": "sha256",
              "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }
          }]
        }"#;
        assert!(matches!(
            DesktopArtifactRegistry::parse(unofficial).unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));

        let wrong_algorithm = br#"{
          "schema_version": 1,
          "artifacts": [{
            "edition": "pro",
            "os": "windows",
            "arch": "x86_64",
            "version": "3.1.15",
            "url": "https://cdn.chat2db-ai.com/download/latest/windows/Chat2DB-Pro-3.1.15.exe",
            "size": 123,
            "checksum": {
              "algorithm": "sha256",
              "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            }
          }]
        }"#;
        assert!(matches!(
            DesktopArtifactRegistry::parse(wrong_algorithm).unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));
    }

    #[test]
    fn desktop_registry_rejects_duplicate_targets() {
        let entry = r#"{
          "edition": "local",
          "os": "windows",
          "arch": "x86_64",
          "version": "3.1.16",
          "url": "https://cdn.chat2db-ai.com/offline/latest/windows/Chat2DB-Local-3.1.16.exe",
          "size": 123,
          "checksum": {
            "algorithm": "sha512",
            "value": "QlZ37Lwx5zbkcJ6w2mRVkRK2klFVkbFYhqVXo67VyHd7eLzZ3KwGhJTPUCtTsJ2rfABkXhRxB377OF0E5JwBpA=="
          }
        }"#;
        let body = format!(r#"{{"schema_version":1,"artifacts":[{entry},{entry}]}}"#);
        assert!(matches!(
            DesktopArtifactRegistry::parse(body.as_bytes()).unwrap_err(),
            Chat2DbError::InvalidReleaseManifest(_)
        ));
    }

    #[test]
    fn verifies_file_sha512() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        fs::write(temp.path(), b"chat2db").unwrap();
        let expected = STANDARD.encode(Sha512::digest(b"chat2db"));
        verify_file_sha512(temp.path(), &expected).unwrap();
        assert!(matches!(
            verify_file_sha512(temp.path(), &STANDARD.encode([0_u8; 64])).unwrap_err(),
            Chat2DbError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn verifies_checksum() {
        let bytes = b"chat2db";
        let digest = sha256_hex(bytes);
        verify_sha256(bytes, &digest).unwrap();
        let err = verify_sha256(
            bytes,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap_err();
        assert!(matches!(err, Chat2DbError::ChecksumMismatch { .. }));
    }
}
