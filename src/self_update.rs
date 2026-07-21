use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(any(unix, windows))]
use std::process::Stdio;
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tar::Archive;
use zip::ZipArchive;

use crate::error::{Chat2DbError, Result};
use crate::http::{DownloadProgress, HttpClient, ReqwestHttpClient};
use crate::manifest::{BinaryManifest, binary_manifest_url, sha256_hex, verify_sha256};
#[cfg(any(windows, test))]
use crate::platform::configure_detached_command;
#[cfg(windows)]
use crate::platform::write_private_atomic;
use crate::platform::{
    ExclusiveLock, HostOs, acquire_exclusive_lock, current_arch_label, current_cli_os_label,
    current_os, home_dir, process_exists, process_identity,
};

const SCHEDULE_MARKER_GRACE: Duration = Duration::from_secs(30);
const SCHEDULE_MARKER_VERSION: u8 = 1;

#[derive(Debug, Deserialize, Serialize)]
struct ScheduledTransactionOwner {
    version: u8,
    pid: u32,
    process_identity: String,
}

#[derive(Debug, Clone)]
pub struct UpdateRequest {
    pub target_path: Option<PathBuf>,
    pub progress: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateOutcome {
    pub command: &'static str,
    pub status: &'static str,
    pub target_path: PathBuf,
    pub version: String,
    pub manifest_url: String,
    pub artifact_url: String,
}

#[derive(Debug, Clone)]
pub struct UninstallCliRequest {
    pub target_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallCliOutcome {
    pub command: &'static str,
    pub status: &'static str,
    pub target_path: PathBuf,
}

pub fn update_cli(request: UpdateRequest) -> Result<UpdateOutcome> {
    update_cli_with_client(&ReqwestHttpClient, request)
}

pub fn update_cli_with_client<C: HttpClient>(
    client: &C,
    request: UpdateRequest,
) -> Result<UpdateOutcome> {
    let explicit_target = request.target_path.is_some();
    let requested_target_path = match request.target_path {
        Some(path) => path,
        None => std::env::current_exe()?,
    };
    let target_path = normalized_cli_target(&requested_target_path)?;
    let lock_path = cli_target_lock_path(&target_path);
    let _lock = acquire_cli_target_lock(&lock_path)?;
    let target_exists = validate_cli_target(&target_path)?;
    validate_explicit_update_target(explicit_target, &target_path, target_exists)?;
    let installing_new_target = explicit_target && !target_exists;

    let os = current_cli_os_label()?;
    let arch = current_arch_label()?;
    let manifest_url = binary_manifest_url(os, arch);
    if request.progress {
        eprintln!("Fetching CLI manifest...");
    }
    let manifest_bytes = client.get(&manifest_url)?;
    let manifest = BinaryManifest::parse(&manifest_bytes, os, arch)?;
    if !should_apply_update(
        installing_new_target,
        env!("CARGO_PKG_VERSION"),
        &manifest.version,
    )? {
        return Ok(UpdateOutcome {
            command: "update",
            status: "up_to_date",
            target_path: requested_target_path,
            version: manifest.version,
            manifest_url,
            artifact_url: manifest.artifact.url,
        });
    }

    let target_dir = target_path
        .parent()
        .ok_or_else(|| Chat2DbError::InvalidCliTarget {
            path: target_path.clone(),
        })?;
    fs::create_dir_all(target_dir)?;
    let temp_dir = tempfile::Builder::new()
        .prefix(".chat2db-cli.update.")
        .tempdir_in(target_dir)?;
    let archive_kind = ReleaseArchiveKind::from_url(&manifest.artifact.url)?;
    let archive_path = temp_dir.path().join(format!(
        "chat2db.{}",
        match archive_kind {
            ReleaseArchiveKind::TarGz => "tar.gz",
            ReleaseArchiveKind::Zip => "zip",
        }
    ));
    client.download_to(
        &manifest.artifact.url,
        &archive_path,
        DownloadProgress::new(request.progress, "chat2db CLI"),
    )?;
    let bytes = fs::read(&archive_path)?;
    verify_sha256(&bytes, &manifest.artifact.sha256)?;

    let staged_binary = temp_dir
        .path()
        .join(format!("pending-{}", cli_binary_name()));
    extract_cli_archive(&bytes, archive_kind, &staged_binary)?;
    if installing_new_target && validate_cli_target(&target_path)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "refusing to replace explicit CLI target {} because it appeared during the update",
                target_path.display()
            ),
        )
        .into());
    }
    let status = replace_binary(
        temp_dir,
        &staged_binary,
        &target_path,
        &lock_path,
        installing_new_target,
    )?;

    refresh_completions(&target_path);

    Ok(UpdateOutcome {
        command: "update",
        status,
        target_path: requested_target_path,
        version: manifest.version,
        manifest_url,
        artifact_url: manifest.artifact.url,
    })
}

fn should_apply_update(
    installing_new_target: bool,
    current_version: &str,
    manifest_version: &str,
) -> Result<bool> {
    let current = semver::Version::parse(current_version).map_err(|_| {
        Chat2DbError::InvalidReleaseManifest(format!(
            "running CLI version is not valid SemVer: {current_version}"
        ))
    })?;
    let manifest = semver::Version::parse(manifest_version).map_err(|_| {
        Chat2DbError::InvalidReleaseManifest(format!(
            "manifest version is not valid SemVer: {manifest_version}"
        ))
    })?;

    Ok(manifest > current || (installing_new_target && manifest == current))
}

fn validate_explicit_update_target(
    explicit_target: bool,
    target: &Path,
    target_exists: bool,
) -> Result<()> {
    if !explicit_target || !target_exists {
        return Ok(());
    }

    let current = normalized_cli_target(&std::env::current_exe()?)?;
    let is_current = if current_os() == HostOs::Windows {
        paths_equal(&current, target)
    } else {
        current == target
    };
    if is_current {
        return Ok(());
    }

    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "refusing to replace existing explicit CLI target {} because its version cannot be verified safely; run that binary's update command instead",
            target.display()
        ),
    )
    .into())
}

