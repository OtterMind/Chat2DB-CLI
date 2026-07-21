use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::app_plist::read_app_bundle_metadata;
use crate::edition::{DesktopArtifactKind, Edition};
use crate::error::{Chat2DbError, Result};
use crate::http::{DownloadProgress, HttpClient};
use crate::manifest::{
    DesktopArtifactChecksum, DesktopArtifactRegistry, embedded_desktop_artifacts,
    verify_file_sha256, verify_file_sha512,
};
use crate::platform::{
    ExclusiveLock, HostOs, current_arch_label, current_os, home_dir, windows_local_app_data_dir,
};
use crate::process::{PgrepProbe, ProcessProbe, ensure_app_stopped};
use crate::runtime::{acquire_edition_lifecycle_lock, ensure_runtime_stopped_locked};

#[derive(Debug, Clone)]
pub struct SystemPaths {
    pub applications_dir: PathBuf,
    pub home_dir: PathBuf,
    pub background_state_dir: PathBuf,
}

impl SystemPaths {
    pub fn production() -> Self {
        let home_dir = home_dir();
        let os = current_os();
        let background_state_dir = crate::background::state_dir();
        let applications_dir = match os {
            HostOs::Windows => windows_local_app_data_dir(&home_dir),
            _ => PathBuf::from("/Applications"),
        };
        Self {
            home_dir,
            applications_dir,
            background_state_dir,
        }
    }

