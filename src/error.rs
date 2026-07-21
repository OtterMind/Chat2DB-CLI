use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorAdvice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk: Option<ErrorRisk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorRisk {
    pub level: &'static str,
    pub action: String,
}

impl ErrorAdvice {
    fn empty() -> Self {
        Self {
            hint: None,
            next_action: None,
            retryable: None,
            risk: None,
        }
    }

    fn new(
        hint: &'static str,
        next_action: Option<String>,
        retryable: bool,
        level: &'static str,
        action: impl Into<String>,
    ) -> Self {
        Self {
            hint: Some(hint.to_string()),
            next_action,
            retryable: Some(retryable),
            risk: Some(ErrorRisk {
                level,
                action: action.into(),
            }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Chat2DbError {
    #[error("unsupported platform: this Chat2DB CLI operation is not available on this OS")]
    UnsupportedPlatform,

    #[error("{action} requires confirmation; pass --yes to confirm")]
    ConfirmationRequired { action: &'static str },

    #[error("invalid connection-test arguments: {message}")]
    InvalidConnectionTestArgs { message: String },

    #[error("invalid datasource-create arguments: {message}")]
    InvalidDatasourceCreateArgs { message: String },

    #[error("datasource connection test failed: {message}")]
    DatasourceConnectionFailed { message: String },

    #[error("unsupported CPU architecture: {0}")]
    UnsupportedArch(String),

    #[error("failed to fetch {url}: {source}")]
    HttpRequest {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("failed to fetch {url}: HTTP {status}")]
    HttpStatus {
        url: String,
        status: reqwest::StatusCode,
    },

    #[error("Chat2DB manifest is not published yet: {url}")]
    ManifestUnavailable { url: String },

    #[error("invalid Chat2DB release manifest: {0}")]
    InvalidReleaseManifest(String),

    #[error("failed to run {program}: {source}")]
    CommandIo {
        program: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("{program} failed with status {status}: {stderr}")]
    CommandFailed {
        program: &'static str,
        status: String,
        stderr: String,
    },

    #[error("dmg must contain exactly one top-level .app bundle")]
    InvalidDmg,

    #[error("refusing to use invalid CLI binary target: {path}")]
    InvalidCliTarget { path: PathBuf },

    #[error("invalid manifest JSON: {0}")]
    ManifestJson(#[from] serde_json::Error),

    #[error("invalid MCP settings JSON at {path}: {source}")]
    McpSettingsInvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid MCP settings shape at {path}: expected JSON object")]
    McpSettingsNotObject { path: PathBuf },

    #[error(
        "cannot manage MCP for {target} while {other} is running; stop the other edition first because both editions share an MCP port"
    )]
    McpPortAmbiguous { target: String, other: String },

    #[error("MCP endpoint for {edition} was not ready at {url} after {waited_seconds}s")]
    McpEndpointNotReady {
        edition: String,
        url: String,
        waited_seconds: u64,
    },

    #[error("MCP endpoint for {edition} was still alive at {url} after {waited_seconds}s")]
    McpEndpointStillAlive {
        edition: String,
        url: String,
        waited_seconds: u64,
    },

    #[error("Chat2DB {edition} did not stop within {waited_seconds}s while applying MCP settings")]
    McpAppNotStopped {
        edition: String,
        waited_seconds: u64,
    },

    #[error("Chat2DB {edition} did not start within {waited_seconds}s while applying MCP settings")]
    McpAppNotStarted {
        edition: String,
        waited_seconds: u64,
    },

    #[error("failed to generate MCP auth token: {message}")]
    McpTokenGeneration { message: String },

    #[error("failed to update MCP desktop UI state at {path}: {message}")]
    McpFrontendStore { path: PathBuf, message: String },

    #[error("invalid MCP desktop UI state JSON at {path}: {source}")]
    McpFrontendStoreInvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("MCP token is missing for {edition}; run chat2db mcp enable --edition {edition} first")]
    McpTokenMissing { edition: String },

    #[error("MCP call to {url} failed: {message}")]
    McpClientHttp { url: String, message: String },

    #[error("MCP protocol error: {message}")]
    McpClientProtocol { message: String },

    #[error("failed to parse Chat2DB app cfg at {path}: {message}")]
    RuntimeCfgInvalid { path: PathBuf, message: String },

    #[error(
        "Chat2DB runtime endpoint for {edition} was not ready at {url} after {waited_seconds}s"
    )]
    RuntimeEndpointNotReady {
        edition: String,
        url: String,
        waited_seconds: u64,
    },

    #[error("Chat2DB runtime state is invalid at {path}: {source}")]
    RuntimeStateInvalidJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("Chat2DB runtime API call to {url} failed: {message}")]
    RuntimeClientHttp { url: String, message: String },