fn normalized_cli_target(target: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(target)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| Chat2DbError::InvalidCliTarget {
            path: absolute.clone(),
        })?;
    let file_name = absolute
        .file_name()
        .ok_or_else(|| Chat2DbError::InvalidCliTarget {
            path: absolute.clone(),
        })?;
    let parent = if current_os() == HostOs::Windows {
        parent.to_path_buf()
    } else {
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
    };
    Ok(parent.join(file_name))
}

fn cli_target_lock_path(target: &Path) -> PathBuf {
    let identity = if current_os() == HostOs::Windows {
        target
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase()
    } else {
        target.to_string_lossy().into_owned()
    };
    home_dir().join(".chat2db-cli").join("locks").join(format!(
        "cli-target-{}.lock",
        sha256_hex(identity.as_bytes())
    ))
}

fn acquire_cli_target_lock(lock_path: &Path) -> Result<ExclusiveLock> {
    let lock = acquire_exclusive_lock(lock_path)?;
    let marker_path = scheduled_transaction_marker_path(lock_path);
    let marker = match fs::read_to_string(&marker_path) {
        Ok(marker) => marker,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(lock),
        Err(error) => return Err(error.into()),
    };

    if let Some(diagnostic) = scheduled_transaction_failure(&marker) {
        return Err(io::Error::other(format!(
            "previous scheduled CLI transaction failed: {diagnostic}; recovery marker is at {}",
            marker_path.display()
        ))
        .into());
    }

    let scheduled_process_is_alive = scheduled_transaction_is_active(&marker, &marker_path)?;
    if scheduled_process_is_alive {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            format!(
                "another scheduled CLI transaction is still active for lock {}",
                lock_path.display()
            ),
        )
        .into());
    }

    remove_file_if_exists(&marker_path)?;
    Ok(lock)
}

fn scheduled_transaction_is_active(marker: &str, marker_path: &Path) -> Result<bool> {
    let marker = marker.strip_prefix('\u{feff}').unwrap_or(marker).trim();
    if let Ok(owner) = serde_json::from_str::<ScheduledTransactionOwner>(marker)
        && owner.version == SCHEDULE_MARKER_VERSION
        && owner.pid > 0
        && !owner.process_identity.is_empty()
    {
        return Ok(process_identity(owner.pid)?.as_deref() == Some(owner.process_identity.as_str()));
    }

    // Versions before 0.2.0 wrote only a PID. Trust a live legacy PID only
    // during the handoff grace period: this protects an in-flight old helper
    // without allowing a reused PID to keep a stale marker active forever.
    if let Ok(pid) = marker.parse::<u32>()
        && pid > 0
    {
        return Ok(marker_is_recent(marker_path) && process_exists(pid)?);
    }

    // `starting` covers the spawn-to-identity-write handoff. Unknown markers
    // receive the same bounded fail-closed treatment and then become stale.
    Ok(marker_is_recent(marker_path))
}

#[cfg(any(windows, test))]
fn scheduled_transaction_owner_marker(pid: u32, process_identity: String) -> Result<Vec<u8>> {
    let mut marker = serde_json::to_vec(&ScheduledTransactionOwner {
        version: SCHEDULE_MARKER_VERSION,
        pid,
        process_identity,
    })?;
    marker.push(b'\n');
    Ok(marker)
}

fn scheduled_transaction_marker_path(lock_path: &Path) -> PathBuf {
    lock_path.with_extension("pending")
}

fn scheduled_transaction_failure(marker: &str) -> Option<&str> {
    let marker = marker.strip_prefix('\u{feff}').unwrap_or(marker);
    let (state, diagnostic) = marker.split_once('\n').unwrap_or((marker, ""));
    if state.trim() != "failed" {
        return None;
    }

    let diagnostic = diagnostic.trim();
    Some(if diagnostic.is_empty() {
        "no diagnostic was recorded"
    } else {
        diagnostic
    })
}

fn marker_is_recent(marker_path: &Path) -> bool {
    marker_path
        .metadata()
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| modified.elapsed().map_err(io::Error::other))
        .map(|elapsed| elapsed < SCHEDULE_MARKER_GRACE)
        .unwrap_or(true)
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Best-effort: re-run `chat2db completions --install` against the freshly
/// installed binary so the generated script matches the new clap definitions.
/// Skip silently when the user has not opted into shell completions yet (no
/// rc marker) so we never modify a user's rc unilaterally during update.
#[cfg(windows)]
fn refresh_completions(target_path: &Path) {
    let _ = target_path;
}

