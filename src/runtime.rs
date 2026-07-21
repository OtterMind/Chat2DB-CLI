use std::fmt;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::{SystemPaths, verify_app_bundle};
use crate::platform::{
    ExclusiveLock, acquire_exclusive_lock, configure_detached_command, process_exists,
    process_identity, terminate_process, write_private_atomic,
};
use crate::runtime_client::{
    RuntimeCapability, RuntimeClient, RuntimeHealth, validate_runtime_health,
};

const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(45);
const DEFAULT_WAIT_INTERVAL: Duration = Duration::from_millis(500);
const RUNTIME_TOKEN_ENV: &str = "CHAT2DB_CLI_RUNTIME_TOKEN";

#[derive(Debug, Clone)]
pub struct RuntimeRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl RuntimeRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub status: RuntimeStateKind,
    pub state_path: PathBuf,
    pub app_path: PathBuf,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub healthy: bool,
    pub health: Option<RuntimeHealth>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStateKind {
    Running,
    Started,
    Stopped,
    Stale,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub edition: Edition,
    pub pid: u32,
    pub process_identity: String,
    pub port: u16,
    pub token: String,
    pub app_path: PathBuf,
    pub started_at_unix: u64,
}

impl fmt::Debug for RuntimeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeState")
            .field("edition", &self.edition)
            .field("pid", &self.pid)
            .field("process_identity", &"<recorded>")
            .field("port", &self.port)
            .field("token", &"<redacted>")
            .field("app_path", &self.app_path)
            .field("started_at_unix", &self.started_at_unix)
            .finish()
    }
}

trait RuntimeProcessControl {
    fn is_alive(&self, pid: u32) -> Result<bool>;
    fn identity(&self, pid: u32) -> Result<Option<String>>;
    fn terminate(&self, pid: u32) -> Result<()>;
}

#[derive(Debug, Clone, Copy)]
struct SystemRuntimeProcessControl;

impl RuntimeProcessControl for SystemRuntimeProcessControl {
    fn is_alive(&self, pid: u32) -> Result<bool> {
        process_exists(pid)
    }

    fn identity(&self, pid: u32) -> Result<Option<String>> {
        process_identity(pid)
    }

    fn terminate(&self, pid: u32) -> Result<()> {
        terminate_process(pid)
    }
}

trait RuntimeShutdown {
    fn shutdown(&self, state: &RuntimeState) -> Result<()>;
}

#[derive(Debug, Clone, Copy)]
struct SystemRuntimeShutdown;

impl RuntimeShutdown for SystemRuntimeShutdown {
    fn shutdown(&self, state: &RuntimeState) -> Result<()> {
        RuntimeClient::new(base_url(state.port), state.token.clone())
            .shutdown()
            .map(|_| ())
    }
}

pub fn runtime_status(request: RuntimeRequest) -> Result<RuntimeOutcome> {
    let app_path = request.edition.app_path(&request.paths.applications_dir);
    let state_path = state_path(&request.paths, request.edition);
    let Some(state) = read_state(&state_path)? else {
        return Ok(RuntimeOutcome {
            command: "runtime_status",
            edition: request.edition,
            status: RuntimeStateKind::Stopped,
            state_path,
            app_path,
            pid: None,
            port: None,
            url: None,
            healthy: false,
            health: None,
        });
    };

    let url = base_url(state.port);
    let process = SystemRuntimeProcessControl;
    let alive = verify_owned_process(&state, &process).unwrap_or(false);
    let health = if alive {
        RuntimeClient::new(url.clone(), state.token.clone())
            .health()
            .ok()
    } else {
        None
    };
    let healthy = health
        .as_ref()
        .is_some_and(|health| validate_runtime_health(health, request.edition).is_ok());
    Ok(RuntimeOutcome {
        command: "runtime_status",
        edition: request.edition,
        status: if alive && healthy {
            RuntimeStateKind::Running
        } else {
            RuntimeStateKind::Stale
        },
        state_path,
        app_path: state.app_path,
        pid: Some(state.pid),
        port: Some(state.port),
        url: Some(url),
        healthy,
        health,
    })
}

