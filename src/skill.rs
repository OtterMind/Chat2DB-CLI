//! `chat2db skill install|uninstall|update` for the Chat2DB Agent Skill bundle.
//!
//! Drops the bundle into `./chat2db-cli/` by default, or into a supported
//! client's global skill directory when `--client` is explicitly provided.
//! Source mode:
//!
//! * **Release** (default): fetch `SkillManifest` from the latest GitHub
//!   Release, then download and SHA-256-verify its canonical CDN bundle.
//!
//! Foreign-content protection: an existing target directory is classified as
//! Ours / Git / Foreign / Empty. We refuse to overwrite Git checkouts always
//! (the user should `git pull`); foreign content is only overwritten with
//! `--force`. Allowed replacements are staged on the destination filesystem
//! and retain the prior target until the verified bundle is atomically moved.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::download::safe_extract_zip;
use crate::error::{Chat2DbError, Result};
use crate::http::{DownloadProgress, HttpClient};
use crate::manifest::{
    OFFICIAL_SKILL_BUNDLE_PATH, SkillManifest, sha256_hex, skill_manifest_url, verify_sha256,
};
use crate::platform::{ExclusiveLock, acquire_exclusive_lock, sync_file, write_private_atomic};

/// Canonical release bundle and on-disk directory name.
pub const SKILL_DIR_NAME: &str = OFFICIAL_SKILL_BUNDLE_PATH;

/// Frontmatter `name:` value the bundle's `SKILL.md` must declare for us to
/// treat an existing target as ours.
pub const SKILL_FRONTMATTER_NAME: &str = "chat2db-cli";

const SKILL_TRANSACTION_PENDING: &str = ".chat2db-cli.pending";
const SKILL_TRANSACTION_BACKUP: &str = ".chat2db-cli.backup";
const SKILL_TRANSACTION_CONFLICT: &str = ".chat2db-cli.conflict";
const SKILL_TRANSACTION_MARKER: &str = ".chat2db-cli.transaction";
const SKILL_TRANSACTION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SkillClient {
    /// Claude Code: `~/.claude/skills/chat2db-cli/`.
    ClaudeCode,
    /// Codex: `~/.codex/skills/chat2db-cli/`.
    Codex,
}

impl SkillClient {
    pub fn skills_dir(self, home_dir: &Path) -> PathBuf {
        match self {
            SkillClient::ClaudeCode => home_dir.join(".claude").join("skills"),
            SkillClient::Codex => home_dir.join(".codex").join("skills"),
        }
    }

    pub fn target_dir(self, home_dir: &Path) -> PathBuf {
        self.skills_dir(home_dir).join(SKILL_DIR_NAME)
    }
}

