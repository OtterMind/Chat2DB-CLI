use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::error::{Chat2DbError, Result};
use crate::platform::{HostOs, current_os};

/// Detects Chat2DB GUI processes by matching the edition-specific desktop
/// app target. macOS matches the `.app/Contents/MacOS/` process path; Windows
/// matches the executable image name from `tasklist`.
pub trait ProcessProbe {
    fn pids(&self, app_path: &Path) -> Result<Vec<u32>>;

    fn is_running(&self, app_path: &Path) -> Result<bool> {
        Ok(!self.pids(app_path)?.is_empty())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PgrepProbe;

impl ProcessProbe for PgrepProbe {
    fn pids(&self, app_path: &Path) -> Result<Vec<u32>> {
        if current_os() == HostOs::Windows {
            return tasklist_pids(app_path);
        }

        let pattern = app_process_pattern(app_path);
        let output = Command::new("pgrep")
            .arg("-f")
            .arg(&pattern)
            .output()
            .map_err(|source| Chat2DbError::CommandIo {
                program: "pgrep",
                source,
            })?;

        match output.status.code() {
            Some(0) => Ok(String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| line.trim().parse::<u32>().ok())
                .collect()),
            Some(1) => Ok(Vec::new()),
            Some(code) => Err(Chat2DbError::CommandFailed {
                program: "pgrep",
                status: code.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
            None => Err(Chat2DbError::CommandFailed {
                program: "pgrep",
                status: "signal".to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }
}

fn tasklist_pids(app_path: &Path) -> Result<Vec<u32>> {
    let image_name = app_path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| Chat2DbError::UnsafeOpen {
            path: app_path.to_path_buf(),
            reason: "app executable has no file name".to_string(),
        })?;
    let filter = format!("IMAGENAME eq {image_name}");
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
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(parse_tasklist_csv(&String::from_utf8_lossy(&output.stdout)))
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessReport {
    pub state: ProcessState,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pids: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Running,
    Stopped,
    Unknown,
}

pub fn process_report<P: ProcessProbe + ?Sized>(probe: &P, app_path: &Path) -> ProcessReport {
    match probe.pids(app_path) {
        Ok(pids) if pids.is_empty() => ProcessReport {
            state: ProcessState::Stopped,
            pids,
            error: None,
        },
        Ok(pids) => ProcessReport {
            state: ProcessState::Running,
            pids,
            error: None,
        },
        Err(error) => ProcessReport {
            state: ProcessState::Unknown,
            pids: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

pub(crate) fn ensure_app_stopped<P: ProcessProbe + ?Sized>(
    probe: &P,
    app_path: &Path,
    edition: crate::edition::Edition,
) -> Result<()> {
    let pids = probe.pids(app_path)?;
    if pids.is_empty() {
        Ok(())
    } else {
        Err(Chat2DbError::AppProcessStillRunning {
            edition: edition.to_string(),
            pids,
        })
    }
}

pub(crate) fn app_process_pattern(app_path: &Path) -> String {
    if app_path.extension() == Some(OsStr::new("exe")) {
        return app_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_string();
    }
    format!("{}/Contents/MacOS/", app_path.display())
}

pub(crate) fn parse_tasklist_csv(stdout: &str) -> Vec<u32> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("INFO:") {
                return None;
            }
            trimmed
                .trim_matches('"')
                .split("\",\"")
                .nth(1)
                .and_then(|pid| pid.parse().ok())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_pattern_uses_executable_name() {
        assert_eq!(
            app_process_pattern(Path::new(
                "C:/Users/tester/AppData/Local/Chat2DB Pro/Chat2DB Pro.exe"
            )),
            "Chat2DB Pro.exe"
        );
    }

    #[test]
    fn parses_tasklist_csv_pids() {
        let stdout = "\"Chat2DB Pro.exe\",\"1234\",\"Console\",\"1\",\"42,000 K\"\r\n\
                      \"Chat2DB Pro.exe\",\"5678\",\"Console\",\"1\",\"30,000 K\"\r\n";
        assert_eq!(parse_tasklist_csv(stdout), vec![1234, 5678]);
        assert!(parse_tasklist_csv("INFO: No tasks are running\n").is_empty());
    }
}