pub fn runtime_start(request: RuntimeRequest) -> Result<RuntimeOutcome> {
    let state = ensure_runtime(&request, DEFAULT_WAIT_TIMEOUT, DEFAULT_WAIT_INTERVAL)?;
    let url = base_url(state.port);
    let health = RuntimeClient::new(url.clone(), state.token.clone())
        .health()
        .ok();
    Ok(RuntimeOutcome {
        command: "runtime_start",
        edition: request.edition,
        status: RuntimeStateKind::Started,
        state_path: state_path(&request.paths, request.edition),
        app_path: state.app_path,
        pid: Some(state.pid),
        port: Some(state.port),
        url: Some(url),
        healthy: health.as_ref().map(|h| h.ready).unwrap_or(false),
        health,
    })
}

pub fn runtime_stop(request: RuntimeRequest) -> Result<RuntimeOutcome> {
    runtime_stop_with(
        request,
        &SystemRuntimeProcessControl,
        &SystemRuntimeShutdown,
        Duration::from_secs(10),
        Duration::from_millis(250),
    )
}

fn runtime_stop_with<P, S>(
    request: RuntimeRequest,
    process: &P,
    shutdown: &S,
    timeout: Duration,
    interval: Duration,
) -> Result<RuntimeOutcome>
where
    P: RuntimeProcessControl + ?Sized,
    S: RuntimeShutdown + ?Sized,
{
    let state_path = state_path(&request.paths, request.edition);
    let app_path = request.edition.app_path(&request.paths.applications_dir);
    let _lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;
    let Some(state) = read_state(&state_path)? else {
        return Ok(stopped_outcome(request.edition, state_path, app_path));
    };

    match verify_owned_process(&state, process) {
        Ok(false) => {
            remove_state(&state_path)?;
            return Ok(stopped_outcome(request.edition, state_path, app_path));
        }
        Ok(true) => {}
        Err(error) => {
            if matches!(error, Chat2DbError::RuntimeProcessOwnershipMismatch { .. }) {
                remove_state(&state_path)?;
            }
            return Err(error);
        }
    }

    let _shutdown_error = shutdown.shutdown(&state).err();
    if wait_until_dead_with(process, state.pid, timeout, interval)? {
        remove_state(&state_path)?;
        return Ok(stopped_outcome(request.edition, state_path, app_path));
    }

    match verify_owned_process(&state, process) {
        Ok(false) => {
            remove_state(&state_path)?;
            return Ok(stopped_outcome(request.edition, state_path, app_path));
        }
        Ok(true) => {}
        Err(error) => {
            if matches!(error, Chat2DbError::RuntimeProcessOwnershipMismatch { .. }) {
                remove_state(&state_path)?;
            }
            return Err(error);
        }
    }

    if let Err(error) = process.terminate(state.pid)
        && process.is_alive(state.pid)?
    {
        return Err(error);
    }
    if !wait_until_dead_with(process, state.pid, timeout, interval)? {
        return Err(Chat2DbError::RuntimeProcessStillRunning { pid: state.pid });
    }

    remove_state(&state_path)?;
    Ok(stopped_outcome(request.edition, state_path, app_path))
}

fn stopped_outcome(edition: Edition, state_path: PathBuf, app_path: PathBuf) -> RuntimeOutcome {
    RuntimeOutcome {
        command: "runtime_stop",
        edition,
        status: RuntimeStateKind::Stopped,
        state_path,
        app_path,
        pid: None,
        port: None,
        url: None,
        healthy: false,
        health: None,
    }
}

pub fn ensure_runtime(
    request: &RuntimeRequest,
    timeout: Duration,
    interval: Duration,
) -> Result<RuntimeState> {
    let _lock = acquire_edition_lifecycle_lock(&request.paths, request.edition)?;
    ensure_runtime_locked(request, timeout, interval)
}

