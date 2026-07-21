use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::SystemPaths;
use crate::platform::{HostOs, current_os, process_exists};
use crate::process::parse_tasklist_csv;
use crate::runtime::acquire_edition_lifecycle_lock;
use crate::status::{
    CHAT2DB_COOKIE, LoginState, ORGANIZATION_ID_COOKIE, ORGANIZATION_TOKEN_COOKIE, USER_ID_COOKIE,
    check_edition_status, login_cookie_file_path,
};

const PRO_LOGOUT_COOKIES: &[&str] = &[
    USER_ID_COOKIE,
    CHAT2DB_COOKIE,
    ORGANIZATION_TOKEN_COOKIE,
    ORGANIZATION_ID_COOKIE,
];
const LOCAL_LOGOUT_COOKIES: &[&str] = &[USER_ID_COOKIE];

#[derive(Debug, Clone)]
pub struct LogoutRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogoutOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: &'static str,
    pub cookie_path: PathBuf,
    pub login_state: LoginState,
}

pub trait AppQuitter {
    fn quit_app(&self, edition: Edition) -> Result<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemAppQuitter;

impl AppQuitter for SystemAppQuitter {
    fn quit_app(&self, edition: Edition) -> Result<()> {
        if current_os() == HostOs::Windows {
            return windows_taskkill_app(edition.app_name());
        }

        let script = format!(
            r#"
try
    set isRunning to application id "{bundle_id}" is running
on error
    return
end try
if isRunning then
    tell application id "{bundle_id}" to quit
    repeat 25 times
        if not application id "{bundle_id}" is running then return
        delay 0.2
    end repeat
    error "application did not quit"
end if
"#,
            bundle_id = edition.bundle_id()
        );
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|source| Chat2DbError::CommandIo {
                program: "osascript",
                source,
            })?;

