//! Background install daemon helpers.
//!
//! `chat2db install --background --edition <e>` self-respawns a detached
//! child running `chat2db install --edition <e>` with stdio redirected to a
//! log file. The parent returns immediately. State is persisted under
//! `$XDG_STATE_HOME/chat2db/` (default: `~/.local/state/chat2db/`):
//!
//! - `install-<edition>.log`  — child stdout+stderr
//! - `install-<edition>.pid`  — child PID and process identity while running
//! - `install-<edition>.done` — exit code, written by the child on exit
//!
//! `.done` always wins over `.pid` when reading state — a finished install
//! supersedes a stale pid file.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
#[cfg(windows)]
use crate::platform::configure_detached_command;
use crate::platform::{
    ExclusiveLock, HostOs, acquire_exclusive_lock, current_os, home_dir, process_identity,
    write_private_atomic,
};

/// Set by the parent to mark the child as the install worker. Value is the
/// edition name. Presence triggers `mark_done_if_child` on exit.
const ENV_EDITION: &str = "CHAT2DB_BACKGROUND_EDITION";

pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(dir).join("chat2db");
    }
    let home = home_dir();
    match current_os() {
        HostOs::Windows => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"))
            .join("chat2db-cli")
            .join("state"),
        _ => home.join(".local/state/chat2db"),
    }
}

pub fn log_path(edition: Edition) -> PathBuf {
    log_path_in(&state_dir(), edition)
}

pub fn pid_path(edition: Edition) -> PathBuf {
    pid_path_in(&state_dir(), edition)
}

pub fn done_path(edition: Edition) -> PathBuf {
    done_path_in(&state_dir(), edition)
}

pub fn log_path_in(state_dir: &Path, edition: Edition) -> PathBuf {
    state_dir.join(format!("install-{}.log", edition.as_str()))
}

pub fn pid_path_in(state_dir: &Path, edition: Edition) -> PathBuf {
    state_dir.join(format!("install-{}.pid", edition.as_str()))
}

pub fn done_path_in(state_dir: &Path, edition: Edition) -> PathBuf {
    state_dir.join(format!("install-{}.done", edition.as_str()))
}

fn state_lock_path_in(state_dir: &Path, edition: Edition) -> PathBuf {
    state_dir.join(format!("background-install-{}.lock", edition.as_str()))
}

// This lock protects only background state and spawn deduplication. The child
// acquires the app operation lock inside Installer after the parent releases it.
fn acquire_state_lock(state_dir: &Path, edition: Edition) -> Result<ExclusiveLock> {
    acquire_exclusive_lock(&state_lock_path_in(state_dir, edition))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum BackgroundState {
    NotRunning,
    Running { pid: u32 },
    Completed { exit_code: i32 },
    Unknown { error: String },
}

#[derive(Debug, Deserialize, Serialize)]
struct BackgroundProcessState {
    pid: u32,
    process_identity: String,
}

pub fn read_state(edition: Edition) -> BackgroundState {
    read_state_in(&state_dir(), edition)
}

pub fn read_state_in(state_dir: &Path, edition: Edition) -> BackgroundState {
    read_state_in_with(state_dir, edition, process_identity).unwrap_or_else(|error| {
        BackgroundState::Unknown {
            error: error.to_string(),
        }
    })
}

fn read_state_in_with<F>(state_dir: &Path, edition: Edition, identity: F) -> Result<BackgroundState>
where
    F: Fn(u32) -> Result<Option<String>>,
{
    match fs::read_to_string(done_path_in(state_dir, edition)) {
        Ok(text) => {
            let exit_code = text.trim().parse().unwrap_or(-1);
            return Ok(BackgroundState::Completed { exit_code });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let text = match fs::read_to_string(pid_path_in(state_dir, edition)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BackgroundState::NotRunning);
        }
        Err(error) => return Err(error.into()),
    };
    let process = match serde_json::from_str::<BackgroundProcessState>(&text) {
        Ok(process) => process,
        Err(_) if text.trim().bytes().all(|byte| byte.is_ascii_digit()) => {
            return Ok(BackgroundState::NotRunning);
        }
        Err(error) => return Err(error.into()),
    };
    if process.pid == 0 || process.process_identity.is_empty() {
        return Ok(BackgroundState::NotRunning);
    }
    if identity(process.pid)?.as_deref() == Some(process.process_identity.as_str()) {
        Ok(BackgroundState::Running { pid: process.pid })
    } else {
        Ok(BackgroundState::NotRunning)
    }
}

/// Called from `main` after `run` finishes. If the env marker is set, write
/// the exit code to `.done` and remove the `.pid` file. Best-effort: errors
/// are swallowed because we are about to exit anyway.
pub fn mark_done_if_child(exit_code: i32) {
    let Some(value) = std::env::var_os(ENV_EDITION) else {
        return;
    };
    let Some(name) = value.to_str() else {
        return;
    };
    let edition = match name {
        "community" => Edition::Community,
        "pro" => Edition::Pro,
        "local" => Edition::Local,
        _ => return,
    };

    let dir = state_dir();
    let Ok(_lock) = acquire_state_lock(&dir, edition) else {
        return;
    };
    let _ = write_private_atomic(
        &done_path_in(&dir, edition),
        format!("{exit_code}\n").as_bytes(),
    );
    let _ = fs::remove_file(pid_path_in(&dir, edition));
}

pub fn is_child() -> bool {
    std::env::var_os(ENV_EDITION).is_some()
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundStartOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub pid: u32,
    pub log_path: PathBuf,
}

pub fn spawn_install(
    edition: Edition,
    force: bool,
    yes: bool,
    json: bool,
) -> Result<BackgroundStartOutcome> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let _lock = acquire_state_lock(&dir, edition)?;

    if let BackgroundState::Running { pid } = read_state_in_with(&dir, edition, process_identity)? {
        return Err(Chat2DbError::BackgroundInstallAlreadyRunning {
            edition: edition.to_string(),
            pid,
        });
    }

    // Clear stale markers from a prior completed run; the log is truncated
    // below so users don't see two interleaved runs.
    let _ = fs::remove_file(done_path_in(&dir, edition));
    let _ = fs::remove_file(pid_path_in(&dir, edition));

    let log = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path_in(&dir, edition))?;

    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(&exe);
    cmd.arg("install").arg("--edition").arg(edition.as_str());
    if force {
        cmd.arg("--force");
    }
    if yes {
        cmd.arg("--yes");
    }
    if json {
        cmd.arg("--json");
    }
    cmd.env(ENV_EDITION, edition.as_str());
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(log.try_clone()?));
    cmd.stderr(Stdio::from(log));

    detach_background_child(&mut cmd);

    let mut child = cmd.spawn().map_err(|source| Chat2DbError::CommandIo {
        program: "chat2db install (background)",
        source,
    })?;

    let pid = child.id();
    let identity = match process_identity(pid) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            cleanup_untracked_child(&mut child)?;
            return Err(std::io::Error::other(format!(
                "background install process {pid} exited before its identity could be recorded"
            ))
            .into());
        }
        Err(error) => {
            cleanup_untracked_child(&mut child)?;
            return Err(error);
        }
    };
    let process = BackgroundProcessState {
        pid,
        process_identity: identity,
    };
    let mut process_json = serde_json::to_vec(&process)?;
    process_json.push(b'\n');
    if let Err(error) = write_private_atomic(&pid_path_in(&dir, edition), &process_json) {
        cleanup_untracked_child(&mut child)?;
        return Err(error);
    }
    drop(child);

    Ok(BackgroundStartOutcome {
        command: "install",
        edition,
        status: "started",
        pid,
        log_path: log_path_in(&dir, edition),
    })
}