fn ensure_runtime_locked(
    request: &RuntimeRequest,
    timeout: Duration,
    interval: Duration,
) -> Result<RuntimeState> {
    ensure_runtime_locked_with_process(request, timeout, interval, &SystemRuntimeProcessControl)
}

fn ensure_runtime_locked_with_process<P: RuntimeProcessControl + ?Sized>(
    request: &RuntimeRequest,
    timeout: Duration,
    interval: Duration,
    process: &P,
) -> Result<RuntimeState> {
    let path = state_path(&request.paths, request.edition);
    if let Some(state) = read_state(&path)? {
        match verify_owned_process(&state, process) {
            Ok(true) => {
                let client = RuntimeClient::new(base_url(state.port), state.token.clone());
                if let Ok(health) = client.health()
                    && health.ready
                {
                    if let Err(error) = validate_runtime_health(&health, request.edition) {
                        discard_runtime(&path, &state, process)?;
                        return Err(error);
                    }
                    return Ok(state);
                }
                discard_runtime(&path, &state, process)?;
            }
            Ok(false) => remove_state(&path)?,
            Err(error) => {
                if matches!(error, Chat2DbError::RuntimeProcessOwnershipMismatch { .. }) {
                    remove_state(&path)?;
                }
                return Err(error);
            }
        }
    }

    fs::create_dir_all(&request.paths.background_state_dir)?;

    let app_path = request.edition.app_path(&request.paths.applications_dir);
    if !app_path.exists() {
        return Err(Chat2DbError::AppNotInstalled {
            edition: request.edition.to_string(),
            path: app_path,
        });
    }
    verify_app_bundle(&app_path, request.edition).map_err(|error| Chat2DbError::UnsafeOpen {
        path: app_path.clone(),
        reason: error.to_string(),
    })?;

    let app = RuntimeApp::load(&app_path)?;
    let port = allocate_port()?;
    let token = generate_token()?;
    let mut cmd = build_runtime_command(&app, request.edition, port, &token);
    detach_runtime_process(&mut cmd);

    let mut child = cmd.spawn().map_err(|source| Chat2DbError::CommandIo {
        program: "chat2db runtime java",
        source,
    })?;

    let pid = child.id();
    let process_identity = match process.identity(pid) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            cleanup_untracked_child(&mut child)?;
            return Err(Chat2DbError::RuntimeProcessIdentityUnavailable {
                pid,
                message: "process exited before its identity could be recorded".to_string(),
            });
        }
        Err(error) => {
            cleanup_untracked_child(&mut child)?;
            return Err(Chat2DbError::RuntimeProcessIdentityUnavailable {
                pid,
                message: error.to_string(),
            });
        }
    };

    let state = RuntimeState {
        edition: request.edition,
        pid,
        process_identity,
        port,
        token,
        app_path,
        started_at_unix: now_unix(),
    };
    if let Err(error) = write_state(&path, &state) {
        cleanup_untracked_child(&mut child)?;
        return Err(error);
    }
    drop(child);

    let client = RuntimeClient::new(base_url(state.port), state.token.clone());
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(health) = client.health()
            && health.ready
        {
            if let Err(error) = validate_runtime_health(&health, request.edition) {
                discard_runtime(&path, &state, process)?;
                return Err(error);
            }
            return Ok(state);
        }
        sleep(interval);
    }

    discard_runtime(&path, &state, process)?;
    Err(Chat2DbError::RuntimeEndpointNotReady {
        edition: request.edition.to_string(),
        url: base_url(state.port),
        waited_seconds: timeout.as_secs(),
    })
}

fn discard_runtime<P: RuntimeProcessControl + ?Sized>(
    state_path: &Path,
    state: &RuntimeState,
    process: &P,
) -> Result<()> {
    match verify_owned_process(state, process) {
        Ok(false) => {
            remove_state(state_path)?;
            return Ok(());
        }
        Ok(true) => {}
        Err(error) => {
            if matches!(error, Chat2DbError::RuntimeProcessOwnershipMismatch { .. }) {
                remove_state(state_path)?;
            }
            return Err(error);
        }
    }
    if let Err(error) = process.terminate(state.pid)
        && process.is_alive(state.pid)?
    {
        return Err(error);
    }
    if !wait_until_dead_with(
        process,
        state.pid,
        Duration::from_secs(5),
        Duration::from_millis(100),
    )? {
        return Err(Chat2DbError::RuntimeProcessStillRunning { pid: state.pid });
    }
    remove_state(state_path)
}

