use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};

use fs2::{FileExt, lock_contended_error};

use crate::error::{Chat2DbError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Macos,
    Windows,
    Linux,
    Other,
}

impl HostOs {
    pub fn from_const(os: &str) -> Self {
        match os {
            "macos" => Self::Macos,
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }

    pub fn is_desktop_supported(self) -> bool {
        matches!(self, Self::Macos | Self::Windows)
    }

    pub fn cli_manifest_label(self) -> Result<&'static str> {
        match self {
            Self::Macos => Ok("darwin"),
            Self::Windows => Ok("windows"),
            Self::Linux => Ok("linux"),
            Self::Other => Err(Chat2DbError::UnsupportedPlatform),
        }
    }
}

pub fn current_os() -> HostOs {
    HostOs::from_const(std::env::consts::OS)
}

pub fn ensure_macos() -> Result<()> {
    if current_os() == HostOs::Macos {
        Ok(())
    } else {
        Err(Chat2DbError::UnsupportedPlatform)
    }
}

pub fn ensure_desktop_supported() -> Result<()> {
    if current_os().is_desktop_supported() {
        Ok(())
    } else {
        Err(Chat2DbError::UnsupportedPlatform)
    }
}

/// Prevents long-lived Windows workers from inheriting the caller's console.
/// Stdio still needs to be redirected by the caller before spawning.
#[cfg(windows)]
pub(crate) fn configure_detached_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(detached_creation_flags());
}

#[cfg(not(windows))]
pub(crate) fn configure_detached_command(_command: &mut Command) {}

#[cfg(any(windows, test))]
const fn detached_creation_flags() -> u32 {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
}

pub(crate) fn spawn_detached_child(command: &mut Command, program: &'static str) -> Result<Child> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    configure_detached_command(command);
    command
        .spawn()
        .map_err(|source| Chat2DbError::CommandIo { program, source })
}

pub fn current_cli_os_label() -> Result<&'static str> {
    current_os().cli_manifest_label()
}

pub fn current_arch_label() -> Result<&'static str> {
    arch_label(std::env::consts::ARCH)
}

pub fn arch_label(arch: &str) -> Result<&'static str> {
    match arch {
        "aarch64" | "arm64" => Ok("arm64"),
        "x86_64" | "amd64" => Ok("x86_64"),
        other => Err(Chat2DbError::UnsupportedArch(other.to_string())),
    }
}

pub fn home_dir() -> std::path::PathBuf {
    std::env::var_os(if current_os() == HostOs::Windows {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .or_else(|| std::env::var_os("HOME"))
    .map(std::path::PathBuf::from)
    .unwrap_or_else(|| std::path::PathBuf::from("."))
}

pub fn windows_local_app_data_dir(home_dir: &std::path::Path) -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home_dir.join("AppData").join("Local"))
}

pub fn pid_alive(pid: u32) -> bool {
    process_exists(pid).unwrap_or(false)
}

pub fn process_exists(pid: u32) -> Result<bool> {
    process_exists_impl(pid)
}

pub fn terminate_process(pid: u32) -> Result<()> {
    terminate_process_impl(pid)
}

pub fn process_identity(pid: u32) -> Result<Option<String>> {
    process_identity_impl(pid)
}

pub struct ExclusiveLock {
    file: File,
}

impl Drop for ExclusiveLock {
    fn drop(&mut self) {
        // Explicitly unlock before close so a concurrently forked child cannot
        // keep an inherited file description locked until exec.
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn acquire_exclusive_lock(path: &Path) -> Result<ExclusiveLock> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = open_private_file(path)?;
    FileExt::lock_exclusive(&file)?;
    Ok(ExclusiveLock { file })
}

pub fn try_acquire_exclusive_lock(path: &Path) -> Result<Option<ExclusiveLock>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = open_private_file(path)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(ExclusiveLock { file })),
        Err(error) if is_lock_contended(&error) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn is_lock_contended(error: &std::io::Error) -> bool {
    let expected_raw_error = lock_contended_error().raw_os_error();
    error.kind() == std::io::ErrorKind::WouldBlock
        || expected_raw_error.is_some() && error.raw_os_error() == expected_raw_error
}

pub fn sync_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    let file = fs::OpenOptions::new().read(true).write(true).open(path)?;
    #[cfg(not(windows))]
    let file = File::open(path)?;

    file.sync_all()?;
    Ok(())
}

pub fn write_private_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic file target must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    set_private_permissions(temporary.path())?;
    temporary.write_all(contents)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    set_private_permissions(path)?;

    #[cfg(unix)]
    File::open(parent)?.sync_all()?;

    Ok(())
}

#[cfg(unix)]
fn open_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    set_private_permissions(path)?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_private_file(path: &Path) -> std::io::Result<File> {
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn process_exists_impl(pid: u32) -> Result<bool> {
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
fn process_identity_impl(pid: u32) -> Result<Option<String>> {
    let stat_path = format!("/proc/{pid}/stat");
    let stat = match fs::read_to_string(&stat_path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let Some(command_end) = stat.rfind(')') else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid process stat at {stat_path}"),
        )
        .into());
    };
    let start_time = stat[command_end + 1..]
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("process stat at {stat_path} has no start time"),
            )
        })?;
    let executable = match fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(format!("linux:{start_time}:{}", executable.display())))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn process_identity_impl(pid: u32) -> Result<Option<String>> {
    let output = Command::new("ps")
        .args([
            "-ww",
            "-p",
            &pid.to_string(),
            "-o",
            "lstart=",
            "-o",
            "command=",
        ])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "ps",
            source,
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!identity.is_empty()).then_some(format!("unix:{identity}")))
}

