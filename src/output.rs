use std::io::{self, Write};

use serde::Serialize;
use serde_json::{Value, json};

use crate::background::BackgroundStartOutcome;
use crate::completions::{CompletionsOutcome, RcAction};
use crate::direct::DirectOutcome;
use crate::dry_run::DryRunOutcome;
use crate::error::{Chat2DbError, ErrorRisk, Result};
use crate::installer::{InstallOutcome, UninstallOutcome};
use crate::login::LoginOutcome;
use crate::logout::LogoutOutcome;
use crate::logs::LogsOutcome;
use crate::mcp::McpOutcome;
use crate::mcp_client::{McpCallOutcome, McpToolsOutcome};
use crate::mcp_config::McpConfigOutcome;
use crate::restart::RestartOutcome;
use crate::runtime::{RuntimeOutcome, RuntimeStateKind};
use crate::self_update::{UninstallCliOutcome, UpdateOutcome};
use crate::skill::SkillOutcome;
use crate::start::StartOutcome;
use crate::status::{StatusAppState, StatusCliRuntimeState, StatusOutcome, StatusRuntimeSummary};
use crate::stop::StopOutcome;
use crate::stop_all::StopAllOutcome;

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    ok: bool,
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk: Option<ErrorRisk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<&'a Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionOutcome {
    pub command: &'static str,
    pub version: &'static str,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Outcome {
    Version(VersionOutcome),
    Status(StatusOutcome),
    Login(LoginOutcome),
    Logout(LogoutOutcome),
    Start(StartOutcome),
    Stop(StopOutcome),
    StopAll(StopAllOutcome),
    Restart(RestartOutcome),
    Update(UpdateOutcome),
    Install(InstallOutcome),
    InstallStarted(BackgroundStartOutcome),
    Uninstall(UninstallOutcome),
    UninstallCli(UninstallCliOutcome),
    Completions(CompletionsOutcome),
    Logs(LogsOutcome),
    Mcp(McpOutcome),
    McpTools(McpToolsOutcome),
    McpCall(McpCallOutcome),
    McpConfig(McpConfigOutcome),
    Runtime(RuntimeOutcome),
    Direct(DirectOutcome),
    Skill(SkillOutcome),
    DryRun(DryRunOutcome),
}