    #[error("Chat2DB runtime API error {code}: {message}")]
    RuntimeApi {
        code: String,
        message: String,
        details: Value,
    },

    #[error("could not verify ownership of runtime process {pid}: {message}")]
    RuntimeProcessIdentityUnavailable { pid: u32, message: String },

    #[error("runtime process {pid} no longer matches the process recorded by Chat2DB CLI")]
    RuntimeProcessOwnershipMismatch { pid: u32 },

    #[error("runtime process {pid} is still running after shutdown and termination attempts")]
    RuntimeProcessStillRunning { pid: u32 },

    #[error("Chat2DB {edition} was still running after {waited_seconds}s; restart was aborted")]
    RestartAppStillRunning {
        edition: String,
        waited_seconds: u64,
    },

    #[error(
        "Chat2DB {edition} is still running with process IDs {pids:?}; stop it before changing its installation"
    )]
    AppProcessStillRunning { edition: String, pids: Vec<u32> },

    #[error("failed to generate Chat2DB runtime token: {message}")]
    RuntimeTokenGeneration { message: String },

    #[error("invalid MCP args JSON: expected JSON object: {source}")]
    McpArgsInvalidJson {
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid MCP args JSON: expected JSON object")]
    McpArgsNotObject,

    #[error("manifest is missing field: {0}")]
    MissingManifestField(&'static str),

    #[error("manifest artifact URL must use https://cdn.chat2db-ai.com")]
    UnofficialArtifactUrl,

    #[error("manifest artifact suffix is not supported")]
    UnsupportedArtifact,

    #[error("manifest sha256 must be 64 lowercase or uppercase hex characters")]
    InvalidSha256,

    #[error("artifact checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("zip path is unsafe: {0}")]
    UnsafeZipPath(String),

    #[error("zip must contain exactly one top-level .app bundle")]
    InvalidAppZip,

    #[error("zip contains a file outside the .app bundle: {0}")]
    UnexpectedZipEntry(String),

    #[error("app bundle is missing Contents/Info.plist: {0}")]
    MissingInfoPlist(PathBuf),

    #[error("app bundle id mismatch: expected {expected}, got {actual}")]
    BundleIdMismatch { expected: String, actual: String },

    #[error("app already exists at {path}; pass --force to replace a verified Chat2DB app")]
    AppAlreadyExists { path: PathBuf },

    #[error(
        "{edition} app is not installed at {path}; run chat2db install --edition {edition} first"
    )]
    AppNotInstalled { edition: String, path: PathBuf },

    #[error("refusing to open invalid Chat2DB app at {path}: {reason}")]
    UnsafeOpen { path: PathBuf, reason: String },

    #[error("login timed out for {edition} after {seconds}s")]
    LoginTimedOut { edition: String, seconds: u64 },

    #[error("invalid Chat2DB cookie file {path}: {source}")]
    InvalidCookieFile {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid Chat2DB cookie file shape: expected JSON object at {path}")]
    InvalidCookieShape { path: PathBuf },

    #[error("skill bundle already installed at {path}; pass --force to overwrite")]
    SkillAlreadyInstalled { path: PathBuf },

    #[error("skill bundle is not installed at {path}")]
    SkillNotInstalled { path: PathBuf },

    #[error(
        "skill bundle requires chat2db CLI {required}; current CLI is {current}. Run chat2db update first."
    )]
    SkillVersionIncompatible { required: String, current: String },

    #[error(
        "refusing to overwrite foreign content at {path} (no chat2db SKILL.md found); pass --force to replace it"
    )]
    SkillTargetForeign { path: PathBuf },

    #[error(
        "refusing to overwrite skill bundle at {path}: it is a git checkout. Run `git -C {path} pull` instead, or remove .git first."
    )]
    SkillTargetManagedExternally { path: PathBuf },

    #[error("downloaded skill bundle is corrupt: {reason}")]
    SkillBundleCorrupt { reason: String },

    #[error("refusing to replace non-Chat2DB app at {path}: {reason}")]
    UnsafeReplace { path: PathBuf, reason: String },

    #[error("refusing to uninstall non-Chat2DB app at {path}: {reason}")]
    UnsafeUninstall { path: PathBuf, reason: String },

    #[error(
        "background install for {edition} is already running (pid {pid}); check the log under ~/.local/state/chat2db/"
    )]
    BackgroundInstallAlreadyRunning { edition: String, pid: u32 },

    #[error(
        "no {kind} log found for {edition} at {path}; run chat2db install --background --edition {edition} first"
    )]
    LogNotFound {
        kind: &'static str,
        edition: String,
        path: PathBuf,
    },

    #[error("could not detect shell from $SHELL; pass --shell <zsh|bash|fish>")]
    CompletionsShellUnknown,

    #[error("unsupported shell for completions: {0}")]
    CompletionsShellUnsupported(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),

    #[error("invalid app bundle plist: {0}")]
    Plist(String),

    #[error(transparent)]
    Url(#[from] url::ParseError),
}