        if output.status.success() {
            Ok(())
        } else {
            Err(Chat2DbError::CommandFailed {
                program: "osascript",
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }
}

fn windows_taskkill_app(app_name: &str) -> Result<()> {
    windows_taskkill_app_with(app_name, &SystemWindowsAppProcess)
}

trait WindowsAppProcess {
    fn pids_for_image(&self, image_name: &str) -> Result<Vec<u32>>;
    fn terminate(&self, pid: u32) -> Result<()>;
    fn is_alive(&self, pid: u32) -> Result<bool>;
}

struct SystemWindowsAppProcess;

impl WindowsAppProcess for SystemWindowsAppProcess {
    fn pids_for_image(&self, image_name: &str) -> Result<Vec<u32>> {
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
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(parse_tasklist_csv(&String::from_utf8_lossy(&output.stdout)))
    }

    fn terminate(&self, pid: u32) -> Result<()> {
        let pid = pid.to_string();
        let output = Command::new("taskkill")
            .args(["/PID", &pid, "/T"])
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
                status: output.status.to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        process_exists(pid)
    }
}

fn windows_taskkill_app_with<P: WindowsAppProcess + ?Sized>(
    app_name: &str,
    process: &P,
) -> Result<()> {
    let mut pids = Vec::new();
    for image_name in windows_image_names(app_name) {
        for pid in process.pids_for_image(&image_name)? {
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    for pid in pids {
        if let Err(error) = process.terminate(pid)
            && process.is_alive(pid)?
        {
            return Err(error);
        }
    }
    Ok(())
}

fn windows_image_names(app_name: &str) -> Vec<String> {
    let mut names = vec![format!("{app_name}.exe")];
    let dashed = format!("{}.exe", app_name.replace(' ', "-"));
    if !names.contains(&dashed) {
        names.push(dashed);
    }
    names
}

pub fn logout(request: LogoutRequest) -> Result<LogoutOutcome> {
    logout_with_quitter(request, SystemAppQuitter)
}

pub fn logout_with_quitter<Q: AppQuitter>(
    request: LogoutRequest,
    quitter: Q,
) -> Result<LogoutOutcome> {
    if !request.edition.requires_login() {
        return Ok(LogoutOutcome {
            command: "logout",
            edition: request.edition,
            status: "not_applicable",
            cookie_path: login_cookie_file_path(request.edition, &request.paths.home_dir),
            login_state: LoginState::NotApplicable,
        });
    }

    let _lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;

    quitter.quit_app(request.edition)?;

    let cookie_path = login_cookie_file_path(request.edition, &request.paths.home_dir);
    if !cookie_path.is_file() {
        return Ok(LogoutOutcome {
            command: "logout",
            edition: request.edition,
            status: "already_logged_out",
            cookie_path,
            login_state: LoginState::NotLoggedIn,
        });
    }

    let cookie_bytes = fs::read(&cookie_path)?;
    let mut cookies = serde_json::from_slice::<Value>(&cookie_bytes).map_err(|source| {
        Chat2DbError::InvalidCookieFile {
            path: cookie_path.clone(),
            source,
        }
    })?;

    let Some(cookie_object) = cookies.as_object_mut() else {
        return Err(Chat2DbError::InvalidCookieShape { path: cookie_path });
    };

    let mut changed = false;
    for key in logout_cookie_keys(request.edition) {
        changed |= cookie_object.remove(*key).is_some();
    }

    if changed {
        let mut serialized = serde_json::to_vec_pretty(&cookies)?;
        serialized.push(b'\n');
        fs::write(&cookie_path, serialized)?;
    }

    let current_status = check_edition_status(request.edition, &request.paths);
    Ok(LogoutOutcome {
        command: "logout",
        edition: request.edition,
        status: if changed {
            "logged_out"
        } else {
            "already_logged_out"
        },
        cookie_path,
        login_state: current_status.login.state,
    })
}

fn logout_cookie_keys(edition: Edition) -> &'static [&'static str] {
    match edition {
        Edition::Community => &[],
        Edition::Pro => PRO_LOGOUT_COOKIES,
        Edition::Local => LOCAL_LOGOUT_COOKIES,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn community_logout_is_not_applicable_without_quitting_or_touching_cookies() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = login_cookie_file_path(Edition::Community, &paths.home_dir);
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, b"community-cookie-must-remain-untouched").unwrap();
        let quitter = RecordingQuitter::default();

        let outcome = logout_with_quitter(
            LogoutRequest {
                edition: Edition::Community,
                paths,
            },
            &quitter,
        )
        .unwrap();

        assert_eq!(outcome.status, "not_applicable");
        assert_eq!(outcome.login_state, LoginState::NotApplicable);
        assert_eq!(outcome.cookie_path, cookie_file);
        assert!(quitter.calls.borrow().is_empty());
        assert_eq!(
            fs::read(&cookie_file).unwrap(),
            b"community-cookie-must-remain-untouched"
        );
    }

    #[test]
    fn logout_removes_pro_login_cookies_but_keeps_other_cookie_data() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = paths
            .home_dir
            .join(".chat2db/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(
            &cookie_file,
            r#"{
                "CHAT2DB":{"value":"session"},
                "Chat2db-BS-TOKEN":{"value":"MTIz"},
                "Chat2db-Organization-Token":{"value":"organization-token"},
                "Chat2db-Organization-Id":{"value":"123"},
                "Chat2DB-Country":{"value":"CN"}
            }"#,
        )
        .unwrap();

        let quitter = RecordingQuitter::default();
        let outcome = logout_with_quitter(
            LogoutRequest {
                edition: Edition::Pro,
                paths,
            },
            &quitter,
        )
        .unwrap();

        assert_eq!(outcome.status, "logged_out");
        assert_eq!(outcome.login_state, LoginState::NotLoggedIn);
        assert_eq!(quitter.calls.borrow().as_slice(), &[Edition::Pro]);

        let cookies: Value = serde_json::from_slice(&fs::read(&cookie_file).unwrap()).unwrap();
        assert!(cookies.get("CHAT2DB").is_none());
        assert!(cookies.get("Chat2db-BS-TOKEN").is_none());
        assert!(cookies.get("Chat2db-Organization-Token").is_none());
        assert!(cookies.get("Chat2db-Organization-Id").is_none());
        assert!(cookies.get("Chat2DB-Country").is_some());
    }

    #[test]
    fn logout_local_keeps_license_cookie() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = paths
            .home_dir
            .join(".chat2db_local_edition/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(
            &cookie_file,
            r#"{"Chat2db-BS-TOKEN":{"value":"MTIz"},"AAAAAAAAAAAAAAAA":{"value":"license"}}"#,
        )
        .unwrap();