#[cfg(not(windows))]
fn refresh_completions(target_path: &Path) {
    let home_dir = crate::platform::home_dir();

    let zshrc = home_dir.join(".zshrc");
    let bashrc = home_dir.join(".bashrc");
    let fish_existing = home_dir.join(".config/fish/completions/chat2db.fish");

    let shells: Vec<&str> = [("zsh", &zshrc), ("bash", &bashrc)]
        .into_iter()
        .filter(|(_, path)| {
            fs::read_to_string(path)
                .map(|text| text.contains("# >>> chat2db completions >>>"))
                .unwrap_or(false)
        })
        .map(|(shell, _)| shell)
        .chain(fish_existing.is_file().then_some("fish"))
        .collect();

    for shell in shells {
        let _ = Command::new(target_path)
            .args(["completions", "--install", "--shell", shell])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn uninstall_cli(request: UninstallCliRequest) -> Result<UninstallCliOutcome> {
    let requested_target_path = match request.target_path {
        Some(path) => path,
        None => std::env::current_exe()?,
    };
    let target_path = normalized_cli_target(&requested_target_path)?;
    let lock_path = cli_target_lock_path(&target_path);
    let _lock = acquire_cli_target_lock(&lock_path)?;
    let target_exists = validate_cli_target(&target_path)?;

    let status = if target_exists {
        if current_os() == HostOs::Windows
            && std::env::current_exe()
                .ok()
                .as_ref()
                .is_some_and(|current| paths_equal(current, &target_path))
        {
            let target_dir =
                target_path
                    .parent()
                    .ok_or_else(|| Chat2DbError::InvalidCliTarget {
                        path: target_path.clone(),
                    })?;
            let transaction = tempfile::Builder::new()
                .prefix(".chat2db-cli.uninstall.")
                .tempdir_in(target_dir)?;
            schedule_windows_self_delete(transaction, &target_path, &lock_path)?;
            return Ok(UninstallCliOutcome {
                command: "uninstall",
                status: "scheduled",
                target_path: requested_target_path,
            });
        }
        fs::remove_file(&target_path)?;
        "removed"
    } else {
        "already_absent"
    };

    Ok(UninstallCliOutcome {
        command: "uninstall",
        status,
        target_path: requested_target_path,
    })
}

fn validate_cli_target(target_path: &Path) -> Result<bool> {
    if !is_valid_cli_binary_name(target_path) {
        return Err(Chat2DbError::InvalidCliTarget {
            path: target_path.to_path_buf(),
        });
    }

    match fs::symlink_metadata(target_path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err(Chat2DbError::InvalidCliTarget {
            path: target_path.to_path_buf(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn cli_binary_name() -> &'static str {
    if cfg!(windows) {
        "chat2db.exe"
    } else {
        "chat2db"
    }
}

fn is_valid_cli_binary_name(target_path: &Path) -> bool {
    matches!(
        target_path.file_name().and_then(|name| name.to_str()),
        Some("chat2db") | Some("chat2db.exe")
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseArchiveKind {
    TarGz,
    Zip,
}

impl ReleaseArchiveKind {
    fn from_url(url: &str) -> Result<Self> {
        if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
            Ok(Self::TarGz)
        } else if url.ends_with(".zip") {
            Ok(Self::Zip)
        } else {
            Err(Chat2DbError::UnsupportedArtifact)
        }
    }
}

fn extract_cli_archive(bytes: &[u8], kind: ReleaseArchiveKind, staged_binary: &Path) -> Result<()> {
    match kind {
        ReleaseArchiveKind::TarGz => extract_cli_tarball(bytes, staged_binary),
        ReleaseArchiveKind::Zip => extract_cli_zip(bytes, staged_binary),
    }
}

fn extract_cli_tarball(bytes: &[u8], staged_binary: &Path) -> Result<()> {
    let decoder = GzDecoder::new(bytes);
    let mut archive = Archive::new(decoder);
    let mut found_binary = false;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        if is_ignorable_tar_metadata_path(path.as_ref()) {
            continue;
        }
        if !is_archive_cli_binary_entry(path.as_ref()) {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: format!("CLI tarball contains unexpected entry: {}", path.display()),
            });
        }
        if !entry.header().entry_type().is_file() {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: "CLI tarball chat2db entry is not a file".to_string(),
            });
        }
        if found_binary {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: "CLI tarball contains duplicate chat2db entries".to_string(),
            });
        }

        let parent = staged_binary
            .parent()
            .ok_or_else(|| Chat2DbError::InvalidCliTarget {
                path: staged_binary.to_path_buf(),
            })?;
        fs::create_dir_all(parent)?;
        let mut file = fs::File::create(staged_binary)?;
        io::copy(&mut entry, &mut file)?;

        #[cfg(unix)]
        fs::set_permissions(staged_binary, fs::Permissions::from_mode(0o755))?;

        found_binary = true;
    }

    if found_binary {
        Ok(())
    } else {
        Err(Chat2DbError::SkillBundleCorrupt {
            reason: "CLI tarball is missing chat2db binary".to_string(),
        })
    }
}

fn extract_cli_zip(bytes: &[u8], staged_binary: &Path) -> Result<()> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)?;
    let mut found_binary = false;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(path) = entry.enclosed_name().map(|path| path.to_owned()) else {
            return Err(Chat2DbError::UnsafeZipPath(entry.name().to_string()));
        };
        if is_ignorable_zip_metadata_path(&path) || entry.is_dir() {
            continue;
        }
        if !is_archive_cli_binary_entry(&path) {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: format!("CLI zip contains unexpected entry: {}", path.display()),
            });
        }
        if found_binary {
            return Err(Chat2DbError::SkillBundleCorrupt {
                reason: "CLI zip contains duplicate chat2db entries".to_string(),
            });
        }

        let parent = staged_binary
            .parent()
            .ok_or_else(|| Chat2DbError::InvalidCliTarget {
                path: staged_binary.to_path_buf(),
            })?;
        fs::create_dir_all(parent)?;
        let mut file = fs::File::create(staged_binary)?;
        io::copy(&mut entry, &mut file)?;

        #[cfg(unix)]
        fs::set_permissions(staged_binary, fs::Permissions::from_mode(0o755))?;

        found_binary = true;
    }

    if found_binary {
        Ok(())
    } else {
        Err(Chat2DbError::SkillBundleCorrupt {
            reason: "CLI zip is missing chat2db binary".to_string(),
        })
    }
}

fn is_archive_cli_binary_entry(path: &Path) -> bool {
    path == Path::new("chat2db") || path == Path::new("chat2db.exe")
}

fn is_ignorable_tar_metadata_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path == ".DS_Store"
        || path.starts_with("__MACOSX/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("._"))
}

fn is_ignorable_zip_metadata_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path == ".DS_Store"
        || path.starts_with("__MACOSX/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("._"))
}

fn replace_binary(
    transaction: tempfile::TempDir,
    source: &Path,
    target: &Path,
    lock_path: &Path,
    installing_new_target: bool,
) -> Result<&'static str> {
    fs::File::open(source)?.sync_all()?;

    if current_os() == HostOs::Windows
        && std::env::current_exe()
            .ok()
            .as_ref()
            .is_some_and(|current| paths_equal(current, target))
    {
        schedule_windows_self_replace(transaction, source, target, lock_path)?;
        return Ok("scheduled");
    }

    if installing_new_target {
        commit_new_binary_no_clobber(source, target)?;
        drop(transaction);
        return Ok("updated");
    }

    let replacing = target.is_file();
    let backup = transaction
        .path()
        .join(format!("previous-{}", cli_binary_name()));
    if replacing {
        fs::copy(target, &backup)?;
        fs::File::open(&backup)?.sync_all()?;
    }

    if let Err(update_error) = fs::rename(source, target) {
        if replacing
            && !target.exists()
            && let Err(rollback_error) = fs::rename(&backup, target)
        {
            let recovery_dir = transaction.keep();
            return Err(io::Error::other(format!(
                "failed to replace CLI: {update_error}; failed to restore previous CLI: {rollback_error}; recovery data is at {}",
                recovery_dir.display()
            ))
            .into());
        }
        return Err(update_error.into());
    }

    drop(transaction);
    Ok("updated")
}