impl Chat2DbError {
    pub fn code(&self) -> &str {
        match self {
            Chat2DbError::UnsupportedPlatform => "unsupported_platform",
            Chat2DbError::ConfirmationRequired { .. } => "confirmation_required",
            Chat2DbError::InvalidConnectionTestArgs { .. } => "invalid_connection_test_args",
            Chat2DbError::InvalidDatasourceCreateArgs { .. } => "invalid_datasource_create_args",
            Chat2DbError::DatasourceConnectionFailed { .. } => "datasource_connection_failed",
            Chat2DbError::UnsupportedArch(_) => "unsupported_arch",
            Chat2DbError::HttpRequest { .. } => "http_request_error",
            Chat2DbError::HttpStatus { .. } => "http_status_error",
            Chat2DbError::ManifestUnavailable { .. } => "manifest_unavailable",
            Chat2DbError::InvalidReleaseManifest(_) => "invalid_release_manifest",
            Chat2DbError::CommandIo { .. } => "command_io_error",
            Chat2DbError::CommandFailed { .. } => "command_failed",
            Chat2DbError::InvalidDmg => "invalid_dmg",
            Chat2DbError::InvalidCliTarget { .. } => "invalid_cli_target",
            Chat2DbError::ManifestJson(_) => "manifest_json_error",
            Chat2DbError::McpSettingsInvalidJson { .. } => "mcp_settings_invalid_json",
            Chat2DbError::McpSettingsNotObject { .. } => "mcp_settings_not_object",
            Chat2DbError::McpPortAmbiguous { .. } => "mcp_port_ambiguous",
            Chat2DbError::McpEndpointNotReady { .. } => "mcp_endpoint_not_ready",
            Chat2DbError::McpEndpointStillAlive { .. } => "mcp_endpoint_still_alive",
            Chat2DbError::McpAppNotStopped { .. } => "mcp_app_not_stopped",
            Chat2DbError::McpAppNotStarted { .. } => "mcp_app_not_started",
            Chat2DbError::McpTokenGeneration { .. } => "mcp_token_generation_error",
            Chat2DbError::McpFrontendStore { .. } => "mcp_frontend_store_error",
            Chat2DbError::McpFrontendStoreInvalidJson { .. } => "mcp_frontend_store_invalid_json",
            Chat2DbError::McpTokenMissing { .. } => "mcp_token_missing",
            Chat2DbError::McpClientHttp { .. } => "mcp_client_http_error",
            Chat2DbError::McpClientProtocol { .. } => "mcp_client_protocol_error",
            Chat2DbError::RuntimeCfgInvalid { .. } => "runtime_cfg_invalid",
            Chat2DbError::RuntimeEndpointNotReady { .. } => "runtime_endpoint_not_ready",
            Chat2DbError::RuntimeStateInvalidJson { .. } => "runtime_state_invalid_json",
            Chat2DbError::RuntimeClientHttp { .. } => "runtime_client_http_error",
            Chat2DbError::RuntimeApi { code, .. } => code,
            Chat2DbError::RuntimeProcessIdentityUnavailable { .. } => {
                "runtime_process_identity_unavailable"
            }
            Chat2DbError::RuntimeProcessOwnershipMismatch { .. } => {
                "runtime_process_ownership_mismatch"
            }
            Chat2DbError::RuntimeProcessStillRunning { .. } => "runtime_process_still_running",
            Chat2DbError::RestartAppStillRunning { .. } => "restart_app_still_running",
            Chat2DbError::AppProcessStillRunning { .. } => "app_process_still_running",
            Chat2DbError::RuntimeTokenGeneration { .. } => "runtime_token_generation_error",
            Chat2DbError::McpArgsInvalidJson { .. } | Chat2DbError::McpArgsNotObject => {
                "mcp_args_invalid_json"
            }
            Chat2DbError::MissingManifestField(_) => "missing_manifest_field",
            Chat2DbError::UnofficialArtifactUrl => "unofficial_artifact_url",
            Chat2DbError::UnsupportedArtifact => "unsupported_artifact",
            Chat2DbError::InvalidSha256 => "invalid_sha256",
            Chat2DbError::ChecksumMismatch { .. } => "checksum_mismatch",
            Chat2DbError::UnsafeZipPath(_) => "unsafe_zip_path",
            Chat2DbError::InvalidAppZip => "invalid_app_zip",
            Chat2DbError::UnexpectedZipEntry(_) => "unexpected_zip_entry",
            Chat2DbError::MissingInfoPlist(_) => "missing_info_plist",
            Chat2DbError::BundleIdMismatch { .. } => "bundle_id_mismatch",
            Chat2DbError::AppAlreadyExists { .. } => "app_already_exists",
            Chat2DbError::AppNotInstalled { .. } => "app_not_installed",
            Chat2DbError::UnsafeOpen { .. } => "unsafe_open",
            Chat2DbError::LoginTimedOut { .. } => "login_timed_out",
            Chat2DbError::InvalidCookieFile { .. } => "invalid_cookie_file",
            Chat2DbError::InvalidCookieShape { .. } => "invalid_cookie_shape",
            Chat2DbError::SkillAlreadyInstalled { .. } => "skill_already_installed",
            Chat2DbError::SkillNotInstalled { .. } => "skill_not_installed",
            Chat2DbError::SkillVersionIncompatible { .. } => "skill_version_incompatible",
            Chat2DbError::SkillTargetForeign { .. } => "skill_target_foreign",
            Chat2DbError::SkillTargetManagedExternally { .. } => "skill_target_managed_externally",
            Chat2DbError::SkillBundleCorrupt { .. } => "skill_bundle_corrupt",
            Chat2DbError::UnsafeReplace { .. } => "unsafe_replace",
            Chat2DbError::UnsafeUninstall { .. } => "unsafe_uninstall",
            Chat2DbError::BackgroundInstallAlreadyRunning { .. } => {
                "background_install_already_running"
            }
            Chat2DbError::LogNotFound { .. } => "log_not_found",
            Chat2DbError::CompletionsShellUnknown => "completions_shell_unknown",
            Chat2DbError::CompletionsShellUnsupported(_) => "completions_shell_unsupported",
            Chat2DbError::Io(_) => "io_error",
            Chat2DbError::Zip(_) => "zip_error",
            Chat2DbError::Plist(_) => "plist_error",
            Chat2DbError::Url(_) => "url_error",
        }
    }

