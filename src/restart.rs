use std::path::PathBuf;
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::{AppVerifier, SystemPaths, verify_app_bundle};
use crate::login::AppOpener;
use crate::process::{PgrepProbe, ProcessProbe};
use crate::runtime::acquire_edition_lifecycle_lock;
use crate::start::{BackgroundAppOpener, StartRequest, start_with_opener_locked_and_verifier};
use crate::stop::{AppQuitter, OsascriptQuitter, StopRequest, stop_with_locked_and_verifier};

const RESTART_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const RESTART_WAIT_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct RestartRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl RestartRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RestartOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub stop_status: &'static str,
    pub start_status: &'static str,
    pub app_path: PathBuf,
}

pub fn restart(request: RestartRequest) -> Result<RestartOutcome> {
    restart_with(request, PgrepProbe, OsascriptQuitter, BackgroundAppOpener)
}

pub fn restart_with<P: ProcessProbe + Clone, Q: AppQuitter, O: AppOpener>(
    request: RestartRequest,
    probe: P,
    quitter: Q,
    opener: O,
) -> Result<RestartOutcome> {
    restart_with_and_verifier(request, probe, quitter, opener, verify_app_bundle)
}

fn restart_with_and_verifier<P: ProcessProbe + Clone, Q: AppQuitter, O: AppOpener>(
    request: RestartRequest,
    probe: P,
    quitter: Q,
    opener: O,
    app_verifier: AppVerifier,
) -> Result<RestartOutcome> {
    restart_with_timing(
        request,
        probe,
        quitter,
        opener,
        RESTART_WAIT_TIMEOUT,
        RESTART_WAIT_INTERVAL,
        app_verifier,
    )
}

fn restart_with_timing<P: ProcessProbe + Clone, Q: AppQuitter, O: AppOpener>(
    request: RestartRequest,
    probe: P,
    quitter: Q,
    opener: O,
    wait_timeout: Duration,
    wait_interval: Duration,
    app_verifier: AppVerifier,
) -> Result<RestartOutcome> {
    let _lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;
    let stop_outcome = stop_with_locked_and_verifier(
        StopRequest {
            edition: request.edition,
            paths: request.paths.clone(),
        },
        probe.clone(),
        quitter,
        app_verifier,
    )?;

    if stop_outcome.status == "stopped"
        && !wait_for_app_exit(&probe, &stop_outcome.app_path, wait_timeout, wait_interval)?
    {
        return Err(Chat2DbError::RestartAppStillRunning {
            edition: request.edition.to_string(),
            waited_seconds: wait_timeout.as_secs(),
        });
    }

    let start_outcome = start_with_opener_locked_and_verifier(
        StartRequest {
            edition: request.edition,
            paths: request.paths,
        },
        opener,
        app_verifier,
    )?;

    Ok(RestartOutcome {
        command: "restart",
        edition: request.edition,
        stop_status: stop_outcome.status,
        start_status: start_outcome.status,
        app_path: start_outcome.app_path,
    })
}

fn wait_for_app_exit<P: ProcessProbe + ?Sized>(
    probe: &P,
    app_path: &std::path::Path,
    timeout: Duration,
    interval: Duration,
) -> Result<bool> {
    if !probe.is_running(app_path)? {
        return Ok(true);
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        sleep(interval.min(deadline.saturating_duration_since(Instant::now())));
        if !probe.is_running(app_path)? {
            return Ok(true);
        }
    }
    Ok(!probe.is_running(app_path)?)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::path::Path;

    use super::*;
    use crate::test_support::{create_installed_app, test_paths, verify_app_fixture};
    #[derive(Debug)]
    struct FakeProbe {
        running: RefCell<VecDeque<bool>>,
        last: Cell<bool>,
        calls: Cell<usize>,
    }

    impl FakeProbe {
        fn new(states: impl IntoIterator<Item = bool>) -> Self {
            Self {
                running: RefCell::new(states.into_iter().collect()),
                last: Cell::new(false),
                calls: Cell::new(0),
            }
        }
    }

    impl ProcessProbe for &FakeProbe {
        fn pids(&self, _app_path: &Path) -> Result<Vec<u32>> {
            self.calls.set(self.calls.get() + 1);
            let running = self
                .running
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| self.last.get());
            self.last.set(running);
            Ok(if running { vec![4242] } else { vec![] })
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
    fn restart_running_app_quits_then_opens() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Pro);

        let probe = FakeProbe::new([true, true, false]);
        let quitter = FakeQuitter::default();
        let opener = FakeOpener::default();

        let outcome = restart_with_and_verifier(
            RestartRequest {
                edition: Edition::Pro,
                paths,
            },
            &probe,
            &quitter,
            &opener,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.stop_status, "stopped");
        assert_eq!(outcome.start_status, "opened");
        assert_eq!(probe.calls.get(), 3);
        assert_eq!(quitter.quit_calls.borrow().as_slice(), &["Chat2DB Pro"]);
        assert_eq!(opener.opened.borrow().as_slice(), &[app_path]);
    }

    #[test]
    fn restart_when_not_running_skips_quit_but_still_opens() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let app_path = create_installed_app(&paths, Edition::Local);

        let probe = FakeProbe::new([false]);
        let quitter = FakeQuitter::default();
        let opener = FakeOpener::default();

        let outcome = restart_with_and_verifier(
            RestartRequest {
                edition: Edition::Local,
                paths,
            },
            &probe,
            &quitter,
            &opener,
            verify_app_fixture,
        )
        .unwrap();

        assert_eq!(outcome.stop_status, "already_stopped");
        assert_eq!(outcome.start_status, "opened");
        assert_eq!(probe.calls.get(), 1);
        assert!(quitter.quit_calls.borrow().is_empty());
        assert_eq!(opener.opened.borrow().as_slice(), &[app_path]);
    }

    #[test]
    fn refuses_missing_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let probe = FakeProbe::new([false]);
        let quitter = FakeQuitter::default();
        let opener = FakeOpener::default();

        let error = restart_with_and_verifier(
            RestartRequest {
                edition: Edition::Pro,
                paths,
            },
            &probe,
            &quitter,
            &opener,
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::AppNotInstalled { .. }));
        assert!(opener.opened.borrow().is_empty());
        assert!(quitter.quit_calls.borrow().is_empty());
    }

    #[test]
    fn restart_aborts_when_previous_process_does_not_exit() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_installed_app(&paths, Edition::Pro);
        let probe = FakeProbe::new([true, true]);
        let quitter = FakeQuitter::default();
        let opener = FakeOpener::default();

        let error = restart_with_timing(
            RestartRequest {
                edition: Edition::Pro,
                paths,
            },
            &probe,
            &quitter,
            &opener,
            Duration::from_millis(3),
            Duration::from_millis(1),
            verify_app_fixture,
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::RestartAppStillRunning { .. }));
        assert!(opener.opened.borrow().is_empty());
    }
}