fn commit_new_binary_no_clobber(source: &Path, target: &Path) -> Result<()> {
    // The transaction directory is created beside the target, so both paths
    // are on the same filesystem. Creating a hard link is an atomic
    // create-if-absent operation on every supported CLI platform and never
    // follows or replaces a destination symlink.
    fs::hard_link(source, target)?;
    Ok(())
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    let normalized_left = left
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let normalized_right = right
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized_left == normalized_right
}

#[cfg(windows)]
fn schedule_windows_self_replace(
    transaction: tempfile::TempDir,
    source: &Path,
    target: &Path,
    lock_path: &Path,
) -> Result<()> {
    let transaction_dir = transaction.path().to_path_buf();
    let backup = transaction_dir.join(format!("previous-{}", cli_binary_name()));
    let script = transaction_dir.join("replace.ps1");
    let operation =
        windows_self_replace_operation(source, target, &backup, &script, &transaction_dir);
    let marker_path = scheduled_transaction_marker_path(lock_path);
    let body = windows_scheduled_script(
        std::process::id(),
        lock_path,
        &marker_path,
        &transaction_dir,
        &operation,
    );
    fs::write(&script, windows_powershell_script_bytes(&body))?;
    spawn_windows_transaction(transaction, &script, lock_path)
}

#[cfg(any(windows, test))]
fn windows_self_replace_operation(
    source: &Path,
    target: &Path,
    backup: &Path,
    script: &Path,
    transaction_dir: &Path,
) -> String {
    format!(
        r#"$hadTarget = Test-Path -LiteralPath '{target}' -PathType Leaf
if ($hadTarget) {{ Copy-Item -LiteralPath '{target}' -Destination '{backup}' -Force }}
try {{
  $promoted = $false
  for ($attempt = 0; $attempt -lt 200; $attempt++) {{
    try {{
      Move-Item -LiteralPath '{source}' -Destination '{target}' -Force
      $promoted = $true
      break
    }} catch {{
      if ($attempt -eq 199) {{ throw }}
      Start-Sleep -Milliseconds 50
    }}
  }}
  if (-not $promoted -or -not (Test-Path -LiteralPath '{target}' -PathType Leaf)) {{
    throw 'Unable to promote the pending Chat2DB CLI binary.'
  }}
}} catch {{
  if ($hadTarget -and (-not (Test-Path -LiteralPath '{target}')) -and (Test-Path -LiteralPath '{backup}' -PathType Leaf)) {{
    Copy-Item -LiteralPath '{backup}' -Destination '{target}' -Force
  }}
  throw
}}
Remove-Item -LiteralPath '{source}' -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath '{backup}' -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath '{script}' -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath '{transaction_dir}' -Recurse -Force -ErrorAction SilentlyContinue
"#,
        target = powershell_literal(target),
        backup = powershell_literal(backup),
        source = powershell_literal(source),
        script = powershell_literal(script),
        transaction_dir = powershell_literal(transaction_dir),
    )
}

#[cfg(not(windows))]
fn schedule_windows_self_replace(
    _transaction: tempfile::TempDir,
    _source: &Path,
    _target: &Path,
    _lock_path: &Path,
) -> Result<()> {
    Err(Chat2DbError::UnsupportedPlatform)
}

#[cfg(windows)]
fn schedule_windows_self_delete(
    transaction: tempfile::TempDir,
    target: &Path,
    lock_path: &Path,
) -> Result<()> {
    let transaction_dir = transaction.path().to_path_buf();
    let script = transaction_dir.join("uninstall.ps1");
    let operation = format!(
        r#"try {{
  $removed = $false
  for ($attempt = 0; $attempt -lt 200; $attempt++) {{
    if (-not (Test-Path -LiteralPath '{target}')) {{
      $removed = $true
      break
    }}
    try {{
      Remove-Item -LiteralPath '{target}' -Force -ErrorAction Stop
      if (-not (Test-Path -LiteralPath '{target}')) {{
        $removed = $true
        break
      }}
    }} catch {{
      if ($attempt -eq 199) {{ throw }}
    }}
    Start-Sleep -Milliseconds 50
  }}
  if (-not $removed) {{ throw 'Unable to remove the Chat2DB CLI binary.' }}
}} finally {{
  if (-not (Test-Path -LiteralPath '{target}')) {{
    Remove-Item -LiteralPath '{script}' -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath '{transaction_dir}' -Recurse -Force -ErrorAction SilentlyContinue
  }}
}}
"#,
        target = powershell_literal(target),
        script = powershell_literal(&script),
        transaction_dir = powershell_literal(&transaction_dir),
    );
    let marker_path = scheduled_transaction_marker_path(lock_path);
    let body = windows_scheduled_script(
        std::process::id(),
        lock_path,
        &marker_path,
        &transaction_dir,
        &operation,
    );
    fs::write(&script, windows_powershell_script_bytes(&body))?;
    spawn_windows_transaction(transaction, &script, lock_path)
}

#[cfg(not(windows))]
fn schedule_windows_self_delete(
    _transaction: tempfile::TempDir,
    _target: &Path,
    _lock_path: &Path,
) -> Result<()> {
    Err(Chat2DbError::UnsupportedPlatform)
}

