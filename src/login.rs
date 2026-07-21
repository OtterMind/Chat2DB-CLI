use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::{AppVerifier, SystemPaths, verify_app_bundle};
use crate::platform::{HostOs, current_os, spawn_detached_child};
use crate::runtime::acquire_edition_lifecycle_lock;
use crate::status::{LoginState, check_edition_status_with_verifier};

#[derive(Debug, Clone)]
pub struct LoginRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
    pub timeout: Duration,
    pub interval: Duration,
    pub progress: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub app_path: PathBuf,
    pub login_state: LoginState,
    pub waited_seconds: u64,
}

pub trait AppOpener {
    fn open_app(&self, app_path: &Path) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemAppOpener;

impl AppOpener for SystemAppOpener {
    fn open_app(&self, app_path: &Path) -> Result<()> {
        match current_os() {
            HostOs::Macos => {
                let output = Command::new("open")
                    .arg(app_path)
                    .output()
                    .map_err(|source| Chat2DbError::CommandIo {
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
}

pub fn login(request: LoginRequest) -> Result<LoginOutcome> {
    login_with_opener(request, SystemAppOpener)
}

pub fn login_with_opener<O: AppOpener>(request: LoginRequest, opener: O) -> Result<LoginOutcome> {
    login_with_opener_and_verifier(request, opener, verify_app_bundle)
}

fn login_with_opener_and_verifier<O: AppOpener>(
    request: LoginRequest,
    opener: O,
    app_verifier: AppVerifier,
) -> Result<LoginOutcome> {
    let app_path = request.edition.app_path(&request.paths.applications_dir);
    if !request.edition.requires_login() {
        return Ok(LoginOutcome {
            command: "login",
            edition: request.edition,
            status: "not_applicable",
            app_path,
            login_state: LoginState::NotApplicable,
            waited_seconds: 0,
        });
    }

    let lifecycle_lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;

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

    let initial_status =
        check_edition_status_with_verifier(request.edition, &request.paths, app_verifier);
    if initial_status.login.state == LoginState::LoggedIn {
        return Ok(LoginOutcome {
            command: "login",
            edition: request.edition,
            status: "already_logged_in",
            app_path,
            login_state: LoginState::LoggedIn,
            waited_seconds: 0,
        });
    }

    opener.open_app(&app_path)?;
    drop(lifecycle_lock);
    print_progress(
        request.progress,
        &format!(
            "Opened {}. Waiting for login...",
            request.edition.app_name()
        ),
    );

    let started_at = Instant::now();
    let timeout = request.timeout;
    let interval = if request.interval.is_zero() {
        Duration::from_secs(1)
    } else {
        request.interval
    };

    loop {
        let status =
            check_edition_status_with_verifier(request.edition, &request.paths, app_verifier);
        if status.login.state == LoginState::LoggedIn {
            return Ok(LoginOutcome {
                command: "login",
                edition: request.edition,
                status: "logged_in",
                app_path,
                login_state: LoginState::LoggedIn,
                waited_seconds: started_at.elapsed().as_secs(),
            });
        }

        if started_at.elapsed() >= timeout {
            return Err(Chat2DbError::LoginTimedOut {
                edition: request.edition.to_string(),
                seconds: timeout.as_secs(),
            });
        }

        let remaining = timeout.saturating_sub(started_at.elapsed());
        thread::sleep(interval.min(remaining));
    }
}

fn print_progress(enabled: bool, message: &str) {
    if enabled {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

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

    #[derive(Debug)]
    struct LoginWritingOpener {
        opened: RefCell<Vec<PathBuf>>,
        home_dir: PathBuf,
    }

    impl AppOpener for &LoginWritingOpener {
        fn open_app(&self, app_path: &Path) -> Result<()> {
            self.opened.borrow_mut().push(app_path.to_path_buf());
            write_pro_login_cookie(&self.home_dir);
            Ok(())
        }
    }

    #[test]
    fn community_login_is_not_applicable_without_opening_or_requiring_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let expected_app_path = Edition::Community.app_path(&paths.applications_dir);
        let opener = FakeOpener::default();

        let outcome = login_with_opener(
            LoginRequest {
                edition: Edition::Community,
                paths,
                timeout: Duration::ZERO,
                interval: Duration::ZERO,
                progress: false,
            },
            &opener,
        )
        .unwrap();

        assert_eq!(outcome.status, "not_applicable");
        assert_eq!(outcome.login_state, LoginState::NotApplicable);
        assert_eq!(outcome.app_path, expected_app_path);
        assert_eq!(outcome.waited_seconds, 0);
        assert!(opener.opened.borrow().is_empty());
    }

    #[test]
    fn already_logged_in_returns_without_opening_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Pro);
        write_pro_login_cookie(&paths.home_dir);
        let opener = FakeOpener::default();

        let outcome = login_with_opener_and_verifier(
            LoginRequest {
                edition: Edition::Pro,
                paths,
                timeout: Duration::from_secs(1),
                interval: Duration::from_millis(1),
                progress: false,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.status, "already_logged_in");
        assert_eq!(outcome.login_state, LoginState::LoggedIn);
        assert_eq!(outcome.app_path, app_path);
        assert_eq!(outcome.waited_seconds, 0);
        assert!(opener.opened.borrow().is_empty());
    }

    #[test]
    fn pro_oauth_cookies_without_user_id_do_not_short_circuit_login() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Pro);
        write_pro_oauth_cookies_without_user_id(&paths.home_dir);
        let opener = FakeOpener::default();

        let error = login_with_opener_and_verifier(
            LoginRequest {
                edition: Edition::Pro,
                paths,
                timeout: Duration::ZERO,
                interval: Duration::ZERO,
                progress: false,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::LoginTimedOut { .. }));
        assert_eq!(opener.opened.borrow().as_slice(), &[app_path]);
    }

    #[test]
    fn opens_app_and_returns_when_login_marker_appears() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Pro);
        let opener = LoginWritingOpener {
            opened: RefCell::new(Vec::new()),
            home_dir: paths.home_dir.clone(),
        };

        let outcome = login_with_opener_and_verifier(
            LoginRequest {
                edition: Edition::Pro,
                paths,
                timeout: Duration::from_secs(1),
                interval: Duration::from_millis(1),
                progress: false,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.status, "logged_in");
        assert_eq!(outcome.login_state, LoginState::LoggedIn);
        assert_eq!(opener.opened.borrow().as_slice(), &[app_path]);
    }

    #[test]
    fn refuses_missing_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let opener = FakeOpener::default();

        let error = login_with_opener_and_verifier(
            LoginRequest {
                edition: Edition::Local,
                paths,
                timeout: Duration::ZERO,
                interval: Duration::ZERO,
                progress: false,
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

        let error = login_with_opener_and_verifier(
            LoginRequest {
                edition: Edition::Pro,
                paths,
                timeout: Duration::ZERO,
                interval: Duration::ZERO,
                progress: false,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::UnsafeOpen { .. }));
        assert!(opener.opened.borrow().is_empty());
    }

    #[test]
    fn times_out_when_login_state_does_not_change() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Local);
        let opener = FakeOpener::default();

        let error = login_with_opener_and_verifier(
            LoginRequest {
                edition: Edition::Local,
                paths,
                timeout: Duration::ZERO,
                interval: Duration::ZERO,
                progress: false,
            },
            &opener,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::LoginTimedOut { .. }));
        assert_eq!(opener.opened.borrow().as_slice(), &[app_path]);
    }

    fn write_pro_login_cookie(home_dir: &Path) {
        let cookie_file = home_dir.join(".chat2db/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(
            cookie_file,
            r#"{
                "CHAT2DB":{"value":"session"},
                "Chat2db-BS-TOKEN":{"value":"MTIz"},
                "Chat2db-Organization-Token":{"value":"organization-token"},
                "Chat2db-Organization-Id":{"value":"123"}
            }"#,
        )
        .unwrap();
    }

    fn write_pro_oauth_cookies_without_user_id(home_dir: &Path) {
        let cookie_file = home_dir.join(".chat2db/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(
            cookie_file,
            r#"{
                "CHAT2DB":{"value":"session"},
                "Chat2db-Organization-Token":{"value":"organization-token"},
                "Chat2db-Organization-Id":{"value":"123"}
            }"#,
        )
        .unwrap();
    }
}
