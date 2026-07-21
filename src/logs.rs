use std::fs;
use std::path::PathBuf;
use std::process::Command;

use clap::ValueEnum;
use serde::Serialize;

use crate::background::log_path_in;
use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::SystemPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LogsKind {
    /// Background install log written by `chat2db install --background`.
    Install,
}

impl LogsKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LogsKind::Install => "install",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogsRequest {
    pub kind: LogsKind,
    pub edition: Edition,
    pub follow: bool,
    pub paths: SystemPaths,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsOutcome {
    pub command: &'static str,
    pub kind: LogsKind,
    pub edition: Edition,
    pub log_path: PathBuf,
    pub size_bytes: u64,
    /// Suppressed from JSON: emitted as raw bytes by the printer instead.
    #[serde(skip_serializing)]
    pub contents: Vec<u8>,
}

pub fn logs(request: LogsRequest) -> Result<LogsOutcome> {
    let log_path = match request.kind {
        LogsKind::Install => log_path_in(&request.paths.background_state_dir, request.edition),
    };

    if !log_path.exists() {
        return Err(Chat2DbError::LogNotFound {
            kind: request.kind.as_str(),
            edition: request.edition.to_string(),
            path: log_path,
        });
    }

    if request.follow {
        return follow_log(request.kind, request.edition, &log_path);
    }

    let contents = fs::read(&log_path)?;
    let size_bytes = contents.len() as u64;

    Ok(LogsOutcome {
        command: "logs",
        kind: request.kind,
        edition: request.edition,
        log_path,
        size_bytes,
        contents,
    })
}

#[cfg(unix)]
fn follow_log(
    _kind: LogsKind,
    _edition: Edition,
    log_path: &std::path::Path,
) -> Result<LogsOutcome> {
    use std::os::unix::process::CommandExt;
    let err = Command::new("tail")
        .args(["-n", "+1", "-f"])
        .arg(log_path)
        .exec();
    Err(Chat2DbError::CommandIo {
        program: "tail",
        source: err,
    })
}

#[cfg(windows)]
fn follow_log(kind: LogsKind, edition: Edition, log_path: &std::path::Path) -> Result<LogsOutcome> {
    let script = format!(
        "Get-Content -LiteralPath '{}' -Wait",
        log_path.display().to_string().replace('\'', "''")
    );
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|source| Chat2DbError::CommandIo {
            program: "powershell.exe",
            source,
        })?;
    if status.success() {
        Ok(LogsOutcome {
            command: "logs",
            kind,
            edition,
            log_path: log_path.to_path_buf(),
            size_bytes: 0,
            contents: Vec::new(),
        })
    } else {
        Err(Chat2DbError::CommandFailed {
            program: "powershell.exe",
            status: status.to_string(),
            stderr: String::new(),
        })
    }
}

#[cfg(not(any(unix, windows)))]
fn follow_log(
    _kind: LogsKind,
    _edition: Edition,
    _log_path: &std::path::Path,
) -> Result<LogsOutcome> {
    Err(Chat2DbError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn reads_log_when_present() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(&paths.background_state_dir).unwrap();
        let log = paths.background_state_dir.join("install-pro.log");
        fs::write(&log, b"hello world\n").unwrap();

        let outcome = logs(LogsRequest {
            kind: LogsKind::Install,
            edition: Edition::Pro,
            follow: false,
            paths,
        })
        .unwrap();

        assert_eq!(outcome.log_path, log);
        assert_eq!(outcome.contents, b"hello world\n");
        assert_eq!(outcome.size_bytes, 12);
        assert_eq!(outcome.command, "logs");
    }

    #[test]
    fn errors_when_log_missing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let error = logs(LogsRequest {
            kind: LogsKind::Install,
            edition: Edition::Local,
            follow: false,
            paths,
        })
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::LogNotFound { .. }));
    }

    fn test_paths(root: &Path) -> SystemPaths {
        let applications_dir = root.join("Applications");
        let home_dir = root.join("home");
        fs::create_dir_all(&applications_dir).unwrap();
        fs::create_dir_all(&home_dir).unwrap();
        SystemPaths {
            applications_dir,
            home_dir,
            background_state_dir: root.join("state"),
        }
    }
}