fn cleanup_untracked_child(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let _ = child.wait()?;
    Ok(())
}

#[derive(Debug, Clone)]
struct RuntimeApp {
    app_dir: PathBuf,
    java_bin: PathBuf,
    classpath: String,
    mainclass: String,
    java_options: Vec<String>,
}

impl RuntimeApp {
    fn load(app_path: &Path) -> Result<Self> {
        let (app_dir, java_bin, classpath_separator) = runtime_layout(app_path)?;
        let cfg_path = app_dir.join(format!(
            "{}.cfg",
            app_path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Chat2DB Pro")
        ));
        let cfg = RuntimeCfg::parse(&cfg_path)?;
        if !java_bin.is_file() {
            return Err(Chat2DbError::RuntimeCfgInvalid {
                path: cfg_path,
                message: format!("missing java binary at {}", java_bin.display()),
            });
        }
        let classpath = cfg
            .classpath
            .iter()
            .map(|item| item.replace("$APPDIR", &app_dir.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(classpath_separator);
        Ok(Self {
            app_dir,
            java_bin,
            classpath,
            mainclass: cfg.mainclass,
            java_options: cfg.java_options,
        })
    }
}

fn runtime_layout(app_path: &Path) -> Result<(PathBuf, PathBuf, &'static str)> {
    if app_path.extension().and_then(|ext| ext.to_str()) == Some("exe") {
        let root = app_path
            .parent()
            .ok_or_else(|| Chat2DbError::RuntimeCfgInvalid {
                path: app_path.to_path_buf(),
                message: "Windows executable has no install directory".to_string(),
            })?;
        return Ok((
            root.join("app"),
            root.join("runtime").join("bin").join("java.exe"),
            ";",
        ));
    }

    Ok((
        app_path.join("Contents/app"),
        app_path.join("Contents/runtime/Contents/Home/bin/java"),
        ":",
    ))
}

#[derive(Debug, Clone)]
struct RuntimeCfg {
    classpath: Vec<String>,
    mainclass: String,
    java_options: Vec<String>,
}

impl RuntimeCfg {
    fn parse(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut classpath = Vec::new();
        let mut mainclass = None;
        let mut java_options = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('[') || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "app.classpath" => classpath.push(value.to_string()),
                "app.mainclass" => mainclass = Some(value.to_string()),
                "java-options" => java_options.push(value.to_string()),
                _ => {}
            }
        }
        let mainclass = mainclass.ok_or_else(|| Chat2DbError::RuntimeCfgInvalid {
            path: path.to_path_buf(),
            message: "missing app.mainclass".to_string(),
        })?;
        if classpath.is_empty() {
            return Err(Chat2DbError::RuntimeCfgInvalid {
                path: path.to_path_buf(),
                message: "missing app.classpath".to_string(),
            });
        }
        Ok(Self {
            classpath,
            mainclass,
            java_options,
        })
    }
}

fn keep_java_option(option: &str) -> bool {
    !option.starts_with("-Xdock:")
        && !option.starts_with("-Dapple.")
        && !option.starts_with("-Dserver.address=")
        && !option.starts_with("-Dserver.port=")
        && !option.starts_with("-Dchat2db.gui=")
        && !option.starts_with("-Dchat2db.cli.runtime.edition=")
        && !option.starts_with("-Dchat2db.cli.runtime.token=")
        && !option.starts_with("-Dspring.ai.mcp.server.enabled=")
}