impl Outcome {
    pub fn print<W: Write>(&self, mut writer: W, json_output: bool) -> Result<()> {
        // `completions` without --install streams the raw script to stdout
        // regardless of --json so the output stays sourceable.
        if let Outcome::Completions(outcome) = self
            && let Some(script) = &outcome.script
        {
            writer.write_all(script)?;
            return Ok(());
        }
        // `logs` always streams raw bytes — wrapping arbitrary log content in
        // a JSON envelope would corrupt UTF-8 expectations and lose newlines.
        if let Outcome::Logs(outcome) = self {
            writer.write_all(&outcome.contents)?;
            return Ok(());
        }
        // `mcp config` is itself a copy-pasteable client config. Do not wrap
        // it in the normal ok/data envelope, even when --json is passed.
        if let Outcome::McpConfig(outcome) = self {
            print_raw_json(&mut writer, &outcome.config)?;
            return Ok(());
        }

        if json_output {
            match self {
                Outcome::Version(outcome) => print_json(&mut writer, outcome),
                Outcome::Status(outcome) => print_json(&mut writer, outcome),
                Outcome::Login(outcome) => print_json(&mut writer, outcome),
                Outcome::Logout(outcome) => print_json(&mut writer, outcome),
                Outcome::Start(outcome) => print_json(&mut writer, outcome),
                Outcome::Stop(outcome) => print_json(&mut writer, outcome),
                Outcome::StopAll(outcome) => print_json(&mut writer, outcome),
                Outcome::Restart(outcome) => print_json(&mut writer, outcome),
                Outcome::Update(outcome) => print_json(&mut writer, outcome),
                Outcome::Install(outcome) => print_json(&mut writer, outcome),
                Outcome::InstallStarted(outcome) => print_json(&mut writer, outcome),
                Outcome::Uninstall(outcome) => print_json(&mut writer, outcome),
                Outcome::UninstallCli(outcome) => print_json(&mut writer, outcome),
                Outcome::Completions(outcome) => print_json(&mut writer, outcome),
                Outcome::Mcp(outcome) => print_json(&mut writer, outcome),
                Outcome::McpTools(outcome) => print_json(&mut writer, outcome),
                Outcome::McpCall(outcome) => print_json(&mut writer, outcome),
                Outcome::McpConfig(_) => unreachable!("McpConfig printed above"),
                Outcome::Runtime(outcome) => print_json(&mut writer, outcome),
                Outcome::Direct(outcome) => print_json(&mut writer, outcome),
                Outcome::Skill(outcome) => print_json(&mut writer, outcome),
                Outcome::DryRun(outcome) => print_json(&mut writer, outcome),
                Outcome::Logs(_) => unreachable!("Logs streamed above"),
            }
        } else {
            match self {
                Outcome::Version(outcome) => {
                    writeln!(writer, "chat2db {}", outcome.version)?;
                }
                Outcome::Status(outcome) => {
                    print_status_summary(&mut writer, &outcome.summary)?;
                }
                Outcome::Login(outcome) => {
                    if outcome.status == "not_applicable" {
                        writeln!(
                            writer,
                            "{} does not use account login.",
                            outcome.edition.app_name()
                        )?;
                    } else if outcome.status == "already_logged_in" {
                        writeln!(
                            writer,
                            "Already logged in to {}.",
                            outcome.edition.app_name()
                        )?;
                    } else {
                        writeln!(writer, "Logged in to {}.", outcome.edition.app_name())?;
                    }
                }
                Outcome::Logout(outcome) if outcome.status == "not_applicable" => {
                    writeln!(
                        writer,
                        "{} does not use account login.",
                        outcome.edition.app_name()
                    )?;
                }
                Outcome::Logout(outcome) if outcome.status == "logged_out" => {
                    writeln!(writer, "Logged out of {}.", outcome.edition.app_name())?;
                }
                Outcome::Logout(outcome) => {
                    writeln!(
                        writer,
                        "Already logged out of {}.",
                        outcome.edition.app_name()
                    )?;
                }
                Outcome::Start(outcome) => {
                    writeln!(
                        writer,
                        "Opened {} at {}",
                        outcome.edition.app_name(),
                        outcome.app_path.display()
                    )?;
                }
                Outcome::Stop(outcome) if outcome.status == "stopped" => {
                    writeln!(writer, "Quit {}.", outcome.edition.app_name())?;
                }
                Outcome::Stop(outcome) => {
                    writeln!(writer, "{} is not running.", outcome.edition.app_name())?;
                }
                Outcome::StopAll(outcome) => {
                    for edition in &outcome.editions {
                        writeln!(writer, "{}", edition.edition.app_name())?;
                        writeln!(writer, "  MCP: {}", edition.steps.mcp.status)?;
                        writeln!(writer, "  CLI runtime: {}", edition.steps.cli.status)?;
                        writeln!(writer, "  App: {}", edition.steps.app.status)?;
                        writeln!(
                            writer,
                            "  Safe to exit: {}",
                            yes_no(edition.final_summary.safe_to_exit)
                        )?;
                    }
                }
                Outcome::Restart(outcome) => {
                    if outcome.stop_status == "stopped" {
                        writeln!(
                            writer,
                            "Restarted {} at {}",
                            outcome.edition.app_name(),
                            outcome.app_path.display()
                        )?;
                    } else {
                        writeln!(
                            writer,
                            "Started {} at {} (was not running)",
                            outcome.edition.app_name(),
                            outcome.app_path.display()
                        )?;
                    }
                }
                Outcome::Update(outcome) => match outcome.status {
                    "scheduled" => {
                        writeln!(
                            writer,
                            "Scheduled chat2db CLI update at {} after this process exits",
                            outcome.target_path.display()
                        )?;
                    }
                    "up_to_date" => {
                        writeln!(
                            writer,
                            "chat2db CLI is already up to date at {}",
                            outcome.target_path.display()
                        )?;
                    }
                    _ => {
                        writeln!(
                            writer,
                            "Updated chat2db CLI at {}",
                            outcome.target_path.display()
                        )?;
                    }
                },
                Outcome::Install(outcome) => {
                    writeln!(
                        writer,
                        "Installed {} to {}",
                        outcome.edition.app_name(),
                        outcome.app_path.display()
                    )?;
                }
                Outcome::InstallStarted(outcome) => {
                    writeln!(
                        writer,
                        "Started {} install in background (pid {}). Log: {}",
                        outcome.edition.app_name(),
                        outcome.pid,
                        outcome.log_path.display()
                    )?;
                }
                Outcome::Uninstall(outcome) if outcome.status == "removed" => {
                    writeln!(writer, "Removed {}", outcome.app_path.display())?;
                }
                Outcome::Uninstall(outcome) => {
                    writeln!(writer, "Already absent: {}", outcome.app_path.display())?;
                }
                Outcome::UninstallCli(outcome) if outcome.status == "removed" => {
                    writeln!(
                        writer,
                        "Removed chat2db CLI at {}",
                        outcome.target_path.display()
                    )?;
                }
                Outcome::UninstallCli(outcome) if outcome.status == "scheduled" => {
                    writeln!(
                        writer,
                        "Scheduled chat2db CLI removal at {} after this process exits",
                        outcome.target_path.display()
                    )?;
                }
                Outcome::UninstallCli(outcome) => {
                    writeln!(writer, "Already absent: {}", outcome.target_path.display())?;
                }
                Outcome::Completions(outcome) => {
                    let script_path = outcome
                        .script_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    writeln!(
                        writer,
                        "Installed {} completions at {script_path}",
                        outcome.shell
                    )?;
                    match (outcome.rc_action, outcome.rc_path.as_ref()) {
                        (RcAction::Added, Some(rc)) => {
                            writeln!(
                                writer,
                                "Added source line to {}. Open a new shell or run `exec $SHELL`.",
                                rc.display()
                            )?;
                        }
                        (RcAction::AlreadyConfigured, Some(rc)) => {
                            writeln!(
                                writer,
                                "{} already sources chat2db completions.",
                                rc.display()
                            )?;
                        }
                        (RcAction::NotApplicable, _) => {
                            writeln!(writer, "fish auto-loads completions on next shell.")?;
                        }
                        (_, None) => {}
                    }
                }
                Outcome::Skill(outcome) => {
                    let target_label = skill_target_label(outcome.client);
                    match (outcome.operation, outcome.status) {
                        ("install", "installed") => {
                            writeln!(
                                writer,
                                "Installed chat2db skill bundle{target_label} at {}",
                                outcome.target_path.display()
                            )?;
                        }
                        ("install", "updated") => {
                            writeln!(
                                writer,
                                "Reinstalled chat2db skill bundle{target_label} at {}",
                                outcome.target_path.display()
                            )?;
                        }
                        ("update", _) => {
                            writeln!(
                                writer,
                                "Updated chat2db skill bundle{target_label} at {}",
                                outcome.target_path.display()
                            )?;
                        }
                        ("uninstall", "removed") => {
                            writeln!(
                                writer,
                                "Removed chat2db skill bundle at {}",
                                outcome.target_path.display()
                            )?;
                        }
                        ("uninstall", "already_absent") => {
                            writeln!(writer, "Already absent: {}", outcome.target_path.display())?;
                        }
                        _ => {
                            writeln!(
                                writer,
                                "{} chat2db skill bundle at {} (status: {})",
                                outcome.operation,
                                outcome.target_path.display(),
                                outcome.status
                            )?;
                        }
                    }
                }
                Outcome::Mcp(outcome) => {
                    for edition in &outcome.editions {
                        writeln!(writer, "MCP {}", edition.edition.app_name())?;
                        writeln!(writer, "  settings: {}", edition.settings_path.display())?;
                        writeln!(writer, "  enabled: {}", edition.enabled)?;
                        writeln!(
                            writer,
                            "  token: {}",
                            token_state_label(edition.token_state)
                        )?;
                        writeln!(
                            writer,
                            "  app: {}",
                            if edition.app_running {
                                "running"
                            } else {
                                "stopped"
                            }
                        )?;
                        writeln!(
                            writer,
                            "  endpoint: {} ({})",
                            endpoint_state_label(edition.endpoint_state),
                            edition.url
                        )?;
                        writeln!(writer, "  needs restart: {}", edition.needs_restart)?;
                        writeln!(
                            writer,
                            "  next: {}",
                            mcp_next_action_label(edition.next_action)
                        )?;
                    }
                }
                Outcome::McpTools(outcome) => {
                    for tool in &outcome.tools {
                        writeln!(writer, "{}", tool.name)?;
                        if let Some(description) = &tool.description {
                            writeln!(writer, "  {description}")?;
                        }
                        if let Some(input_schema) = &tool.input_schema {
                            writeln!(writer, "  input: {}", compact_json(input_schema)?)?;
                        }
                    }
                }
                Outcome::McpCall(outcome) => {
                    print_mcp_call_result(&mut writer, outcome)?;
                }
                Outcome::McpConfig(_) => unreachable!("McpConfig printed above"),
                Outcome::Runtime(outcome) => {
                    writeln!(
                        writer,
                        "{} runtime: {}",
                        outcome.edition.app_name(),
                        runtime_state_label(outcome.status)
                    )?;
                    if let Some(pid) = outcome.pid {
                        writeln!(writer, "  pid: {pid}")?;
                    }
                    if let Some(url) = &outcome.url {
                        writeln!(writer, "  url: {url}")?;
                    }
                    writeln!(writer, "  healthy: {}", outcome.healthy)?;
                }
                Outcome::Direct(outcome) => {
                    writeln!(writer, "{}", pretty_json(&outcome.data)?)?;
                }
                Outcome::DryRun(outcome) => {
                    writeln!(writer, "{} dry run", outcome.operation)?;
                    writeln!(writer, "  risk: {}", outcome.risk.level)?;
                    writeln!(
                        writer,
                        "  confirmation required: {}",
                        outcome.confirmation_required
                    )?;
                    if let Some(command) = &outcome.next_action_command {
                        writeln!(writer, "  next: {command}")?;
                    }
                }
                Outcome::Logs(_) => unreachable!("Logs streamed above"),
            }
            Ok(())
        }
    }
}