#[derive(Debug, Clone)]
pub struct SkillTargetRequest {
    pub client: Option<SkillClient>,
    pub home_dir: PathBuf,
    pub current_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SkillInstallRequest {
    pub target: SkillTargetRequest,
    pub force: bool,
    pub progress: bool,
    /// Running CLI version (`env!("CARGO_PKG_VERSION")` in production).
    pub cli_version: String,
}

#[derive(Debug, Clone)]
pub struct SkillUninstallRequest {
    pub target: SkillTargetRequest,
}

#[derive(Debug, Clone)]
pub struct SkillUpdateRequest {
    pub target: SkillTargetRequest,
    pub progress: bool,
    pub cli_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillOutcome {
    pub command: &'static str,
    pub operation: &'static str,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<SkillClient>,
    pub target_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallMode {
    Install,
    Update,
}

pub fn install<C: HttpClient>(client: &C, request: SkillInstallRequest) -> Result<SkillOutcome> {
    install_with_mode(client, request, InstallMode::Install)
}

fn install_with_mode<C: HttpClient>(
    client: &C,
    request: SkillInstallRequest,
    mode: InstallMode,
) -> Result<SkillOutcome> {
    let resolved = resolve_target(&request.target);
    let target = resolved.target_dir;

    fs::create_dir_all(&resolved.parent_dir)?;
    let _lock = acquire_skill_target_lock(&request.target, &target)?;
    let transaction = SkillTransactionPaths::new(&resolved.parent_dir);
    recover_skill_transaction(&target, &transaction)?;

    let prior = classify_existing(&target)?;
    match (mode, prior) {
        (InstallMode::Install, ExistingState::Empty) => {}
        (InstallMode::Update, ExistingState::Empty) => {
            return Err(Chat2DbError::SkillNotInstalled { path: target });
        }
        (_, ExistingState::Git) => {
            return Err(Chat2DbError::SkillTargetManagedExternally { path: target });
        }
        (InstallMode::Install, ExistingState::Ours) => {
            if !request.force {
                return Err(Chat2DbError::SkillAlreadyInstalled { path: target });
            }
        }
        (InstallMode::Update, ExistingState::Ours) => {}
        (InstallMode::Install, ExistingState::Foreign) => {
            if !request.force {
                return Err(Chat2DbError::SkillTargetForeign { path: target });
            }
        }
        (InstallMode::Update, ExistingState::Foreign) => {
            return Err(Chat2DbError::SkillTargetForeign { path: target });
        }
    }

    let release = fetch_skill_release(client, request.progress, &request.cli_version)?;
    if mode == InstallMode::Update {
        let installed_version = installed_skill_version(&target)?;
        let available_version =
            semver::Version::parse(&release.manifest.skill_version).map_err(|_| {
                Chat2DbError::InvalidReleaseManifest(format!(
                    "skill_version is not valid SemVer: {}",
                    release.manifest.skill_version
                ))
            })?;
        match available_version.cmp(&installed_version) {
            std::cmp::Ordering::Less => {
                return Err(Chat2DbError::InvalidReleaseManifest(format!(
                    "refusing to downgrade the installed Skill from {installed_version} to {available_version}"
                )));
            }
            std::cmp::Ordering::Equal => {
                return Ok(SkillOutcome {
                    command: "skill",
                    operation: "update",
                    status: "up_to_date",
                    client: request.target.client,
                    target_path: target,
                    skill_version: Some(release.manifest.skill_version.clone()),
                    manifest_url: Some(release.manifest_url),
                    artifact_url: Some(release.manifest.artifact.url.clone()),
                });
            }
            std::cmp::Ordering::Greater => {}
        }
    }

    let replacing = matches!(prior, ExistingState::Ours | ExistingState::Foreign);
    let previous_identity = replacing.then(|| path_identity(&target)).transpose()?;
    let operation = (|| {
        let mut marker = begin_skill_transaction(&target, &transaction, previous_identity.clone())?;
        fs::create_dir(&transaction.pending)?;

        let staged = stage_from_release(client, &transaction.pending, request.progress, release)?;
        verify_staged_bundle(&staged.path, staged.manifest_skill_version.as_deref())?;
        sync_skill_bundle(&staged.path)?;
        let candidate = SkillBundleIdentity {
            version: staged.skill_version.clone().ok_or_else(|| {
                Chat2DbError::SkillBundleCorrupt {
                    reason: "staged Skill has no version identity".to_string(),
                }
            })?,
            tree_sha256: path_identity(&staged.path)?,
        };
        marker.phase = SkillTransactionPhase::Prepared;
        marker.candidate = Some(candidate.clone());
        write_skill_transaction_marker(&transaction, &marker)?;

        if replacing {
            ensure_path_identity(
                &target,
                previous_identity.as_deref(),
                "Skill target changed before backup",
            )?;
            rename_no_replace(&target, &transaction.backup)?;
            ensure_path_identity(
                &transaction.backup,
                previous_identity.as_deref(),
                "Skill backup does not match the original target",
            )?;
        }
        rename_no_replace(&staged.path, &target)?;
        sync_directory(&resolved.parent_dir)?;
        ensure_candidate_identity(&target, &candidate)?;
        marker.phase = SkillTransactionPhase::Activated;
        write_skill_transaction_marker(&transaction, &marker)?;

        let status = if matches!(prior, ExistingState::Ours) {
            "updated"
        } else {
            "installed"
        };
        let outcome = SkillOutcome {
            command: "skill",
            operation: match mode {
                InstallMode::Install => "install",
                InstallMode::Update => "update",
            },
            status,
            client: request.target.client,
            target_path: target.clone(),
            skill_version: staged.skill_version,
            manifest_url: staged.manifest_url,
            artifact_url: staged.artifact_url,
        };
        finish_skill_transaction(&target, &transaction, &marker)?;
        Ok(outcome)
    })();

    match operation {
        Ok(outcome) => Ok(outcome),
        Err(operation_error) => match recover_skill_transaction(&target, &transaction) {
            Ok(()) => Err(operation_error),
            Err(recovery_error) => Err(std::io::Error::other(format!(
                "failed to install Skill: {operation_error}; transaction recovery also failed: {recovery_error}"
            ))
            .into()),
        },
    }
}

pub fn uninstall(request: SkillUninstallRequest) -> Result<SkillOutcome> {
    let resolved = resolve_target(&request.target);
    let target = resolved.target_dir;
    let _lock = acquire_skill_target_lock(&request.target, &target)?;
    recover_skill_transaction(&target, &SkillTransactionPaths::new(&resolved.parent_dir))?;
    let prior = classify_existing(&target)?;
    let status = match prior {
        ExistingState::Empty => "already_absent",
        ExistingState::Git => {
            return Err(Chat2DbError::SkillTargetManagedExternally { path: target });
        }
        ExistingState::Foreign => {
            return Err(Chat2DbError::SkillTargetForeign { path: target });
        }
        ExistingState::Ours => {
            fs::remove_dir_all(&target)?;
            "removed"
        }
    };

    Ok(SkillOutcome {
        command: "skill",
        operation: "uninstall",
        status,
        client: request.target.client,
        target_path: target,
        skill_version: None,
        manifest_url: None,
        artifact_url: None,
    })
}

pub fn update<C: HttpClient>(client: &C, request: SkillUpdateRequest) -> Result<SkillOutcome> {
    let install_request = SkillInstallRequest {
        target: request.target,
        force: true,
        progress: request.progress,
        cli_version: request.cli_version,
    };
    install_with_mode(client, install_request, InstallMode::Update)
}

#[derive(Debug)]
struct ResolvedSkillTarget {
    target_dir: PathBuf,
    parent_dir: PathBuf,
}

#[derive(Debug)]
struct SkillTransactionPaths {
    pending: PathBuf,
    backup: PathBuf,
    conflict: PathBuf,
    marker: PathBuf,
}

impl SkillTransactionPaths {
    fn new(parent_dir: &Path) -> Self {
        Self {
            pending: parent_dir.join(SKILL_TRANSACTION_PENDING),
            backup: parent_dir.join(SKILL_TRANSACTION_BACKUP),
            conflict: parent_dir.join(SKILL_TRANSACTION_CONFLICT),
            marker: parent_dir.join(SKILL_TRANSACTION_MARKER),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SkillTransactionPhase {
    Staging,
    Prepared,
    Activated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SkillBundleIdentity {
    version: String,
    tree_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SkillTransactionMarker {
    schema_version: u32,
    phase: SkillTransactionPhase,
    target_sha256: String,
    previous_sha256: Option<String>,
    candidate: Option<SkillBundleIdentity>,
}

fn resolve_target(request: &SkillTargetRequest) -> ResolvedSkillTarget {
    let parent_dir = match request.client {
        Some(client) => client.skills_dir(&request.home_dir),
        None => request.current_dir.clone(),
    };
    let target_dir = parent_dir.join(SKILL_DIR_NAME);
    ResolvedSkillTarget {
        target_dir,
        parent_dir,
    }
}

fn begin_skill_transaction(
    target: &Path,
    paths: &SkillTransactionPaths,
    previous_sha256: Option<String>,
) -> Result<SkillTransactionMarker> {
    for path in [
        &paths.pending,
        &paths.backup,
        &paths.conflict,
        &paths.marker,
    ] {
        if path_entry_exists(path)? {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "reserved Skill transaction path already exists: {}",
                    path.display()
                ),
            )
            .into());
        }
    }
    let marker = SkillTransactionMarker {
        schema_version: SKILL_TRANSACTION_SCHEMA_VERSION,
        phase: SkillTransactionPhase::Staging,
        target_sha256: target_path_identity(target),
        previous_sha256,
        candidate: None,
    };
    write_skill_transaction_marker(paths, &marker)?;
    Ok(marker)
}

fn recover_skill_transaction(target: &Path, paths: &SkillTransactionPaths) -> Result<()> {
    let marker_metadata = match fs::symlink_metadata(&paths.marker) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };

    let Some(marker_metadata) = marker_metadata else {
        for path in [&paths.pending, &paths.backup, &paths.conflict] {
            if path_entry_exists(path)? {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "incomplete Skill transaction data exists without marker: {}",
                        path.display()
                    ),
                )
                .into());
            }
        }
        return Ok(());
    };