#[cfg(windows)]
fn process_exists_impl(pid: u32) -> Result<bool> {
    let filter = format!("PID eq {pid}");
    let output = Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "tasklist",
            source,
        })?;
    if !output.status.success() {
        return Err(Chat2DbError::CommandFailed {
            program: "tasklist",
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| {
        parse_tasklist_pid(line)
            .map(|task_pid| task_pid == pid)
            .unwrap_or(false)
    }))
}

#[cfg(windows)]
fn process_identity_impl(pid: u32) -> Result<Option<String>> {
    let script = format!(
        "$p = Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}'; if ($null -ne $p) {{ $p | Select-Object CreationDate,ExecutablePath | ConvertTo-Json -Compress }}"
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "powershell",
            source,
        })?;
    if !output.status.success() {
        return Err(Chat2DbError::CommandFailed {
            program: "powershell",
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    let identity = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!identity.is_empty()).then_some(format!("windows:{identity}")))
}

#[cfg(not(any(unix, windows)))]
fn process_exists_impl(_pid: u32) -> Result<bool> {
    Ok(false)
}

#[cfg(not(any(unix, windows)))]
fn process_identity_impl(_pid: u32) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(unix)]
fn terminate_process_impl(pid: u32) -> Result<()> {
    let status = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if status == 0 {
        Ok(())
    } else {
        Err(Chat2DbError::CommandFailed {
            program: "kill",
            status: "failed".to_string(),
            stderr: std::io::Error::last_os_error().to_string(),
        })
    }
}

#[cfg(windows)]
fn terminate_process_impl(pid: u32) -> Result<()> {
    let pid = pid.to_string();
    let output = std::process::Command::new("taskkill")
        .args(["/PID", &pid, "/T"])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "taskkill",
            source,
        })?;
    if output.status.success() {
        return Ok(());
    }

    let output = std::process::Command::new("taskkill")
        .args(["/PID", &pid, "/T", "/F"])
        .output()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "taskkill",
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Chat2DbError::CommandFailed {
            program: "taskkill",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_impl(_pid: u32) -> Result<()> {
    Err(Chat2DbError::UnsupportedPlatform)
}

#[cfg(windows)]
fn parse_tasklist_pid(line: &str) -> Option<u32> {
    let mut fields = line.trim().split("\",\"");
    let _image = fields.next()?;
    fields.next()?.trim_matches('"').parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_detached_process_flags_do_not_create_or_inherit_a_console() {
        assert_eq!(detached_creation_flags(), 0x0000_0208);
    }

    #[test]
    fn maps_host_os_labels() {
        assert_eq!(HostOs::from_const("macos"), HostOs::Macos);
        assert_eq!(HostOs::from_const("windows"), HostOs::Windows);
        assert_eq!(HostOs::from_const("linux"), HostOs::Linux);
        assert_eq!(HostOs::Macos.cli_manifest_label().unwrap(), "darwin");
        assert_eq!(HostOs::Windows.cli_manifest_label().unwrap(), "windows");
        assert_eq!(HostOs::Linux.cli_manifest_label().unwrap(), "linux");
        assert!(HostOs::Macos.is_desktop_supported());
        assert!(HostOs::Windows.is_desktop_supported());
        assert!(!HostOs::Linux.is_desktop_supported());
    }

    #[test]
    fn maps_macos_arch_labels() {
        assert_eq!(arch_label("aarch64").unwrap(), "arm64");
        assert_eq!(arch_label("arm64").unwrap(), "arm64");
        assert_eq!(arch_label("x86_64").unwrap(), "x86_64");
        assert_eq!(arch_label("amd64").unwrap(), "x86_64");
    }

    #[test]
    fn atomic_private_write_replaces_complete_contents() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");

        write_private_atomic(&path, b"first\n").unwrap();
        write_private_atomic(&path, b"second\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second\n");
    }

    #[test]
    fn sync_file_accepts_a_writable_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("staged.bin");
        fs::write(&path, b"staged").unwrap();

        sync_file(&path).unwrap();
    }

    #[test]
    fn nonblocking_lock_reports_contention() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("operation.lock");
        let first = acquire_exclusive_lock(&path).unwrap();

        assert!(try_acquire_exclusive_lock(&path).unwrap().is_none());
        drop(first);
        assert!(try_acquire_exclusive_lock(&path).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_private_write_uses_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");

        write_private_atomic(&path, b"{}\n").unwrap();

        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_identity_is_stable_for_the_current_process() {
        let first = process_identity(std::process::id()).unwrap().unwrap();
        let second = process_identity(std::process::id()).unwrap().unwrap();

        assert_eq!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn process_existence_distinguishes_live_and_missing_pids() {
        assert!(process_exists(std::process::id()).unwrap());
        assert!(!process_exists(99_999_999).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn process_identity_distinguishes_processes() {
        let current = process_identity(std::process::id()).unwrap().unwrap();
        let mut child = Command::new("sleep").arg("5").spawn().unwrap();
        let child_identity = process_identity(child.id()).unwrap().unwrap();

        child.kill().unwrap();
        child.wait().unwrap();

        assert_ne!(current, child_identity);
    }
}