#[cfg(windows)]
fn spawn_windows_transaction(
    transaction: tempfile::TempDir,
    script: &Path,
    lock_path: &Path,
) -> Result<()> {
    let marker_path = scheduled_transaction_marker_path(lock_path);
    write_private_atomic(&marker_path, b"starting\n")?;

    let mut command = windows_transaction_command(script);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            let _ = remove_file_if_exists(&marker_path);
            return Err(Chat2DbError::CommandIo {
                program: "powershell.exe",
                source,
            });
        }
    };

    let child_identity = match process_identity(child.id()) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = remove_file_if_exists(&marker_path);
            return Err(io::Error::other(format!(
                "scheduled CLI transaction process {} exited before its identity could be recorded",
                child.id()
            ))
            .into());
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = remove_file_if_exists(&marker_path);
            return Err(error);
        }
    };
    let owner_marker = match scheduled_transaction_owner_marker(child.id(), child_identity) {
        Ok(marker) => marker,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = remove_file_if_exists(&marker_path);
            return Err(error);
        }
    };
    if let Err(error) = write_private_atomic(&marker_path, &owner_marker) {
        let _ = child.kill();
        let _ = child.wait();
        let _ = remove_file_if_exists(&marker_path);
        return Err(error);
    }

    drop(child);
    let _ = transaction.keep();
    Ok(())
}

#[cfg(any(windows, test))]
fn windows_transaction_command(script: &Path) -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script.to_string_lossy(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    configure_detached_command(&mut command);

    command
}

#[cfg(any(windows, test))]
fn windows_scheduled_script(
    parent_pid: u32,
    lock_path: &Path,
    marker_path: &Path,
    recovery_path: &Path,
    operation: &str,
) -> String {
    format!(
        r#"$ErrorActionPreference='Stop'
$transactionSucceeded = $false
$lockStream = $null
try {{
  Wait-Process -Id {parent_pid} -ErrorAction SilentlyContinue
  for ($attempt = 0; $attempt -lt 200 -and $null -eq $lockStream; $attempt++) {{
    $candidate = $null
    try {{
      $candidate = [System.IO.File]::Open('{lock_path}', [System.IO.FileMode]::OpenOrCreate, [System.IO.FileAccess]::ReadWrite, [System.IO.FileShare]::ReadWrite)
      $candidate.Lock(0, 1)
      $lockStream = $candidate
    }} catch [System.IO.IOException] {{
      if ($null -ne $candidate) {{ $candidate.Dispose() }}
      if ($attempt -eq 199) {{ throw }}
      Start-Sleep -Milliseconds 50
    }}
  }}
  if ($null -eq $lockStream) {{ throw 'Unable to acquire the Chat2DB CLI transaction lock.' }}
{operation}  $transactionSucceeded = $true
}} catch {{
  $failure = ($_ | Out-String).Trim()
  $diagnostic = 'failed' + [Environment]::NewLine + $failure + [Environment]::NewLine + 'Recovery data: {recovery_path}'
  [System.IO.File]::WriteAllText('{marker_path}', $diagnostic)
}} finally {{
  if ($transactionSucceeded) {{
    Remove-Item -LiteralPath '{marker_path}' -Force -ErrorAction SilentlyContinue
  }}
  if ($null -ne $lockStream) {{
    try {{ $lockStream.Unlock(0, 1) }} catch {{}}
    $lockStream.Dispose()
  }}
}}
if (-not $transactionSucceeded) {{ exit 1 }}
"#,
        lock_path = powershell_literal(lock_path),
        marker_path = powershell_literal(marker_path),
        recovery_path = powershell_literal(recovery_path),
    )
}

#[cfg(any(windows, test))]
fn windows_powershell_script_bytes(script: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(3 + script.len());
    bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    bytes.extend_from_slice(script.as_bytes());
    bytes
}