fn print_mcp_call_result<W: Write>(writer: &mut W, outcome: &McpCallOutcome) -> Result<()> {
    if let Some(content) = outcome
        .result
        .get("content")
        .and_then(serde_json::Value::as_array)
    {
        let mut printed = false;
        for item in content {
            if item.get("type").and_then(serde_json::Value::as_str) == Some("text")
                && let Some(text) = item.get("text").and_then(serde_json::Value::as_str)
            {
                write!(writer, "{text}")?;
                if !text.ends_with('\n') {
                    writeln!(writer)?;
                }
                printed = true;
            }
        }
        if printed {
            return Ok(());
        }
    }

    writeln!(writer, "{}", pretty_json(&outcome.result)?)?;
    Ok(())
}

fn compact_json(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string(value).map_err(Chat2DbError::ManifestJson)
}

fn pretty_json(value: &serde_json::Value) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(Chat2DbError::ManifestJson)
}

fn token_state_label(state: crate::mcp::TokenState) -> &'static str {
    match state {
        crate::mcp::TokenState::Present => "present",
        crate::mcp::TokenState::Missing => "missing",
    }
}

fn endpoint_state_label(state: crate::mcp::EndpointState) -> &'static str {
    match state {
        crate::mcp::EndpointState::Alive => "alive",
        crate::mcp::EndpointState::NotListening => "not listening",
        crate::mcp::EndpointState::Timeout => "timeout",
        crate::mcp::EndpointState::NotChecked => "not checked",
        crate::mcp::EndpointState::Ambiguous => "ambiguous",
    }
}

