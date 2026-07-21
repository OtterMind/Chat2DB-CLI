use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::{AppVerifier, SystemPaths, verify_app_bundle};
use crate::platform::{HostOs, current_os};
use crate::process::{PgrepProbe, ProcessProbe};
use crate::runtime::acquire_edition_lifecycle_lock;

#[derive(Debug, Clone)]
pub struct StopRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl StopRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StopOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub app_path: PathBuf,
}

/// Sends a graceful quit to the running Chat2DB app.
///
/// On macOS this triggers the app's registered `Desktop.APP_QUIT_HANDLER`,
/// which disposes JCEF and the JFrame before exiting (see Chat2DB-Enterprise
/// `MainJFrame.java` setupDesktopIntegration). Force-killing instead would
/// leave jcef Helper subprocesses orphaned and may corrupt the CEF cache.
pub trait AppQuitter {
    fn quit_app(&self, app_name: &str) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsascriptQuitter;

impl AppQuitter for OsascriptQuitter {
    fn quit_app(&self, app_name: &str) -> Result<()> {
        if current_os() == HostOs::Windows {
            return windows_taskkill_app(app_name);
        }

        let script = format!(r#"tell application "{app_name}" to quit"#);
        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|source| Chat2DbError::CommandIo {
                program: "osascript",
                source,
            })?;

        if output.status.success() {
            return Ok(());
        }

        Err(Chat2DbError::CommandFailed {
            program: "osascript",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn windows_taskkill_app(app_name: &str) -> Result<()> {
    let mut last_error = None;
    for image_name in windows_image_names(app_name) {
        let output = Command::new("taskkill")
            .args(["/IM", &image_name, "/T"])
            .output()
            .map_err(|source| Chat2DbError::CommandIo {
                program: "taskkill",
                source,
            })?;

        if output.status.success() {
            return Ok(());
        }
        last_error = Some(Chat2DbError::CommandFailed {
            program: "taskkill",
            status: output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Err(last_error.unwrap_or(Chat2DbError::UnsupportedPlatform))
}

fn windows_image_names(app_name: &str) -> Vec<String> {
    let mut names = vec![format!("{app_name}.exe")];
    let dashed = format!("{}.exe", app_name.replace(' ', "-"));
    if !names.contains(&dashed) {
        names.push(dashed);
    }
    names
}

pub fn stop(request: StopRequest) -> Result<StopOutcome> {
    stop_with(request, PgrepProbe, OsascriptQuitter)
}

pub fn stop_with<P: ProcessProbe, Q: AppQuitter>(
    request: StopRequest,
    probe: P,
    quitter: Q,
) -> Result<StopOutcome> {
    stop_with_and_verifier(request, probe, quitter, verify_app_bundle)
}

fn stop_with_and_verifier<P: ProcessProbe, Q: AppQuitter>(
    request: StopRequest,
    probe: P,
    quitter: Q,
    app_verifier: AppVerifier,
) -> Result<StopOutcome> {
    let _lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;
    stop_with_locked_and_verifier(request, probe, quitter, app_verifier)
}

pub(crate) fn stop_with_locked<P: ProcessProbe, Q: AppQuitter>(
    request: StopRequest,
    probe: P,
    quitter: Q,
) -> Result<StopOutcome> {
    stop_with_locked_and_verifier(request, probe, quitter, verify_app_bundle)
}

pub(crate) fn stop_with_locked_and_verifier<P: ProcessProbe, Q: AppQuitter>(
    request: StopRequest,
    probe: P,
    quitter: Q,
    app_verifier: AppVerifier,
) -> Result<StopOutcome> {
    let app_path = request.edition.app_path(&request.paths.applications_dir);
    if !app_path.exists() {
        return Err(Chat2DbError::AppNotInstalled {
            edition: request.edition.to_string(),
            path: app_path,
        });
    }

    app_verifier(&app_path, request.edition).map_err(|error| Chat2DbError::UnsafeOpen {
        path: app_path.clone(),
        reason: error.to_string(),
    })?;

    // Avoid AppleScript's auto-launch quirk: `tell application "X" to quit`
    // launches the app first if it isn't running, then quits it. Skip the
    // quit entirely when no process matches the bundle path.
    if !probe.is_running(&app_path)? {
        return Ok(StopOutcome {
            command: "stop",
            edition: request.edition,
            status: "already_stopped",
            app_path,
        });
    }

    quitter.quit_app(request.edition.app_name())?;

    Ok(StopOutcome {
        command: "stop",
        edition: request.edition,
        status: "stopped",
        app_path,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::Path;

    use super::*;
    use crate::test_support::{
        create_installed_app, create_invalid_app, test_paths, verify_app_fixture,
    };

    #[derive(Debug)]
    struct FakeProbe {
        running: Cell<bool>,
        calls: Cell<usize>,
    }

    impl FakeProbe {
        fn new(running: bool) -> Self {
            Self {
                running: Cell::new(running),
                calls: Cell::new(0),
            }
        }
    }

    impl ProcessProbe for &FakeProbe {
        fn pids(&self, _app_path: &Path) -> Result<Vec<u32>> {
            self.calls.set(self.calls.get() + 1);
            Ok(if self.running.get() {
                vec![4242]
            } else {
                vec![]
            })
        }
    }

    #[derive(Debug, Default)]
    struct FakeQuitter {
        quit_calls: RefCell<Vec<String>>,
    }

    impl AppQuitter for &FakeQuitter {
        fn quit_app(&self, app_name: &str) -> Result<()> {
            self.quit_calls.borrow_mut().push(app_name.to_string());
            Ok(())
        }
    }

    #[test]
    fn already_stopped_when_process_absent_does_not_call_quitter() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_installed_app(&paths, Edition::Pro);
        let probe = FakeProbe::new(false);
        let quitter = FakeQuitter::default();

        let outcome = stop_with_and_verifier(
            StopRequest {
                edition: Edition::Pro,
                paths,
            },
            &probe,
            &quitter,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.status, "already_stopped");
        assert_eq!(probe.calls.get(), 1);
        assert!(quitter.quit_calls.borrow().is_empty());
    }

    #[test]
    fn stops_running_app_with_apple_script() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_installed_app(&paths, Edition::Local);
        let probe = FakeProbe::new(true);
        let quitter = FakeQuitter::default();

        let outcome = stop_with_and_verifier(
            StopRequest {
                edition: Edition::Local,
                paths,
            },
            &probe,
            &quitter,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.status, "stopped");
        assert_eq!(outcome.edition, Edition::Local);
        assert_eq!(quitter.quit_calls.borrow().as_slice(), &["Chat2DB Local"]);
    }

    #[test]
    fn refuses_missing_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let probe = FakeProbe::new(true);
        let quitter = FakeQuitter::default();

        let error = stop_with_and_verifier(
            StopRequest {
                edition: Edition::Pro,
                paths,
            },
            &probe,
            &quitter,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::AppNotInstalled { .. }));
        assert_eq!(probe.calls.get(), 0);
        assert!(quitter.quit_calls.borrow().is_empty());
    }

    #[test]
    fn refuses_invalid_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_invalid_app(&paths, Edition::Pro);
        let probe = FakeProbe::new(true);
        let quitter = FakeQuitter::default();

        let error = stop_with_and_verifier(
            StopRequest {
                edition: Edition::Pro,
                paths,
            },
            &probe,
            &quitter,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::UnsafeOpen { .. }));
        assert_eq!(probe.calls.get(), 0);
        assert!(quitter.quit_calls.borrow().is_empty());
    }
}