    if !marker_metadata.file_type().is_file() {
        return Err(std::io::Error::other(format!(
            "Skill transaction marker is not a regular file: {}",
            paths.marker.display()
        ))
        .into());
    }
    let marker = read_skill_transaction_marker(target, paths)?;
    match marker.phase {
        SkillTransactionPhase::Staging => recover_staging_transaction(target, paths, &marker),
        SkillTransactionPhase::Prepared => recover_uncommitted_transaction(target, paths, &marker),
        SkillTransactionPhase::Activated => {
            let candidate = marker_candidate(&marker)?;
            if candidate_matches(target, candidate)? {
                commit_skill_transaction(target, paths, &marker, true)
            } else if marker.previous_sha256.is_some() {
                rollback_to_previous(target, paths, &marker)
            } else {
                preserve_failed_fresh_activation(target, paths, candidate)
            }
        }
    }
}

fn finish_skill_transaction(
    target: &Path,
    paths: &SkillTransactionPaths,
    marker: &SkillTransactionMarker,
) -> Result<()> {
    commit_skill_transaction(target, paths, marker, false)
}

fn write_skill_transaction_marker(
    paths: &SkillTransactionPaths,
    marker: &SkillTransactionMarker,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(marker)?;
    bytes.push(b'\n');
    write_private_atomic(&paths.marker, &bytes)
}

fn read_skill_transaction_marker(
    target: &Path,
    paths: &SkillTransactionPaths,
) -> Result<SkillTransactionMarker> {
    let marker: SkillTransactionMarker = serde_json::from_slice(&fs::read(&paths.marker)?)
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "invalid Skill transaction marker {}: {error}",
                    paths.marker.display()
                ),
            )
        })?;
    if marker.schema_version != SKILL_TRANSACTION_SCHEMA_VERSION
        || marker.target_sha256 != target_path_identity(target)
        || marker
            .previous_sha256
            .as_deref()
            .is_some_and(|value| !is_sha256(value))
        || marker
            .candidate
            .as_ref()
            .is_some_and(|candidate| !is_sha256(&candidate.tree_sha256))
        || matches!(marker.phase, SkillTransactionPhase::Staging) && marker.candidate.is_some()
        || !matches!(marker.phase, SkillTransactionPhase::Staging) && marker.candidate.is_none()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "Skill transaction marker is invalid: {}",
                paths.marker.display()
            ),
        )
        .into());
    }
    Ok(marker)
}

fn recover_staging_transaction(
    target: &Path,
    paths: &SkillTransactionPaths,
    marker: &SkillTransactionMarker,
) -> Result<()> {
    if path_entry_exists(&paths.backup)? || path_entry_exists(&paths.conflict)? {
        return Err(transaction_state_error(
            paths,
            "staging transaction unexpectedly contains recovery data",
        ));
    }
    if let Some(previous) = marker.previous_sha256.as_deref() {
        ensure_path_identity(
            target,
            Some(previous),
            "Skill target changed during staging",
        )?;
    }
    cleanup_transaction(paths)
}

fn recover_uncommitted_transaction(
    target: &Path,
    paths: &SkillTransactionPaths,
    marker: &SkillTransactionMarker,
) -> Result<()> {
    let candidate = marker_candidate(marker)?;
    if marker.previous_sha256.is_some() {
        return rollback_to_previous(target, paths, marker);
    }

    if !path_entry_exists(target)? {
        return cleanup_transaction(paths);
    }
    if !candidate_matches(target, candidate)? {
        cleanup_transaction(paths)?;
        return Err(std::io::Error::other(format!(
            "an unexpected Skill target appeared during installation and was preserved at {}",
            target.display()
        ))
        .into());
    }

    move_target_to_conflict(target, paths)?;
    if !candidate_matches(&paths.conflict, candidate)? {
        return Err(transaction_state_error(
            paths,
            "the uncommitted target changed while it was being preserved",
        ));
    }
    remove_path_entry(&paths.conflict)?;
    cleanup_transaction(paths)
}

fn rollback_to_previous(
    target: &Path,
    paths: &SkillTransactionPaths,
    marker: &SkillTransactionMarker,
) -> Result<()> {
    let previous = marker.previous_sha256.as_deref().ok_or_else(|| {
        transaction_state_error(paths, "rollback marker has no previous target identity")
    })?;
    let candidate = marker_candidate(marker)?;

    if path_entry_exists(&paths.conflict)? {
        return finish_conflict_rollback(target, paths, previous);
    }

    if !path_entry_exists(&paths.backup)? {
        if path_entry_exists(target)? && path_identity(target)? == previous {
            return cleanup_transaction(paths);
        }
        return Err(transaction_state_error(
            paths,
            "cannot roll back because the verified previous Skill backup is missing",
        ));
    }
    ensure_path_identity(
        &paths.backup,
        Some(previous),
        "Skill backup no longer matches the previous target",
    )?;

    let mut preserved_conflict = false;
    if path_entry_exists(target)? {
        if path_identity(target)? == previous {
            remove_path_entry(&paths.backup)?;
            cleanup_transaction(paths)?;
            return Ok(());
        }
        move_target_to_conflict(target, paths)?;
        preserved_conflict = !candidate_matches(&paths.conflict, candidate)?;
    }

    rename_no_replace(&paths.backup, target)?;
    sync_directory(paths.parent_dir()?)?;
    ensure_path_identity(
        target,
        Some(previous),
        "restored Skill does not match the previous target",
    )?;

    if path_entry_exists(&paths.conflict)? && !preserved_conflict {
        remove_path_entry(&paths.conflict)?;
    }
    cleanup_transaction(paths)?;
    if preserved_conflict {
        return Err(std::io::Error::other(format!(
            "restored the previous Skill and preserved an unexpected target at {}",
            paths.conflict.display()
        ))
        .into());
    }
    Ok(())
}

fn finish_conflict_rollback(
    target: &Path,
    paths: &SkillTransactionPaths,
    previous: &str,
) -> Result<()> {
    if !path_entry_exists(target)? {
        ensure_path_identity(
            &paths.backup,
            Some(previous),
            "Skill backup no longer matches the previous target",
        )?;
        rename_no_replace(&paths.backup, target)?;
        sync_directory(paths.parent_dir()?)?;
    }
    ensure_path_identity(
        target,
        Some(previous),
        "cannot finish rollback because the target is not the previous Skill",
    )?;
    if path_entry_exists(&paths.backup)? {
        ensure_path_identity(
            &paths.backup,
            Some(previous),
            "remaining Skill backup no longer matches the previous target",
        )?;
        remove_path_entry(&paths.backup)?;
    }
    cleanup_transaction(paths)?;
    Err(std::io::Error::other(format!(
        "restored the previous Skill and preserved an unexpected target at {}",
        paths.conflict.display()
    ))
    .into())
}

fn preserve_failed_fresh_activation(
    target: &Path,
    paths: &SkillTransactionPaths,
    _candidate: &SkillBundleIdentity,
) -> Result<()> {
    if path_entry_exists(target)? {
        move_target_to_conflict(target, paths)?;
    }
    cleanup_transaction(paths)?;
    Err(std::io::Error::other(format!(
        "the activated Skill could not be verified; preserved it at {}",
        paths.conflict.display()
    ))
    .into())
}

fn commit_skill_transaction(
    target: &Path,
    paths: &SkillTransactionPaths,
    marker: &SkillTransactionMarker,
    allow_missing_backup: bool,
) -> Result<()> {
    if path_entry_exists(&paths.conflict)? {
        return Err(transaction_state_error(
            paths,
            "cannot commit while preserved conflict data exists",
        ));
    }
    ensure_candidate_identity(target, marker_candidate(marker)?)?;
    match marker.previous_sha256.as_deref() {
        Some(previous) if path_entry_exists(&paths.backup)? => {
            ensure_path_identity(
                &paths.backup,
                Some(previous),
                "Skill backup no longer matches the previous target",
            )?;
            remove_path_entry(&paths.backup)?;
        }
        Some(_) if !allow_missing_backup => {
            return Err(transaction_state_error(
                paths,
                "verified previous Skill backup is missing before commit",
            ));
        }
        Some(_) | None => {}
    }
    cleanup_transaction(paths)
}