    pub fn edition_home(&self, edition: Edition) -> PathBuf {
        edition.data_dir(&self.home_dir)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallRequest {
    pub edition: Edition,
    pub force: bool,
    pub progress: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct UninstallRequest {
    pub edition: Edition,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub app_path: PathBuf,
    pub artifact_url: String,
    pub artifact_type: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub app_path: PathBuf,
    pub preserved_data_dir: PathBuf,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemDesktopArtifactInstaller;

pub trait DesktopArtifactInstaller {
    fn install_artifact(
        &self,
        artifact_path: &Path,
        stage_dir: &Path,
        target_path: &Path,
        edition: Edition,
        kind: DesktopArtifactKind,
    ) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct Installer<C, D = SystemDesktopArtifactInstaller> {
    client: C,
    paths: SystemPaths,
    artifact_installer: D,
    desktop_artifacts: Vec<u8>,
}

impl<C: HttpClient> Installer<C> {
    pub fn new(client: C, paths: SystemPaths) -> Self {
        Self {
            client,
            paths,
            artifact_installer: SystemDesktopArtifactInstaller,
            desktop_artifacts: embedded_desktop_artifacts().to_vec(),
        }
    }
}

impl<C: HttpClient, D: DesktopArtifactInstaller> Installer<C, D> {
    pub fn with_artifact_installer(client: C, paths: SystemPaths, artifact_installer: D) -> Self {
        Self {
            client,
            paths,
            artifact_installer,
            desktop_artifacts: embedded_desktop_artifacts().to_vec(),
        }
    }

    pub fn with_desktop_artifact_registry(mut self, manifest: Vec<u8>) -> Self {
        self.desktop_artifacts = manifest;
        self
    }

    pub fn install(&self, request: InstallRequest) -> Result<InstallOutcome> {
        self.install_for_os(request, current_os())
    }

    pub fn install_for_os(&self, request: InstallRequest, os: HostOs) -> Result<InstallOutcome> {
        self.install_for_os_with_probe(request, os, &PgrepProbe)
    }

    fn install_for_os_with_probe<P: ProcessProbe + ?Sized>(
        &self,
        request: InstallRequest,
        os: HostOs,
        process_probe: &P,
    ) -> Result<InstallOutcome> {
        let _lock = acquire_app_operation_lock(&self.paths, request.edition)?;
        ensure_runtime_stopped_locked(&self.paths, request.edition)?;
        let arch = current_arch_label()?;
        let target_path = request
            .edition
            .resolve_app_path_for_os(&self.paths.applications_dir, os);
        ensure_app_stopped(process_probe, &target_path, request.edition)?;
        if os == HostOs::Macos {
            recover_macos_app_transaction(&target_path, request.edition)?;
        }
        if target_path.exists() {
            if !request.force {
                return Err(Chat2DbError::AppAlreadyExists { path: target_path });
            }
            verify_app_bundle(&target_path, request.edition).map_err(|err| {
                Chat2DbError::UnsafeReplace {
                    path: target_path.clone(),
                    reason: err.to_string(),
                }
            })?;
        }

        let artifact = self.resolve_desktop_artifact(request.edition, os, arch)?;
        let artifact_kind = artifact.kind;
        let artifact_url = artifact.url;

        fs::create_dir_all(&self.paths.applications_dir)?;
        let temp_dir = tempfile::tempdir()?;
        let stage_dir = temp_dir.path().join("stage");
        fs::create_dir(&stage_dir)?;

        let artifact_path = temp_dir
            .path()
            .join(format!("artifact.{}", artifact_kind.extension()));
        self.client.download_to(
            &artifact_url,
            &artifact_path,
            DownloadProgress::new(
                request.progress,
                &format!(
                    "{} {}",
                    request.edition.app_name(),
                    artifact_kind.progress_label()
                ),
            ),
        )?;
        let actual_size = fs::metadata(&artifact_path)?.len();
        if actual_size != artifact.size {
            return Err(Chat2DbError::InvalidReleaseManifest(format!(
                "artifact size mismatch: expected {}, got {actual_size}",
                artifact.size
            )));
        }
        match &artifact.checksum {
            DesktopArtifactChecksum::Sha256(expected) => {
                verify_file_sha256(&artifact_path, expected)?
            }
            DesktopArtifactChecksum::Sha512(expected) => {
                verify_file_sha512(&artifact_path, expected)?
            }
        }
        print_progress(
            request.progress,
            &format!("Installing {}...", request.edition.app_name()),
        );

        ensure_app_stopped(process_probe, &target_path, request.edition)?;

        self.artifact_installer.install_artifact(
            &artifact_path,
            &stage_dir,
            &target_path,
            request.edition,
            artifact_kind,
        )?;
        let target_path = request
            .edition
            .resolve_app_path_for_os(&self.paths.applications_dir, os);
        verify_app_bundle(&target_path, request.edition)?;

        Ok(InstallOutcome {
            command: "install",
            edition: request.edition,
            status: "installed",
            app_path: target_path,
            artifact_url,
            artifact_type: artifact_kind.as_str(),
        })
    }

    pub fn uninstall(&self, request: UninstallRequest) -> Result<UninstallOutcome> {
        self.uninstall_for_os(request, current_os())
    }

    pub fn uninstall_for_os(
        &self,
        request: UninstallRequest,
        os: HostOs,
    ) -> Result<UninstallOutcome> {
        self.uninstall_for_os_with_probe(request, os, &PgrepProbe)
    }

    fn uninstall_for_os_with_probe<P: ProcessProbe + ?Sized>(
        &self,
        request: UninstallRequest,
        os: HostOs,
        process_probe: &P,
    ) -> Result<UninstallOutcome> {
        let _lock = acquire_app_operation_lock(&self.paths, request.edition)?;
        ensure_runtime_stopped_locked(&self.paths, request.edition)?;
        let target_path = request
            .edition
            .resolve_app_path_for_os(&self.paths.applications_dir, os);
        ensure_app_stopped(process_probe, &target_path, request.edition)?;
        if os == HostOs::Macos {
            recover_macos_app_transaction(&target_path, request.edition)?;
        }
        let status = if target_path.exists() {
            let windows_identity = if os == HostOs::Windows {
                verify_windows_exe(&target_path, request.edition)
            } else {
                verify_app_bundle(&target_path, request.edition).map(|()| None)
            }
            .map_err(|err| Chat2DbError::UnsafeUninstall {
                path: target_path.clone(),
                reason: err.to_string(),
            })?;
            ensure_app_stopped(process_probe, &target_path, request.edition)?;
            remove_app_installation(&target_path, request.edition, os, windows_identity.as_ref())?;
            "removed"
        } else {
            "already_absent"
        };

        Ok(UninstallOutcome {
            command: "uninstall",
            edition: request.edition,
            status,
            app_path: target_path,
            preserved_data_dir: self.paths.edition_home(request.edition),
        })
    }

    fn resolve_desktop_artifact(
        &self,
        edition: Edition,
        os: HostOs,
        arch: &str,
    ) -> Result<ResolvedDesktopArtifact> {
        let kind = edition.desktop_artifact_kind(os)?;
        let registry = DesktopArtifactRegistry::parse(&self.desktop_artifacts)?;
        let artifact = registry.artifact(edition, os, arch)?;
        Ok(ResolvedDesktopArtifact {
            url: artifact.url.clone(),
            kind,
            checksum: artifact.checksum.clone(),
            size: artifact.size,
        })
    }
}

// Desktop mutation and headless Runtime lifecycle share one edition lock.
fn acquire_app_operation_lock(paths: &SystemPaths, edition: Edition) -> Result<ExclusiveLock> {
    acquire_edition_lifecycle_lock(paths, edition)
}

#[derive(Debug, Clone)]
struct ResolvedDesktopArtifact {
    url: String,
    kind: DesktopArtifactKind,
    checksum: DesktopArtifactChecksum,
    size: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
struct WindowsUninstallRecord {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    install_location: String,
    #[serde(default)]
    display_icon: String,
    #[serde(default)]
    product_code: String,
    #[serde(default)]
    upgrade_code: String,
}

#[cfg(any(windows, test))]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WindowsUninstallRecords {
    Many(Vec<WindowsUninstallRecord>),
    One(WindowsUninstallRecord),
}

fn print_progress(enabled: bool, message: &str) {
    if enabled {
        eprintln!("{message}");
    }
}

impl DesktopArtifactInstaller for SystemDesktopArtifactInstaller {
    fn install_artifact(
        &self,
        artifact_path: &Path,
        stage_dir: &Path,
        target_path: &Path,
        edition: Edition,
        kind: DesktopArtifactKind,
    ) -> Result<()> {
        match kind {
            DesktopArtifactKind::Dmg => install_dmg(artifact_path, stage_dir, target_path, edition),
            DesktopArtifactKind::Exe => install_windows_exe(artifact_path),
            DesktopArtifactKind::Msi => install_windows_msi(artifact_path),
        }
    }
}

fn install_dmg(
    dmg_path: &Path,
    _stage_dir: &Path,
    target_path: &Path,
    edition: Edition,
) -> Result<()> {
    let mount_temp = tempfile::tempdir()?;
    let mount_point = mount_temp.path().join("mount");
    fs::create_dir(&mount_point)?;

    run_command(
        "hdiutil",
        [
            OsStr::new("attach"),
            dmg_path.as_os_str(),
            OsStr::new("-nobrowse"),
            OsStr::new("-readonly"),
            OsStr::new("-mountpoint"),
            mount_point.as_os_str(),
        ],
    )?;

    let install_result = (|| {
        let mounted_app = find_single_app_in_dir(&mount_point)?;
        verify_app_bundle(&mounted_app, edition)?;
        let (pending_path, backup_path) = macos_transaction_paths(target_path);
        remove_transaction_entry(&pending_path)?;
        copy_app_bundle(&mounted_app, &pending_path)?;
        verify_app_bundle(&pending_path, edition)?;
        promote_macos_app(&pending_path, target_path, &backup_path)?;
        Ok(())
    })();

    let detach_result = run_command(
        "hdiutil",
        [
            OsStr::new("detach"),
            mount_point.as_os_str(),
            OsStr::new("-quiet"),
        ],
    );

    match (install_result, detach_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
    }
}

fn macos_transaction_paths(target_path: &Path) -> (PathBuf, PathBuf) {
    let mut pending = target_path.as_os_str().to_os_string();
    pending.push(".chat2db-cli.pending");
    let mut backup = target_path.as_os_str().to_os_string();
    backup.push(".chat2db-cli.backup");
    (pending.into(), backup.into())
}

fn recover_macos_app_transaction(target_path: &Path, edition: Edition) -> Result<()> {
    let (pending_path, backup_path) = macos_transaction_paths(target_path);

    if backup_path.exists() {
        if target_path.exists() {
            verify_transaction_app(target_path, edition)?;
            remove_transaction_entry(&backup_path)?;
        } else {
            verify_transaction_app(&backup_path, edition)?;
            fs::rename(&backup_path, target_path)?;
        }
    }

    if pending_path.exists() {
        if target_path.exists() {
            remove_transaction_entry(&pending_path)?;
        } else if verify_app_bundle(&pending_path, edition).is_ok() {
            fs::rename(&pending_path, target_path)?;
        } else {
            remove_transaction_entry(&pending_path)?;
        }
    }

    Ok(())
}

fn verify_transaction_app(path: &Path, edition: Edition) -> Result<()> {
    verify_app_bundle(path, edition).map_err(|error| Chat2DbError::UnsafeReplace {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

fn promote_macos_app(pending_path: &Path, target_path: &Path, backup_path: &Path) -> Result<()> {
    promote_macos_app_with(
        pending_path,
        target_path,
        backup_path,
        |source, destination| fs::rename(source, destination),
    )
}

fn promote_macos_app_with<F>(
    pending_path: &Path,
    target_path: &Path,
    backup_path: &Path,
    mut rename: F,
) -> Result<()>
where
    F: FnMut(&Path, &Path) -> std::io::Result<()>,
{
    let replacing = target_path.exists();
    if replacing {
        rename(target_path, backup_path)?;
    }

    if let Err(install_error) = rename(pending_path, target_path) {
        if replacing && let Err(rollback_error) = rename(backup_path, target_path) {
            return Err(std::io::Error::other(format!(
                "failed to install app: {install_error}; failed to restore previous app: {rollback_error}; recovery data remains at {} and {}",
                pending_path.display(),
                backup_path.display()
            ))
            .into());
        }
        return Err(install_error.into());
    }

    if replacing {
        remove_transaction_entry(backup_path)?;
    }
    Ok(())
}

fn remove_transaction_entry(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn install_windows_exe(exe_path: &Path) -> Result<()> {
    let primary = run_installer_command(
        exe_path,
        &["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"],
    );
    match primary {
        Ok(()) => Ok(()),
        Err(primary_error) => run_installer_command(exe_path, &["/S"]).map_err(|fallback_error| {
            Chat2DbError::CommandFailed {
                program: "windows app installer",
                status: "failed".to_string(),
                stderr: format!("{primary_error}; fallback /S failed: {fallback_error}"),
            }
        }),
    }
}

fn install_windows_msi(msi_path: &Path) -> Result<()> {
    let output = Command::new("msiexec.exe")
        .arg("/i")
        .arg(msi_path)
        .args(["/quiet", "/norestart"])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "msiexec.exe",
            source,
        })?;
    if output.status.success() || output.status.code() == Some(3010) {
        Ok(())
    } else {
        Err(Chat2DbError::CommandFailed {
            program: "msiexec.exe",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn run_installer_command(exe_path: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new(exe_path)
        .args(args)
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "windows app installer",
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Chat2DbError::CommandFailed {
            program: "windows app installer",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn run_command<const N: usize>(program: &'static str, args: [&OsStr; N]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|source| Chat2DbError::CommandIo { program, source })?;

    if output.status.success() {
        return Ok(());
    }

    Err(Chat2DbError::CommandFailed {
        program,
        status: output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

pub fn find_single_app_in_dir(dir: &Path) -> Result<PathBuf> {
    let mut apps = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension() == Some(OsStr::new("app")) && path.is_dir() {
            apps.push(path);
        }
    }

    if apps.len() == 1 {
        Ok(apps.remove(0))
    } else {
        Err(Chat2DbError::InvalidDmg)
    }
}

fn copy_app_bundle(source: &Path, destination: &Path) -> Result<()> {
    run_command(
        "/bin/cp",
        [
            OsStr::new("-R"),
            source.as_os_str(),
            destination.as_os_str(),
        ],
    )
}

pub fn verify_app_bundle(app_path: &Path, edition: Edition) -> Result<()> {
    if app_path.extension() == Some(OsStr::new("exe")) {
        return verify_windows_exe(app_path, edition).map(|_| ());
    }

    let info_plist = app_path.join("Contents/Info.plist");
    if !info_plist.is_file() {
        return Err(Chat2DbError::MissingInfoPlist(info_plist));
    }

    let metadata = read_app_bundle_metadata(&info_plist)?;
    let actual = metadata
        .bundle_id
        .as_deref()
        .ok_or_else(|| Chat2DbError::BundleIdMismatch {
            expected: edition.bundle_id().to_string(),
            actual: "<missing>".to_string(),
        })?;

    if actual != edition.bundle_id() {
        return Err(Chat2DbError::BundleIdMismatch {
            expected: edition.bundle_id().to_string(),
            actual: actual.to_string(),
        });
    }

    Ok(())
}

fn verify_windows_exe(app_path: &Path, edition: Edition) -> Result<Option<WindowsUninstallRecord>> {
    if !app_path.is_file() {
        return Err(Chat2DbError::AppNotInstalled {
            edition: edition.to_string(),
            path: app_path.to_path_buf(),
        });
    }

    let actual = app_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<missing>");
    let expected = edition.windows_exe_name();
    let accepted_dash_name = match edition {
        Edition::Community => "Chat2DB-Community.exe",
        Edition::Pro => "Chat2DB-Pro.exe",
        Edition::Local => "Chat2DB-Local.exe",
    };
    if actual != expected && actual != accepted_dash_name {
        return Err(Chat2DbError::BundleIdMismatch {
            expected: expected.to_string(),
            actual: actual.to_string(),
        });
    }

    #[cfg(windows)]
    {
        let records = query_windows_uninstall_records(edition)?;
        select_windows_install_identity(&records, app_path, edition)
            .map(Some)
            .map_err(|actual| Chat2DbError::BundleIdMismatch {
                expected: format!(
                    "{} MSI ({}) at {}",
                    edition.app_name(),
                    edition.windows_upgrade_code(),
                    app_path.display()
                ),
                actual,
            })
    }

    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

fn remove_app_installation(
    app_path: &Path,
    edition: Edition,
    os: HostOs,
    windows_identity: Option<&WindowsUninstallRecord>,
) -> Result<()> {
    match os {
        HostOs::Windows => {
            let identity = windows_identity.ok_or_else(|| Chat2DbError::UnsafeUninstall {
                path: app_path.to_path_buf(),
                reason: format!(
                    "no verified {} Windows Installer identity is available",
                    edition.app_name()
                ),
            })?;
            run_windows_msi_uninstaller(identity)
        }
        _ => fs::remove_dir_all(app_path).map_err(Into::into),
    }
}

fn run_windows_msi_uninstaller(identity: &WindowsUninstallRecord) -> Result<()> {
    let product_code =
        normalize_guid(&identity.product_code).ok_or_else(|| Chat2DbError::UnsafeUninstall {
            path: PathBuf::from(&identity.install_location),
            reason: "verified Windows Installer record has an invalid ProductCode".to_string(),
        })?;
    let product_code = format!("{{{product_code}}}");
    let output = Command::new("msiexec.exe")
        .args(["/x", &product_code, "/quiet", "/norestart"])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "msiexec.exe",
            source,
        })?;
    if output.status.success() || output.status.code() == Some(3010) {
        Ok(())
    } else {
        Err(Chat2DbError::CommandFailed {
            program: "msiexec.exe",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[cfg(windows)]
fn query_windows_uninstall_records(edition: Edition) -> Result<Vec<WindowsUninstallRecord>> {
    let display_name = edition.app_name().replace('\'', "''");
    let upgrade_code = edition.windows_upgrade_code();
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
using System.Text;
public static class Chat2DbWindowsInstaller {{
    [DllImport("msi.dll", CharSet = CharSet.Unicode)]
    public static extern uint MsiEnumRelatedProducts(string upgradeCode, uint reserved, uint index, StringBuilder productCode);
}}
'@
$upgradeCode = '{{{upgrade_code}}}'
$relatedProducts = @()
for ($index = 0; ; $index++) {{
  $productCode = [System.Text.StringBuilder]::new(39)
  $result = [Chat2DbWindowsInstaller]::MsiEnumRelatedProducts($upgradeCode, 0, $index, $productCode)
  if ($result -eq 259) {{ break }}
  if ($result -ne 0) {{ throw "MsiEnumRelatedProducts failed with status $result" }}
  $relatedProducts += $productCode.ToString().Trim('{{}}').ToUpperInvariant()
}}
$roots = @(
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
)
$records = @(
  Get-ItemProperty $roots -ErrorAction SilentlyContinue |
    Where-Object {{ [string]$_.DisplayName -eq '{display_name}' }} |
    ForEach-Object {{
      $productCode = [string]$_.PSChildName
      $isRelated = $relatedProducts -contains $productCode.Trim('{{}}').ToUpperInvariant()
      [pscustomobject]@{{
        DisplayName = [string]$_.DisplayName
        InstallLocation = [Environment]::ExpandEnvironmentVariables([string]$_.InstallLocation)
        DisplayIcon = [Environment]::ExpandEnvironmentVariables([string]$_.DisplayIcon)
        ProductCode = $productCode
        UpgradeCode = $(if ($isRelated) {{ $upgradeCode }} else {{ '' }})
      }}
    }}
)
ConvertTo-Json -InputObject @($records) -Compress"#
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "powershell.exe",
            source,
        })?;
    if !output.status.success() {
        return Err(Chat2DbError::CommandFailed {
            program: "powershell.exe",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    parse_windows_uninstall_records(&output.stdout).map_err(|reason| Chat2DbError::CommandFailed {
        program: "powershell.exe",
        status: "invalid_output".to_string(),
        stderr: reason,
    })
}

#[cfg(any(windows, test))]
fn parse_windows_uninstall_records(
    output: &[u8],
) -> std::result::Result<Vec<WindowsUninstallRecord>, String> {
    if output.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    match serde_json::from_slice::<WindowsUninstallRecords>(output)
        .map_err(|error| format!("invalid Windows uninstall registry JSON: {error}"))?
    {
        WindowsUninstallRecords::Many(records) => Ok(records),
        WindowsUninstallRecords::One(record) => Ok(vec![record]),
    }
}

#[cfg(any(windows, test))]
fn select_windows_install_identity(
    records: &[WindowsUninstallRecord],
    app_path: &Path,
    edition: Edition,
) -> std::result::Result<WindowsUninstallRecord, String> {
    let expected_upgrade_code = normalize_guid(edition.windows_upgrade_code())
        .expect("edition Windows UpgradeCode must be a GUID");
    let mut matches = Vec::new();

    for record in records {
        let Some(product_code) = normalize_guid(&record.product_code) else {
            continue;
        };
        if !record.display_name.eq_ignore_ascii_case(edition.app_name())
            || normalize_guid(&record.upgrade_code).as_deref()
                != Some(expected_upgrade_code.as_str())
            || !windows_record_path_matches(record, app_path)
        {
            continue;
        }
        if matches.iter().any(|matched: &WindowsUninstallRecord| {
            normalize_guid(&matched.product_code).as_deref() == Some(product_code.as_str())
        }) {
            continue;
        }
        matches.push(record.clone());
    }

    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(format!(
            "no uninstall registry record matched DisplayName, UpgradeCode, ProductCode, and installed path for {}",
            app_path.display()
        )),
        count => Err(format!(
            "{count} distinct uninstall registry records matched {}; refusing an ambiguous Windows installation",
            app_path.display()
        )),
    }
}

#[cfg(any(windows, test))]
fn windows_record_path_matches(record: &WindowsUninstallRecord, app_path: &Path) -> bool {
    let app_path = normalize_windows_path(&app_path.to_string_lossy());
    let install_dir = app_path
        .rsplit_once('\\')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let location_matches = !record.install_location.trim().is_empty()
        && normalize_windows_path(&record.install_location) == install_dir;
    let icon_matches = windows_display_icon_path(&record.display_icon)
        .is_some_and(|icon| normalize_windows_path(icon) == app_path);
    location_matches || icon_matches
}

#[cfg(any(windows, test))]
fn windows_display_icon_path(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(quoted) = value.strip_prefix('"') {
        return quoted.split_once('"').map(|(path, _)| path);
    }
    match value.rsplit_once(',') {
        Some((path, index)) if index.trim().parse::<i32>().is_ok() => Some(path.trim()),
        _ => Some(value),
    }
}

#[cfg(any(windows, test))]
fn normalize_windows_path(value: &str) -> String {
    let mut normalized = value
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .to_ascii_lowercase();
    if let Some(path) = normalized.strip_prefix(r"\\?\") {
        normalized = path.to_string();
    }
    while normalized.len() > 3 && normalized.ends_with('\\') {
        normalized.pop();
    }
    normalized
}

fn normalize_guid(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('{').trim_end_matches('}');
    let groups: Vec<&str> = value.split('-').collect();
    if groups.len() != 5
        || [8, 4, 4, 4, 12]
            .iter()
            .zip(&groups)
            .any(|(expected, group)| group.len() != *expected)
        || !groups
            .iter()
            .all(|group| group.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(value.to_ascii_uppercase())
}

#[cfg(test)]
mod windows_identity_tests {
    use super::*;

    const COMMUNITY_PRODUCT_CODE: &str = "{0A1B2C3D-4E5F-6789-A012-3456789ABCDE}";
    const OTHER_PRODUCT_CODE: &str = "{11111111-2222-3333-4444-555555555555}";
    const COMMUNITY_APP_PATH: &str = r"C:\Program Files\Chat2DB Community\Chat2DB Community.exe";

    #[test]
    fn parses_windows_registry_json_array() {
        let output = br#"[{"DisplayName":"Chat2DB Community","InstallLocation":"C:\\Program Files\\Chat2DB Community","DisplayIcon":"","ProductCode":"{0A1B2C3D-4E5F-6789-A012-3456789ABCDE}","UpgradeCode":"{4D7C78BC-B42F-4F81-9F5F-56E3F5E4E9B2}"}]"#;

        let records = parse_windows_uninstall_records(output).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].product_code, COMMUNITY_PRODUCT_CODE);
        assert_eq!(
            records[0].install_location,
            r"C:\Program Files\Chat2DB Community"
        );
    }

    #[test]
    fn selects_only_record_with_expected_msi_identity_and_install_path() {
        let records = vec![
            record(
                COMMUNITY_PRODUCT_CODE,
                "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE",
                r"C:\Program Files\Chat2DB Community",
                "",
            ),
            record(
                OTHER_PRODUCT_CODE,
                Edition::Community.windows_upgrade_code(),
                r"C:\Other App",
                "",
            ),
            record(
                COMMUNITY_PRODUCT_CODE,
                Edition::Community.windows_upgrade_code(),
                r"C:\Program Files\Chat2DB Community\",
                "",
            ),
        ];

        let selected = select_windows_install_identity(
            &records,
            Path::new(COMMUNITY_APP_PATH),
            Edition::Community,
        )
        .unwrap();

        assert_eq!(selected.product_code, COMMUNITY_PRODUCT_CODE);
    }

    #[test]
    fn display_icon_can_prove_the_executable_path() {
        let records = vec![record(
            COMMUNITY_PRODUCT_CODE,
            Edition::Community.windows_upgrade_code(),
            "",
            r#""C:\Program Files\Chat2DB Community\Chat2DB Community.exe",0"#,
        )];

        let selected = select_windows_install_identity(
            &records,
            Path::new(COMMUNITY_APP_PATH),
            Edition::Community,
        )
        .unwrap();

        assert_eq!(selected.product_code, COMMUNITY_PRODUCT_CODE);
    }

    #[test]
    fn rejects_ambiguous_matching_product_codes() {
        let records = vec![
            record(
                COMMUNITY_PRODUCT_CODE,
                Edition::Community.windows_upgrade_code(),
                r"C:\Program Files\Chat2DB Community",
                "",
            ),
            record(
                OTHER_PRODUCT_CODE,
                Edition::Community.windows_upgrade_code(),
                r"C:\Program Files\Chat2DB Community",
                "",
            ),
        ];

        let error = select_windows_install_identity(
            &records,
            Path::new(COMMUNITY_APP_PATH),
            Edition::Community,
        )
        .unwrap_err();

        assert!(error.contains("2 distinct uninstall registry records"));
    }

    fn record(
        product_code: &str,
        upgrade_code: &str,
        install_location: &str,
        display_icon: &str,
    ) -> WindowsUninstallRecord {
        WindowsUninstallRecord {
            display_name: Edition::Community.app_name().to_string(),
            install_location: install_location.to_string(),
            display_icon: display_icon.to_string(),
            product_code: product_code.to_string(),
            upgrade_code: upgrade_code.to_string(),
        }
    }
}

#[cfg(test)]
mod app_operation_tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[derive(Debug, Clone, Copy)]
    struct NoHttp;

    impl HttpClient for NoHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            panic!("unexpected HTTP request: {url}")
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct RunningApp;

    impl ProcessProbe for RunningApp {
        fn pids(&self, _app_path: &Path) -> Result<Vec<u32>> {
            Ok(vec![4242])
        }
    }

    fn write_live_runtime_state(paths: &SystemPaths, edition: Edition) {
        let pid = std::process::id();
        let process_identity = crate::platform::process_identity(pid)
            .unwrap()
            .expect("the test process must expose an identity");
        let state = crate::runtime::RuntimeState {
            edition,
            pid,
            process_identity,
            port: 1,
            token: "test-token".to_string(),
            app_path: edition.resolve_app_path_for_os(&paths.applications_dir, HostOs::Macos),
            started_at_unix: 1,
        };
        fs::create_dir_all(&paths.background_state_dir).unwrap();
        fs::write(
            paths
                .background_state_dir
                .join(format!("runtime-{}.json", edition.as_str())),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn desktop_mutation_refuses_a_live_runtime_for_the_same_edition() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let target = Edition::Local.resolve_app_path_for_os(&paths.applications_dir, HostOs::Macos);
        create_test_app(&target, Edition::Local.bundle_id());
        write_live_runtime_state(&paths, Edition::Local);

        let installer = Installer::new(NoHttp, paths);
        let error = installer
            .uninstall_for_os(
                UninstallRequest {
                    edition: Edition::Local,
                },
                HostOs::Macos,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::RuntimeProcessStillRunning { pid } if pid == std::process::id()
        ));
        assert!(target.exists());
    }

    #[test]
    fn desktop_uninstall_refuses_a_live_gui_for_the_same_edition() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let target = Edition::Pro.resolve_app_path_for_os(&paths.applications_dir, HostOs::Macos);
        create_test_app(&target, Edition::Pro.bundle_id());

        let error = Installer::new(NoHttp, paths)
            .uninstall_for_os_with_probe(
                UninstallRequest {
                    edition: Edition::Pro,
                },
                HostOs::Macos,
                &RunningApp,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::AppProcessStillRunning { edition, pids }
                if edition == "pro" && pids == vec![4242]
        ));
        assert!(target.exists());
    }

    #[test]
    fn forced_desktop_install_refuses_a_live_gui_before_download() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let target = Edition::Local.resolve_app_path_for_os(&paths.applications_dir, HostOs::Macos);
        create_test_app(&target, Edition::Local.bundle_id());

        let error = Installer::new(NoHttp, paths)
            .install_for_os_with_probe(
                InstallRequest {
                    edition: Edition::Local,
                    force: true,
                    progress: false,
                },
                HostOs::Macos,
                &RunningApp,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::AppProcessStillRunning { edition, pids }
                if edition == "local" && pids == vec![4242]
        ));
        assert!(target.exists());
    }

    #[test]
    fn install_waits_for_same_edition_operation_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let target = Edition::Pro.resolve_app_path_for_os(&paths.applications_dir, HostOs::Macos);
        create_test_app(&target, Edition::Pro.bundle_id());
        let first = acquire_app_operation_lock(&paths, Edition::Pro).unwrap();
        let thread_paths = paths.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = Installer::new(NoHttp, thread_paths).install_for_os(
                InstallRequest {
                    edition: Edition::Pro,
                    force: false,
                    progress: false,
                },
                HostOs::Macos,
            );
            done_tx.send(result).unwrap();
        });

        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        let error = done_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(matches!(error, Chat2DbError::AppAlreadyExists { .. }));
        handle.join().unwrap();
    }

    #[test]
    fn uninstall_waits_for_same_edition_operation_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let target = Edition::Local.resolve_app_path_for_os(&paths.applications_dir, HostOs::Macos);
        create_test_app(&target, Edition::Local.bundle_id());
        let first = acquire_app_operation_lock(&paths, Edition::Local).unwrap();
        let thread_paths = paths.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = Installer::new(NoHttp, thread_paths).uninstall_for_os(
                UninstallRequest {
                    edition: Edition::Local,
                },
                HostOs::Macos,
            );
            done_tx.send(result).unwrap();
        });

        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        let outcome = done_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(outcome.status, "removed");
        handle.join().unwrap();
    }

    #[test]
    fn different_editions_do_not_share_operation_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let _pro_lock = acquire_app_operation_lock(&paths, Edition::Pro).unwrap();
        let outcome = Installer::new(NoHttp, paths)
            .uninstall_for_os(
                UninstallRequest {
                    edition: Edition::Local,
                },
                HostOs::Macos,
            )
            .unwrap();

        assert_eq!(outcome.status, "already_absent");
    }

    #[test]
    fn failed_macos_promotion_restores_previous_app() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("Chat2DB Pro.app");
        let pending = temp.path().join("Chat2DB Pro.app.pending");
        let backup = temp.path().join("Chat2DB Pro.app.backup");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("version"), "old").unwrap();
        fs::create_dir(&pending).unwrap();
        fs::write(pending.join("version"), "new").unwrap();
        let mut rename_count = 0;

        let error = promote_macos_app_with(&pending, &target, &backup, |source, destination| {
            rename_count += 1;
            if rename_count == 2 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected promotion failure",
                ));
            }
            fs::rename(source, destination)
        })
        .unwrap_err();

        assert!(error.to_string().contains("injected promotion failure"));
        assert_eq!(fs::read_to_string(target.join("version")).unwrap(), "old");
        assert_eq!(fs::read_to_string(pending.join("version")).unwrap(), "new");
        assert!(!backup.exists());
    }