        let quitter = RecordingQuitter::default();
        let outcome = logout_with_quitter(
            LogoutRequest {
                edition: Edition::Local,
                paths,
            },
            &quitter,
        )
        .unwrap();

        assert_eq!(outcome.status, "logged_out");
        assert_eq!(quitter.calls.borrow().as_slice(), &[Edition::Local]);
        let cookies: Value = serde_json::from_slice(&fs::read(&cookie_file).unwrap()).unwrap();
        assert!(cookies.get("Chat2db-BS-TOKEN").is_none());
        assert!(cookies.get("AAAAAAAAAAAAAAAA").is_some());
    }

    #[test]
    fn logout_is_noop_when_cookie_file_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let quitter = RecordingQuitter::default();
        let outcome = logout_with_quitter(
            LogoutRequest {
                edition: Edition::Pro,
                paths,
            },
            &quitter,
        )
        .unwrap();

        assert_eq!(outcome.status, "already_logged_out");
        assert_eq!(outcome.login_state, LoginState::NotLoggedIn);
        assert_eq!(quitter.calls.borrow().as_slice(), &[Edition::Pro]);
    }

    #[test]
    fn logout_does_not_clear_cookies_when_app_quit_fails() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = paths
            .home_dir
            .join(".chat2db/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, r#"{"Chat2db-BS-TOKEN":{"value":"MTIz"}}"#).unwrap();

        let error = logout_with_quitter(
            LogoutRequest {
                edition: Edition::Pro,
                paths,
            },
            FailingQuitter,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::CommandFailed {
                program: "osascript",
                ..
            }
        ));
        let cookies: Value = serde_json::from_slice(&fs::read(&cookie_file).unwrap()).unwrap();
        assert!(cookies.get("Chat2db-BS-TOKEN").is_some());
    }

    #[test]
    fn windows_logout_succeeds_when_app_is_absent() {
        let process = FakeWindowsAppProcess {
            pids: vec![],
            alive: Cell::new(false),
            terminate_calls: RefCell::new(Vec::new()),
            terminate_fails: false,
        };

        windows_taskkill_app_with("Chat2DB Pro", &process).unwrap();

        assert!(process.terminate_calls.borrow().is_empty());
    }

    #[test]
    fn windows_logout_ignores_process_that_exits_before_taskkill() {
        let process = FakeWindowsAppProcess {
            pids: vec![4242],
            alive: Cell::new(false),
            terminate_calls: RefCell::new(Vec::new()),
            terminate_fails: true,
        };

        windows_taskkill_app_with("Chat2DB Local", &process).unwrap();

        assert_eq!(process.terminate_calls.borrow().as_slice(), &[4242]);
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

    #[derive(Default)]
    struct RecordingQuitter {
        calls: RefCell<Vec<Edition>>,
    }

    impl AppQuitter for &RecordingQuitter {
        fn quit_app(&self, edition: Edition) -> Result<()> {
            self.calls.borrow_mut().push(edition);
            Ok(())
        }
    }

    struct FailingQuitter;

    impl AppQuitter for FailingQuitter {
        fn quit_app(&self, _edition: Edition) -> Result<()> {
            Err(Chat2DbError::CommandFailed {
                program: "osascript",
                status: "exit status: 1".to_string(),
                stderr: "application did not quit".to_string(),
            })
        }
    }

    struct FakeWindowsAppProcess {
        pids: Vec<u32>,
        alive: Cell<bool>,
        terminate_calls: RefCell<Vec<u32>>,
        terminate_fails: bool,
    }

    impl WindowsAppProcess for FakeWindowsAppProcess {
        fn pids_for_image(&self, _image_name: &str) -> Result<Vec<u32>> {
            Ok(self.pids.clone())
        }

        fn terminate(&self, pid: u32) -> Result<()> {
            self.terminate_calls.borrow_mut().push(pid);
            if self.terminate_fails {
                Err(Chat2DbError::CommandFailed {
                    program: "taskkill",
                    status: "128".to_string(),
                    stderr: "process disappeared".to_string(),
                })
            } else {
                self.alive.set(false);
                Ok(())
            }
        }

        fn is_alive(&self, _pid: u32) -> Result<bool> {
            Ok(self.alive.get())
        }
    }
}