fn build_runtime_command(app: &RuntimeApp, edition: Edition, port: u16, token: &str) -> Command {
    let mut cmd = Command::new(&app.java_bin);
    cmd.args(
        app.java_options
            .iter()
            .filter(|option| keep_java_option(option)),
    );
    cmd.arg("-Dchat2db.runtime.mode=cli");
    cmd.arg(format!(
        "-Dchat2db.cli.runtime.edition={}",
        edition.as_str()
    ));
    cmd.arg("-Dchat2db.gui=false");
    cmd.arg("-Dspring.ai.mcp.server.enabled=false");
    cmd.arg("-Dserver.address=127.0.0.1");
    cmd.arg(format!("-Dserver.port={port}"));
    if edition == Edition::Local {
        cmd.arg("-Dchat2db.network.status=OFFLINE");
    }
    cmd.arg("-cp");
    cmd.arg(&app.classpath);
    cmd.arg(&app.mainclass);
    cmd.current_dir(&app.app_dir);
    cmd.env(RUNTIME_TOKEN_ENV, token);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    configure_detached_command(&mut cmd);
    cmd
}

pub fn runtime_client(
    request: &RuntimeRequest,
    required_capability: RuntimeCapability,
) -> Result<RuntimeClient> {
    let state = ensure_runtime(request, DEFAULT_WAIT_TIMEOUT, DEFAULT_WAIT_INTERVAL)?;
    let client = RuntimeClient::new(base_url(state.port), state.token);
    client.verify_contract(request.edition, required_capability)?;
    Ok(client)
}

fn state_path(paths: &SystemPaths, edition: Edition) -> PathBuf {
    paths
        .background_state_dir
        .join(format!("runtime-{}.json", edition.as_str()))
}

fn lock_path(paths: &SystemPaths, edition: Edition) -> PathBuf {
    paths
        .background_state_dir
        .join(format!("runtime-{}.lock", edition.as_str()))
}

pub(crate) fn acquire_edition_lifecycle_lock(
    paths: &SystemPaths,
    edition: Edition,
) -> Result<ExclusiveLock> {
    acquire_exclusive_lock(&lock_path(paths, edition))
}

pub(crate) fn ensure_runtime_stopped_locked(paths: &SystemPaths, edition: Edition) -> Result<()> {
    let path = state_path(paths, edition);
    let Some(state) = read_state(&path)? else {
        return Ok(());
    };
    match verify_owned_process(&state, &SystemRuntimeProcessControl) {
        Ok(false) => remove_state(&path),
        Ok(true) => Err(Chat2DbError::RuntimeProcessStillRunning { pid: state.pid }),
        Err(error) => Err(error),
    }
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/api/cli/v1")
}

fn read_state(path: &Path) -> Result<Option<RuntimeState>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| Chat2DbError::RuntimeStateInvalidJson {
            path: path.to_path_buf(),
            source,
        })
}

fn write_state(path: &Path, state: &RuntimeState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(state).map_err(Chat2DbError::ManifestJson)?;
    write_private_atomic(path, format!("{text}\n").as_bytes())
}