fn mcp_next_action_label(action: crate::mcp::McpNextAction) -> &'static str {
    match action {
        crate::mcp::McpNextAction::None => "none",
        crate::mcp::McpNextAction::Enable => "enable",
        crate::mcp::McpNextAction::Start => "start",
        crate::mcp::McpNextAction::Restart => "restart",
        crate::mcp::McpNextAction::StopOtherEdition => "stop_other_edition",
        crate::mcp::McpNextAction::InspectRuntimeLog => "inspect_runtime_log",
    }
}

fn runtime_state_label(state: RuntimeStateKind) -> &'static str {
    match state {
        RuntimeStateKind::Running => "running",
        RuntimeStateKind::Started => "started",
        RuntimeStateKind::Stopped => "stopped",
        RuntimeStateKind::Stale => "stale",
    }
}

fn skill_target_label(client: Option<crate::skill::SkillClient>) -> &'static str {
    match client {
        Some(crate::skill::SkillClient::ClaudeCode) => " for claude-code",
        Some(crate::skill::SkillClient::Codex) => " for codex",
        None => "",
    }
}

fn print_status_summary<W: Write>(writer: &mut W, summary: &StatusRuntimeSummary) -> Result<()> {
    print_edition_status_summary(writer, "Chat2DB Community", &summary.community)?;
    print_edition_status_summary(writer, "Chat2DB Pro", &summary.pro)?;
    print_edition_status_summary(writer, "Chat2DB Local", &summary.local)?;
    Ok(())
}

