use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::app_plist::read_app_bundle_metadata;
use crate::background::{BackgroundState, log_path_in, read_state_in};
use crate::edition::{Edition, EditionSelector};
use crate::installer::{SystemPaths, verify_app_bundle};
use crate::mcp::{
    EndpointState, McpEditionStatus, McpEndpointProbe, McpStatusRequest, TcpEndpointProbe,
    mcp_status_with,
};
use crate::process::{PgrepProbe, ProcessProbe, ProcessReport, ProcessState, process_report};
use crate::runtime::{RuntimeRequest, RuntimeStateKind, runtime_status};
use crate::skill::{SKILL_DIR_NAME, SKILL_FRONTMATTER_NAME, SkillClient};

const PROFILE: &str = "release";
pub(crate) const CHAT2DB_COOKIE: &str = "CHAT2DB";
pub(crate) const USER_ID_COOKIE: &str = "Chat2db-BS-TOKEN";
pub(crate) const ORGANIZATION_TOKEN_COOKIE: &str = "Chat2db-Organization-Token";
pub(crate) const ORGANIZATION_ID_COOKIE: &str = "Chat2db-Organization-Id";
pub(crate) const LOCAL_LICENSE_COOKIE: &str = "AAAAAAAAAAAAAAAA";
pub(crate) const LOCAL_CER_COOKIE: &str = "BBBBBBBBBBBBBBBB";
const AUTH_COOKIES: &[&str] = &[
    CHAT2DB_COOKIE,
    USER_ID_COOKIE,
    ORGANIZATION_TOKEN_COOKIE,
    ORGANIZATION_ID_COOKIE,
    LOCAL_LICENSE_COOKIE,
    LOCAL_CER_COOKIE,
];

#[derive(Debug, Clone)]
pub struct StatusRequest {
    pub paths: SystemPaths,
}