fn cleanup_untracked_child(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let _ = child.wait()?;
    Ok(())
}

#[cfg(unix)]
fn detach_background_child(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn detach_background_child(cmd: &mut Command) {
    configure_detached_command(cmd);
}

#[cfg(not(any(unix, windows)))]
fn detach_background_child(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn read_state_returns_completed_when_done_file_exists() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        fs::create_dir_all(&dir).unwrap();

        fs::write(done_path_in(&dir, Edition::Pro), "0\n").unwrap();

        let state = read_state_in(&dir, Edition::Pro);
        assert_eq!(state, BackgroundState::Completed { exit_code: 0 });
    }

    #[test]
    fn read_state_with_dead_pid_returns_not_running() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        fs::create_dir_all(&dir).unwrap();

        // 99999999 is well above macOS's default pid_max so kill(pid, 0)
        // returns ESRCH. (pid 0 has special "process group" semantics — it
        // is not "no such process".)
        fs::write(
            pid_path_in(&dir, Edition::Local),
            serde_json::to_vec(&BackgroundProcessState {
                pid: 99_999_999,
                process_identity: "missing-process".to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        let state = read_state_in(&dir, Edition::Local);
        assert_eq!(state, BackgroundState::NotRunning);
    }

    #[test]
    fn read_state_requires_the_recorded_process_identity() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        fs::create_dir_all(&dir).unwrap();
        let pid = std::process::id();
        fs::write(
            pid_path_in(&dir, Edition::Community),
            serde_json::to_vec(&BackgroundProcessState {
                pid,
                process_identity: "reused-pid".to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            read_state_in(&dir, Edition::Community),
            BackgroundState::NotRunning
        );
    }

    #[test]
    fn read_state_accepts_a_matching_live_process_identity() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        fs::create_dir_all(&dir).unwrap();
        let pid = std::process::id();
        let identity = process_identity(pid).unwrap().unwrap();
        fs::write(
            pid_path_in(&dir, Edition::Pro),
            serde_json::to_vec(&BackgroundProcessState {
                pid,
                process_identity: identity,
            })
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            read_state_in(&dir, Edition::Pro),
            BackgroundState::Running { pid }
        );
    }

    #[test]
    fn legacy_pid_only_state_is_treated_as_stale() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            pid_path_in(&dir, Edition::Local),
            std::process::id().to_string(),
        )
        .unwrap();

        assert_eq!(
            read_state_in(&dir, Edition::Local),
            BackgroundState::NotRunning
        );
    }

    #[test]
    fn process_identity_probe_failure_is_not_treated_as_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            pid_path_in(&dir, Edition::Pro),
            serde_json::to_vec(&BackgroundProcessState {
                pid: 4242,
                process_identity: "expected".to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        let error = read_state_in_with(&dir, Edition::Pro, |_| {
            Err(std::io::Error::other("identity probe failed").into())
        })
        .unwrap_err();

        assert!(error.to_string().contains("identity probe failed"));
    }

    #[test]
    fn background_state_lock_serializes_same_edition() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("chat2db");
        let first = acquire_state_lock(&dir, Edition::Community).unwrap();
        let thread_dir = dir.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let second = acquire_state_lock(&thread_dir, Edition::Community).unwrap();
            acquired_tx.send(()).unwrap();
            drop(second);
        });

        started_rx.recv().unwrap();
        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }
}