fn print_edition_status_summary<W: Write>(
    writer: &mut W,
    title: &str,
    summary: &crate::status::EditionRuntimeSummary,
) -> Result<()> {
    writeln!(writer, "{title}")?;
    writeln!(
        writer,
        "  App: {}{}",
        app_state_label(summary.app.state),
        pids_suffix(&summary.app.pids)
    )?;
    writeln!(
        writer,
        "  CLI runtime: {}{}",
        cli_runtime_state_label(summary.cli.state),
        option_pid_suffix(summary.cli.pid)
    )?;
    writeln!(
        writer,
        "  MCP: {}, {}",
        if summary.mcp.enabled {
            "enabled"
        } else {
            "disabled"
        },
        endpoint_state_label(summary.mcp.endpoint)
    )?;
    if let Some(background) = &summary.background_install {
        writeln!(
            writer,
            "  Background install: {}",
            background_state_label(&background.state)
        )?;
    }
    writeln!(writer, "  Safe to exit: {}", yes_no(summary.safe_to_exit))?;
    if let Some(next_action) = &summary.next_action {
        writeln!(
            writer,
            "  Next action: {}",
            next_action.strip_suffix(" --json").unwrap_or(next_action)
        )?;
    }
    Ok(())
}

fn background_state_label(state: &crate::background::BackgroundState) -> String {
    match state {
        crate::background::BackgroundState::NotRunning => "not_running".to_string(),
        crate::background::BackgroundState::Running { pid } => format!("running (pid {pid})"),
        crate::background::BackgroundState::Completed { exit_code } => {
            format!("completed (exit {exit_code})")
        }
        crate::background::BackgroundState::Unknown { error } => {
            format!("unknown ({error})")
        }
    }
}

fn app_state_label(state: StatusAppState) -> &'static str {
    match state {
        StatusAppState::Running => "running",
        StatusAppState::Stopped => "stopped",
        StatusAppState::NotInstalled => "not_installed",
        StatusAppState::Invalid => "invalid",
        StatusAppState::Unknown => "unknown",
    }
}

fn cli_runtime_state_label(state: StatusCliRuntimeState) -> &'static str {
    match state {
        StatusCliRuntimeState::Running => "running",
        StatusCliRuntimeState::Stopped => "stopped",
        StatusCliRuntimeState::Stale => "stale",
        StatusCliRuntimeState::Unknown => "unknown",
    }
}

fn pids_suffix(pids: &[u32]) -> String {
    if pids.is_empty() {
        return String::new();
    }
    let pids = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(" (pid {pids})")
}

fn option_pid_suffix(pid: Option<u32>) -> String {
    pid.map(|pid| format!(" (pid {pid})")).unwrap_or_default()
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn print_json<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    let envelope = json!({
        "ok": true,
        "data": value,
    });
    serde_json::to_writer(&mut *writer, &envelope)?;
    writeln!(writer)?;
    Ok(())
}

fn print_raw_json<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    serde_json::to_writer_pretty(&mut *writer, value)?;
    writeln!(writer)?;
    Ok(())
}

