use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::{AppVerifier, SystemPaths, verify_app_bundle};
use crate::login::AppOpener;
use crate::platform::{HostOs, current_os, spawn_detached_child};
use crate::runtime::acquire_edition_lifecycle_lock;

/// Launches the app without stealing focus.
///
/// `open -g` keeps the calling Terminal in the foreground, matching CLI
/// ergonomics. `login` deliberately uses the focus-stealing
/// `SystemAppOpener` because logging in requires the GUI to come forward.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackgroundAppOpener;

impl AppOpener for BackgroundAppOpener {
    fn open_app(&self, app_path: &Path) -> Result<()> {
        launch_app(app_path, LaunchStyle::Background)
    }
}

/// Launches the app hidden for CLI side-effect restarts.
///
/// MCP enable/disable/restart needs the app process back so settings take
/// effect, but that should not interrupt the user's current foreground app.
#[derive(Debug, Default, Clone, Copy)]
pub struct HiddenAppOpener;

impl AppOpener for HiddenAppOpener {
    fn open_app(&self, app_path: &Path) -> Result<()> {
        launch_app(app_path, LaunchStyle::Hidden)
    }
}

#[derive(Debug, Clone, Copy)]
enum LaunchStyle {
    Background,
    Hidden,
}

fn launch_app(app_path: &Path, style: LaunchStyle) -> Result<()> {
    match current_os() {
        HostOs::Macos => {
            let mut command = Command::new("open");
            command.arg("-g");
            if matches!(style, LaunchStyle::Hidden) {
                command.arg("-j");
            }
            command.arg(app_path);
            let output = command.output().map_err(|source| Chat2DbError::CommandIo {
                program: "open",
                source,
            })?;

            if output.status.success() {
                Ok(())
            } else {
                Err(Chat2DbError::CommandFailed {
                    program: "open",
                    status: output
                        .status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_string()),
                    stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
                })
            }
        }
        HostOs::Windows => {
            let child = spawn_detached_child(&mut Command::new(app_path), "Chat2DB app")?;
            drop(child);
            Ok(())
        }
        HostOs::Linux | HostOs::Other => Err(Chat2DbError::UnsupportedPlatform),
    }
}

#[derive(Debug, Clone)]
pub struct StartRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl StartRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StartOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub app_path: PathBuf,
}

pub fn start(request: StartRequest) -> Result<StartOutcome> {
    start_with_opener(request, BackgroundAppOpener)
}

pub fn start_with_opener<O: AppOpener>(request: StartRequest, opener: O) -> Result<StartOutcome> {
    start_with_opener_and_verifier(request, opener, verify_app_bundle)
}

fn start_with_opener_and_verifier<O: AppOpener>(
    request: StartRequest,
    opener: O,
    app_verifier: AppVerifier,
) -> Result<StartOutcome> {
    let _lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;
    start_with_opener_locked_and_verifier(request, opener, app_verifier)
}

pub(crate) fn start_with_opener_locked<O: AppOpener>(
    request: StartRequest,
    opener: O,
) -> Result<StartOutcome> {
    start_with_opener_locked_and_verifier(request, opener, verify_app_bundle)
}

pub(crate) fn start_with_opener_locked_and_verifier<O: AppOpener>(
    request: StartRequest,
    opener: O,
    app_verifier: AppVerifier,
) -> Result<StartOutcome> {
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

    opener.open_app(&app_path)?;

    Ok(StartOutcome {
        command: "start",
        edition: request.edition,
        status: "opened",
        app_path,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::test_support::{
        create_installed_app, create_invalid_app, test_paths, verify_app_fixture,
    };

    #[derive(Debug, Default)]
    struct FakeOpener {
        opened: RefCell<Vec<PathBuf>>,
    }

    impl AppOpener for &FakeOpener {
        fn open_app(&self, app_path: &Path) -> Result<()> {
            self.opened.borrow_mut().push(app_path.to_path_buf());
            Ok(())
        }
    }

    #[test]
    fn opens_verified_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Pro);
        let opener = FakeOpener::default();

        let outcome = start_with_opener_and_verifier(
            StartRequest {
                edition: Edition::Pro,
                paths,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.status, "opened");
        assert_eq!(outcome.command, "start");
        assert_eq!(outcome.edition, Edition::Pro);
        assert_eq!(outcome.app_path, app_path);
        assert_eq!(opener.opened.borrow().as_slice(), &[app_path]);
    }

    #[test]
    fn refuses_missing_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let opener = FakeOpener::default();

        let error = start_with_opener_and_verifier(
            StartRequest {
                edition: Edition::Local,
                paths,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::AppNotInstalled { .. }));
        assert!(opener.opened.borrow().is_empty());
    }

    #[test]
    fn refuses_invalid_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_invalid_app(&paths, Edition::Pro);
        let opener = FakeOpener::default();

        let error = start_with_opener_and_verifier(
            StartRequest {
                edition: Edition::Pro,
                paths,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::UnsafeOpen { .. }));
        assert!(opener.opened.borrow().is_empty());
    }

    #[test]
    fn start_waits_for_the_shared_edition_lifecycle_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_installed_app(&paths, Edition::Pro);
        let held_lock = acquire_edition_lifecycle_lock(&paths, Edition::Pro).unwrap();
        let (result_tx, result_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let opener = FakeOpener::default();
            result_tx
                .send(start_with_opener_and_verifier(
                    StartRequest {
                        edition: Edition::Pro,
                        paths,
                    },
                    &opener,
                    verify_app_fixture,
                ))
                .unwrap();
        });

        assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(held_lock);
        assert_eq!(
            result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap()
                .status,
            "opened"
        );
        handle.join().unwrap();
    }
}
