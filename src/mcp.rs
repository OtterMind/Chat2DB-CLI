use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::edition::{Edition, EditionSelector};
use crate::error::{Chat2DbError, Result};
use crate::frontend_store::{FrontendStoreDefaults, sync_mcp_enabled};
use crate::installer::SystemPaths;
use crate::platform::{ExclusiveLock, write_private_atomic};
use crate::process::{PgrepProbe, ProcessProbe, ProcessState, process_report};
use crate::runtime::acquire_edition_lifecycle_lock;
use crate::start::{HiddenAppOpener, StartOutcome, StartRequest, start_with_opener_locked};
use crate::stop::{OsascriptQuitter, StopOutcome, StopRequest, stop_with_locked};

pub fn mcp_url(edition: Edition) -> &'static str {
    edition.mcp_url()
}
const ENABLE_MCP_KEY: &str = "enableMcp";
const MCP_AUTH_TOKEN_KEY: &str = "mcpAuthToken";
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WAIT_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const DEFAULT_SETTLE_DURATION: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct McpStatusRequest {
    pub edition: EditionSelector,
    pub paths: SystemPaths,
}

impl McpStatusRequest {
    pub fn production(edition: EditionSelector) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpChangeRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
    pub wait_timeout: Duration,
    pub wait_interval: Duration,
    pub settle_duration: Duration,
    pub progress: bool,
}