fn remove_state(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn verify_owned_process<P: RuntimeProcessControl + ?Sized>(
    state: &RuntimeState,
    process: &P,
) -> Result<bool> {
    if !process.is_alive(state.pid)? {
        return Ok(false);
    }
    match process.identity(state.pid) {
        Ok(Some(identity)) if identity == state.process_identity => Ok(true),
        Ok(Some(_)) => Err(Chat2DbError::RuntimeProcessOwnershipMismatch { pid: state.pid }),
        Ok(None) => {
            if process.is_alive(state.pid)? {
                Err(Chat2DbError::RuntimeProcessIdentityUnavailable {
                    pid: state.pid,
                    message: "process identity is unavailable".to_string(),
                })
            } else {
                Ok(false)
            }
        }
        Err(error) => {
            if process.is_alive(state.pid)? {
                Err(Chat2DbError::RuntimeProcessIdentityUnavailable {
                    pid: state.pid,
                    message: error.to_string(),
                })
            } else {
                Ok(false)
            }
        }
    }
}

fn allocate_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn generate_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|source| Chat2DbError::RuntimeTokenGeneration {
        message: source.to_string(),
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn wait_until_dead_with<P: RuntimeProcessControl + ?Sized>(
    process: &P,
    pid: u32,
    timeout: Duration,
    interval: Duration,
) -> Result<bool> {
    if !process.is_alive(pid)? {
        return Ok(true);
    }
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        sleep(interval.min(deadline.saturating_duration_since(std::time::Instant::now())));
        if !process.is_alive(pid)? {
            return Ok(true);
        }
    }
    Ok(!process.is_alive(pid)?)
}

#[cfg(unix)]
fn detach_runtime_process(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_runtime_process(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    #[derive(Clone)]
    struct FakeRuntimeProcess {
        alive: Rc<Cell<bool>>,
        identity: String,
        identity_available: bool,
        terminate_calls: Rc<Cell<usize>>,
        terminate_fails: bool,
    }

    impl FakeRuntimeProcess {
        fn new(identity: &str) -> Self {
            Self {
                alive: Rc::new(Cell::new(true)),
                identity: identity.to_string(),
                identity_available: true,
                terminate_calls: Rc::new(Cell::new(0)),
                terminate_fails: false,
            }
        }
    }

    impl RuntimeProcessControl for FakeRuntimeProcess {
        fn is_alive(&self, _pid: u32) -> Result<bool> {
            Ok(self.alive.get())
        }

        fn identity(&self, _pid: u32) -> Result<Option<String>> {
            Ok((self.alive.get() && self.identity_available).then(|| self.identity.clone()))
        }

        fn terminate(&self, _pid: u32) -> Result<()> {
            self.terminate_calls.set(self.terminate_calls.get() + 1);
            if self.terminate_fails {
                return Err(Chat2DbError::CommandFailed {
                    program: "fake-terminate",
                    status: "failed".to_string(),
                    stderr: "process is still running".to_string(),
                });
            }
            self.alive.set(false);
            Ok(())
        }
    }

    struct FakeRuntimeShutdown {
        alive: Rc<Cell<bool>>,
        exits: bool,
    }

    impl RuntimeShutdown for FakeRuntimeShutdown {
        fn shutdown(&self, _state: &RuntimeState) -> Result<()> {
            if self.exits {
                self.alive.set(false);
            }
            Ok(())
        }
    }

    #[test]
    fn loads_windows_jpackage_runtime_layout() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Chat2DB Pro");
        let app_dir = root.join("app");
        let java_bin = root.join("runtime").join("bin").join("java.exe");
        fs::create_dir_all(&app_dir).unwrap();
        fs::create_dir_all(java_bin.parent().unwrap()).unwrap();
        fs::write(root.join("Chat2DB Pro.exe"), b"fake-exe").unwrap();
        fs::write(&java_bin, b"fake-java").unwrap();
        fs::write(
            app_dir.join("Chat2DB Pro.cfg"),
            "app.classpath=$APPDIR/chat2db-enterprise.jar\n\
             app.classpath=$APPDIR/lib/*\n\
             app.mainclass=org.springframework.boot.loader.launch.JarLauncher\n\
             java-options=-Dserver.port=11924\n",
        )
        .unwrap();

        let app = RuntimeApp::load(&root.join("Chat2DB Pro.exe")).unwrap();

        assert_eq!(app.app_dir, app_dir);
        assert_eq!(app.java_bin, java_bin);
        assert!(app.classpath.contains("chat2db-enterprise.jar;"));
        assert_eq!(
            app.mainclass,
            "org.springframework.boot.loader.launch.JarLauncher"
        );
    }

    #[test]
    fn loads_macos_jpackage_runtime_layout() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Chat2DB Pro.app");
        let app_dir = root.join("Contents").join("app");
        let java_bin = root
            .join("Contents")
            .join("runtime")
            .join("Contents")
            .join("Home")
            .join("bin")
            .join("java");
        fs::create_dir_all(&app_dir).unwrap();
        fs::create_dir_all(java_bin.parent().unwrap()).unwrap();
        fs::write(&java_bin, b"fake-java").unwrap();
        fs::write(
            app_dir.join("Chat2DB Pro.cfg"),
            "app.classpath=$APPDIR/chat2db-enterprise.jar\n\
             app.classpath=$APPDIR/lib/*\n\
             app.mainclass=org.springframework.boot.loader.launch.JarLauncher\n",
        )
        .unwrap();

        let app = RuntimeApp::load(&root).unwrap();

        assert_eq!(app.app_dir, app_dir);
        assert_eq!(app.java_bin, java_bin);
        assert!(app.classpath.contains("chat2db-enterprise.jar:"));
    }

    #[test]
    fn runtime_command_keeps_token_out_of_arguments() {
        let app = RuntimeApp {
            app_dir: PathBuf::from("/tmp/chat2db"),
            java_bin: PathBuf::from("/tmp/chat2db/java"),
            classpath: "app.jar".to_string(),
            mainclass: "org.example.Main".to_string(),
            java_options: vec![
                "-Xmx512m".to_string(),
                "-Dserver.port=9999".to_string(),
                "-Dchat2db.cli.runtime.token=stale-token".to_string(),
            ],
        };

        let cmd = build_runtime_command(&app, Edition::Pro, 11921, "fresh-token");
        let args = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();

        assert!(!args.iter().any(|arg| arg.contains("fresh-token")));
        assert!(!args.iter().any(|arg| arg.contains("stale-token")));
        assert!(args.iter().any(|arg| arg.as_ref() == "-Dserver.port=11921"));
        assert!(
            args.iter()
                .any(|arg| arg.as_ref() == "-Dchat2db.cli.runtime.edition=pro")
        );
        assert_eq!(
            cmd.get_envs()
                .find(|(name, _)| name.to_string_lossy() == RUNTIME_TOKEN_ENV)
                .and_then(|(_, value)| value)
                .map(|value| value.to_string_lossy().to_string())
                .as_deref(),
            Some("fresh-token")
        );
    }

    #[test]
    fn ensure_runtime_fails_fast_when_app_is_not_installed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::installer::SystemPaths {
            applications_dir: temp.path().join("Applications"),
            home_dir: temp.path().join("home"),
            background_state_dir: temp.path().join("state"),
        };
        let request = RuntimeRequest {
            edition: Edition::Local,
            paths,
        };

        let error = ensure_runtime(
            &request,
            Duration::from_secs(45),
            Duration::from_millis(500),
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::AppNotInstalled { .. }));
    }

    #[test]
    fn runtime_stop_refuses_reused_pid_without_terminating_it() {
        let temp = tempfile::tempdir().unwrap();
        let request = test_runtime_request(temp.path());
        let path = state_path(&request.paths, request.edition);
        write_state(&path, &test_runtime_state("original-process")).unwrap();
        let process = FakeRuntimeProcess::new("different-process");
        let shutdown = FakeRuntimeShutdown {
            alive: process.alive.clone(),
            exits: false,
        };

        let error = runtime_stop_with(request, &process, &shutdown, Duration::ZERO, Duration::ZERO)
            .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::RuntimeProcessOwnershipMismatch { pid: 4242 }
        ));
        assert_eq!(process.terminate_calls.get(), 0);
        assert!(process.alive.get());
        assert!(!path.exists());
    }

    #[test]
    fn runtime_stop_preserves_state_when_termination_fails() {
        let temp = tempfile::tempdir().unwrap();
        let request = test_runtime_request(temp.path());
        let path = state_path(&request.paths, request.edition);
        write_state(&path, &test_runtime_state("owned-process")).unwrap();
        let mut process = FakeRuntimeProcess::new("owned-process");
        process.terminate_fails = true;
        let shutdown = FakeRuntimeShutdown {
            alive: process.alive.clone(),
            exits: false,
        };

        let error = runtime_stop_with(request, &process, &shutdown, Duration::ZERO, Duration::ZERO)
            .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::CommandFailed {
                program: "fake-terminate",
                ..
            }
        ));
        assert_eq!(process.terminate_calls.get(), 1);
        assert!(path.exists());
    }

    #[test]
    fn runtime_stop_preserves_state_when_identity_is_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let request = test_runtime_request(temp.path());
        let path = state_path(&request.paths, request.edition);
        write_state(&path, &test_runtime_state("owned-process")).unwrap();
        let mut process = FakeRuntimeProcess::new("owned-process");
        process.identity_available = false;
        let shutdown = FakeRuntimeShutdown {
            alive: process.alive.clone(),
            exits: false,
        };

        let error = runtime_stop_with(request, &process, &shutdown, Duration::ZERO, Duration::ZERO)
            .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::RuntimeProcessIdentityUnavailable { pid: 4242, .. }
        ));
        assert_eq!(process.terminate_calls.get(), 0);
        assert!(path.exists());
    }

    #[test]
    fn ensure_runtime_preserves_state_when_identity_is_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let request = test_runtime_request(temp.path());
        let path = state_path(&request.paths, request.edition);
        write_state(&path, &test_runtime_state("owned-process")).unwrap();
        let mut process = FakeRuntimeProcess::new("owned-process");
        process.identity_available = false;

        let error =
            ensure_runtime_locked_with_process(&request, Duration::ZERO, Duration::ZERO, &process)
                .unwrap_err();

        assert!(matches!(
            error,
            Chat2DbError::RuntimeProcessIdentityUnavailable { pid: 4242, .. }
        ));
        assert_eq!(process.terminate_calls.get(), 0);
        assert!(path.exists());
    }

    #[test]
    fn runtime_stop_removes_state_after_confirmed_graceful_exit() {
        let temp = tempfile::tempdir().unwrap();
        let request = test_runtime_request(temp.path());
        let path = state_path(&request.paths, request.edition);
        write_state(&path, &test_runtime_state("owned-process")).unwrap();
        let process = FakeRuntimeProcess::new("owned-process");
        let shutdown = FakeRuntimeShutdown {
            alive: process.alive.clone(),
            exits: true,
        };

        let outcome =
            runtime_stop_with(request, &process, &shutdown, Duration::ZERO, Duration::ZERO)
                .unwrap();

        assert_eq!(outcome.status, RuntimeStateKind::Stopped);
        assert_eq!(process.terminate_calls.get(), 0);
        assert!(!path.exists());
    }

    #[test]
    fn runtime_lock_serializes_same_edition() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_runtime_request(temp.path()).paths;
        let first = acquire_edition_lifecycle_lock(&paths, Edition::Pro).unwrap();
        let thread_paths = paths.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let second = acquire_edition_lifecycle_lock(&thread_paths, Edition::Pro).unwrap();
            acquired_tx.send(()).unwrap();
            drop(second);
        });

        started_rx.recv().unwrap();
        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        handle.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_state_uses_private_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let state_path = temp.path().join("runtime-pro.json");
        let state = RuntimeState {
            edition: Edition::Pro,
            pid: 12345,
            process_identity: "test-process".to_string(),
            port: 11921,
            token: "state-token".to_string(),
            app_path: PathBuf::from("/Applications/Chat2DB Pro.app"),
            started_at_unix: 1,
        };

        write_state(&state_path, &state).unwrap();

        let mode = fs::metadata(&state_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    fn test_runtime_request(root: &Path) -> RuntimeRequest {
        RuntimeRequest {
            edition: Edition::Pro,
            paths: crate::installer::SystemPaths {
                applications_dir: root.join("Applications"),
                home_dir: root.join("home"),
                background_state_dir: root.join("state"),
            },
        }
    }

    fn test_runtime_state(identity: &str) -> RuntimeState {
        RuntimeState {
            edition: Edition::Pro,
            pid: 4242,
            process_identity: identity.to_string(),
            port: 11921,
            token: "state-token".to_string(),
            app_path: PathBuf::from("/Applications/Chat2DB Pro.app"),
            started_at_unix: 1,
        }
    }
}