#[cfg(any(windows, test))]
fn powershell_literal(path: &Path) -> String {
    path.display().to_string().replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn uninstalls_cli_target_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("chat2db");
        fs::write(&target, b"fake-cli").unwrap();

        let outcome = uninstall_cli(UninstallCliRequest {
            target_path: Some(target.clone()),
        })
        .unwrap();

        assert_eq!(outcome.status, "removed");
        assert!(!target.exists());
    }

    #[test]
    fn uninstall_waits_for_the_shared_target_lock() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("chat2db");
        fs::write(&target, b"fake-cli").unwrap();
        let normalized_target = normalized_cli_target(&target).unwrap();
        let lock_path = cli_target_lock_path(&normalized_target);
        let held_lock = acquire_exclusive_lock(&lock_path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = uninstall_cli(UninstallCliRequest {
                target_path: Some(target),
            });
            result_tx.send(result).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(result_rx.recv_timeout(Duration::from_millis(150)).is_err());
        assert!(normalized_target.exists());
        drop(held_lock);

        let outcome = result_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        handle.join().unwrap();
        assert_eq!(outcome.status, "removed");
        assert!(!normalized_target.exists());
        let _ = fs::remove_file(lock_path);
    }

    #[test]
    fn live_scheduled_transaction_marker_rejects_a_second_command() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        let pid = std::process::id();
        let identity = process_identity(pid).unwrap().unwrap();
        fs::write(
            &marker_path,
            scheduled_transaction_owner_marker(pid, identity).unwrap(),
        )
        .unwrap();

        let error = match acquire_cli_target_lock(&lock_path) {
            Ok(_) => panic!("expected a live scheduled transaction to reject the command"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            Chat2DbError::Io(error) if error.kind() == io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn reused_pid_scheduled_transaction_marker_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        fs::write(
            &marker_path,
            scheduled_transaction_owner_marker(
                std::process::id(),
                "identity-from-previous-process".to_string(),
            )
            .unwrap(),
        )
        .unwrap();

        let lock = acquire_cli_target_lock(&lock_path).unwrap();

        assert!(!marker_path.exists());
        drop(lock);
    }

    #[test]
    fn recent_legacy_pid_marker_remains_compatible_during_handoff() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        fs::write(&marker_path, format!("{}\n", std::process::id())).unwrap();

        let error = match acquire_cli_target_lock(&lock_path) {
            Ok(_) => panic!("expected a recent legacy helper marker to reject the command"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            Chat2DbError::Io(error) if error.kind() == io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn old_legacy_pid_marker_does_not_trust_a_reused_live_pid() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        fs::write(&marker_path, format!("{}\n", std::process::id())).unwrap();
        fs::File::options()
            .write(true)
            .open(&marker_path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
            .unwrap();

        let lock = acquire_cli_target_lock(&lock_path).unwrap();

        assert!(!marker_path.exists());
        drop(lock);
    }

    #[test]
    fn fresh_handoff_marker_rejects_a_second_command() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        fs::write(&marker_path, "starting\n").unwrap();

        let error = match acquire_cli_target_lock(&lock_path) {
            Ok(_) => panic!("expected an in-progress lock handoff to reject the command"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            Chat2DbError::Io(error) if error.kind() == io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn stale_scheduled_transaction_marker_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        let stale_pid = 999_999_999;
        assert!(!process_exists(stale_pid).unwrap());
        fs::write(&marker_path, format!("{stale_pid}\n")).unwrap();

        let lock = acquire_cli_target_lock(&lock_path).unwrap();

        assert!(!marker_path.exists());
        drop(lock);
    }

    #[test]
    fn failed_scheduled_transaction_is_reported_and_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("cli-target.lock");
        let marker_path = scheduled_transaction_marker_path(&lock_path);
        fs::write(
            &marker_path,
            "failed\r\nMove-Item failed\r\nRecovery data: C:\\recovery",
        )
        .unwrap();

        let error = match acquire_cli_target_lock(&lock_path) {
            Ok(_) => panic!("expected the previous scheduled failure to be reported"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Move-Item failed"));
        assert!(error.to_string().contains("C:\\recovery"));
        assert_eq!(
            fs::read_to_string(&marker_path).unwrap(),
            "failed\r\nMove-Item failed\r\nRecovery data: C:\\recovery"
        );
    }

    #[test]
    fn windows_transaction_command_is_noninteractive_and_script_bound() {
        let script = Path::new("C:/Temp/chat2db-update/replace.ps1");
        let command = windows_transaction_command(script);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(command.get_program(), "powershell.exe");
        assert_eq!(
            args,
            [
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                "C:/Temp/chat2db-update/replace.ps1",
            ]
        );
    }

    #[test]
    fn windows_scheduled_script_locks_the_target_transaction() {
        let script = windows_scheduled_script(
            42,
            Path::new("C:/Users/tester/.chat2db-cli/locks/cli-target.lock"),
            Path::new("C:/Users/tester/.chat2db-cli/locks/cli-target.pending"),
            Path::new("C:/Users/tester/AppData/Local/Temp/chat2db-update"),
            "Write-Output 'transaction'\n",
        );

        assert!(script.contains("Wait-Process -Id 42 -ErrorAction SilentlyContinue"));
        assert!(script.contains("$candidate.Lock(0, 1)"));
        assert!(script.contains("$lockStream.Unlock(0, 1)"));
        assert!(script.contains("$attempt -lt 200"));
        assert!(script.contains("cli-target.pending"));
        assert!(script.contains("$transactionSucceeded = $false"));
        assert!(script.contains("[System.IO.File]::WriteAllText"));
        assert!(
            script.contains("Recovery data: C:/Users/tester/AppData/Local/Temp/chat2db-update")
        );
        let success_guard = script
            .find("if ($transactionSucceeded)")
            .expect("marker cleanup must be success-gated");
        let marker_cleanup = script
            .find(
                "Remove-Item -LiteralPath 'C:/Users/tester/.chat2db-cli/locks/cli-target.pending'",
            )
            .expect("successful transactions must remove their marker");
        assert!(success_guard < marker_cleanup);
        assert!(
            script.find("try {\n  Wait-Process").unwrap()
                < script.find("$candidate.Lock(0, 1)").unwrap()
        );
        assert!(
            script.find("cli-target.pending").unwrap()
                < script.find("$lockStream.Unlock(0, 1)").unwrap()
        );
    }

    #[test]
    fn windows_powershell_script_uses_utf8_bom_for_non_ascii_paths() {
        let script = windows_scheduled_script(
            42,
            Path::new("C:/Users/tester/.chat2db-cli/locks/cli-target.lock"),
            Path::new("C:/Users/tester/.chat2db-cli/locks/cli-target.pending"),
            Path::new("C:/Users/Ren\u{00e9}e/AppData/Local/Temp/Chat2DB-update"),
            "Write-Output 'pr\u{00e9}pare'\n",
        );
        let bytes = windows_powershell_script_bytes(&script);

        assert_eq!(&bytes[..3], &[0xef, 0xbb, 0xbf]);
        assert_eq!(std::str::from_utf8(&bytes[3..]).unwrap(), script);
        assert!(script.contains("C:/Users/Ren\u{00e9}e"));
        assert!(script.contains("pr\u{00e9}pare"));
    }

    #[test]
    fn windows_self_replace_cleans_recovery_data_only_after_promotion() {
        let operation = windows_self_replace_operation(
            Path::new("C:/Temp/update/pending-chat2db.exe"),
            Path::new("C:/Tools/chat2db.exe"),
            Path::new("C:/Temp/update/previous-chat2db.exe"),
            Path::new("C:/Temp/update/replace.ps1"),
            Path::new("C:/Temp/update"),
        );

        let promotion_check = operation
            .find("if (-not $promoted -or")
            .expect("promotion must be confirmed");
        let cleanup = operation
            .find("Remove-Item -LiteralPath 'C:/Temp/update/pending-chat2db.exe'")
            .expect("pending binary cleanup must be generated");
        assert!(promotion_check < cleanup);
        assert!(!operation.contains("finally"));
        assert!(operation.contains(
            "Copy-Item -LiteralPath 'C:/Temp/update/previous-chat2db.exe' -Destination 'C:/Tools/chat2db.exe' -Force"
        ));
    }

    #[test]
    fn rejects_non_chat2db_cli_target_name() {
        let error = validate_cli_target(Path::new("/tmp/not-chat2db")).unwrap_err();
        assert!(matches!(error, Chat2DbError::InvalidCliTarget { .. }));
    }

    #[test]
    fn accepts_windows_cli_target_name() {
        validate_cli_target(Path::new(
            "C:/Users/tester/AppData/Local/Programs/chat2db-cli/chat2db.exe",
        ))
        .unwrap();
    }

    #[test]
    fn extracts_release_tarball_binary() {
        let tarball = build_cli_tarball(b"fake-cli");
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("chat2db");

        extract_cli_tarball(&tarball, &staged).unwrap();

        assert_eq!(fs::read(&staged).unwrap(), b"fake-cli");
    }

    #[test]
    fn ignores_macos_tar_metadata_entries() {
        let tarball = build_cli_tarball_with_entries(&[
            ("._chat2db", b"metadata" as &[u8]),
            ("chat2db", b"fake-cli" as &[u8]),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("chat2db");

        extract_cli_tarball(&tarball, &staged).unwrap();

        assert_eq!(fs::read(&staged).unwrap(), b"fake-cli");
    }

    #[test]
    fn extracts_windows_zip_binary() {
        let zip = build_cli_zip(&[("chat2db.exe", b"fake-windows-cli" as &[u8])]);
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("chat2db.exe");

        extract_cli_zip(&zip, &staged).unwrap();

        assert_eq!(fs::read(&staged).unwrap(), b"fake-windows-cli");
    }

    #[test]
    fn rejects_extra_entries_in_windows_zip() {
        let zip = build_cli_zip(&[
            ("chat2db.exe", b"fake" as &[u8]),
            ("README.md", b"unexpected" as &[u8]),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("chat2db.exe");

        let err = extract_cli_zip(&zip, &staged).unwrap_err();

        assert!(matches!(err, Chat2DbError::SkillBundleCorrupt { .. }));
    }

    #[test]
    fn updates_cli_from_target_bound_manifest() {
        let new_binary = b"#!/bin/sh\nexit 0\n";
        let os = current_cli_os_label().unwrap();
        let arch = current_arch_label().unwrap();
        let archive = if os == "windows" {
            build_cli_zip(&[("chat2db.exe", new_binary as &[u8])])
        } else {
            build_cli_tarball(new_binary)
        };
        let client = FakeHttpClient {
            manifest: manifest_for(&archive, "999.999.999", os, arch),
            archive,
        };
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());

        let outcome = update_cli_with_client(
            &client,
            UpdateRequest {
                target_path: Some(target.clone()),
                progress: false,
            },
        )
        .unwrap();

        assert_eq!(outcome.status, "updated");
        assert_eq!(outcome.version, "999.999.999");
        assert_eq!(outcome.target_path, target);
        assert!(outcome.manifest_url.ends_with(".json"));
        assert_eq!(fs::read(&outcome.target_path).unwrap(), new_binary);
        assert!(
            fs::read_dir(dir.path()).unwrap().all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".chat2db-cli.update.")
            }),
            "successful updates must clean their transaction directory"
        );
    }

    #[test]
    fn update_waits_for_the_target_lock_before_fetching() {
        let new_binary = b"#!/bin/sh\nexit 0\n";
        let os = current_cli_os_label().unwrap();
        let arch = current_arch_label().unwrap();
        let archive = if os == "windows" {
            build_cli_zip(&[("chat2db.exe", new_binary as &[u8])])
        } else {
            build_cli_tarball(new_binary)
        };
        let inner = FakeHttpClient {
            manifest: manifest_for(&archive, "999.999.999", os, arch),
            archive,
        };
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        let normalized_target = normalized_cli_target(&target).unwrap();
        let lock_path = cli_target_lock_path(&normalized_target);
        let held_lock = acquire_exclusive_lock(&lock_path).unwrap();
        let (called_tx, called_rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            update_cli_with_client(
                &NotifyingHttpClient {
                    inner,
                    called: called_tx,
                },
                UpdateRequest {
                    target_path: Some(target),
                    progress: false,
                },
            )
        });

        assert!(called_rx.recv_timeout(Duration::from_millis(150)).is_err());
        drop(held_lock);
        called_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fs::remove_file(lock_path);
    }

    #[test]
    fn failed_promotion_preserves_the_previous_binary() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        fs::write(&target, b"old-cli").unwrap();
        let transaction = tempfile::Builder::new()
            .prefix(".chat2db-cli.update.")
            .tempdir_in(dir.path())
            .unwrap();
        let invalid_source = transaction.path().join("pending-directory");
        fs::create_dir(&invalid_source).unwrap();

        replace_binary(
            transaction,
            &invalid_source,
            &target,
            &dir.path().join("cli-target.lock"),
            false,
        )
        .unwrap_err();

        assert_eq!(fs::read(target).unwrap(), b"old-cli");
    }

    #[test]
    fn new_target_created_in_the_final_commit_window_is_not_clobbered() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        let transaction = tempfile::Builder::new()
            .prefix(".chat2db-cli.update.")
            .tempdir_in(dir.path())
            .unwrap();
        let source = transaction.path().join("pending-chat2db");
        fs::write(&source, b"downloaded-cli").unwrap();

        assert!(!validate_cli_target(&target).unwrap());
        fs::write(&target, b"second-writer-cli").unwrap();

        replace_binary(
            transaction,
            &source,
            &target,
            &dir.path().join("cli-target.lock"),
            true,
        )
        .unwrap_err();

        assert_eq!(fs::read(target).unwrap(), b"second-writer-cli");
    }

    #[cfg(unix)]
    #[test]
    fn new_target_symlink_created_in_the_final_commit_window_is_not_replaced() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim-data").unwrap();
        let transaction = tempfile::Builder::new()
            .prefix(".chat2db-cli.update.")
            .tempdir_in(dir.path())
            .unwrap();
        let source = transaction.path().join("pending-chat2db");
        fs::write(&source, b"downloaded-cli").unwrap();

        assert!(!validate_cli_target(&target).unwrap());
        symlink(&victim, &target).unwrap();

        replace_binary(
            transaction,
            &source,
            &target,
            &dir.path().join("cli-target.lock"),
            true,
        )
        .unwrap_err();

        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(victim).unwrap(), b"victim-data");
    }

    #[cfg(unix)]
    #[test]
    fn existing_target_replacement_behavior_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        fs::write(&target, b"old-cli").unwrap();
        let transaction = tempfile::Builder::new()
            .prefix(".chat2db-cli.update.")
            .tempdir_in(dir.path())
            .unwrap();
        let source = transaction.path().join("pending-chat2db");
        fs::write(&source, b"new-cli").unwrap();

        let status = replace_binary(
            transaction,
            &source,
            &target,
            &dir.path().join("cli-target.lock"),
            false,
        )
        .unwrap();

        assert_eq!(status, "updated");
        assert_eq!(fs::read(target).unwrap(), b"new-cli");
    }

    #[test]
    fn update_rejects_manifest_for_another_target_before_replacement() {
        let (wrong_os, wrong_arch) = if current_cli_os_label().unwrap() == "windows" {
            ("darwin", "arm64")
        } else {
            ("windows", "x86_64")
        };
        let archive = if wrong_os == "windows" {
            build_cli_zip(&[("chat2db.exe", b"wrong-target" as &[u8])])
        } else {
            build_cli_tarball(b"wrong-target")
        };
        let client = FakeHttpClient {
            manifest: manifest_for(&archive, "0.1.99", wrong_os, wrong_arch),
            archive,
        };
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());

        let error = update_cli_with_client(
            &client,
            UpdateRequest {
                target_path: Some(target.clone()),
                progress: false,
            },
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::InvalidReleaseManifest(_)));
        assert!(!target.exists());
    }

    #[test]
    fn default_update_allows_only_strictly_newer_versions() {
        assert!(!should_apply_update(false, "1.2.3", "1.2.3").unwrap());
        assert!(!should_apply_update(false, "1.2.3", "1.2.2").unwrap());
        assert!(should_apply_update(false, "1.2.3", "1.2.4").unwrap());
    }

    #[test]
    fn new_explicit_target_allows_current_version_but_not_downgrade() {
        assert!(should_apply_update(true, "1.2.3", "1.2.3").unwrap());
        assert!(!should_apply_update(true, "1.2.3", "1.2.2").unwrap());
        assert!(should_apply_update(true, "1.2.3", "1.2.4").unwrap());
    }

    #[test]
    fn rejects_existing_explicit_target_that_is_not_the_running_binary() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        fs::write(&target, b"newer-cli").unwrap();

        let error = validate_explicit_update_target(true, &target, true).unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::Io(error) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert_eq!(fs::read(target).unwrap(), b"newer-cli");
    }

    #[test]
    fn target_created_while_waiting_for_lock_is_not_replaced() {
        let os = current_cli_os_label().unwrap();
        let arch = current_arch_label().unwrap();
        let archive = if os == "windows" {
            build_cli_zip(&[("chat2db.exe", b"pending-cli" as &[u8])])
        } else {
            build_cli_tarball(b"pending-cli")
        };
        let inner = FakeHttpClient {
            manifest: manifest_for(&archive, "999.999.999", os, arch),
            archive,
        };
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(cli_binary_name());
        let lock_path = cli_target_lock_path(&normalized_cli_target(&target).unwrap());
        let held_lock = acquire_exclusive_lock(&lock_path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (called_tx, called_rx) = mpsc::channel();
        let thread_target = target.clone();

        let handle = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            update_cli_with_client(
                &NotifyingHttpClient {
                    inner,
                    called: called_tx,
                },
                UpdateRequest {
                    target_path: Some(thread_target),
                    progress: false,
                },
            )
        });

        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(called_rx.recv_timeout(Duration::from_millis(150)).is_err());
        fs::write(&target, b"newer-cli").unwrap();
        drop(held_lock);

        let error = handle.join().unwrap().unwrap_err();
        assert!(matches!(
            error,
            Chat2DbError::Io(error) if error.kind() == io::ErrorKind::PermissionDenied
        ));
        assert!(called_rx.recv_timeout(Duration::from_millis(150)).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"newer-cli");
        let _ = fs::remove_file(lock_path);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_dangling_symlink_target() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("chat2db");
        symlink(dir.path().join("missing-chat2db"), &target).unwrap();

        let error = validate_cli_target(&target).unwrap_err();

        assert!(matches!(error, Chat2DbError::InvalidCliTarget { .. }));
        assert!(
            fs::symlink_metadata(target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn update_version_policy_uses_semver_prerelease_ordering() {
        assert!(should_apply_update(false, "1.0.0-rc.1", "1.0.0").unwrap());
        assert!(should_apply_update(false, "1.0.0-alpha.1", "1.0.0-alpha.2").unwrap());
        assert!(!should_apply_update(false, "1.0.0", "1.0.0-rc.1").unwrap());
        assert!(!should_apply_update(true, "1.0.0-alpha.2", "1.0.0-alpha.1").unwrap());
    }

    fn build_cli_tarball(contents: &[u8]) -> Vec<u8> {
        build_cli_tarball_with_entries(&[("chat2db", contents)])
    }

    fn build_cli_tarball_with_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            for (path, contents) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_path(path).unwrap();
                header.set_size(contents.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append(&header, *contents).unwrap();
            }
            builder.finish().unwrap();
        }
        tar_bytes
    }

    fn build_cli_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, contents) in entries {
            writer.start_file(*name, opts).unwrap();
            std::io::Write::write_all(&mut writer, contents).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn manifest_for(archive: &[u8], version: &str, os: &str, arch: &str) -> Vec<u8> {
        let (suffix, artifact_type) = if os == "windows" {
            ("zip", "zip")
        } else {
            ("tar.gz", "tarball")
        };
        format!(
            r#"{{
  "version": "{version}",
  "artifact": {{
    "url": "https://cdn.chat2db-ai.com/download/chat2db-cli/{version}/{os}/{arch}/chat2db-{version}-{os}-{arch}.{suffix}",
    "sha256": "{}",
    "artifact_type": "{artifact_type}"
  }}
}}"#,
            sha256_hex(archive)
        )
        .into_bytes()
    }

    struct FakeHttpClient {
        manifest: Vec<u8>,
        archive: Vec<u8>,
    }

    impl HttpClient for FakeHttpClient {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            if url.ends_with(".json") {
                Ok(self.manifest.clone())
            } else if url.ends_with(".tar.gz") || url.ends_with(".zip") {
                Ok(self.archive.clone())
            } else {
                panic!("unexpected URL: {url}");
            }
        }
    }

    struct NotifyingHttpClient {
        inner: FakeHttpClient,
        called: mpsc::Sender<()>,
    }

    impl HttpClient for NotifyingHttpClient {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            let _ = self.called.send(());
            self.inner.get(url)
        }
    }
}