impl McpChangeRequest {
    pub fn production(edition: Edition, progress: bool) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
            wait_timeout: DEFAULT_WAIT_TIMEOUT,
            wait_interval: DEFAULT_WAIT_INTERVAL,
            settle_duration: DEFAULT_SETTLE_DURATION,
            progress,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct McpOutcome {
    pub command: &'static str,
    pub editions: Vec<McpEditionStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpEditionStatus {
    pub edition: Edition,
    pub settings_path: PathBuf,
    pub enabled: bool,
    pub token_state: TokenState,
    pub app_running: bool,
    pub url: &'static str,
    pub endpoint_state: EndpointState,
    pub needs_restart: bool,
    pub next_action: McpNextAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenState {
    Present,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointState {
    Alive,
    NotListening,
    Timeout,
    NotChecked,
    Ambiguous,
}

impl EndpointState {
    pub fn is_alive(self) -> bool {
        matches!(self, EndpointState::Alive | EndpointState::Ambiguous)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpNextAction {
    None,
    Enable,
    Start,
    Restart,
    StopOtherEdition,
    InspectRuntimeLog,
}

pub trait McpEndpointProbe {
    fn probe(&self, url: &str, timeout: Duration) -> EndpointState;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TcpEndpointProbe;

impl McpEndpointProbe for TcpEndpointProbe {
    fn probe(&self, url: &str, timeout: Duration) -> EndpointState {
        probe_http_endpoint(url, timeout)
    }
}

pub trait McpAppLifecycle {
    fn stop_app(&self, edition: Edition, paths: SystemPaths) -> Result<StopOutcome>;
    fn start_app(&self, edition: Edition, paths: SystemPaths) -> Result<StartOutcome>;
}

#[derive(Debug, Default, Clone, Copy)]
struct LockedProductionAppLifecycle;

impl McpAppLifecycle for LockedProductionAppLifecycle {
    fn stop_app(&self, edition: Edition, paths: SystemPaths) -> Result<StopOutcome> {
        stop_with_locked(StopRequest { edition, paths }, PgrepProbe, OsascriptQuitter)
    }

    fn start_app(&self, edition: Edition, paths: SystemPaths) -> Result<StartOutcome> {
        start_with_opener_locked(StartRequest { edition, paths }, HiddenAppOpener)
    }
}

pub trait McpTokenGenerator {
    fn generate_token(&self) -> Result<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTokenGenerator;

impl McpTokenGenerator for SystemTokenGenerator {
    fn generate_token(&self) -> Result<String> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(|source| Chat2DbError::McpTokenGeneration {
            message: source.to_string(),
        })?;
        Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

pub fn mcp_status(request: McpStatusRequest) -> Result<McpOutcome> {
    mcp_status_with(request, &PgrepProbe, &TcpEndpointProbe)
}

pub fn mcp_enable(request: McpChangeRequest) -> Result<McpOutcome> {
    let _locks = acquire_mcp_lifecycle_locks(&request.paths, request.edition)?;
    mcp_enable_with(
        request,
        &PgrepProbe,
        &TcpEndpointProbe,
        &LockedProductionAppLifecycle,
        &SystemTokenGenerator,
    )
}

pub fn mcp_disable(request: McpChangeRequest) -> Result<McpOutcome> {
    let _locks = acquire_mcp_lifecycle_locks(&request.paths, request.edition)?;
    mcp_disable_with(
        request,
        &PgrepProbe,
        &TcpEndpointProbe,
        &LockedProductionAppLifecycle,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFrontendSyncStatus {
    Synced,
    SkippedLocked,
}

pub fn mcp_disable_without_restart(request: McpChangeRequest) -> Result<McpFrontendSyncStatus> {
    write_target_settings::<SystemTokenGenerator>(&request, false, None)?;
    match sync_frontend_mcp_state(&request, false) {
        Ok(()) => Ok(McpFrontendSyncStatus::Synced),
        Err(error) if is_frontend_store_lock_error(&error) => {
            Ok(McpFrontendSyncStatus::SkippedLocked)
        }
        Err(error) => Err(error),
    }
}

fn is_frontend_store_lock_error(error: &Chat2DbError) -> bool {
    let Chat2DbError::McpFrontendStore { message, .. } = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    message.contains("lockerror") || message.contains("database lock is held")
}

pub fn mcp_restart(request: McpChangeRequest) -> Result<McpOutcome> {
    let _locks = acquire_mcp_lifecycle_locks(&request.paths, request.edition)?;
    mcp_restart_with(
        request,
        &PgrepProbe,
        &TcpEndpointProbe,
        &LockedProductionAppLifecycle,
    )
}

fn acquire_mcp_lifecycle_locks(
    paths: &SystemPaths,
    edition: Edition,
) -> Result<Vec<ExclusiveLock>> {
    Edition::ALL
        .into_iter()
        .filter(|candidate| candidate.mcp_url() == edition.mcp_url())
        .map(|candidate| acquire_edition_lifecycle_lock(paths, candidate))
        .collect()
}

pub fn mcp_status_with<P, E>(
    request: McpStatusRequest,
    process_probe: &P,
    endpoint_probe: &E,
) -> Result<McpOutcome>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
{
    let editions = request
        .edition
        .editions()
        .iter()
        .map(|edition| {
            build_edition_status(
                *edition,
                &request.paths,
                process_probe,
                endpoint_probe,
                DEFAULT_PROBE_TIMEOUT,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(McpOutcome {
        command: "mcp_status",
        editions,
    })
}

pub fn mcp_enable_with<P, E, R, G>(
    request: McpChangeRequest,
    process_probe: &P,
    endpoint_probe: &E,
    lifecycle: &R,
    token_generator: &G,
) -> Result<McpOutcome>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
    R: McpAppLifecycle + ?Sized,
    G: McpTokenGenerator + ?Sized,
{
    ensure_no_other_edition_running(request.edition, &request.paths, process_probe)?;

    quit_app_for_settings_update(&request, process_probe, endpoint_probe, lifecycle)?;

    print_progress(request.progress, request.edition, "updating settings...");
    write_target_settings(&request, true, Some(token_generator))?;
    print_progress(
        request.progress,
        request.edition,
        "syncing desktop UI state...",
    );
    sync_frontend_mcp_state(&request, true)?;

    start_app_for_settings_update(&request, process_probe, lifecycle)?;
    print_progress(
        request.progress,
        request.edition,
        "waiting for MCP endpoint...",
    );
    let endpoint_state = wait_for_endpoint(
        endpoint_probe,
        mcp_url(request.edition),
        true,
        request.wait_timeout,
        request.wait_interval,
    );
    if endpoint_state != EndpointState::Alive {
        return Err(Chat2DbError::McpEndpointNotReady {
            edition: request.edition.to_string(),
            url: mcp_url(request.edition).to_string(),
            waited_seconds: request.wait_timeout.as_secs(),
        });
    }
    print_progress(request.progress, request.edition, "finalizing settings...");
    write_target_settings(&request, true, Some(token_generator))?;
    print_progress(request.progress, request.edition, "MCP endpoint is ready.");

    single_status_outcome(
        "mcp_enable",
        request.edition,
        &request.paths,
        process_probe,
        endpoint_probe,
    )
}

pub fn mcp_disable_with<P, E, R>(
    request: McpChangeRequest,
    process_probe: &P,
    endpoint_probe: &E,
    lifecycle: &R,
) -> Result<McpOutcome>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
    R: McpAppLifecycle + ?Sized,
{
    ensure_no_other_edition_running(request.edition, &request.paths, process_probe)?;

    quit_app_for_settings_update(&request, process_probe, endpoint_probe, lifecycle)?;

    print_progress(request.progress, request.edition, "updating settings...");
    write_target_settings::<SystemTokenGenerator>(&request, false, None)?;
    print_progress(
        request.progress,
        request.edition,
        "syncing desktop UI state...",
    );
    sync_frontend_mcp_state(&request, false)?;

    start_app_for_settings_update(&request, process_probe, lifecycle)?;
    print_progress(
        request.progress,
        request.edition,
        "waiting for MCP endpoint to stop...",
    );
    let endpoint_state = wait_for_endpoint_stable(
        endpoint_probe,
        mcp_url(request.edition),
        false,
        request.wait_timeout,
        request.wait_interval,
        request.settle_duration,
    );
    if endpoint_state.is_alive() {
        return Err(Chat2DbError::McpEndpointStillAlive {
            edition: request.edition.to_string(),
            url: mcp_url(request.edition).to_string(),
            waited_seconds: request.wait_timeout.as_secs(),
        });
    }
    print_progress(request.progress, request.edition, "finalizing settings...");
    write_target_settings::<SystemTokenGenerator>(&request, false, None)?;
    print_progress(
        request.progress,
        request.edition,
        "MCP endpoint is stopped.",
    );

    single_status_outcome(
        "mcp_disable",
        request.edition,
        &request.paths,
        process_probe,
        endpoint_probe,
    )
}

pub fn mcp_restart_with<P, E, R>(
    request: McpChangeRequest,
    process_probe: &P,
    endpoint_probe: &E,
    lifecycle: &R,
) -> Result<McpOutcome>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
    R: McpAppLifecycle + ?Sized,
{
    ensure_no_other_edition_running(request.edition, &request.paths, process_probe)?;

    let settings = SettingsFile::read(request.edition, &request.paths.home_dir)?;
    if !settings.enabled() {
        print_progress(
            request.progress,
            request.edition,
            "MCP is disabled; restart skipped.",
        );
        return single_status_outcome(
            "mcp_restart",
            request.edition,
            &request.paths,
            process_probe,
            endpoint_probe,
        );
    }

    quit_app_for_settings_update(&request, process_probe, endpoint_probe, lifecycle)?;
    print_progress(
        request.progress,
        request.edition,
        "syncing desktop UI state...",
    );
    sync_frontend_mcp_state(&request, true)?;
    start_app_for_settings_update(&request, process_probe, lifecycle)?;
    print_progress(
        request.progress,
        request.edition,
        "waiting for MCP endpoint...",
    );
    let endpoint_state = wait_for_endpoint(
        endpoint_probe,
        mcp_url(request.edition),
        true,
        request.wait_timeout,
        request.wait_interval,
    );
    if endpoint_state != EndpointState::Alive {
        return Err(Chat2DbError::McpEndpointNotReady {
            edition: request.edition.to_string(),
            url: mcp_url(request.edition).to_string(),
            waited_seconds: request.wait_timeout.as_secs(),
        });
    }
    print_progress(request.progress, request.edition, "finalizing settings...");
    write_target_settings::<SystemTokenGenerator>(&request, true, None)?;
    print_progress(request.progress, request.edition, "MCP endpoint is ready.");

    single_status_outcome(
        "mcp_restart",
        request.edition,
        &request.paths,
        process_probe,
        endpoint_probe,
    )
}

pub fn mcp_settings_path(edition: Edition, home_dir: &Path) -> PathBuf {
    edition.settings_path(home_dir)
}

fn single_status_outcome<P, E>(
    command: &'static str,
    edition: Edition,
    paths: &SystemPaths,
    process_probe: &P,
    endpoint_probe: &E,
) -> Result<McpOutcome>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
{
    Ok(McpOutcome {
        command,
        editions: vec![build_edition_status(
            edition,
            paths,
            process_probe,
            endpoint_probe,
            DEFAULT_PROBE_TIMEOUT,
        )?],
    })
}

fn build_edition_status<P, E>(
    edition: Edition,
    paths: &SystemPaths,
    process_probe: &P,
    endpoint_probe: &E,
    probe_timeout: Duration,
) -> Result<McpEditionStatus>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
{
    let settings = SettingsFile::read(edition, &paths.home_dir)?;
    let reports = process_reports(paths, process_probe);
    let app_running = reports
        .get(&edition)
        .is_some_and(|state| *state == ProcessState::Running);
    let other_running = reports.iter().any(|(other, state)| {
        *other != edition && other.mcp_url() == edition.mcp_url() && *state == ProcessState::Running
    });
    let raw_endpoint_state = endpoint_probe.probe(mcp_url(edition), probe_timeout);
    let endpoint_state = if raw_endpoint_state == EndpointState::Alive && other_running {
        EndpointState::Ambiguous
    } else {
        raw_endpoint_state
    };
    let (needs_restart, next_action) =
        derive_runtime_action(settings.enabled(), app_running, endpoint_state);

    Ok(McpEditionStatus {
        edition,
        settings_path: settings.path.clone(),
        enabled: settings.enabled(),
        token_state: settings.token_state(),
        app_running,
        url: mcp_url(edition),
        endpoint_state,
        needs_restart,
        next_action,
    })
}

fn process_reports<P: ProcessProbe + ?Sized>(
    paths: &SystemPaths,
    process_probe: &P,
) -> HashMap<Edition, ProcessState> {
    Edition::ALL
        .into_iter()
        .map(|edition| {
            let app_path = edition.app_path(&paths.applications_dir);
            (edition, process_report(process_probe, &app_path).state)
        })
        .collect()
}

fn derive_runtime_action(
    enabled: bool,
    app_running: bool,
    endpoint_state: EndpointState,
) -> (bool, McpNextAction) {
    if endpoint_state == EndpointState::Ambiguous {
        return (false, McpNextAction::StopOtherEdition);
    }

    if endpoint_state == EndpointState::Timeout {
        return (enabled && app_running, McpNextAction::InspectRuntimeLog);
    }

    if !enabled {
        return if endpoint_state.is_alive() {
            (true, McpNextAction::Restart)
        } else {
            (false, McpNextAction::Enable)
        };
    }

    if !app_running {
        return (false, McpNextAction::Start);
    }

    if endpoint_state == EndpointState::Alive {
        (false, McpNextAction::None)
    } else {
        (true, McpNextAction::Restart)
    }
}

fn ensure_no_other_edition_running<P: ProcessProbe + ?Sized>(
    edition: Edition,
    paths: &SystemPaths,
    process_probe: &P,
) -> Result<()> {
    for other in Edition::ALL
        .into_iter()
        .filter(|other| *other != edition && other.mcp_url() == edition.mcp_url())
    {
        let app_path = other.app_path(&paths.applications_dir);
        if process_probe.is_running(&app_path)? {
            return Err(Chat2DbError::McpPortAmbiguous {
                target: edition.to_string(),
                other: other.to_string(),
            });
        }
    }
    Ok(())
}

fn quit_app_for_settings_update<P, E, L>(
    request: &McpChangeRequest,
    process_probe: &P,
    endpoint_probe: &E,
    lifecycle: &L,
) -> Result<()>
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
    L: McpAppLifecycle + ?Sized,
{
    print_progress(request.progress, request.edition, "quitting app...");
    lifecycle.stop_app(request.edition, request.paths.clone())?;

    print_progress(
        request.progress,
        request.edition,
        "waiting for app to stop...",
    );
    if !wait_for_app_process(
        process_probe,
        request.edition,
        &request.paths,
        false,
        request.wait_timeout,
        request.wait_interval,
    )? {
        return Err(Chat2DbError::McpAppNotStopped {
            edition: request.edition.to_string(),
            waited_seconds: request.wait_timeout.as_secs(),
        });
    }

    print_progress(
        request.progress,
        request.edition,
        "waiting for old MCP endpoint to stop...",
    );
    let endpoint_state = wait_for_endpoint(
        endpoint_probe,
        mcp_url(request.edition),
        false,
        request.wait_timeout,
        request.wait_interval,
    );
    if endpoint_state.is_alive() {
        return Err(Chat2DbError::McpEndpointStillAlive {
            edition: request.edition.to_string(),
            url: mcp_url(request.edition).to_string(),
            waited_seconds: request.wait_timeout.as_secs(),
        });
    }

    Ok(())
}

fn start_app_for_settings_update<P, L>(
    request: &McpChangeRequest,
    process_probe: &P,
    lifecycle: &L,
) -> Result<()>
where
    P: ProcessProbe + ?Sized,
    L: McpAppLifecycle + ?Sized,
{
    print_progress(request.progress, request.edition, "starting app...");
    lifecycle.start_app(request.edition, request.paths.clone())?;

    print_progress(
        request.progress,
        request.edition,
        "waiting for app process...",
    );
    if !wait_for_app_process(
        process_probe,
        request.edition,
        &request.paths,
        true,
        request.wait_timeout,
        request.wait_interval,
    )? {
        return Err(Chat2DbError::McpAppNotStarted {
            edition: request.edition.to_string(),
            waited_seconds: request.wait_timeout.as_secs(),
        });
    }

    Ok(())
}

fn wait_for_app_process<P: ProcessProbe + ?Sized>(
    process_probe: &P,
    edition: Edition,
    paths: &SystemPaths,
    want_running: bool,
    wait_timeout: Duration,
    wait_interval: Duration,
) -> Result<bool> {
    let app_path = edition.app_path(&paths.applications_dir);
    let deadline = Instant::now() + wait_timeout;
    loop {
        if process_probe.is_running(&app_path)? == want_running {
            return Ok(true);
        }

        if Instant::now() >= deadline {
            return Ok(false);
        }

        sleep(wait_interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn wait_for_endpoint<E: McpEndpointProbe + ?Sized>(
    endpoint_probe: &E,
    url: &str,
    want_alive: bool,
    wait_timeout: Duration,
    wait_interval: Duration,
) -> EndpointState {
    let deadline = Instant::now() + wait_timeout;
    loop {
        let state = endpoint_probe.probe(url, DEFAULT_PROBE_TIMEOUT);
        if want_alive {
            if state == EndpointState::Alive {
                return state;
            }
        } else if !state.is_alive() {
            return state;
        }

        if Instant::now() >= deadline {
            return state;
        }

        sleep(wait_interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn wait_for_endpoint_stable<E: McpEndpointProbe + ?Sized>(
    endpoint_probe: &E,
    url: &str,
    want_alive: bool,
    wait_timeout: Duration,
    wait_interval: Duration,
    settle_duration: Duration,
) -> EndpointState {
    let deadline = Instant::now() + wait_timeout;
    let mut matching_since = None;
    loop {
        let state = endpoint_probe.probe(url, DEFAULT_PROBE_TIMEOUT);
        let matches_target = if want_alive {
            state == EndpointState::Alive
        } else {
            !state.is_alive()
        };

        if matches_target {
            let since = *matching_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= settle_duration {
                return state;
            }
        } else {
            matching_since = None;
        }

        if Instant::now() >= deadline {
            return state;
        }

        sleep(wait_interval.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn print_progress(enabled: bool, edition: Edition, message: &str) {
    if enabled {
        eprintln!("MCP {}: {message}", edition.app_name());
    }
}

fn write_target_settings<G: McpTokenGenerator + ?Sized>(
    request: &McpChangeRequest,
    enabled: bool,
    token_generator: Option<&G>,
) -> Result<()> {
    let mut settings = SettingsFile::read(request.edition, &request.paths.home_dir)?;
    settings.set_enabled(enabled);
    if let Some(token_generator) = token_generator {
        settings.ensure_token(token_generator)?;
    }
    settings.write()
}

fn sync_frontend_mcp_state(request: &McpChangeRequest, enabled: bool) -> Result<()> {
    let settings = SettingsFile::read(request.edition, &request.paths.home_dir)?;
    sync_mcp_enabled(
        request.edition,
        &request.paths.home_dir,
        enabled,
        FrontendStoreDefaults {
            appearance: settings
                .string_value("appearance")
                .unwrap_or("dark")
                .to_string(),
            language: settings
                .string_value("language")
                .unwrap_or("zh-CN")
                .to_string(),
        },
    )
}

#[derive(Clone)]
struct SettingsFile {
    path: PathBuf,
    values: Map<String, Value>,
}

impl fmt::Debug for SettingsFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut values = self.values.clone();
        if values.contains_key(MCP_AUTH_TOKEN_KEY) {
            values.insert(
                MCP_AUTH_TOKEN_KEY.to_string(),
                Value::String("<redacted>".to_string()),
            );
        }
        f.debug_struct("SettingsFile")
            .field("path", &self.path)
            .field("values", &values)
            .finish()
    }
}

impl SettingsFile {
    fn read(edition: Edition, home_dir: &Path) -> Result<Self> {
        let path = mcp_settings_path(edition, home_dir);
        let values = read_settings_map(&path)?;
        Ok(Self { path, values })
    }

    fn enabled(&self) -> bool {
        match self.values.get(ENABLE_MCP_KEY) {
            Some(Value::Bool(value)) => *value,
            Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
            _ => false,
        }
    }

    fn token_state(&self) -> TokenState {
        match self.values.get(MCP_AUTH_TOKEN_KEY).and_then(Value::as_str) {
            Some(token) if !token.trim().is_empty() => TokenState::Present,
            _ => TokenState::Missing,
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.values
            .insert(ENABLE_MCP_KEY.to_string(), Value::Bool(enabled));
    }

    fn string_value(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(Value::as_str)
    }

    fn ensure_token<G: McpTokenGenerator + ?Sized>(&mut self, generator: &G) -> Result<()> {
        if self.token_state() == TokenState::Present {
            return Ok(());
        }
        self.values.insert(
            MCP_AUTH_TOKEN_KEY.to_string(),
            Value::String(generator.generate_token()?),
        );
        Ok(())
    }

    fn write(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.values)?;
        write_private_atomic(&self.path, format!("{json}\n").as_bytes())
    }
}

pub fn read_mcp_auth_token(edition: Edition, home_dir: &Path) -> Result<Option<String>> {
    let settings = SettingsFile::read(edition, home_dir)?;
    Ok(settings
        .values
        .get(MCP_AUTH_TOKEN_KEY)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned))
}

fn read_settings_map(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }

    let content = fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(Map::new());
    }

    let value: Value =
        serde_json::from_str(&content).map_err(|source| Chat2DbError::McpSettingsInvalidJson {
            path: path.to_path_buf(),
            source,
        })?;
    let Some(object) = value.as_object() else {
        return Err(Chat2DbError::McpSettingsNotObject {
            path: path.to_path_buf(),
        });
    };
    Ok(object.clone())
}

fn probe_http_endpoint(url: &str, timeout: Duration) -> EndpointState {
    let Ok(parsed) = url::Url::parse(url) else {
        return EndpointState::NotListening;
    };
    let Some(host) = parsed.host_str() else {
        return EndpointState::NotListening;
    };
    let Some(port) = parsed.port_or_known_default() else {
        return EndpointState::NotListening;
    };
    let path = match parsed.query() {
        Some(query) => format!("{}?{query}", parsed.path()),
        None => parsed.path().to_string(),
    };
    let Ok(mut addrs) = (host, port).to_socket_addrs() else {
        return EndpointState::NotListening;
    };
    let Some(addr) = addrs.next() else {
        return EndpointState::NotListening;
    };

    let mut stream = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(stream) => stream,
        Err(error) => return connect_error_state(addr, &error),
    };

    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    if let Err(error) = stream.write_all(request.as_bytes()) {
        return if is_timeout(&error) {
            EndpointState::Timeout
        } else {
            EndpointState::NotListening
        };
    }

    let mut buffer = [0_u8; 128];
    match stream.read(&mut buffer) {
        Ok(0) => EndpointState::Alive,
        Ok(read) if is_http_status(&buffer[..read]) => EndpointState::Alive,
        Ok(_) => EndpointState::Alive,
        Err(error) if is_timeout(&error) => EndpointState::Timeout,
        Err(_) => EndpointState::NotListening,
    }
}

fn is_http_status(bytes: &[u8]) -> bool {
    let line = String::from_utf8_lossy(bytes);
    let Some(status) = line.lines().next() else {
        return false;
    };
    if !status.starts_with("HTTP/") {
        return false;
    }
    status
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|code| (100..600).contains(&code))
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn connect_error_state(addr: SocketAddr, error: &std::io::Error) -> EndpointState {
    // Windows may surface a closed loopback port as a connect timeout. Reserve
    // Timeout for remote connects or an established local socket that stalls.
    if is_timeout(error) && !addr.ip().is_loopback() {
        EndpointState::Timeout
    } else {
        EndpointState::NotListening
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::io::Write;
    use std::net::TcpListener;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::frontend_store::read_mcp_enabled_for_test;

    #[test]
    fn pro_and_local_mcp_transactions_share_lifecycle_locks() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let pro_locks = acquire_mcp_lifecycle_locks(&paths, Edition::Pro).unwrap();
        let thread_paths = paths.clone();
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        let handle = thread::spawn(move || {
            let locks = acquire_mcp_lifecycle_locks(&thread_paths, Edition::Local);
            result_tx.send(locks.map(|locks| locks.len())).unwrap();
        });

        assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(pro_locks);
        assert_eq!(
            result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap(),
            2
        );
        handle.join().unwrap();
    }

    #[derive(Debug)]
    struct FakeProcessProbe {
        pro_running: RefCell<Vec<bool>>,
        local_running: RefCell<Vec<bool>>,
    }

    impl FakeProcessProbe {
        fn new(pro_running: Vec<bool>, local_running: Vec<bool>) -> Self {
            Self {
                pro_running: RefCell::new(pro_running),
                local_running: RefCell::new(local_running),
            }
        }

        fn next_state(states: &RefCell<Vec<bool>>) -> bool {
            let mut states = states.borrow_mut();
            if states.len() > 1 {
                states.remove(0)
            } else {
                *states.first().unwrap_or(&false)
            }
        }
    }

    impl ProcessProbe for FakeProcessProbe {
        fn pids(&self, app_path: &Path) -> Result<Vec<u32>> {
            let running = if app_path_matches_edition(app_path, Edition::Pro) {
                Self::next_state(&self.pro_running)
            } else if app_path_matches_edition(app_path, Edition::Local) {
                Self::next_state(&self.local_running)
            } else {
                false
            };
            Ok(if running { vec![4242] } else { vec![] })
        }
    }

    fn app_path_matches_edition(app_path: &Path, edition: Edition) -> bool {
        let Some(file_name) = app_path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        let macos_name = format!("{}.app", edition.app_name());
        let dashed_windows_name = format!("{}.exe", edition.app_name().replace(' ', "-"));
        file_name == macos_name
            || file_name.eq_ignore_ascii_case(edition.windows_exe_name())
            || file_name.eq_ignore_ascii_case(&dashed_windows_name)
    }

    #[derive(Debug)]
    struct FakeEndpointProbe {
        states: RefCell<Vec<EndpointState>>,
    }

    impl FakeEndpointProbe {
        fn new(states: Vec<EndpointState>) -> Self {
            Self {
                states: RefCell::new(states),
            }
        }
    }

    impl McpEndpointProbe for FakeEndpointProbe {
        fn probe(&self, _url: &str, _timeout: Duration) -> EndpointState {
            let mut states = self.states.borrow_mut();
            if states.len() > 1 {
                states.remove(0)
            } else {
                *states.first().unwrap_or(&EndpointState::NotListening)
            }
        }
    }

    #[derive(Debug, Default)]
    struct FakeAppLifecycle {
        events: RefCell<Vec<&'static str>>,
        stop_calls: RefCell<Vec<Edition>>,
        start_calls: RefCell<Vec<Edition>>,
        expected_enabled_at_stop: Option<bool>,
        expected_enabled_at_start: Option<bool>,
    }

    impl FakeAppLifecycle {
        fn expecting_settings(stop_enabled: bool, start_enabled: bool) -> Self {
            Self {
                expected_enabled_at_stop: Some(stop_enabled),
                expected_enabled_at_start: Some(start_enabled),
                ..Self::default()
            }
        }

        fn assert_settings(
            expected: Option<bool>,
            edition: Edition,
            paths: &SystemPaths,
        ) -> Result<()> {
            if let Some(expected) = expected {
                let settings = SettingsFile::read(edition, &paths.home_dir)?;
                assert_eq!(settings.enabled(), expected);
            }
            Ok(())
        }
    }

    impl McpAppLifecycle for FakeAppLifecycle {
        fn stop_app(&self, edition: Edition, paths: SystemPaths) -> Result<StopOutcome> {
            Self::assert_settings(self.expected_enabled_at_stop, edition, &paths)?;
            self.events.borrow_mut().push("stop");
            self.stop_calls.borrow_mut().push(edition);
            Ok(StopOutcome {
                command: "stop",
                edition,
                status: "stopped",
                app_path: PathBuf::from(format!("/Applications/{}.app", edition.app_name())),
            })
        }

        fn start_app(&self, edition: Edition, paths: SystemPaths) -> Result<StartOutcome> {
            Self::assert_settings(self.expected_enabled_at_start, edition, &paths)?;
            self.events.borrow_mut().push("start");
            self.start_calls.borrow_mut().push(edition);
            Ok(StartOutcome {
                command: "start",
                edition,
                status: "opened",
                app_path: PathBuf::from(format!("/Applications/{}.app", edition.app_name())),
            })
        }
    }

    #[derive(Debug)]
    struct FakeTokenGenerator(&'static str);

    impl McpTokenGenerator for FakeTokenGenerator {
        fn generate_token(&self) -> Result<String> {
            Ok(self.0.to_string())
        }
    }

    #[test]
    fn settings_missing_file_is_created_on_enable() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let mut settings = SettingsFile::read(Edition::Pro, &paths.home_dir).unwrap();

        settings.set_enabled(true);
        settings
            .ensure_token(&FakeTokenGenerator("0123456789abcdef0123456789abcdef"))
            .unwrap();
        settings.write().unwrap();

        let path = mcp_settings_path(Edition::Pro, &paths.home_dir);
        let json: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json["enableMcp"], true);
        assert_eq!(json["mcpAuthToken"], "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn settings_preserve_unknown_fields_and_only_update_enable_flag() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let path = mcp_settings_path(Edition::Local, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"appearance":"dark","enableMcp":false,"mcpAuthToken":"keep"}"#,
        )
        .unwrap();

        let mut settings = SettingsFile::read(Edition::Local, &paths.home_dir).unwrap();
        settings.set_enabled(true);
        settings
            .ensure_token(&FakeTokenGenerator("new-token"))
            .unwrap();
        settings.write().unwrap();

        let json: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json["appearance"], "dark");
        assert_eq!(json["enableMcp"], true);
        assert_eq!(json["mcpAuthToken"], "keep");
    }

    #[cfg(unix)]
    #[test]
    fn settings_write_restricts_existing_file_to_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let path = mcp_settings_path(Edition::Pro, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"enableMcp":false,"mcpAuthToken":"secret"}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let mut settings = SettingsFile::read(Edition::Pro, &paths.home_dir).unwrap();
        settings.set_enabled(true);
        settings.write().unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let json: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json["enableMcp"], true);
        assert_eq!(json["mcpAuthToken"], "secret");
    }

    #[test]
    fn invalid_settings_json_returns_stable_error() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let path = mcp_settings_path(Edition::Pro, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{not-json").unwrap();

        let error = SettingsFile::read(Edition::Pro, &paths.home_dir).unwrap_err();

        assert!(matches!(error, Chat2DbError::McpSettingsInvalidJson { .. }));
        assert_eq!(error.code(), "mcp_settings_invalid_json");
    }

    #[test]
    fn non_object_settings_json_returns_stable_error() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let path = mcp_settings_path(Edition::Local, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[]").unwrap();

        let error = SettingsFile::read(Edition::Local, &paths.home_dir).unwrap_err();

        assert!(matches!(error, Chat2DbError::McpSettingsNotObject { .. }));
        assert_eq!(error.code(), "mcp_settings_not_object");
    }

    #[test]
    fn system_token_generator_returns_32_hex_chars() {
        let token = SystemTokenGenerator.generate_token().unwrap();

        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn fake_process_probe_recognizes_host_specific_app_paths() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let probe = FakeProcessProbe::new(vec![true], vec![true]);

        assert_eq!(
            probe
                .pids(&Edition::Pro.app_path(&paths.applications_dir))
                .unwrap(),
            vec![4242]
        );
        assert_eq!(
            probe
                .pids(&Edition::Local.app_path(&paths.applications_dir))
                .unwrap(),
            vec![4242]
        );
        assert!(
            probe
                .pids(&Edition::Community.app_path(&paths.applications_dir))
                .unwrap()
                .is_empty()
        );
        assert!(app_path_matches_edition(
            Path::new("C:/Chat2DB/Chat2DB Pro/Chat2DB Pro.exe"),
            Edition::Pro
        ));
        assert!(app_path_matches_edition(
            Path::new("C:/Chat2DB/Chat2DB-Local/Chat2DB-Local.exe"),
            Edition::Local
        ));
    }

    #[test]
    fn loopback_connect_timeout_is_not_listening_but_remote_timeout_is_timeout() {
        let timeout = std::io::Error::from(std::io::ErrorKind::TimedOut);

        assert_eq!(
            connect_error_state("127.0.0.1:11924".parse().unwrap(), &timeout),
            EndpointState::NotListening
        );
        assert_eq!(
            connect_error_state("192.0.2.1:11924".parse().unwrap(), &timeout),
            EndpointState::Timeout
        );
    }

    #[test]
    fn probe_reports_connection_refused_as_not_listening() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        drop(listener);

        let state = TcpEndpointProbe.probe(&url, Duration::from_millis(100));

        assert_eq!(state, EndpointState::NotListening);
    }

    #[test]
    fn probe_reports_http_statuses_as_alive() {
        for status in [200, 401, 404, 405, 500] {
            let (url, handle) = one_shot_http_server(status, Duration::ZERO);

            let state = TcpEndpointProbe.probe(&url, Duration::from_secs(1));
            handle.join().unwrap();

            assert_eq!(state, EndpointState::Alive, "status {status}");
        }
    }

    #[test]
    fn probe_reports_unresponsive_listener_as_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(150));
        });

        let state = TcpEndpointProbe.probe(&url, Duration::from_millis(30));
        handle.join().unwrap();

        assert_eq!(state, EndpointState::Timeout);
    }

    #[test]
    fn enable_writes_settings_generates_token_restarts_and_waits() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let process_probe = FakeProcessProbe::new(vec![false, true, true], vec![false, false]);
        let endpoint_probe = FakeEndpointProbe::new(vec![
            EndpointState::NotListening,
            EndpointState::Alive,
            EndpointState::Alive,
        ]);
        let lifecycle = FakeAppLifecycle::expecting_settings(false, true);

        let outcome = mcp_enable_with(
            test_change_request(Edition::Pro, paths.clone()),
            &process_probe,
            &endpoint_probe,
            &lifecycle,
            &FakeTokenGenerator("0123456789abcdef0123456789abcdef"),
        )
        .unwrap();

        assert_eq!(outcome.command, "mcp_enable");
        assert_eq!(lifecycle.events.borrow().as_slice(), &["stop", "start"]);
        assert_eq!(lifecycle.stop_calls.borrow().as_slice(), &[Edition::Pro]);
        assert_eq!(lifecycle.start_calls.borrow().as_slice(), &[Edition::Pro]);
        let settings = SettingsFile::read(Edition::Pro, &paths.home_dir).unwrap();
        assert!(settings.enabled());
        assert_eq!(settings.token_state(), TokenState::Present);
        assert_eq!(
            read_mcp_enabled_for_test(Edition::Pro, &paths.home_dir).unwrap(),
            Some(true)
        );
        assert_eq!(outcome.editions[0].endpoint_state, EndpointState::Alive);
    }

    #[test]
    fn disable_preserves_token_restarts_and_waits_until_endpoint_stops() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let path = mcp_settings_path(Edition::Pro, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"enableMcp":true,"mcpAuthToken":"keep-token","appearance":"dark"}"#,
        )
        .unwrap();
        let process_probe = FakeProcessProbe::new(vec![false, true, true], vec![false, false]);
        let endpoint_probe = FakeEndpointProbe::new(vec![
            EndpointState::Alive,
            EndpointState::NotListening,
            EndpointState::NotListening,
        ]);
        let lifecycle = FakeAppLifecycle::expecting_settings(true, false);

        let outcome = mcp_disable_with(
            test_change_request(Edition::Pro, paths.clone()),
            &process_probe,
            &endpoint_probe,
            &lifecycle,
        )
        .unwrap();

        assert_eq!(outcome.command, "mcp_disable");
        assert_eq!(lifecycle.events.borrow().as_slice(), &["stop", "start"]);
        assert_eq!(lifecycle.stop_calls.borrow().as_slice(), &[Edition::Pro]);
        assert_eq!(lifecycle.start_calls.borrow().as_slice(), &[Edition::Pro]);
        let json: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(json["enableMcp"], false);
        assert_eq!(json["mcpAuthToken"], "keep-token");
        assert_eq!(json["appearance"], "dark");
        assert_eq!(
            read_mcp_enabled_for_test(Edition::Pro, &paths.home_dir).unwrap(),
            Some(false)
        );
    }

    #[test]
    fn restart_does_not_restart_when_mcp_is_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let path = mcp_settings_path(Edition::Local, &paths.home_dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"enableMcp":false}"#).unwrap();
        let process_probe = FakeProcessProbe::new(vec![false], vec![true]);
        let endpoint_probe = FakeEndpointProbe::new(vec![EndpointState::NotListening]);
        let lifecycle = FakeAppLifecycle::default();

        let outcome = mcp_restart_with(
            test_change_request(Edition::Local, paths),
            &process_probe,
            &endpoint_probe,
            &lifecycle,
        )
        .unwrap();

        assert_eq!(outcome.command, "mcp_restart");
        assert!(lifecycle.events.borrow().is_empty());
        assert!(lifecycle.stop_calls.borrow().is_empty());
        assert!(lifecycle.start_calls.borrow().is_empty());
        assert_eq!(outcome.editions[0].next_action, McpNextAction::Enable);
    }

    #[test]
    fn mutating_command_refuses_when_other_edition_is_running() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let process_probe = FakeProcessProbe::new(vec![false], vec![true]);
        let endpoint_probe = FakeEndpointProbe::new(vec![EndpointState::NotListening]);
        let lifecycle = FakeAppLifecycle::default();

        let error = mcp_enable_with(
            test_change_request(Edition::Pro, paths),
            &process_probe,
            &endpoint_probe,
            &lifecycle,
            &FakeTokenGenerator("unused"),
        )
        .unwrap_err();

        assert!(matches!(error, Chat2DbError::McpPortAmbiguous { .. }));
        assert_eq!(error.code(), "mcp_port_ambiguous");
        assert!(lifecycle.events.borrow().is_empty());
        assert!(lifecycle.stop_calls.borrow().is_empty());
        assert!(lifecycle.start_calls.borrow().is_empty());
    }

    #[test]
    fn status_marks_endpoint_ambiguous_when_other_edition_is_running() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let process_probe = FakeProcessProbe::new(vec![true], vec![true]);
        let endpoint_probe = FakeEndpointProbe::new(vec![EndpointState::Alive]);

        let outcome = mcp_status_with(
            McpStatusRequest {
                edition: EditionSelector::All,
                paths,
            },
            &process_probe,
            &endpoint_probe,
        )
        .unwrap();

        assert_eq!(outcome.editions.len(), 3);
        let community = outcome
            .editions
            .iter()
            .find(|status| status.edition == Edition::Community)
            .unwrap();
        assert_eq!(community.endpoint_state, EndpointState::Alive);
        for edition in [Edition::Pro, Edition::Local] {
            let status = outcome
                .editions
                .iter()
                .find(|status| status.edition == edition)
                .unwrap();
            assert_eq!(status.endpoint_state, EndpointState::Ambiguous);
            assert_eq!(status.next_action, McpNextAction::StopOtherEdition);
        }
    }

    fn one_shot_http_server(status: u16, delay: Duration) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            if delay > Duration::ZERO {
                thread::sleep(delay);
            }
            let response = format!("HTTP/1.1 {status} OK\r\nContent-Length: 0\r\n\r\n");
            stream.write_all(response.as_bytes()).unwrap();
        });
        (url, handle)
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

    fn test_change_request(edition: Edition, paths: SystemPaths) -> McpChangeRequest {
        McpChangeRequest {
            edition,
            paths,
            wait_timeout: Duration::from_millis(50),
            wait_interval: Duration::from_millis(1),
            settle_duration: Duration::ZERO,
            progress: false,
        }
    }
}