fn move_target_to_conflict(target: &Path, paths: &SkillTransactionPaths) -> Result<()> {
    if path_entry_exists(&paths.conflict)? {
        return Err(transaction_state_error(
            paths,
            "cannot preserve target because conflict path already exists",
        ));
    }
    rename_no_replace(target, &paths.conflict)?;
    sync_directory(paths.parent_dir()?)
}

fn cleanup_transaction(paths: &SkillTransactionPaths) -> Result<()> {
    remove_path_entry(&paths.pending)?;
    remove_file_if_exists(&paths.marker)?;
    sync_directory(paths.parent_dir()?)
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn marker_candidate(marker: &SkillTransactionMarker) -> Result<&SkillBundleIdentity> {
    marker.candidate.as_ref().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Skill transaction marker has no candidate identity",
        )
        .into()
    })
}

fn transaction_state_error(paths: &SkillTransactionPaths, reason: &str) -> Chat2DbError {
    std::io::Error::other(format!(
        "{reason}; transaction data is at {}",
        paths
            .parent_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| paths.marker.display().to_string())
    ))
    .into()
}

impl SkillTransactionPaths {
    fn parent_dir(&self) -> Result<&Path> {
        self.marker.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Skill transaction marker has no parent directory",
            )
            .into()
        })
    }
}

fn installed_skill_version(target: &Path) -> Result<semver::Version> {
    let frontmatter = read_frontmatter(&target.join("SKILL.md"))?;
    let version = frontmatter
        .field("version")
        .ok_or_else(|| Chat2DbError::SkillBundleCorrupt {
            reason: format!(
                "installed SKILL.md at {} has no version field",
                target.display()
            ),
        })?;
    semver::Version::parse(&version).map_err(|_| Chat2DbError::SkillBundleCorrupt {
        reason: format!(
            "installed SKILL.md at {} has invalid SemVer version {version:?}",
            target.display()
        ),
    })
}

fn ensure_candidate_identity(target: &Path, candidate: &SkillBundleIdentity) -> Result<()> {
    if candidate_matches(target, candidate)? {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "Skill target {} does not match candidate version {} with tree SHA-256 {}",
            target.display(),
            candidate.version,
            candidate.tree_sha256
        ))
        .into())
    }
}

fn candidate_matches(target: &Path, candidate: &SkillBundleIdentity) -> Result<bool> {
    if !path_entry_exists(target)? {
        return Ok(false);
    }
    Ok(path_identity(target)? == candidate.tree_sha256)
}

fn ensure_path_identity(path: &Path, expected: Option<&str>, reason: &str) -> Result<()> {
    let expected = expected.ok_or_else(|| std::io::Error::other(reason.to_string()))?;
    let actual = path_identity(path)?;
    if actual == expected {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "{reason}: expected {expected}, got {actual} at {}",
            path.display()
        ))
        .into())
    }
}

fn target_path_identity(path: &Path) -> String {
    sha256_hex(path.to_string_lossy().as_bytes())
}

fn path_identity(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_path_entry(path, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_path_entry(path: &Path, hasher: &mut Sha256) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        hasher.update(b"symlink\0");
        hash_path_value(hasher, &fs::read_link(path)?);
        return Ok(());
    }
    if file_type.is_file() {
        hasher.update(b"file\0");
        hasher.update(metadata.len().to_le_bytes());
        let mut file = File::open(path)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        return Ok(());
    }
    if file_type.is_dir() {
        hasher.update(b"dir\0");
        let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        hasher.update((entries.len() as u64).to_le_bytes());
        for entry in entries {
            hash_path_value(hasher, Path::new(&entry.file_name()));
            hash_path_entry(&entry.path(), hasher)?;
        }
        return Ok(());
    }

    hasher.update(b"other\0");
    hasher.update(metadata.len().to_le_bytes());
    Ok(())
}

fn hash_path_value(hasher: &mut Sha256, value: &Path) {
    let value = value.to_string_lossy();
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sync_skill_bundle(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(Chat2DbError::SkillBundleCorrupt {
            reason: format!("staged bundle contains symlink at {}", path.display()),
        });
    }
    if metadata.is_file() {
        sync_file(path)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(Chat2DbError::SkillBundleCorrupt {
            reason: format!(
                "staged bundle contains unsupported entry at {}",
                path.display()
            ),
        });
    }
    for entry in fs::read_dir(path)? {
        sync_skill_bundle(&entry?.path())?;
    }
    sync_directory(path)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(target_vendor = "apple")]
fn rename_no_replace(source: &Path, target: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("source path contains a NUL byte"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("target path contains a NUL byte"))?;
    if unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_no_replace(source: &Path, target: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("source path contains a NUL byte"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("target path contains a NUL byte"))?;
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().into())
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
fn rename_no_replace(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target)?;
    Ok(())
}