impl StatusRequest {
    pub fn production() -> Self {
        Self {
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusOutcome {
    pub command: &'static str,
    pub cli_version: &'static str,
    pub summary: StatusRuntimeSummary,
    #[serde(skip_serializing)]
    pub editions: Vec<EditionStatus>,
    #[serde(skip_serializing)]
    pub mcp: StatusMcpReport,
    #[serde(skip_serializing)]
    pub skills: SkillStatusReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusRuntimeSummary {
    pub community: EditionRuntimeSummary,
    pub pro: EditionRuntimeSummary,
    pub local: EditionRuntimeSummary,
}

impl StatusRuntimeSummary {
    pub fn for_edition(&self, edition: Edition) -> &EditionRuntimeSummary {
        match edition {
            Edition::Community => &self.community,
            Edition::Pro => &self.pro,
            Edition::Local => &self.local,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EditionRuntimeSummary {
    pub app: StatusAppSummary,
    pub cli: StatusCliRuntimeSummary,
    pub mcp: StatusMcpSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_install: Option<BackgroundReport>,
    pub safe_to_exit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusAppSummary {
    pub state: StatusAppState,
    pub pids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusAppState {
    Running,
    Stopped,
    NotInstalled,
    Invalid,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusCliRuntimeSummary {
    pub state: StatusCliRuntimeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusCliRuntimeState {
    Running,
    Stopped,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusMcpSummary {
    pub enabled: bool,
    pub endpoint: EndpointState,
    pub needs_restart: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditionStatus {
    pub edition: Edition,
    pub app_name: &'static str,
    pub summary: StatusSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<NextAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action_command: Option<String>,
    pub app_path: PathBuf,
    pub bundle_id: &'static str,
    pub installed: bool,
    pub install_state: InstallState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_error: Option<String>,
    pub app: AppReport,
    pub process: ProcessReport,
    pub data_dir: DataDirReport,
    pub cookie_file: CookieFileReport,
    pub login: LoginStatus,
    pub runtime_log: RuntimeLogReport,
    /// Set when a `chat2db install --background` for this edition is currently
    /// running or recently finished. The completed state lingers until the
    /// next background install starts (which truncates the marker files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_install: Option<BackgroundReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundReport {
    #[serde(flatten)]
    pub state: BackgroundState,
    pub log_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLogReport {
    pub path: PathBuf,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppReport {
    pub path: PathBuf,
    pub expected_bundle_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub install_state: InstallState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DataDirReport {
    pub path: PathBuf,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CookieFileReport {
    pub path: PathBuf,
    pub state: CookieFileState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CookieFileState {
    NotApplicable,
    Exists,
    Missing,
    InvalidJson,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSummary {
    Ready,
    NotInstalled,
    InvalidApp,
    Stopped,
    NeedsLogin,
    Installing,
    InstallFailed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextAction {
    Install,
    Start,
    Login,
    TailInstallLog,
    InspectRuntimeLog,
    InspectInvalidApp,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusMcpReport {
    pub command: &'static str,
    pub editions: Vec<McpEditionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<StatusErrorReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusErrorReport {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillStatusReport {
    pub targets: Vec<SkillTargetStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillTargetStatus {
    pub target: SkillStatusTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<SkillClient>,
    pub path: PathBuf,
    pub state: SkillInstallState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_cli_versions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_version: Option<String>,
    pub next_action_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatusTarget {
    Codex,
    ClaudeCode,
    CurrentDirectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInstallState {
    Installed,
    NotInstalled,
    ManagedExternally,
    Foreign,
    Invalid,
}

#[derive(Debug, Clone)]
struct SkillFrontmatter {
    name: Option<String>,
    version: Option<String>,
    compatible_cli_versions: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallState {
    Installed,
    NotInstalled,
    Invalid,
}

fn aggregate_mcp_status<P, E>(
    paths: &SystemPaths,
    process_probe: &P,
    endpoint_probe: &E,
) -> StatusMcpReport
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
{
    match mcp_status_with(
        McpStatusRequest {
            edition: EditionSelector::All,
            paths: paths.clone(),
        },
        process_probe,
        endpoint_probe,
    ) {
        Ok(outcome) => StatusMcpReport {
            command: outcome.command,
            editions: outcome.editions,
            error: None,
        },
        Err(error) => StatusMcpReport {
            command: "mcp_status",
            editions: Vec::new(),
            error: Some(StatusErrorReport {
                code: error.code().to_string(),
                message: error.to_string(),
            }),
        },
    }
}

fn aggregate_skill_status(paths: &SystemPaths) -> SkillStatusReport {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    SkillStatusReport {
        targets: vec![
            skill_target_status(
                SkillStatusTarget::Codex,
                Some(SkillClient::Codex),
                SkillClient::Codex.target_dir(&paths.home_dir),
            ),
            skill_target_status(
                SkillStatusTarget::ClaudeCode,
                Some(SkillClient::ClaudeCode),
                SkillClient::ClaudeCode.target_dir(&paths.home_dir),
            ),
            skill_target_status(
                SkillStatusTarget::CurrentDirectory,
                None,
                current_dir.join(SKILL_DIR_NAME),
            ),
        ],
    }
}

fn skill_target_status(
    target: SkillStatusTarget,
    client: Option<SkillClient>,
    path: PathBuf,
) -> SkillTargetStatus {
    let (state, frontmatter, error) = classify_skill_target(&path);
    SkillTargetStatus {
        target,
        client,
        path,
        state,
        compatible_cli_versions: frontmatter
            .as_ref()
            .and_then(|frontmatter| frontmatter.compatible_cli_versions.clone()),
        skill_version: frontmatter
            .as_ref()
            .and_then(|frontmatter| frontmatter.version.clone()),
        next_action_command: skill_next_action_command(client, state),
        error,
    }
}

fn classify_skill_target(
    target: &Path,
) -> (SkillInstallState, Option<SkillFrontmatter>, Option<String>) {
    if !target.exists() {
        return (SkillInstallState::NotInstalled, None, None);
    }
    if !target.is_dir() {
        return (SkillInstallState::Foreign, None, None);
    }
    if target.join(".git").exists() {
        return (SkillInstallState::ManagedExternally, None, None);
    }

    let skill_md = target.join("SKILL.md");
    if !skill_md.is_file() {
        return (SkillInstallState::Foreign, None, None);
    }

    match read_skill_frontmatter(&skill_md) {
        Ok(frontmatter) => {
            let state = if frontmatter.name.as_deref() == Some(SKILL_FRONTMATTER_NAME) {
                SkillInstallState::Installed
            } else {
                SkillInstallState::Foreign
            };
            (state, Some(frontmatter), None)
        }
        Err(error) => (SkillInstallState::Invalid, None, Some(error)),
    }
}

fn read_skill_frontmatter(path: &Path) -> std::result::Result<SkillFrontmatter, String> {
    let mut file = fs::File::open(path)
        .map_err(|source| format!("failed to open {}: {source}", path.display()))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|source| format!("failed to read {}: {source}", path.display()))?;
    parse_skill_frontmatter(&buf)
        .ok_or_else(|| format!("could not parse SKILL.md frontmatter at {}", path.display()))
}

fn parse_skill_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    let body = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))?;
    let end_idx = body.find("\n---").or_else(|| body.find("\r\n---"))?;
    let yaml = &body[..end_idx];

    let mut name = None;
    let mut version = None;
    let mut compatible_cli_versions = None;
    for line in yaml.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(idx) = line.find(':') else {
            continue;
        };
        let key = line[..idx].trim();
        let value = line[idx + 1..].trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value)
            .to_string();
        match key {
            "name" => name = Some(value),
            "version" => version = Some(value),
            "compatible-cli-versions" => compatible_cli_versions = Some(value),
            _ => {}
        }
    }

    Some(SkillFrontmatter {
        name,
        version,
        compatible_cli_versions,
    })
}

fn skill_next_action_command(client: Option<SkillClient>, state: SkillInstallState) -> String {
    if matches!(
        state,
        SkillInstallState::Installed | SkillInstallState::ManagedExternally
    ) {
        return match client {
            Some(SkillClient::Codex) => "chat2db skill update --client codex --json".to_string(),
            Some(SkillClient::ClaudeCode) => {
                "chat2db skill update --client claude-code --json".to_string()
            }
            None => "chat2db skill update --json".to_string(),
        };
    }

    match client {
        Some(SkillClient::Codex) => "chat2db skill install --client codex --json".to_string(),
        Some(SkillClient::ClaudeCode) => {
            "chat2db skill install --client claude-code --json".to_string()
        }
        None => "chat2db skill install --json".to_string(),
    }
}

fn next_action_command(
    action: NextAction,
    edition: Edition,
    app_path: &Path,
    runtime_log: &RuntimeLogReport,
) -> String {
    match action {
        NextAction::Install => format!("chat2db install --edition {} --json", edition.as_str()),
        NextAction::Start => format!("chat2db start --edition {} --json", edition.as_str()),
        NextAction::Login => format!("chat2db login --edition {} --json", edition.as_str()),
        NextAction::TailInstallLog => {
            format!(
                "chat2db logs install --edition {} --follow",
                edition.as_str()
            )
        }
        NextAction::InspectRuntimeLog => {
            format!("tail -n 200 {}", runtime_log.path.display())
        }
        NextAction::InspectInvalidApp => {
            format!("inspect invalid app at {}", app_path.display())
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginStatus {
    pub state: LoginState,
    pub source: &'static str,
    #[serde(skip_serializing)]
    pub markers: Vec<&'static str>,
    #[serde(skip_serializing)]
    pub checked_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    NotApplicable,
    LoggedIn,
    NotLoggedIn,
    Unknown,
}

pub fn check_status(request: StatusRequest) -> StatusOutcome {
    check_status_with_process_probe(request, &PgrepProbe)
}

pub fn check_status_with_process_probe<P: ProcessProbe + ?Sized>(
    request: StatusRequest,
    probe: &P,
) -> StatusOutcome {
    check_status_with_probes(request, probe, &TcpEndpointProbe)
}

fn check_status_with_probes<P, E>(
    request: StatusRequest,
    process_probe: &P,
    endpoint_probe: &E,
) -> StatusOutcome
where
    P: ProcessProbe + ?Sized,
    E: McpEndpointProbe + ?Sized,
{
    let editions = Edition::ALL
        .into_iter()
        .map(|edition| {
            check_edition_status_with_process_probe(edition, &request.paths, process_probe)
        })
        .collect::<Vec<_>>();
    let mcp = aggregate_mcp_status(&request.paths, process_probe, endpoint_probe);
    let summary = build_runtime_summary(&request.paths, &editions, &mcp);

    StatusOutcome {
        command: "status",
        cli_version: env!("CARGO_PKG_VERSION"),
        summary,
        editions,
        mcp,
        skills: aggregate_skill_status(&request.paths),
    }
}

fn build_runtime_summary(
    paths: &SystemPaths,
    editions: &[EditionStatus],
    mcp: &StatusMcpReport,
) -> StatusRuntimeSummary {
    StatusRuntimeSummary {
        community: build_edition_runtime_summary(Edition::Community, paths, editions, mcp),
        pro: build_edition_runtime_summary(Edition::Pro, paths, editions, mcp),
        local: build_edition_runtime_summary(Edition::Local, paths, editions, mcp),
    }
}

fn build_edition_runtime_summary(
    edition: Edition,
    paths: &SystemPaths,
    editions: &[EditionStatus],
    mcp: &StatusMcpReport,
) -> EditionRuntimeSummary {
    let edition_status = editions.iter().find(|status| status.edition == edition);
    let app = edition_status.map(app_summary).unwrap_or(StatusAppSummary {
        state: StatusAppState::Unknown,
        pids: Vec::new(),
    });
    let background_install = edition_status.and_then(|status| status.background_install.clone());
    let cli = cli_runtime_summary(edition, paths);
    let mcp = mcp_summary(edition, mcp);
    let background_requires_attention = background_install.as_ref().is_some_and(|background| {
        matches!(&background.state, BackgroundState::Running { .. })
            || matches!(&background.state, BackgroundState::Unknown { .. })
            || matches!(
                &background.state,
                BackgroundState::Completed { exit_code } if *exit_code != 0
            )
    });
    let safe_to_exit = matches!(
        app.state,
        StatusAppState::Stopped | StatusAppState::NotInstalled
    ) && cli.state == StatusCliRuntimeState::Stopped
        && !mcp.enabled
        && !mcp.endpoint.is_alive()
        && !background_requires_attention;
    let next_action = if matches!(
        background_install
            .as_ref()
            .map(|background| &background.state),
        Some(BackgroundState::Running { .. })
    ) {
        Some(format!(
            "chat2db logs install --edition {} --follow",
            edition.as_str()
        ))
    } else if matches!(
        background_install.as_ref().map(|background| &background.state),
        Some(BackgroundState::Completed { exit_code }) if *exit_code != 0
    ) || matches!(
        background_install
            .as_ref()
            .map(|background| &background.state),
        Some(BackgroundState::Unknown { .. })
    ) {
        Some(format!(
            "chat2db logs install --edition {}",
            edition.as_str()
        ))
    } else if app.state == StatusAppState::NotInstalled {
        Some(format!(
            "chat2db install --edition {} --json",
            edition.as_str()
        ))
    } else if safe_to_exit {
        None
    } else {
        Some(format!(
            "chat2db stop --edition {} --all --json",
            edition.as_str()
        ))
    };

    EditionRuntimeSummary {
        app,
        cli,
        mcp,
        background_install,
        safe_to_exit,
        next_action,
    }
}

fn app_summary(status: &EditionStatus) -> StatusAppSummary {
    let state = match (status.install_state, status.process.state) {
        (_, ProcessState::Running) => StatusAppState::Running,
        (InstallState::NotInstalled, _) => StatusAppState::NotInstalled,
        (InstallState::Invalid, _) => StatusAppState::Invalid,
        (_, ProcessState::Stopped) => StatusAppState::Stopped,
        (_, ProcessState::Unknown) => StatusAppState::Unknown,
    };
    StatusAppSummary {
        state,
        pids: status.process.pids.clone(),
    }
}

fn cli_runtime_summary(edition: Edition, paths: &SystemPaths) -> StatusCliRuntimeSummary {
    match runtime_status(RuntimeRequest {
        edition,
        paths: paths.clone(),
    }) {
        Ok(outcome) => StatusCliRuntimeSummary {
            state: match outcome.status {
                RuntimeStateKind::Running | RuntimeStateKind::Started => {
                    StatusCliRuntimeState::Running
                }
                RuntimeStateKind::Stopped => StatusCliRuntimeState::Stopped,
                RuntimeStateKind::Stale => StatusCliRuntimeState::Stale,
            },
            pid: outcome.pid,
            port: outcome.port,
            url: outcome.url,
            error: None,
        },
        Err(error) => StatusCliRuntimeSummary {
            state: StatusCliRuntimeState::Unknown,
            pid: None,
            port: None,
            url: None,
            error: Some(error.to_string()),
        },
    }
}

fn mcp_summary(edition: Edition, report: &StatusMcpReport) -> StatusMcpSummary {
    match report
        .editions
        .iter()
        .find(|status| status.edition == edition)
    {
        Some(status) => StatusMcpSummary {
            enabled: status.enabled,
            endpoint: status.endpoint_state,
            needs_restart: status.needs_restart,
            error: None,
        },
        None => StatusMcpSummary {
            enabled: false,
            endpoint: EndpointState::NotChecked,
            needs_restart: false,
            error: report.error.as_ref().map(|error| error.message.clone()),
        },
    }
}

pub fn check_edition_status(edition: Edition, paths: &SystemPaths) -> EditionStatus {
    check_edition_status_with_process_probe(edition, paths, &PgrepProbe)
}

pub fn check_edition_status_with_process_probe<P: ProcessProbe + ?Sized>(
    edition: Edition,
    paths: &SystemPaths,
    probe: &P,
) -> EditionStatus {
    let app_path = edition.app_path(&paths.applications_dir);
    let app = bundle_metadata(&app_path, edition);
    let installed = app.install_state == InstallState::Installed;
    let install_state = app.install_state;
    let install_error = app.error.clone();
    let process = process_report(probe, &app_path);
    let data_dir = data_dir_report(edition, &paths.home_dir);
    let (cookie_file, login) = if edition.requires_login() {
        let (cookie_file, cookie_json) = cookie_file_report(edition, &paths.home_dir);
        let login = detect_login_status(
            edition,
            &data_dir,
            cookie_file.state,
            &cookie_file.path,
            cookie_json.as_ref(),
        );
        (cookie_file, login)
    } else {
        (
            CookieFileReport {
                path: login_cookie_file_path(edition, &paths.home_dir),
                state: CookieFileState::NotApplicable,
            },
            LoginStatus {
                state: LoginState::NotApplicable,
                source: "not_applicable",
                markers: Vec::new(),
                checked_paths: Vec::new(),
            },
        )
    };
    let runtime_log = runtime_log_report(edition, &paths.home_dir);

    let background_install = match read_state_in(&paths.background_state_dir, edition) {
        BackgroundState::NotRunning => None,
        state => Some(BackgroundReport {
            state,
            log_path: log_path_in(&paths.background_state_dir, edition),
        }),
    };
    let (summary, next_action) = derive_summary_and_next_action(
        install_state,
        &process,
        login.state,
        background_install.as_ref(),
    );

    EditionStatus {
        edition,
        app_name: edition.app_name(),
        summary,
        next_action,
        next_action_command: next_action
            .map(|action| next_action_command(action, edition, &app_path, &runtime_log)),
        app_path,
        bundle_id: edition.bundle_id(),
        installed,
        install_state,
        install_error,
        app,
        process,
        data_dir,
        cookie_file,
        login,
        runtime_log,
        background_install,
    }
}

fn detect_login_status(
    edition: Edition,
    data_dir: &DataDirReport,
    cookie_state: CookieFileState,
    cookie_file: &Path,
    cookie_json: Option<&Value>,
) -> LoginStatus {
    let checked_paths = vec![cookie_file.to_path_buf()];

    let Some(cookies) = cookie_json else {
        return LoginStatus {
            state: match cookie_state {
                CookieFileState::NotApplicable => LoginState::NotApplicable,
                CookieFileState::Missing if data_dir.exists => LoginState::NotLoggedIn,
                CookieFileState::Missing => LoginState::Unknown,
                CookieFileState::Exists => LoginState::Unknown,
                CookieFileState::InvalidJson | CookieFileState::Unreadable => LoginState::Unknown,
            },
            source: "local_cookie_file",
            markers: Vec::new(),
            checked_paths,
        };
    };

    let mut markers = present_cookie_markers(cookies);
    markers.sort_unstable();
    markers.dedup();

    let state = if is_logged_in(edition, cookies) {
        LoginState::LoggedIn
    } else {
        LoginState::NotLoggedIn
    };

    LoginStatus {
        state,
        source: "local_cookie_file",
        markers,
        checked_paths,
    }
}

fn bundle_metadata(app_path: &Path, edition: Edition) -> AppReport {
    if !app_path.exists() {
        return AppReport {
            path: app_path.to_path_buf(),
            expected_bundle_id: edition.bundle_id(),
            bundle_id: None,
            version: None,
            executable: None,
            install_state: InstallState::NotInstalled,
            error: None,
        };
    }

    if app_path.extension().and_then(|ext| ext.to_str()) == Some("exe") {
        return windows_app_metadata(app_path, edition);
    }

    let info_plist = app_path.join("Contents/Info.plist");
    let metadata = read_app_bundle_metadata(&info_plist).ok();
    let (bundle_id, version, executable) = metadata
        .map(|metadata| (metadata.bundle_id, metadata.version, metadata.executable))
        .unwrap_or((None, None, None));

    match verify_app_bundle(app_path, edition) {
        Ok(()) => AppReport {
            path: app_path.to_path_buf(),
            expected_bundle_id: edition.bundle_id(),
            bundle_id,
            version,
            executable,
            install_state: InstallState::Installed,
            error: None,
        },
        Err(error) => AppReport {
            path: app_path.to_path_buf(),
            expected_bundle_id: edition.bundle_id(),
            bundle_id,
            version,
            executable,
            install_state: InstallState::Invalid,
            error: Some(error.to_string()),
        },
    }
}

fn windows_app_metadata(app_path: &Path, edition: Edition) -> AppReport {
    match verify_app_bundle(app_path, edition) {
        Ok(()) => AppReport {
            path: app_path.to_path_buf(),
            expected_bundle_id: edition.bundle_id(),
            bundle_id: None,
            version: None,
            executable: app_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned),
            install_state: InstallState::Installed,
            error: None,
        },
        Err(error) => AppReport {
            path: app_path.to_path_buf(),
            expected_bundle_id: edition.bundle_id(),
            bundle_id: None,
            version: None,
            executable: app_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned),
            install_state: InstallState::Invalid,
            error: Some(error.to_string()),
        },
    }
}

fn data_dir_report(edition: Edition, home_dir: &Path) -> DataDirReport {
    let path = chat2db_base_path(edition, home_dir);
    DataDirReport {
        exists: path.exists(),
        path,
    }
}

fn cookie_file_report(edition: Edition, home_dir: &Path) -> (CookieFileReport, Option<Value>) {
    let path = login_cookie_file_path(edition, home_dir);
    if !path.exists() {
        return (
            CookieFileReport {
                path,
                state: CookieFileState::Missing,
            },
            None,
        );
    }

    let Ok(cookie_bytes) = fs::read(&path) else {
        return (
            CookieFileReport {
                path,
                state: CookieFileState::Unreadable,
            },
            None,
        );
    };

    match serde_json::from_slice::<Value>(&cookie_bytes) {
        Ok(value) => (
            CookieFileReport {
                path,
                state: CookieFileState::Exists,
            },
            Some(value),
        ),
        Err(_) => (
            CookieFileReport {
                path,
                state: CookieFileState::InvalidJson,
            },
            None,
        ),
    }
}

fn derive_summary_and_next_action(
    install_state: InstallState,
    process: &ProcessReport,
    login_state: LoginState,
    background_install: Option<&BackgroundReport>,
) -> (StatusSummary, Option<NextAction>) {
    if let Some(background) = background_install {
        match &background.state {
            BackgroundState::Running { .. } => {
                return (StatusSummary::Installing, Some(NextAction::TailInstallLog));
            }
            BackgroundState::Completed { exit_code } if *exit_code != 0 => {
                return (
                    StatusSummary::InstallFailed,
                    Some(NextAction::TailInstallLog),
                );
            }
            BackgroundState::Completed { .. } | BackgroundState::NotRunning => {}
            BackgroundState::Unknown { .. } => {
                return (StatusSummary::Unknown, Some(NextAction::TailInstallLog));
            }
        }
    }

    match install_state {
        InstallState::NotInstalled => (StatusSummary::NotInstalled, Some(NextAction::Install)),
        InstallState::Invalid => (
            StatusSummary::InvalidApp,
            Some(NextAction::InspectInvalidApp),
        ),
        InstallState::Installed if login_state == LoginState::NotLoggedIn => {
            (StatusSummary::NeedsLogin, Some(NextAction::Login))
        }
        InstallState::Installed => match process.state {
            ProcessState::Unknown => (StatusSummary::Unknown, Some(NextAction::InspectRuntimeLog)),
            ProcessState::Stopped => (StatusSummary::Stopped, Some(NextAction::Start)),
            ProcessState::Running
                if matches!(
                    login_state,
                    LoginState::LoggedIn | LoginState::NotApplicable
                ) =>
            {
                (StatusSummary::Ready, None)
            }
            ProcessState::Running => (StatusSummary::Unknown, Some(NextAction::InspectRuntimeLog)),
        },
    }
}

pub(crate) fn chat2db_base_path(edition: Edition, home_dir: &Path) -> PathBuf {
    edition.data_dir(home_dir)
}

pub(crate) fn login_cookie_file_path(edition: Edition, home_dir: &Path) -> PathBuf {
    chat2db_base_path(edition, home_dir)
        .join("cache")
        .join(format!("chat2db-enterprise-cookie-{PROFILE}"))
}

pub(crate) fn runtime_log_file_path(edition: Edition, home_dir: &Path) -> PathBuf {
    edition.runtime_log_path(home_dir)
}

fn runtime_log_report(edition: Edition, home_dir: &Path) -> RuntimeLogReport {
    let path = runtime_log_file_path(edition, home_dir);
    let metadata = fs::metadata(&path)
        .ok()
        .filter(|metadata| metadata.is_file());
    let size_bytes = metadata.as_ref().map(|metadata| metadata.len());
    let modified_at = metadata
        .and_then(|metadata| metadata.modified().ok())
        .and_then(format_system_time);

    RuntimeLogReport {
        path,
        exists: size_bytes.is_some(),
        size_bytes,
        modified_at,
    }
}

fn format_system_time(time: SystemTime) -> Option<String> {
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    let timestamp = OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64).ok()?;
    timestamp.format(&Rfc3339).ok()
}

fn present_cookie_markers(cookies: &Value) -> Vec<&'static str> {
    AUTH_COOKIES
        .iter()
        .copied()
        .filter(|key| cookie_value(cookies, key).is_some())
        .collect()
}

fn is_logged_in(edition: Edition, cookies: &Value) -> bool {
    match edition {
        Edition::Community => false,
        Edition::Pro | Edition::Local => has_valid_user_id_cookie(cookies),
    }
}

fn has_valid_user_id_cookie(cookies: &Value) -> bool {
    cookie_value(cookies, USER_ID_COOKIE).is_some_and(is_valid_user_id_cookie)
}

pub(crate) fn cookie_value<'a>(cookies: &'a Value, key: &str) -> Option<&'a str> {
    let value = cookies.get(key)?;
    let value = value
        .as_str()
        .or_else(|| value.get("value").and_then(Value::as_str))?;
    let value = value.trim();
    if value.is_empty() { None } else { Some(value) }
}

fn is_valid_user_id_cookie(value: &str) -> bool {
    let Ok(decoded) = STANDARD.decode(value) else {
        return false;
    };
    let Ok(user_id) = std::str::from_utf8(&decoded) else {
        return false;
    };
    user_id.parse::<u64>().is_ok_and(|id| id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpEndpointProbe;
    use crate::process::ProcessProbe;

    #[derive(Debug, Clone)]
    struct FakeProcessProbe {
        pids: Vec<u32>,
    }

    impl FakeProcessProbe {
        fn running() -> Self {
            Self { pids: vec![4242] }
        }
    }

    impl ProcessProbe for FakeProcessProbe {
        fn pids(&self, _app_path: &Path) -> crate::Result<Vec<u32>> {
            Ok(self.pids.clone())
        }
    }

    #[derive(Debug, Clone)]
    struct FakeEndpointProbe(EndpointState);

    impl McpEndpointProbe for FakeEndpointProbe {
        fn probe(&self, _url: &str, _timeout: std::time::Duration) -> EndpointState {
            self.0
        }
    }

    #[test]
    fn status_reports_community_first_without_authentication() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Community.app"),
            "com.chat2db.community",
        );
        let cookie_file = login_cookie_file_path(Edition::Community, &paths.home_dir);
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, b"not valid json and must not be read").unwrap();

        let outcome = check_status_with_process_probe(
            StatusRequest {
                paths: paths.clone(),
            },
            &FakeProcessProbe::running(),
        );

        assert_eq!(
            outcome
                .editions
                .iter()
                .map(|status| status.edition)
                .collect::<Vec<_>>(),
            Edition::ALL
        );
        let community = find_edition(&outcome, Edition::Community);
        assert_eq!(community.login.state, LoginState::NotApplicable);
        assert_eq!(community.login.source, "not_applicable");
        assert!(community.login.checked_paths.is_empty());
        assert_eq!(community.cookie_file.state, CookieFileState::NotApplicable);
        assert_eq!(
            community.data_dir.path,
            paths.home_dir.join(".chat2db-community")
        );
        assert_eq!(
            community.runtime_log.path,
            paths
                .home_dir
                .join(".chat2db-community/chat2db-community/logs/application.log")
        );
        assert_eq!(community.summary, StatusSummary::Ready);
        assert_eq!(community.next_action, None);
        assert_eq!(
            outcome.summary.for_edition(Edition::Community).app.state,
            StatusAppState::Running
        );
    }

    #[test]
    fn status_summary_splits_app_cli_runtime_and_mcp() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Pro.app"),
            "com.chat2db.pro",
        );
        let probe = FakeProcessProbe::running();

        let endpoint_probe = FakeEndpointProbe(EndpointState::NotListening);

        let outcome = check_status_with_probes(StatusRequest { paths }, &probe, &endpoint_probe);
        let pro = outcome.summary.for_edition(Edition::Pro);

        assert_eq!(pro.app.state, StatusAppState::Running);
        assert_eq!(pro.app.pids, vec![4242]);
        assert_eq!(pro.cli.state, StatusCliRuntimeState::Stopped);
        assert!(!pro.mcp.enabled);
        assert_eq!(pro.mcp.endpoint, EndpointState::NotListening);
        assert!(!pro.safe_to_exit);
        assert_eq!(
            pro.next_action.as_deref(),
            Some("chat2db stop --edition pro --all --json")
        );
    }

    #[test]
    fn status_summary_treats_uninstalled_edition_as_safe_when_cli_and_mcp_are_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let process_probe = FakeProcessProbe { pids: vec![] };
        let endpoint_probe = FakeEndpointProbe(EndpointState::NotListening);

        let outcome =
            check_status_with_probes(StatusRequest { paths }, &process_probe, &endpoint_probe);
        let local = outcome.summary.for_edition(Edition::Local);

        assert_eq!(local.app.state, StatusAppState::NotInstalled);
        assert_eq!(local.cli.state, StatusCliRuntimeState::Stopped);
        assert!(!local.mcp.enabled);
        assert_eq!(local.mcp.endpoint, EndpointState::NotListening);
        assert!(local.safe_to_exit);
        assert_eq!(
            local.next_action.as_deref(),
            Some("chat2db install --edition local --json")
        );
    }

    #[test]
    fn status_reports_installed_verified_app() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Pro.app"),
            "com.chat2db.pro",
        );

        let outcome = check_status(StatusRequest {
            paths: paths.clone(),
        });
        let pro = find_edition(&outcome, Edition::Pro);

        assert!(pro.installed);
        assert_eq!(pro.install_state, InstallState::Installed);
        assert!(pro.install_error.is_none());
        assert_eq!(pro.summary, StatusSummary::Stopped);
        assert_eq!(pro.next_action, Some(NextAction::Start));
        assert_eq!(pro.app.bundle_id.as_deref(), Some("com.chat2db.pro"));
        assert_eq!(pro.app.version.as_deref(), Some("1.2.3"));
        assert_eq!(pro.app.executable.as_deref(), Some("Chat2DB"));
    }

    #[test]
    fn status_reports_invalid_app_without_failing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Local.app"),
            "com.example.other",
        );

        let outcome = check_status(StatusRequest {
            paths: paths.clone(),
        });
        let local = find_edition(&outcome, Edition::Local);

        assert!(!local.installed);
        assert_eq!(local.install_state, InstallState::Invalid);
        assert!(
            local
                .install_error
                .as_ref()
                .is_some_and(|error| error.contains("bundle id mismatch"))
        );
        assert_eq!(local.summary, StatusSummary::InvalidApp);
        assert_eq!(local.next_action, Some(NextAction::InspectInvalidApp));
        assert_eq!(local.app.bundle_id.as_deref(), Some("com.example.other"));
    }

    #[test]
    fn status_reports_running_process_pids() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Pro.app"),
            "com.chat2db.pro",
        );
        let probe = FakeProcessProbe::running();

        let outcome = check_status_with_process_probe(StatusRequest { paths }, &probe);
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.process.state, ProcessState::Running);
        assert_eq!(pro.process.pids, vec![4242]);
    }

    #[test]
    fn status_summarizes_ready_when_running_and_logged_in() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Local.app"),
            "com.chat2db.local",
        );
        let cookie_file = login_cookie_file_path(Edition::Local, &paths.home_dir);
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, r#"{"Chat2db-BS-TOKEN":{"value":"MTIz"}}"#).unwrap();
        let probe = FakeProcessProbe::running();

        let outcome = check_status_with_process_probe(StatusRequest { paths }, &probe);
        let local = find_edition(&outcome, Edition::Local);

        assert_eq!(local.summary, StatusSummary::Ready);
        assert_eq!(local.next_action, None);
    }

    #[test]
    fn status_summarizes_needs_login_when_cookie_missing_but_data_dir_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        create_fake_app(
            &paths.applications_dir.join("Chat2DB Pro.app"),
            "com.chat2db.pro",
        );
        fs::create_dir_all(chat2db_base_path(Edition::Pro, &paths.home_dir)).unwrap();
        let probe = FakeProcessProbe { pids: vec![] };

        let outcome = check_status_with_process_probe(StatusRequest { paths }, &probe);
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.login.state, LoginState::NotLoggedIn);
        assert_eq!(pro.summary, StatusSummary::NeedsLogin);
        assert_eq!(pro.next_action, Some(NextAction::Login));
    }

    #[test]
    fn status_detects_login_markers_without_values() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = paths
            .home_dir
            .join(".chat2db_local_edition/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(
            &cookie_file,
            r#"{"CHAT2DB":{"value":"secret"},"Chat2db-BS-TOKEN":{"value":"MTIz"}}"#,
        )
        .unwrap();

        let outcome = check_status(StatusRequest { paths });
        let local = find_edition(&outcome, Edition::Local);

        assert_eq!(local.login.state, LoginState::LoggedIn);
        assert_eq!(local.cookie_file.state, CookieFileState::Exists);
        assert!(local.login.markers.contains(&"Chat2db-BS-TOKEN"));
        assert!(local.login.checked_paths.contains(&cookie_file));
    }

    #[test]
    fn status_json_outputs_summary_and_omits_internal_details() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = paths
            .home_dir
            .join(".chat2db/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(
            &cookie_file,
            r#"{
                "CHAT2DB":{"value":"session-secret"},
                "Chat2db-BS-TOKEN":{"value":"MTIz"},
                "Chat2db-Organization-Token":{"value":"organization-secret"},
                "Chat2db-Organization-Id":{"value":"123"}
            }"#,
        )
        .unwrap();