    pub fn details(&self) -> Option<&Value> {
        match self {
            Chat2DbError::RuntimeApi { details, .. } if !is_empty_details(details) => Some(details),
            _ => None,
        }
    }

    pub fn advice(&self) -> ErrorAdvice {
        match self {
            Chat2DbError::McpTokenMissing { edition } => ErrorAdvice::new(
                "Enable MCP for this edition first.",
                Some(format!("chat2db mcp enable --edition {edition} --json")),
                false,
                "write",
                format!("enable MCP for {edition}"),
            ),
            Chat2DbError::McpClientHttp { .. } => ErrorAdvice::new(
                "The MCP endpoint is unavailable or returned an HTTP error.",
                Some("chat2db status --json".to_string()),
                true,
                "read",
                "inspect status before retrying MCP call",
            ),
            Chat2DbError::RuntimeEndpointNotReady { edition, .. } => ErrorAdvice::new(
                "The direct runtime did not become ready before the timeout.",
                Some(format!("chat2db runtime status --edition {edition} --json")),
                true,
                "read",
                format!("inspect {edition} runtime status"),
            ),
            Chat2DbError::RuntimeClientHttp { .. } => ErrorAdvice::new(
                "The direct runtime is unavailable or returned an HTTP error.",
                Some("chat2db runtime status --json".to_string()),
                true,
                "read",
                "inspect runtime status before retrying",
            ),
            Chat2DbError::RuntimeApi { code, .. } => runtime_api_advice(code),
            Chat2DbError::InvalidConnectionTestArgs { .. } => ErrorAdvice::new(
                "Use either --data-source-id for an existing datasource or temporary connection parameters.",
                Some("chat2db db connection-test --help".to_string()),
                false,
                "read",
                "inspect connection-test usage",
            ),
            Chat2DbError::InvalidDatasourceCreateArgs { .. } => ErrorAdvice::new(
                "Use either --url or --host --port --database when creating a datasource.",
                Some("chat2db db datasource-create --help".to_string()),
                false,
                "write",
                "inspect datasource-create usage",
            ),
            Chat2DbError::DatasourceConnectionFailed { .. } => ErrorAdvice::new(
                "Check host, port, database, user, password, JDBC URL, driver, and network.",
                Some("chat2db db connection-test --help".to_string()),
                true,
                "read",
                "retry connection test after fixing connection parameters",
            ),
            Chat2DbError::McpPortAmbiguous { target, other } => ErrorAdvice::new(
                "Only one Chat2DB edition can own MCP port 11924 at a time.",
                Some(format!("chat2db stop --edition {other} --json")),
                false,
                "write",
                format!("stop {other} before managing MCP for {target}"),
            ),
            Chat2DbError::AppNotInstalled { edition, .. } => ErrorAdvice::new(
                "Install this edition first.",
                Some(format!("chat2db install --edition {edition} --json")),
                false,
                "write",
                format!("install {edition}"),
            ),
            Chat2DbError::AppAlreadyExists { path } => ErrorAdvice::new(
                "The app already exists; replacing it is high-risk.",
                Some(format!(
                    "chat2db install --edition {} --force --yes --json",
                    edition_from_app_path(path).unwrap_or("<edition>")
                )),
                false,
                "high-risk-write",
                "replace existing desktop app installation",
            ),
            Chat2DbError::AppProcessStillRunning { edition, .. } => ErrorAdvice::new(
                "Stop the desktop app before installing, replacing, or uninstalling it.",
                Some(format!("chat2db stop --edition {edition} --json")),
                false,
                "write",
                format!("stop {edition} desktop app"),
            ),
            Chat2DbError::BackgroundInstallAlreadyRunning { edition, .. } => ErrorAdvice::new(
                "Do not start another background install for this edition.",
                Some(format!("chat2db logs install --edition {edition} --follow")),
                false,
                "read",
                format!("tail existing {edition} install log"),
            ),
            Chat2DbError::LoginTimedOut { edition, .. } => ErrorAdvice::new(
                "The GUI login flow did not complete before the timeout.",
                Some(format!("chat2db login --edition {edition} --json")),
                false,
                "write",
                format!("retry {edition} login after user confirmation"),
            ),
            Chat2DbError::SkillVersionIncompatible { .. } => ErrorAdvice::new(
                "The skill bundle is not compatible with this CLI version.",
                Some("chat2db update --json".to_string()),
                false,
                "write",
                "update chat2db CLI",
            ),
            Chat2DbError::SkillAlreadyInstalled { .. } => ErrorAdvice::new(
                "The skill bundle is already installed.",
                Some("chat2db skill update --json".to_string()),
                false,
                "write",
                "update installed skill bundle",
            ),
            Chat2DbError::UnsupportedPlatform => ErrorAdvice::new(
                "This command is not supported on the current operating system.",
                Some("chat2db status --json".to_string()),
                false,
                "read",
                "inspect cross-platform status only",
            ),
            Chat2DbError::ConfirmationRequired { action } => ErrorAdvice {
                hint: Some("add --yes to confirm".to_string()),
                next_action: None,
                retryable: Some(false),
                risk: Some(ErrorRisk {
                    level: "high-risk-write",
                    action: (*action).to_string(),
                }),
            },
            _ => ErrorAdvice::empty(),
        }
    }
}