fn path_entry_exists(path: &Path) -> std::io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn remove_path_entry(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn acquire_skill_target_lock(request: &SkillTargetRequest, target: &Path) -> Result<ExclusiveLock> {
    acquire_exclusive_lock(&skill_target_lock_path(request, target))
}

fn skill_target_lock_path(request: &SkillTargetRequest, target: &Path) -> PathBuf {
    request
        .home_dir
        .join(".chat2db-cli")
        .join("locks")
        .join(format!(
            "skill-{}.lock",
            sha256_hex(target.to_string_lossy().as_bytes())
        ))
}

#[derive(Debug)]
struct StagedBundle {
    /// Absolute path to the staged bundle root (its `SKILL.md` parent).
    path: PathBuf,
    skill_version: Option<String>,
    /// `Some(version)` when the bundle came from a CDN manifest. Used by
    /// `verify_staged_bundle` to assert the frontmatter version matches what
    /// the manifest declared. `None` for `LocalDir` since there's no manifest
    /// to cross-check against.
    manifest_skill_version: Option<String>,
    manifest_url: Option<String>,
    artifact_url: Option<String>,
}

#[derive(Debug)]
struct SkillRelease {
    manifest_url: String,
    manifest: SkillManifest,
}

fn fetch_skill_release<C: HttpClient>(
    client: &C,
    progress: bool,
    cli_version: &str,
) -> Result<SkillRelease> {
    let manifest_url = skill_manifest_url();
    if progress {
        eprintln!("Fetching skill manifest...");
    }
    let manifest = SkillManifest::parse(&client.get(&manifest_url)?)?;
    manifest.check_cli_compatibility(cli_version)?;
    Ok(SkillRelease {
        manifest_url,
        manifest,
    })
}

fn stage_from_release<C: HttpClient>(
    client: &C,
    stage_dir: &Path,
    progress: bool,
    release: SkillRelease,
) -> Result<StagedBundle> {
    let SkillRelease {
        manifest_url,
        manifest,
    } = release;

    let temp_zip = stage_dir.join("skill.zip");
    client.download_to(
        &manifest.artifact.url,
        &temp_zip,
        DownloadProgress::new(progress, "chat2db-skill bundle"),
    )?;
    let bytes = fs::read(&temp_zip)?;
    verify_sha256(&bytes, &manifest.artifact.sha256)?;

    let extract_dir = stage_dir.join("extract");
    fs::create_dir(&extract_dir)?;
    safe_extract_zip(&temp_zip, &extract_dir)?;
    fs::remove_file(&temp_zip)?;

    let bundle_root = extract_dir.join(&manifest.bundle_path);
    let bundle_metadata = fs::symlink_metadata(&bundle_root).ok();
    if !matches!(
        bundle_metadata,
        Some(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink()
    ) {
        return Err(Chat2DbError::SkillBundleCorrupt {
            reason: format!(
                "manifest bundle_path={} is missing or is not a real directory",
                manifest.bundle_path
            ),
        });
    }

    Ok(StagedBundle {
        path: bundle_root,
        skill_version: Some(manifest.skill_version.clone()),
        manifest_skill_version: Some(manifest.skill_version),
        manifest_url: Some(manifest_url),
        artifact_url: Some(manifest.artifact.url),
    })
}

fn verify_staged_bundle(bundle_root: &Path, manifest_version: Option<&str>) -> Result<()> {
    let bundle_metadata =
        fs::symlink_metadata(bundle_root).map_err(|_| Chat2DbError::SkillBundleCorrupt {
            reason: format!(
                "staged bundle root is missing or unreadable: {}",
                bundle_root.display()
            ),
        })?;
    if bundle_metadata.file_type().is_symlink() || !bundle_metadata.file_type().is_dir() {
        return Err(Chat2DbError::SkillBundleCorrupt {
            reason: format!(
                "staged bundle root is not a real directory: {}",
                bundle_root.display()
            ),
        });
    }
    let skill_md = bundle_root.join("SKILL.md");
    let skill_metadata = fs::symlink_metadata(&skill_md).ok();
    if !matches!(skill_metadata, Some(metadata) if metadata.file_type().is_file()) {
        return Err(Chat2DbError::SkillBundleCorrupt {
            reason: format!("staged bundle missing SKILL.md at {}", skill_md.display()),
        });
    }
    let frontmatter = read_frontmatter(&skill_md)?;

    match frontmatter.field("name").as_deref() {
        Some(SKILL_FRONTMATTER_NAME) => {}
        Some(other) => {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: format!(
                    "SKILL.md frontmatter name is {other:?}, expected {SKILL_FRONTMATTER_NAME:?}"
                ),
            });
        }
        None => {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: "SKILL.md frontmatter missing name field".to_string(),
            });
        }
    }

    let frontmatter_version =
        frontmatter
            .field("version")
            .ok_or_else(|| Chat2DbError::SkillBundleCorrupt {
                reason: "SKILL.md frontmatter missing version field".to_string(),
            })?;

    if let Some(expected) = manifest_version
        && frontmatter_version != expected
    {
        return Err(Chat2DbError::SkillBundleCorrupt {
            reason: format!(
                "SKILL.md frontmatter version is {frontmatter_version:?} but manifest declared {expected:?}"
            ),
        });
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingState {
    Empty,
    Git,
    Ours,
    Foreign,
}

fn classify_existing(target: &Path) -> Result<ExistingState> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ExistingState::Empty);
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(ExistingState::Foreign);
    }
    if fs::symlink_metadata(target.join(".git")).is_ok() {
        return Ok(ExistingState::Git);
    }
    let skill_md = target.join("SKILL.md");
    if !skill_md.is_file() {
        return Ok(ExistingState::Foreign);
    }
    let frontmatter = read_frontmatter(&skill_md)?;
    if frontmatter.field("name").as_deref() == Some(SKILL_FRONTMATTER_NAME) {
        Ok(ExistingState::Ours)
    } else {
        Ok(ExistingState::Foreign)
    }
}

#[derive(Debug, Default)]
struct Frontmatter {
    fields: Vec<(String, String)>,
}