pub fn print_error<W: Write>(
    mut writer: W,
    error: &Chat2DbError,
    json_output: bool,
) -> io::Result<()> {
    if json_output {
        let advice = error.advice();
        let envelope = ErrorEnvelope {
            ok: false,
            error: ErrorBody {
                code: error.code(),
                message: error.to_string(),
                hint: advice.hint,
                next_action: advice.next_action,
                retryable: advice.retryable,
                risk: advice.risk,
                details: error.details(),
            },
        };
        serde_json::to_writer(&mut writer, &envelope)?;
        writeln!(writer)
    } else {
        let advice = error.advice();
        writeln!(writer, "error: {error}")?;
        if let Some(hint) = advice.hint {
            writeln!(writer, "hint: {hint}")?;
        }
        if let Some(next_action) = advice.next_action {
            writeln!(writer, "next: {next_action}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::edition::Edition;

    #[test]
    fn prints_success_json_envelope() {
        let outcome = Outcome::Install(InstallOutcome {
            command: "install",
            edition: Edition::Pro,
            status: "installed",
            app_path: PathBuf::from("/Applications/Chat2DB Pro.app"),
            artifact_url: "https://cdn.chat2db-ai.com/download/latest/Chat2DB-Pro-arm64-latest.dmg"
                .to_string(),
            artifact_type: "dmg",
        });
        let mut buffer = Vec::new();

        outcome.print(&mut buffer, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["command"], "install");
        assert_eq!(json["data"]["edition"], "pro");
        assert_eq!(json["data"]["status"], "installed");
    }

    #[test]
    fn mcp_config_prints_raw_json_config_even_with_json_output() {
        let outcome = Outcome::McpConfig(crate::mcp_config::McpConfigOutcome {
            command: "mcp_config",
            edition: Edition::Pro,
            config: serde_json::json!({
                "mcpServers": {
                    "chat2db": {
                        "transport": "streamable_http",
                        "url": crate::mcp::mcp_url(Edition::Pro),
                        "headers": {
                            crate::mcp_config::MCP_TOKEN_HEADER: "secret-token",
                        },
                    },
                },
            }),
        });
        let mut buffer = Vec::new();

        outcome.print(&mut buffer, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert!(json.get("ok").is_none());
        assert_eq!(
            json["mcpServers"]["chat2db"]["headers"][crate::mcp_config::MCP_TOKEN_HEADER],
            "secret-token"
        );
    }

    #[test]
    fn prints_error_json_envelope() {
        let error = Chat2DbError::CompletionsShellUnknown;
        let mut buffer = Vec::new();

        print_error(&mut buffer, &error, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "completions_shell_unknown");
        assert!(json["error"].get("hint").is_none());
        assert!(json["error"].get("next_action").is_none());
        assert!(json["error"].get("retryable").is_none());
        assert!(json["error"].get("risk").is_none());
    }

    #[test]
    fn prints_error_json_advice_fields() {
        let error = Chat2DbError::McpTokenMissing {
            edition: "pro".to_string(),
        };
        let mut buffer = Vec::new();

        print_error(&mut buffer, &error, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "mcp_token_missing");
        assert_eq!(json["error"]["hint"], "Enable MCP for this edition first.");
        assert_eq!(
            json["error"]["next_action"],
            "chat2db mcp enable --edition pro --json"
        );
        assert_eq!(json["error"]["retryable"], false);
        assert_eq!(json["error"]["risk"]["level"], "write");
        assert_eq!(json["error"]["risk"]["action"], "enable MCP for pro");
    }

    #[test]
    fn prints_runtime_error_json_details() {
        let error = Chat2DbError::RuntimeApi {
            code: "sql_result_set_not_found".to_string(),
            message: "SQL result set not found: 2".to_string(),
            details: json!({
                "resultSetId": 2,
                "availableResultSetIds": [1]
            }),
        };
        let mut buffer = Vec::new();

        print_error(&mut buffer, &error, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "sql_result_set_not_found");
        assert_eq!(json["error"]["retryable"], false);
        assert_eq!(json["error"]["details"]["resultSetId"], 2);
        assert_eq!(
            json["error"]["details"]["availableResultSetIds"],
            json!([1])
        );
    }

    #[test]
    fn runtime_contract_error_json_is_not_retryable() {
        let error = Chat2DbError::RuntimeApi {
            code: "runtime_edition_mismatch".to_string(),
            message: "runtime edition mismatch".to_string(),
            details: json!({
                "expectedEdition": "community",
                "actualEdition": "local"
            }),
        };
        let mut buffer = Vec::new();

        print_error(&mut buffer, &error, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(json["error"]["code"], "runtime_edition_mismatch");
        assert_eq!(json["error"]["retryable"], false);
    }

    #[test]
    fn prints_text_error_next_action_for_missing_app() {
        let error = Chat2DbError::AppNotInstalled {
            edition: "local".to_string(),
            path: PathBuf::from("/Applications/Chat2DB Local.app"),
        };
        let mut buffer = Vec::new();

        print_error(&mut buffer, &error, false).unwrap();

        let text = String::from_utf8(buffer).unwrap();
        assert!(text.contains("app is not installed"), "got: {text}");
        assert!(
            text.contains("hint: Install this edition first."),
            "got: {text}"
        );
        assert!(
            text.contains("next: chat2db install --edition local --json"),
            "got: {text}"
        );
    }

    #[test]
    fn prints_confirmation_required_json_risk_object() {
        let error = Chat2DbError::ConfirmationRequired {
            action: "remove installed app",
        };
        let mut buffer = Vec::new();

        print_error(&mut buffer, &error, true).unwrap();

        let json: Value = serde_json::from_slice(&buffer).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "confirmation_required");
        assert_eq!(json["error"]["hint"], "add --yes to confirm");
        assert_eq!(json["error"]["retryable"], false);
        assert!(json["error"].get("next_action").is_none());
        assert_eq!(json["error"]["risk"]["level"], "high-risk-write");
        assert_eq!(json["error"]["risk"]["action"], "remove installed app");
    }

    #[test]
    fn status_text_prints_runtime_summary() {
        use crate::status::{
            EditionRuntimeSummary, StatusAppState, StatusAppSummary, StatusCliRuntimeState,
            StatusCliRuntimeSummary, StatusMcpSummary, StatusOutcome, StatusRuntimeSummary,
        };

        let outcome = Outcome::Status(StatusOutcome {
            command: "status",
            cli_version: env!("CARGO_PKG_VERSION"),
            summary: StatusRuntimeSummary {
                community: EditionRuntimeSummary {
                    app: StatusAppSummary {
                        state: StatusAppState::NotInstalled,
                        pids: vec![],
                    },
                    cli: StatusCliRuntimeSummary {
                        state: StatusCliRuntimeState::Stopped,
                        pid: None,
                        port: None,
                        url: None,
                        error: None,
                    },
                    mcp: StatusMcpSummary {
                        enabled: false,
                        endpoint: crate::mcp::EndpointState::NotListening,
                        needs_restart: false,
                        error: None,
                    },
                    background_install: None,
                    safe_to_exit: true,
                    next_action: Some("chat2db install --edition community --json".to_string()),
                },
                pro: EditionRuntimeSummary {
                    app: StatusAppSummary {
                        state: StatusAppState::Running,
                        pids: vec![4242],
                    },
                    cli: StatusCliRuntimeSummary {
                        state: StatusCliRuntimeState::Stopped,
                        pid: None,
                        port: None,
                        url: None,
                        error: None,
                    },
                    mcp: StatusMcpSummary {
                        enabled: false,
                        endpoint: crate::mcp::EndpointState::NotListening,
                        needs_restart: false,
                        error: None,
                    },
                    background_install: None,
                    safe_to_exit: false,
                    next_action: Some("chat2db stop --edition pro --all --json".to_string()),
                },
                local: EditionRuntimeSummary {
                    app: StatusAppSummary {
                        state: StatusAppState::NotInstalled,
                        pids: vec![],
                    },
                    cli: StatusCliRuntimeSummary {
                        state: StatusCliRuntimeState::Stopped,
                        pid: None,
                        port: None,
                        url: None,
                        error: None,
                    },
                    mcp: StatusMcpSummary {
                        enabled: false,
                        endpoint: crate::mcp::EndpointState::NotListening,
                        needs_restart: false,
                        error: None,
                    },
                    background_install: None,
                    safe_to_exit: true,
                    next_action: Some("chat2db install --edition local --json".to_string()),
                },
            },
            editions: vec![],
            mcp: crate::status::StatusMcpReport {
                command: "mcp_status",
                editions: vec![],
                error: None,
            },
            skills: crate::status::SkillStatusReport { targets: vec![] },
        });

        let mut buffer = Vec::new();
        outcome.print(&mut buffer, false).unwrap();
        let text = String::from_utf8(buffer).unwrap();

        assert!(text.contains("Chat2DB Pro"), "got: {text}");
        assert!(text.contains("App: running (pid 4242)"), "got: {text}");
        assert!(text.contains("CLI runtime: stopped"), "got: {text}");
        assert!(text.contains("MCP: disabled, not listening"), "got: {text}");
        assert!(text.contains("Safe to exit: no"), "got: {text}");
        assert!(
            text.contains("Next action: chat2db stop --edition pro --all"),
            "got: {text}"
        );
        assert!(text.contains("Chat2DB Local"), "got: {text}");
        assert!(text.contains("App: not_installed"), "got: {text}");
        assert!(text.contains("Safe to exit: yes"), "got: {text}");
        assert!(
            text.contains("Next action: chat2db install --edition local"),
            "got: {text}"
        );
    }
}
