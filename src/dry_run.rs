use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::edition::Edition;
use crate::installer::SystemPaths;
use crate::mcp::mcp_url;
use crate::skill::{SKILL_DIR_NAME, SkillClient, SkillTargetRequest};

#[derive(Debug, Clone, Serialize)]
pub struct DryRunOutcome {
    pub command: &'static str,
    pub operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition: Option<Edition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<SkillClient>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    pub paths_affected: Vec<PathBuf>,
    pub restart_required: bool,
    pub risk: DryRunRisk,
    pub confirmation_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DryRunRisk {
    pub level: &'static str,
    pub action: String,
}

pub fn install_dry_run(
    edition: Edition,
    force: bool,
    yes: bool,
    paths: &SystemPaths,
) -> DryRunOutcome {
    let risk_level = if force { "high-risk-write" } else { "write" };
    DryRunOutcome {
        command: "install",
        operation: "install",
        edition: Some(edition),
        client: None,
        endpoint: None,
        tool: None,
        args: None,
        paths_affected: vec![edition.app_path(&paths.applications_dir)],
        restart_required: false,
        risk: DryRunRisk {
            level: risk_level,
            action: if force {
                format!("force install {}", edition.as_str())
            } else {
                format!("install {}", edition.as_str())
            },
        },
        confirmation_required: force && !yes,
        next_action_command: Some(if force {
            format!(
                "chat2db install --edition {} --force --yes --json",
                edition.as_str()
            )
        } else {
            format!("chat2db install --edition {} --json", edition.as_str())
        }),
    }
}

pub fn uninstall_dry_run(edition: Edition, yes: bool, paths: &SystemPaths) -> DryRunOutcome {
    DryRunOutcome {
        command: "uninstall",
        operation: "uninstall",
        edition: Some(edition),
        client: None,
        endpoint: None,
        tool: None,
        args: None,
        paths_affected: vec![edition.app_path(&paths.applications_dir)],
        restart_required: false,
        risk: DryRunRisk {
            level: "high-risk-write",
            action: format!("uninstall {}", edition.as_str()),
        },
        confirmation_required: !yes,
        next_action_command: Some(format!(
            "chat2db uninstall --edition {} --yes --json",
            edition.as_str()
        )),
    }
}

pub fn uninstall_cli_dry_run(yes: bool, target_path: Option<PathBuf>) -> DryRunOutcome {
    let path = target_path
        .unwrap_or_else(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from("chat2db")));
    DryRunOutcome {
        command: "uninstall",
        operation: "uninstall_cli",
        edition: None,
        client: None,
        endpoint: None,
        tool: None,
        args: None,
        paths_affected: vec![path],
        restart_required: false,
        risk: DryRunRisk {
            level: "high-risk-write",
            action: "uninstall cli".to_string(),
        },
        confirmation_required: !yes,
        next_action_command: Some("chat2db uninstall --cli --yes --json".to_string()),
    }
}

pub fn skill_install_dry_run(target: SkillTargetRequest, force: bool, yes: bool) -> DryRunOutcome {
    let target_path = match target.client {
        Some(client) => client.target_dir(&target.home_dir),
        None => target.current_dir.join(SKILL_DIR_NAME),
    };
    let risk_level = if force { "high-risk-write" } else { "write" };
    let client_arg = match target.client {
        Some(SkillClient::Codex) => " --client codex",
        Some(SkillClient::ClaudeCode) => " --client claude-code",
        None => "",
    };
    let force_arg = if force { " --force --yes" } else { "" };
    DryRunOutcome {
        command: "skill",
        operation: "skill_install",
        edition: None,
        client: target.client,
        endpoint: None,
        tool: None,
        args: None,
        paths_affected: vec![target_path],
        restart_required: false,
        risk: DryRunRisk {
            level: risk_level,
            action: if force {
                "force install skill".to_string()
            } else {
                "install skill".to_string()
            },
        },
        confirmation_required: force && !yes,
        next_action_command: Some(format!(
            "chat2db skill install{client_arg}{force_arg} --json"
        )),
    }
}

pub fn mcp_call_dry_run(
    edition: Edition,
    tool: String,
    arguments: Map<String, Value>,
) -> DryRunOutcome {
    let risk_level = mcp_tool_risk_level(&tool);
    DryRunOutcome {
        command: "mcp_call",
        operation: "mcp_call",
        edition: Some(edition),
        client: None,
        endpoint: Some(mcp_url(edition)),
        tool: Some(tool.clone()),
        args: Some(Value::Object(arguments.clone())),
        paths_affected: Vec::new(),
        restart_required: false,
        risk: DryRunRisk {
            level: risk_level,
            action: format!("call MCP tool {tool}"),
        },
        confirmation_required: false,
        next_action_command: Some(format!(
            "chat2db mcp call {} --edition {} --args-json '{}' --json",
            tool,
            edition.as_str(),
            serde_json::to_string(&Value::Object(arguments)).unwrap_or_else(|_| "{}".to_string())
        )),
    }
}

pub fn mcp_tool_risk_level(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    let high_risk_terms = [
        "execute", "update", "delete", "drop", "create", "insert", "alter", "import", "export",
        "remove",
    ];
    if high_risk_terms.iter().any(|term| name.contains(term)) {
        return "high-risk-write";
    }

    let read_terms = ["select", "list", "get", "read", "status"];
    if read_terms.iter().any(|term| name.contains(term)) {
        return "read";
    }

    "write"
}