impl Frontmatter {
    fn field(&self, key: &str) -> Option<String> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

fn read_frontmatter(path: &Path) -> Result<Frontmatter> {
    let mut file = File::open(path)?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    parse_frontmatter(&buf).ok_or_else(|| Chat2DbError::SkillBundleCorrupt {
        reason: format!("could not parse SKILL.md frontmatter at {}", path.display()),
    })
}

fn parse_frontmatter(content: &str) -> Option<Frontmatter> {
    let body = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;
    let end_idx = body.find("\n---").or_else(|| body.find("\r\n---"))?;
    let yaml = &body[..end_idx];

    let mut fields = Vec::new();
    for line in yaml.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(idx) = line.find(':') else {
            continue;
        };
        let key = line[..idx].trim().to_string();
        let value = line[idx + 1..].trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value)
            .to_string();
        fields.push((key, value));
    }
    Some(Frontmatter { fields })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Seek, Write};
    use std::sync::mpsc;
    use std::time::Duration;
    use zip::write::SimpleFileOptions;

    fn write_skill_md(dir: &Path, name: &str, compat: &str) {
        write_skill_md_with_version(dir, name, "0.1.0", compat);
    }

    fn write_skill_md_with_version(dir: &Path, name: &str, version: &str, compat: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\nversion: \"{version}\"\ncompatible-cli-versions: \"{compat}\"\n---\n# body\n"
            ),
        )
        .unwrap();
    }

    fn build_skill_zip(name: &str, version: &str, compat: &str, marker: &str) -> Vec<u8> {
        let file = tempfile::tempfile().unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        writer.add_directory("chat2db-cli/", opts).unwrap();
        writer.start_file("chat2db-cli/SKILL.md", opts).unwrap();
        writer
            .write_all(
                format!(
                    "---\nname: {name}\nversion: \"{version}\"\ncompatible-cli-versions: \"{compat}\"\n---\n# body\n"
                )
                .as_bytes(),
            )
            .unwrap();
        writer
            .add_directory("chat2db-cli/references/", opts)
            .unwrap();
        writer
            .start_file("chat2db-cli/references/marker.md", opts)
            .unwrap();
        writer.write_all(marker.as_bytes()).unwrap();
        let mut file = writer.finish().unwrap();
        file.rewind().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    fn manifest_for(zip: &[u8], version: &str, compat: &str) -> Vec<u8> {
        let sha = Sha256::digest(zip);
        format!(
            r#"{{
  "skill_version": "{version}",
  "compatible_cli_versions": "{compat}",
  "bundle_path": "chat2db-cli",
  "artifact": {{
    "url": "https://cdn.chat2db-ai.com/download/chat2db-skill/{version}/chat2db-skill-{version}.zip",
    "sha256": "{sha:x}",
    "artifact_type": "skill_zip"
  }}
}}"#
        )
        .into_bytes()
    }

    fn build_symlink_root_zip(target: &str, include_payload: bool) -> Vec<u8> {
        let file = tempfile::tempfile().unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        if include_payload {
            writer.add_directory("payload/", opts).unwrap();
            writer.start_file("payload/SKILL.md", opts).unwrap();
            writer
                .write_all(
                    b"---\nname: chat2db-cli\nversion: \"0.1.0\"\ncompatible-cli-versions: \">=0.1\"\n---\n# body\n",
                )
                .unwrap();
        }
        writer.add_symlink(SKILL_DIR_NAME, target, opts).unwrap();
        let mut file = writer.finish().unwrap();
        file.rewind().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    struct FakeCdn {
        manifest: Vec<u8>,
        zip: Vec<u8>,
    }

    impl FakeCdn {
        fn new(version: &str, compat: &str, marker: &str) -> Self {
            let zip = build_skill_zip(SKILL_FRONTMATTER_NAME, version, compat, marker);
            let manifest = manifest_for(&zip, version, compat);
            Self { manifest, zip }
        }
    }

    impl HttpClient for FakeCdn {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            if url == skill_manifest_url() {
                Ok(self.manifest.clone())
            } else if url.ends_with(".zip") {
                Ok(self.zip.clone())
            } else {
                Err(Chat2DbError::UnsafeZipPath(format!(
                    "unexpected test URL: {url}"
                )))
            }
        }
    }

    fn target_request(
        client: Option<SkillClient>,
        home: &Path,
        current: &Path,
    ) -> SkillTargetRequest {
        SkillTargetRequest {
            client,
            home_dir: home.to_path_buf(),
            current_dir: current.to_path_buf(),
        }
    }

    fn install_request(
        client: Option<SkillClient>,
        home: &Path,
        current: &Path,
        force: bool,
    ) -> SkillInstallRequest {
        SkillInstallRequest {
            target: target_request(client, home, current),
            force,
            progress: false,
            cli_version: "0.1.0".to_string(),
        }
    }

    fn write_skill_md_without_version(dir: &Path, name: &str, compat: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ncompatible-cli-versions: \"{compat}\"\n---\n# body\n"),
        )
        .unwrap();
    }

    fn write_installed_skill(dir: &Path, version: &str, marker: &str) {
        write_skill_md_with_version(dir, SKILL_FRONTMATTER_NAME, version, ">=0.1");
        fs::create_dir_all(dir.join("references")).unwrap();
        fs::write(dir.join("references/marker.md"), marker).unwrap();
    }

    fn begin_prepared_test_transaction(
        target: &Path,
        transaction: &SkillTransactionPaths,
        previous_sha256: Option<String>,
        candidate_version: &str,
        candidate_marker: &str,
    ) -> (SkillTransactionMarker, PathBuf) {
        let mut marker = begin_skill_transaction(target, transaction, previous_sha256).unwrap();
        fs::create_dir(&transaction.pending).unwrap();
        let candidate_path = transaction.pending.join("candidate");
        write_skill_md_with_version(
            &candidate_path,
            SKILL_FRONTMATTER_NAME,
            candidate_version,
            ">=0.1",
        );
        fs::create_dir_all(candidate_path.join("references")).unwrap();
        fs::write(
            candidate_path.join("references/marker.md"),
            candidate_marker,
        )
        .unwrap();
        sync_skill_bundle(&candidate_path).unwrap();
        marker.phase = SkillTransactionPhase::Prepared;
        marker.candidate = Some(SkillBundleIdentity {
            version: candidate_version.to_string(),
            tree_sha256: path_identity(&candidate_path).unwrap(),
        });
        write_skill_transaction_marker(transaction, &marker).unwrap();
        (marker, candidate_path)
    }

    fn assert_transaction_clean(transaction: &SkillTransactionPaths) {
        assert!(!path_entry_exists(&transaction.pending).unwrap());
        assert!(!path_entry_exists(&transaction.backup).unwrap());
        assert!(!path_entry_exists(&transaction.conflict).unwrap());
        assert!(!path_entry_exists(&transaction.marker).unwrap());
    }

    #[test]
    fn parses_frontmatter_with_quoted_and_unquoted_values() {
        let fm = parse_frontmatter(
            "---\nname: chat2db-cli\ncompatible-cli-versions: \">=0.1, <0.2\"\n---\n# body\n",
        )
        .unwrap();
        assert_eq!(fm.field("name").as_deref(), Some("chat2db-cli"));
        assert_eq!(
            fm.field("compatible-cli-versions").as_deref(),
            Some(">=0.1, <0.2")
        );
    }

    #[test]
    fn classifies_target_states() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("chat2db-cli");

        assert!(matches!(
            classify_existing(&target).unwrap(),
            ExistingState::Empty
        ));

        // Foreign (no SKILL.md)
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("README.md"), "hi").unwrap();
        assert!(matches!(
            classify_existing(&target).unwrap(),
            ExistingState::Foreign
        ));

        // Foreign (wrong name)
        write_skill_md(&target, "other-skill", ">=0.1");
        assert!(matches!(
            classify_existing(&target).unwrap(),
            ExistingState::Foreign
        ));

        // Ours
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");
        assert!(matches!(
            classify_existing(&target).unwrap(),
            ExistingState::Ours
        ));

        // Git wins over Ours
        fs::create_dir_all(target.join(".git")).unwrap();
        assert!(matches!(
            classify_existing(&target).unwrap(),
            ExistingState::Git
        ));
    }

    #[test]
    fn install_defaults_to_current_dir_target() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&current).unwrap();

        let outcome = install(
            &FakeCdn::new("0.1.0", ">=0.1, <0.2", "installed"),
            install_request(None, &home, &current, false),
        )
        .unwrap();
        assert_eq!(outcome.operation, "install");
        assert_eq!(outcome.status, "installed");
        assert_eq!(outcome.client, None);
        let installed = current.join("chat2db-cli");
        assert!(installed.join("SKILL.md").is_file());
        assert!(installed.join("references/marker.md").is_file());
    }

    #[test]
    fn install_client_targets_global_skill_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        fs::create_dir_all(&current).unwrap();

        let claude = install(
            &FakeCdn::new("0.1.0", ">=0.1, <0.2", "claude"),
            install_request(Some(SkillClient::ClaudeCode), &home, &current, false),
        )
        .unwrap();
        assert_eq!(claude.client, Some(SkillClient::ClaudeCode));
        assert_eq!(claude.target_path, home.join(".claude/skills/chat2db-cli"));

        let codex = install(
            &FakeCdn::new("0.1.0", ">=0.1, <0.2", "codex"),
            install_request(Some(SkillClient::Codex), &home, &current, false),
        )
        .unwrap();
        assert_eq!(codex.client, Some(SkillClient::Codex));
        assert_eq!(codex.target_path, home.join(".codex/skills/chat2db-cli"));
    }

    #[test]
    fn install_refuses_existing_ours_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");

        let err = install(
            &FakeCdn::new("0.1.0", ">=0.1, <0.2", "new"),
            install_request(None, &home, &current, false),
        )
        .unwrap_err();
        assert!(matches!(err, Chat2DbError::SkillAlreadyInstalled { .. }));
    }

    #[test]
    fn install_force_overwrites_existing_dir_and_cleans_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let outcome = install(
            &FakeCdn::new("0.1.0", ">=0.1, <0.2", "new"),
            install_request(None, &home, &current, true),
        )
        .unwrap();
        assert_eq!(outcome.status, "updated");
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "new"
        );
        let transaction = SkillTransactionPaths::new(&current);
        assert_transaction_clean(&transaction);
    }

    #[test]
    fn update_same_version_is_up_to_date_without_downloading_or_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_installed_skill(&target, "0.2.0", "old");
        let before = path_identity(&target).unwrap();
        let mut cdn = FakeCdn::new("0.2.0", ">=0.1", "new");
        cdn.zip.clear();

        let outcome = update(
            &cdn,
            SkillUpdateRequest {
                target: target_request(None, &home, &current),
                progress: false,
                cli_version: "0.1.0".to_string(),
            },
        )
        .unwrap();

        assert_eq!(outcome.status, "up_to_date");
        assert_eq!(path_identity(&target).unwrap(), before);
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
        assert_transaction_clean(&SkillTransactionPaths::new(&current));
    }

    #[test]
    fn update_rejects_semver_downgrade_and_preserves_installed_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_installed_skill(&target, "0.3.0", "old");
        let before = path_identity(&target).unwrap();
        let mut cdn = FakeCdn::new("0.2.0", ">=0.1", "new");
        cdn.zip.clear();

        let error = update(
            &cdn,
            SkillUpdateRequest {
                target: target_request(None, &home, &current),
                progress: false,
                cli_version: "0.1.0".to_string(),
            },
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::InvalidReleaseManifest(_)));
        assert!(error.to_string().contains("refusing to downgrade"));
        assert_eq!(path_identity(&target).unwrap(), before);
        assert_transaction_clean(&SkillTransactionPaths::new(&current));
    }

    #[test]
    fn update_installs_a_strictly_newer_skill_version() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_installed_skill(&target, "0.1.0", "old");

        let outcome = update(
            &FakeCdn::new("0.2.0", ">=0.1", "new"),
            SkillUpdateRequest {
                target: target_request(None, &home, &current),
                progress: false,
                cli_version: "0.1.0".to_string(),
            },
        )
        .unwrap();

        assert_eq!(outcome.status, "updated");
        assert_eq!(outcome.skill_version.as_deref(), Some("0.2.0"));
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "new"
        );
        assert_transaction_clean(&SkillTransactionPaths::new(&current));
    }

    #[test]
    fn force_install_can_explicitly_replace_with_an_older_skill_version() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_installed_skill(&target, "0.3.0", "old");

        let outcome = install(
            &FakeCdn::new("0.2.0", ">=0.1", "new"),
            install_request(None, &home, &current, true),
        )
        .unwrap();

        assert_eq!(outcome.status, "updated");
        assert_eq!(outcome.skill_version.as_deref(), Some("0.2.0"));
        assert_eq!(
            installed_skill_version(&target).unwrap(),
            semver::Version::new(0, 2, 0)
        );
        assert_transaction_clean(&SkillTransactionPaths::new(&current));
    }

    #[test]
    fn staging_recovery_keeps_the_untouched_previous_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir(&current).unwrap();
        write_skill_md_with_version(&target, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let transaction = SkillTransactionPaths::new(&current);
        begin_skill_transaction(&target, &transaction, Some(path_identity(&target).unwrap()))
            .unwrap();
        fs::create_dir(&transaction.pending).unwrap();
        fs::write(transaction.pending.join("partial"), "new").unwrap();

        recover_skill_transaction(&target, &transaction).unwrap();

        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
        assert_transaction_clean(&transaction);
    }

    #[test]
    fn recovers_old_bundle_after_interruption_before_activation() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir(&current).unwrap();
        write_skill_md_with_version(&target, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let transaction = SkillTransactionPaths::new(&current);
        let previous = path_identity(&target).unwrap();
        begin_prepared_test_transaction(&target, &transaction, Some(previous), "0.2.0", "new");
        rename_no_replace(&target, &transaction.backup).unwrap();

        recover_skill_transaction(&target, &transaction).unwrap();

        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
        assert_transaction_clean(&transaction);
    }

    #[test]
    fn prepared_candidate_is_rolled_back_when_activation_was_not_committed() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir(&current).unwrap();
        write_skill_md_with_version(&target, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let transaction = SkillTransactionPaths::new(&current);
        let previous = path_identity(&target).unwrap();
        let (_marker, candidate_path) =
            begin_prepared_test_transaction(&target, &transaction, Some(previous), "0.2.0", "new");
        rename_no_replace(&target, &transaction.backup).unwrap();
        rename_no_replace(&candidate_path, &target).unwrap();

        recover_skill_transaction(&target, &transaction).unwrap();

        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
        assert_transaction_clean(&transaction);
    }

    #[test]
    fn activated_candidate_is_committed_only_when_its_identity_matches() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir(&current).unwrap();
        write_skill_md_with_version(&target, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");

        let transaction = SkillTransactionPaths::new(&current);
        let previous = path_identity(&target).unwrap();
        let (mut marker, candidate_path) =
            begin_prepared_test_transaction(&target, &transaction, Some(previous), "0.2.0", "new");
        rename_no_replace(&target, &transaction.backup).unwrap();
        rename_no_replace(&candidate_path, &target).unwrap();
        marker.phase = SkillTransactionPhase::Activated;
        write_skill_transaction_marker(&transaction, &marker).unwrap();

        recover_skill_transaction(&target, &transaction).unwrap();

        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "new"
        );
        assert_transaction_clean(&transaction);
    }

    #[test]
    fn foreign_target_in_activation_gap_is_preserved_while_previous_is_restored() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir(&current).unwrap();
        write_skill_md_with_version(&target, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let transaction = SkillTransactionPaths::new(&current);
        let previous = path_identity(&target).unwrap();
        begin_prepared_test_transaction(&target, &transaction, Some(previous), "0.2.0", "new");
        rename_no_replace(&target, &transaction.backup).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("foreign.txt"), "preserve me").unwrap();

        let error = recover_skill_transaction(&target, &transaction).unwrap_err();

        assert!(error.to_string().contains("preserved an unexpected target"));
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(transaction.conflict.join("foreign.txt")).unwrap(),
            "preserve me"
        );
        assert!(!path_entry_exists(&transaction.backup).unwrap());
        assert!(!path_entry_exists(&transaction.pending).unwrap());
        assert!(!path_entry_exists(&transaction.marker).unwrap());
    }

    #[test]
    fn corrupt_activated_candidate_is_preserved_and_previous_is_restored() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir(&current).unwrap();
        write_skill_md_with_version(&target, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let transaction = SkillTransactionPaths::new(&current);
        let previous = path_identity(&target).unwrap();
        let (mut marker, candidate_path) =
            begin_prepared_test_transaction(&target, &transaction, Some(previous), "0.2.0", "new");
        rename_no_replace(&target, &transaction.backup).unwrap();
        rename_no_replace(&candidate_path, &target).unwrap();
        marker.phase = SkillTransactionPhase::Activated;
        write_skill_transaction_marker(&transaction, &marker).unwrap();
        fs::write(target.join("references/marker.md"), "corrupt").unwrap();

        let error = recover_skill_transaction(&target, &transaction).unwrap_err();

        assert!(error.to_string().contains("preserved an unexpected target"));
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(transaction.conflict.join("references/marker.md")).unwrap(),
            "corrupt"
        );
    }

    #[test]
    fn invalid_transaction_marker_does_not_delete_recovery_data() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(SKILL_DIR_NAME);
        let transaction = SkillTransactionPaths::new(dir.path());
        fs::write(&transaction.marker, "not-ours").unwrap();
        fs::create_dir(&transaction.pending).unwrap();
        fs::write(transaction.pending.join("staged"), "new").unwrap();

        assert!(recover_skill_transaction(&target, &transaction).is_err());
        assert!(transaction.pending.join("staged").is_file());
        assert!(transaction.marker.is_file());
    }

    #[test]
    fn install_refuses_git_target_even_with_force() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");
        fs::create_dir_all(target.join(".git")).unwrap();

        let err = install(
            &FakeCdn::new("0.1.0", ">=0.1, <0.2", "new"),
            install_request(None, &home, &current, true),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            Chat2DbError::SkillTargetManagedExternally { .. }
        ));
    }

    #[test]
    fn install_rejects_incompatible_version() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        fs::create_dir_all(&current).unwrap();

        let err = install(
            &FakeCdn::new("0.1.0", ">=99", "new"),
            install_request(None, &home, &current, false),
        )
        .unwrap_err();
        assert!(matches!(err, Chat2DbError::SkillVersionIncompatible { .. }));
    }

    #[test]
    fn install_rejects_relative_and_dangling_bundle_root_symlinks() {
        for (target_name, include_payload) in [("payload", true), ("missing", false)] {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path().join("home");
            let current = dir.path().join("project");
            fs::create_dir_all(&current).unwrap();
            let zip = build_symlink_root_zip(target_name, include_payload);
            let cdn = FakeCdn {
                manifest: manifest_for(&zip, "0.1.0", ">=0.1"),
                zip,
            };

            let error = install(&cdn, install_request(None, &home, &current, false)).unwrap_err();
            assert!(matches!(error, Chat2DbError::UnsafeZipPath(_)));
            assert!(
                matches!(
                    fs::symlink_metadata(current.join(SKILL_DIR_NAME)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound
                ),
                "bundle root must not be installed for symlink target {target_name}"
            );
        }
    }

    #[test]
    fn failed_update_preserves_existing_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");
        fs::create_dir_all(target.join("references")).unwrap();
        fs::write(target.join("references/marker.md"), "old").unwrap();

        let error = install(
            &FakeCdn::new("0.1.0", ">=99", "new"),
            install_request(None, &home, &current, true),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::SkillVersionIncompatible { .. }
        ));
        assert_eq!(
            fs::read_to_string(target.join("references/marker.md")).unwrap(),
            "old"
        );
    }

    #[test]
    fn uninstall_only_removes_our_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");

        let outcome = uninstall(SkillUninstallRequest {
            target: target_request(None, &home, &current),
        })
        .unwrap();
        assert_eq!(outcome.status, "removed");
        assert!(!target.exists());

        // Re-uninstall returns already_absent
        let outcome2 = uninstall(SkillUninstallRequest {
            target: target_request(None, &home, &current),
        })
        .unwrap();
        assert_eq!(outcome2.status, "already_absent");
    }

    #[test]
    fn uninstall_waits_for_the_shared_target_lock() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        write_skill_md(&target, SKILL_FRONTMATTER_NAME, ">=0.1");
        let request = target_request(None, &home, &current);
        let lock_path = skill_target_lock_path(&request, &target);
        let held_lock = acquire_exclusive_lock(&lock_path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = uninstall(SkillUninstallRequest { target: request });
            result_tx.send(result).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result_rx.recv_timeout(Duration::from_millis(150)).is_err());
        assert!(target.exists());
        drop(held_lock);

        let outcome = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        handle.join().unwrap();
        assert_eq!(outcome.status, "removed");
        assert!(!target.exists());
    }

    #[test]
    fn uninstall_refuses_foreign() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let current = dir.path().join("project");
        let target = current.join(SKILL_DIR_NAME);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("README.md"), "hi").unwrap();

        let err = uninstall(SkillUninstallRequest {
            target: target_request(None, &home, &current),
        })
        .unwrap_err();
        assert!(matches!(err, Chat2DbError::SkillTargetForeign { .. }));
    }

    #[test]
    fn verify_staged_bundle_requires_frontmatter_version() {
        let dir = tempfile::tempdir().unwrap();
        write_skill_md_without_version(dir.path(), SKILL_FRONTMATTER_NAME, ">=0.1");
        let err = verify_staged_bundle(dir.path(), None).unwrap_err();
        let reason = match err {
            Chat2DbError::SkillBundleCorrupt { reason } => reason,
            other => panic!("expected SkillBundleCorrupt, got {other:?}"),
        };
        assert!(
            reason.contains("version"),
            "reason should mention missing version: {reason}"
        );
    }

    #[test]
    fn verify_staged_bundle_rejects_manifest_version_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        write_skill_md_with_version(dir.path(), SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        let err = verify_staged_bundle(dir.path(), Some("0.2.0")).unwrap_err();
        let reason = match err {
            Chat2DbError::SkillBundleCorrupt { reason } => reason,
            other => panic!("expected SkillBundleCorrupt, got {other:?}"),
        };
        assert!(
            reason.contains("0.1.0") && reason.contains("0.2.0"),
            "reason should cite both versions: {reason}"
        );
    }

    #[test]
    fn verify_staged_bundle_passes_with_matching_manifest_version() {
        let dir = tempfile::tempdir().unwrap();
        write_skill_md_with_version(dir.path(), SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        verify_staged_bundle(dir.path(), Some("0.1.0")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn verify_staged_bundle_rejects_symlink_root() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let linked = dir.path().join("linked");
        write_skill_md_with_version(&real, SKILL_FRONTMATTER_NAME, "0.1.0", ">=0.1");
        symlink(&real, &linked).unwrap();

        let error = verify_staged_bundle(&linked, Some("0.1.0")).unwrap_err();
        assert!(matches!(error, Chat2DbError::SkillBundleCorrupt { .. }));
    }
}