    #[test]
    fn recovery_restores_backup_before_discarding_stale_pending() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("Chat2DB Pro.app");
        let (pending, backup) = macos_transaction_paths(&target);
        create_test_app(&backup, Edition::Pro.bundle_id());
        fs::create_dir(&pending).unwrap();
        fs::write(pending.join("partial"), "incomplete").unwrap();

        recover_macos_app_transaction(&target, Edition::Pro).unwrap();

        verify_app_bundle(&target, Edition::Pro).unwrap();
        assert!(!backup.exists());
        assert!(!pending.exists());
    }

    #[test]
    fn recovery_keeps_promoted_app_and_removes_verified_backup() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("Chat2DB Pro.app");
        let (_, backup) = macos_transaction_paths(&target);
        create_test_app(&target, Edition::Pro.bundle_id());
        create_test_app(&backup, Edition::Pro.bundle_id());

        recover_macos_app_transaction(&target, Edition::Pro).unwrap();

        verify_app_bundle(&target, Edition::Pro).unwrap();
        assert!(!backup.exists());
    }

    fn test_paths(root: &Path) -> SystemPaths {
        let paths = SystemPaths {
            applications_dir: root.join("Applications"),
            home_dir: root.join("home"),
            background_state_dir: root.join("state"),
        };
        fs::create_dir_all(&paths.applications_dir).unwrap();
        fs::create_dir_all(&paths.home_dir).unwrap();
        paths
    }

    fn create_test_app(path: &Path, bundle_id: &str) {
        let contents = path.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(
            contents.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>{bundle_id}</string></dict></plist>"#
            ),
        )
        .unwrap();
    }
}