fn runtime_api_advice(code: &str) -> ErrorAdvice {
    let (hint, retryable, action) = match code {
        "runtime_edition_mismatch" => (
            "The running Runtime belongs to a different Chat2DB edition.",
            false,
            "inspect the active Runtime edition",
        ),
        "runtime_api_version_unsupported" => (
            "The running Runtime API version is incompatible with this CLI.",
            false,
            "inspect Runtime and CLI versions",
        ),
        "runtime_capability_missing" | "runtime_capabilities_invalid" => (
            "The running Runtime does not satisfy the required CLI contract.",
            false,
            "inspect Runtime capabilities",
        ),
        "runtime_not_ready" => (
            "The direct runtime is not ready yet.",
            true,
            "inspect runtime status before retrying",
        ),
        _ => (
            "The direct runtime rejected the request.",
            false,
            "inspect and correct the reported error",
        ),
    };
    ErrorAdvice::new(
        hint,
        Some("chat2db runtime status --json".to_string()),
        retryable,
        "read",
        action,
    )
}

fn is_empty_details(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Object(map) => map.is_empty(),
        Value::Array(values) => values.is_empty(),
        _ => false,
    }
}

fn edition_from_app_path(path: &std::path::Path) -> Option<&'static str> {
    let path = path.to_string_lossy();
    if path.contains("Chat2DB Local") {
        Some("local")
    } else if path.contains("Chat2DB Pro") {
        Some("pro")
    } else {
        None
    }
}