        let outcome = check_status(StatusRequest { paths });
        let json = serde_json::to_string(&outcome).unwrap();

        assert!(json.contains(r#""summary""#));
        assert!(json.contains(r#""safe_to_exit""#));
        assert!(!json.contains(r#""editions""#));
        assert!(!json.contains(r#""skills""#));
        assert!(!json.contains("CHAT2DB"));
        assert!(!json.contains("Chat2db-BS-TOKEN"));
        assert!(!json.contains("Chat2db-Organization-Token"));
        assert!(!json.contains("session-secret"));
        assert!(!json.contains("organization-secret"));
    }

    #[test]
    fn status_reports_invalid_json_cookie_file() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = login_cookie_file_path(Edition::Pro, &paths.home_dir);
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, b"{not json").unwrap();

        let outcome = check_status(StatusRequest { paths });
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.cookie_file.state, CookieFileState::InvalidJson);
        assert_eq!(pro.login.state, LoginState::Unknown);
    }

    #[cfg(unix)]
    #[test]
    fn status_reports_unreadable_cookie_file() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = login_cookie_file_path(Edition::Pro, &paths.home_dir);
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, b"{}").unwrap();
        let mut permissions = fs::metadata(&cookie_file).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&cookie_file, permissions).unwrap();

        let outcome = check_status(StatusRequest { paths });
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.cookie_file.state, CookieFileState::Unreadable);
        assert_eq!(pro.login.state, LoginState::Unknown);

        let mut permissions = fs::metadata(&cookie_file).unwrap().permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&cookie_file, permissions).unwrap();
    }

    #[test]
    fn status_does_not_treat_pro_oauth_cookie_set_as_logged_in() {
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
                "Chat2db-Organization-Token":{"value":"organization-token"},
                "Chat2db-Organization-Id":{"value":"123"}
            }"#,
        )
        .unwrap();

        let outcome = check_status(StatusRequest { paths });
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.login.state, LoginState::NotLoggedIn);
        assert!(pro.login.markers.contains(&"CHAT2DB"));
        assert!(pro.login.markers.contains(&"Chat2db-Organization-Token"));
        assert!(pro.login.markers.contains(&"Chat2db-Organization-Id"));
    }

    #[test]
    fn status_does_not_treat_single_chat2db_cookie_as_logged_in() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let cookie_file = paths
            .home_dir
            .join(".chat2db/cache/chat2db-enterprise-cookie-release");
        fs::create_dir_all(cookie_file.parent().unwrap()).unwrap();
        fs::write(&cookie_file, r#"{"CHAT2DB":{"value":"session"}}"#).unwrap();

        let outcome = check_status(StatusRequest { paths });
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.login.state, LoginState::NotLoggedIn);
        assert_eq!(pro.login.markers, vec!["CHAT2DB"]);
    }

    #[test]
    fn status_reports_unknown_login_when_data_dir_is_absent() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let outcome = check_status(StatusRequest {
            paths: paths.clone(),
        });
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.login.state, LoginState::Unknown);
        assert!(pro.login.markers.is_empty());
        assert_eq!(pro.data_dir.path, paths.home_dir.join(".chat2db"));
        assert!(!pro.data_dir.exists);
        assert_eq!(pro.cookie_file.state, CookieFileState::Missing);
    }

    #[test]
    fn status_reports_runtime_log_paths_even_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let outcome = check_status(StatusRequest {
            paths: paths.clone(),
        });
        let pro = find_edition(&outcome, Edition::Pro);
        let local = find_edition(&outcome, Edition::Local);

        assert_eq!(
            pro.runtime_log.path,
            paths
                .home_dir
                .join(".chat2db/chat2db-enterprise/logs/application.log")
        );
        assert!(!pro.runtime_log.exists);
        assert_eq!(pro.runtime_log.size_bytes, None);
        assert_eq!(pro.runtime_log.modified_at, None);
        assert_eq!(
            local.runtime_log.path,
            paths
                .home_dir
                .join(".chat2db_local_edition/chat2db-enterprise/logs/application.log")
        );
        assert!(!local.runtime_log.exists);
        assert_eq!(local.runtime_log.size_bytes, None);
        assert_eq!(local.runtime_log.modified_at, None);
    }

    #[test]
    fn status_reports_runtime_log_size_when_file_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        let log_path = runtime_log_file_path(Edition::Local, &paths.home_dir);
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        fs::write(&log_path, b"runtime log\n").unwrap();

        let outcome = check_status(StatusRequest { paths });
        let local = find_edition(&outcome, Edition::Local);

        assert_eq!(local.runtime_log.path, log_path);
        assert!(local.runtime_log.exists);
        assert_eq!(local.runtime_log.size_bytes, Some(12));
        assert!(local.runtime_log.modified_at.is_some());
    }

    #[test]
    fn status_omits_background_install_when_state_dir_empty() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let outcome = check_status(StatusRequest { paths });

        for edition in &outcome.editions {
            assert!(edition.background_install.is_none());
        }
    }

    #[test]
    fn status_reports_completed_background_install() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(&paths.background_state_dir).unwrap();
        fs::write(paths.background_state_dir.join("install-pro.done"), "0\n").unwrap();

        let outcome = check_status(StatusRequest {
            paths: paths.clone(),
        });
        let pro = find_edition(&outcome, Edition::Pro);

        let bg = pro.background_install.as_ref().expect("expected report");
        assert_eq!(bg.state, BackgroundState::Completed { exit_code: 0 });
        assert_eq!(
            bg.log_path,
            paths.background_state_dir.join("install-pro.log")
        );
        let runtime = outcome.summary.for_edition(Edition::Pro);
        assert_eq!(
            runtime
                .background_install
                .as_ref()
                .map(|background| background.state.clone()),
            Some(BackgroundState::Completed { exit_code: 0 })
        );
    }

    #[test]
    fn failed_background_install_drives_summary_and_next_action() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(&paths.background_state_dir).unwrap();
        fs::write(paths.background_state_dir.join("install-pro.done"), "1\n").unwrap();

        let outcome = check_status(StatusRequest { paths });
        let pro = find_edition(&outcome, Edition::Pro);

        assert_eq!(pro.summary, StatusSummary::InstallFailed);
        assert_eq!(pro.next_action, Some(NextAction::TailInstallLog));
        let runtime = outcome.summary.for_edition(Edition::Pro);
        assert!(!runtime.safe_to_exit);
        assert_eq!(
            runtime.next_action.as_deref(),
            Some("chat2db logs install --edition pro")
        );
    }

    #[test]
    fn status_reports_running_background_install_for_live_pid() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(&paths.background_state_dir).unwrap();
        // The current process is guaranteed alive while this test runs.
        let live_pid = std::process::id();
        let process_identity = crate::platform::process_identity(live_pid)
            .unwrap()
            .unwrap();
        fs::write(
            paths.background_state_dir.join("install-local.pid"),
            serde_json::to_vec(&serde_json::json!({
                "pid": live_pid,
                "process_identity": process_identity,
            }))
            .unwrap(),
        )
        .unwrap();

        let outcome = check_status(StatusRequest { paths });
        let local = find_edition(&outcome, Edition::Local);

        let bg = local.background_install.as_ref().expect("expected report");
        assert_eq!(bg.state, BackgroundState::Running { pid: live_pid });
        assert_eq!(local.summary, StatusSummary::Installing);
        assert_eq!(local.next_action, Some(NextAction::TailInstallLog));
        let runtime = outcome.summary.for_edition(Edition::Local);
        assert!(!runtime.safe_to_exit);
        assert_eq!(
            runtime.next_action.as_deref(),
            Some("chat2db logs install --edition local --follow")
        );
    }

    #[test]
    fn status_does_not_wait_for_a_running_background_install_lifecycle_lock() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());
        fs::create_dir_all(&paths.background_state_dir).unwrap();
        let live_pid = std::process::id();
        let process_identity = crate::platform::process_identity(live_pid)
            .unwrap()
            .unwrap();
        fs::write(
            paths.background_state_dir.join("install-pro.pid"),
            serde_json::to_vec(&serde_json::json!({
                "pid": live_pid,
                "process_identity": process_identity,
            }))
            .unwrap(),
        )
        .unwrap();
        let lifecycle_lock =
            crate::runtime::acquire_edition_lifecycle_lock(&paths, Edition::Pro).unwrap();
        let (result_tx, result_rx) = std::sync::mpsc::channel();

        let handle = std::thread::spawn(move || {
            let outcome = check_status_with_probes(
                StatusRequest { paths },
                &FakeProcessProbe { pids: vec![] },
                &FakeEndpointProbe(EndpointState::NotListening),
            );
            result_tx.send(outcome).unwrap();
        });

        let outcome = result_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("status must not wait for the background installer lifecycle lock");
        let pro = outcome.summary.for_edition(Edition::Pro);
        assert!(!pro.safe_to_exit);
        assert_eq!(
            pro.next_action.as_deref(),
            Some("chat2db logs install --edition pro --follow")
        );
        drop(lifecycle_lock);
        handle.join().unwrap();
    }

    fn find_edition(outcome: &StatusOutcome, edition: Edition) -> &EditionStatus {
        outcome
            .editions
            .iter()
            .find(|status| status.edition == edition)
            .unwrap()
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

    fn create_fake_app(app_path: &Path, bundle_id: &str) {
        let contents = app_path.join("Contents");
        fs::create_dir_all(&contents).unwrap();
        fs::write(contents.join("Info.plist"), plist_xml(bundle_id)).unwrap();
    }

    fn plist_xml(bundle_id: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>{bundle_id}</string>
  <key>CFBundleShortVersionString</key>
  <string>1.2.3</string>
  <key>CFBundleExecutable</key>
  <string>Chat2DB</string>
</dict>
</plist>
"#
        )
    }
}