pub type Result<T> = std::result::Result<T, Chat2DbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_errors_have_json_advice() {
        let cases = [
            Chat2DbError::McpTokenMissing {
                edition: "pro".to_string(),
            },
            Chat2DbError::McpClientHttp {
                url: "http://127.0.0.1:11924/mcp".to_string(),
                message: "connection refused".to_string(),
            },
            Chat2DbError::McpPortAmbiguous {
                target: "pro".to_string(),
                other: "local".to_string(),
            },
            Chat2DbError::AppNotInstalled {
                edition: "pro".to_string(),
                path: PathBuf::from("/Applications/Chat2DB Pro.app"),
            },
            Chat2DbError::AppAlreadyExists {
                path: PathBuf::from("/Applications/Chat2DB Pro.app"),
            },
            Chat2DbError::AppProcessStillRunning {
                edition: "pro".to_string(),
                pids: vec![1234],
            },
            Chat2DbError::BackgroundInstallAlreadyRunning {
                edition: "pro".to_string(),
                pid: 1234,
            },
            Chat2DbError::LoginTimedOut {
                edition: "pro".to_string(),
                seconds: 120,
            },
            Chat2DbError::SkillVersionIncompatible {
                required: ">=0.2.0".to_string(),
                current: "0.1.0".to_string(),
            },
            Chat2DbError::SkillAlreadyInstalled {
                path: PathBuf::from("/Users/tester/.codex/skills/chat2db"),
            },
            Chat2DbError::UnsupportedPlatform,
            Chat2DbError::ConfirmationRequired {
                action: "replace existing desktop app installation",
            },
        ];

        for error in cases {
            let advice = error.advice();
            assert!(advice.hint.is_some(), "missing hint for {}", error.code());
            assert!(
                advice.retryable.is_some(),
                "missing retryable for {}",
                error.code()
            );
            assert!(advice.risk.is_some(), "missing risk for {}", error.code());
        }
    }

    #[test]
    fn unprioritized_errors_have_empty_advice() {
        let advice = Chat2DbError::CompletionsShellUnknown.advice();

        assert_eq!(advice, ErrorAdvice::empty());
    }

    #[test]
    fn runtime_contract_errors_are_not_retryable() {
        for code in [
            "runtime_edition_mismatch",
            "runtime_api_version_unsupported",
            "runtime_capability_missing",
            "runtime_capabilities_invalid",
        ] {
            let error = Chat2DbError::RuntimeApi {
                code: code.to_string(),
                message: "contract mismatch".to_string(),
                details: Value::Null,
            };

            assert_eq!(error.code(), code);
            assert_eq!(error.advice().retryable, Some(false));
        }
    }

    #[test]
    fn transient_runtime_api_errors_remain_retryable() {
        let error = Chat2DbError::RuntimeApi {
            code: "runtime_not_ready".to_string(),
            message: "runtime is starting".to_string(),
            details: Value::Null,
        };

        assert_eq!(error.advice().retryable, Some(true));
    }
}
