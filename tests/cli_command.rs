use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use serde_json::Value;
use std::path::Path;

fn isolated_command(home: &Path) -> Command {
    let local_app_data = home.join("AppData").join("Local");
    let xdg_state_home = home.join(".local").join("state");
    std::fs::create_dir_all(&local_app_data).unwrap();
    std::fs::create_dir_all(&xdg_state_home).unwrap();

    let mut command = Command::cargo_bin("chat2db").unwrap();
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LOCALAPPDATA", local_app_data)
        .env("XDG_STATE_HOME", xdg_state_home);
    command
}

#[test]
fn version_command_prints_package_version() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .arg("version")
        .assert()
        .success()
        .stdout(contains(format!("chat2db {}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn root_short_version_prints_package_version() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .arg("-v")
        .assert()
        .success()
        .stdout(contains(format!("chat2db {}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn root_short_version_supports_json() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["-v", "--json"])
        .assert()
        .success()
        .stdout(contains(format!(
            r#""version":"{}""#,
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn root_single_dash_version_is_accepted() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .arg("-version")
        .assert()
        .success()
        .stdout(contains(format!("chat2db {}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn root_single_dash_version_supports_json() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["-version", "--json"])
        .assert()
        .success()
        .stdout(contains(format!(
            r#""version":"{}""#,
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn root_long_version_prints_package_version() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .arg("--version")
        .assert()
        .success()
        .stdout(contains(format!("chat2db {}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn root_long_version_supports_json() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["--version", "--json"])
        .assert()
        .success()
        .stdout(contains(format!(
            r#""version":"{}""#,
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn version_command_supports_json() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["version", "--json"])
        .assert()
        .success()
        .stdout(contains(format!(
            r#""version":"{}""#,
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn root_help_documents_lowercase_version_flag() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("-v, --version"))
        .stdout(predicates::str::is_match("-V, --version").unwrap().not());
}

#[test]
fn db_connection_test_rejects_mixed_existing_and_temporary_args() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args([
            "db",
            "connection-test",
            "--data-source-id",
            "195652",
            "--db-type",
            "MYSQL",
            "--json",
        ])
        .assert()
        .failure()
        .stderr(contains("invalid_connection_test_args"))
        .stderr(contains("connection-test --help"));
}

#[test]
fn db_connection_test_rejects_unsupported_host_mode_db_type() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args([
            "db",
            "connection-test",
            "--db-type",
            "SQLITE",
            "--host",
            "localhost",
            "--port",
            "0",
            "--database",
            "test.sqlite",
            "--user",
            "root",
            "--password",
            "secret",
            "--json",
        ])
        .assert()
        .failure()
        .stderr(contains("invalid_connection_test_args"))
        .stderr(contains("pass --url instead"));
}

#[cfg(target_os = "linux")]
#[test]
fn db_connection_test_with_valid_args_rejects_unsupported_platform() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args([
            "db",
            "connection-test",
            "--data-source-id",
            "195652",
            "--json",
        ])
        .assert()
        .failure()
        .stderr(contains("unsupported_platform"));
}

#[test]
fn update_help_uses_cdn_and_has_no_source_dir() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(contains("official CDN"))
        .stdout(contains("chat2db update --target ~/.local/bin/chat2db"))
        .stdout(predicates::str::is_match("source-dir").unwrap().not());
}

#[test]
fn status_help_describes_status_command() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["status", "--help"])
        .assert()
        .success()
        .stdout(contains(
            "Show app, CLI runtime, and MCP status for every Chat2DB edition",
        ))
        .stdout(contains("chat2db status --json"));
}

#[test]
fn status_json_reports_runtime_summary_for_agents() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let local_log = home.join(".chat2db_local_edition/chat2db-enterprise/logs/application.log");
    std::fs::create_dir_all(local_log.parent().unwrap()).unwrap();
    std::fs::write(&local_log, b"runtime log\n").unwrap();
    let pro_cookie = home.join(".chat2db/cache/chat2db-enterprise-cookie-release");
    std::fs::create_dir_all(pro_cookie.parent().unwrap()).unwrap();
    std::fs::write(
        &pro_cookie,
        r#"{"Chat2db-BS-TOKEN":{"value":"MTIz"},"CHAT2DB":{"value":"session-secret"}}"#,
    )
    .unwrap();

    let output = isolated_command(&home)
        .args(["status", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("Chat2db-BS-TOKEN"));
    assert!(!stdout.contains("session-secret"));

    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &envelope["data"];
    assert!(data.get("editions").is_none());
    assert!(data.get("skills").is_none());
    assert!(data["summary"]["pro"]["app"]["state"].is_string());
    assert!(data["summary"]["pro"]["app"]["pids"].is_array());
    assert!(data["summary"]["pro"]["cli"]["state"].is_string());
    assert!(data["summary"]["pro"]["mcp"]["enabled"].is_boolean());
    assert!(data["summary"]["pro"]["mcp"]["endpoint"].is_string());
    assert!(data["summary"]["pro"]["safe_to_exit"].is_boolean());
    assert!(data["summary"]["local"]["app"]["state"].is_string());
    assert!(data["summary"]["local"]["safe_to_exit"].is_boolean());
}

#[test]
fn status_json_aggregates_cli_mcp_and_skill_status() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let current = temp.path().join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&current).unwrap();
    let pro_settings = home.join(".chat2db/chat2db_cache_pro/settings.json");
    std::fs::create_dir_all(pro_settings.parent().unwrap()).unwrap();
    std::fs::write(
        &pro_settings,
        r#"{"enableMcp":true,"mcpAuthToken":"secret-token"}"#,
    )
    .unwrap();
    let codex_skill = home.join(".codex/skills/chat2db-cli");
    std::fs::create_dir_all(&codex_skill).unwrap();
    std::fs::write(
        codex_skill.join("SKILL.md"),
        r#"---
name: chat2db-cli
version: 0.1.0
compatible-cli-versions: ">=0.1, <1"
---
"#,
    )
    .unwrap();

    let output = isolated_command(&home)
        .args(["status", "--json"])
        .current_dir(&current)
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("secret-token"));

    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &envelope["data"];
    assert_eq!(data["cli_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(data["summary"]["pro"]["mcp"]["enabled"], true);
    assert!(data["summary"]["pro"]["mcp"]["endpoint"].is_string());
    assert!(data["summary"]["pro"]["mcp"]["needs_restart"].is_boolean());
    assert!(data["summary"]["pro"]["cli"]["state"].is_string());
    assert!(data["summary"]["pro"]["safe_to_exit"].is_boolean());
    assert!(data.get("mcp").is_none());
    assert!(data.get("skills").is_none());
}

#[test]
fn mcp_status_json_reports_both_editions_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let pro_settings = home.join(".chat2db/chat2db_cache_pro/settings.json");
    std::fs::create_dir_all(pro_settings.parent().unwrap()).unwrap();
    std::fs::write(
        &pro_settings,
        r#"{"enableMcp":false,"mcpAuthToken":"secret-token"}"#,
    )
    .unwrap();

    let output = isolated_command(&home)
        .args(["mcp", "status", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("secret-token"));

    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["command"], "mcp_status");
    let editions = envelope["data"]["editions"].as_array().unwrap();
    assert_eq!(editions.len(), 3);
    let community = editions
        .iter()
        .find(|edition| edition["edition"] == "community")
        .unwrap();
    assert_eq!(community["url"], "http://127.0.0.1:10825/mcp");
    let pro = editions
        .iter()
        .find(|edition| edition["edition"] == "pro")
        .unwrap();
    assert_eq!(
        pro["settings_path"],
        pro_settings.to_string_lossy().as_ref()
    );
    assert_eq!(pro["enabled"], false);
    assert_eq!(pro["token_state"], "present");
    assert_eq!(pro["url"], "http://127.0.0.1:11924/mcp");
    assert!(pro["endpoint_state"].is_string());
    assert!(pro["needs_restart"].is_boolean());
    assert!(pro["next_action"].is_string());
}

#[test]
fn mcp_change_commands_reject_edition_all() {
    for op in ["enable", "disable", "restart"] {
        let mut command = Command::cargo_bin("chat2db").unwrap();
        command
            .args(["mcp", op, "--edition", "all"])
            .assert()
            .failure()
            .stderr(contains("invalid value 'all'"));
    }
}

#[test]
fn mcp_help_documents_tool_commands() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["mcp", "--help"])
        .assert()
        .success()
        .stdout(contains("List tools from the Chat2DB desktop MCP endpoint"))
        .stdout(contains(
            "Call one tool on the Chat2DB desktop MCP endpoint",
        ))
        .stdout(contains("chat2db mcp call execute_sql"));
}

#[test]
fn mcp_call_requires_valid_args_json_before_network() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = isolated_command(&home);

    command
        .args(["mcp", "call", "execute_sql", "--args-json", "[]", "--json"])
        .assert()
        .failure()
        .stderr(contains(r#""code":"mcp_args_invalid_json""#));
}

#[test]
fn destructive_commands_require_yes_with_structured_error() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    let output = command
        .args(["uninstall", "--edition", "pro", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "confirmation_required");
    assert_eq!(envelope["error"]["hint"], "add --yes to confirm");
    assert_eq!(envelope["error"]["retryable"], false);
    assert_eq!(envelope["error"]["risk"]["level"], "high-risk-write");
    assert_eq!(envelope["error"]["risk"]["action"], "uninstall app");
}

#[test]
fn dry_run_reports_paths_risk_and_next_action_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let output = isolated_command(&home)
        .args([
            "skill",
            "install",
            "--client",
            "codex",
            "--force",
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &envelope["data"];
    assert_eq!(data["command"], "skill");
    assert_eq!(data["operation"], "skill_install");
    assert_eq!(data["client"], "codex");
    assert_eq!(data["risk"]["level"], "high-risk-write");
    assert_eq!(data["confirmation_required"], true);
    assert_eq!(
        data["paths_affected"][0],
        home.join(".codex/skills/chat2db-cli")
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(
        data["next_action_command"],
        "chat2db skill install --client codex --force --yes --json"
    );
    assert!(!home.join(".codex/skills/chat2db-cli").exists());
}

#[test]
fn mcp_call_dry_run_reports_tool_args_and_risk_without_token() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();

    let output = isolated_command(&home)
        .args([
            "mcp",
            "call",
            "execute_sql",
            "--args-json",
            r#"{"sql":"select 1"}"#,
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &envelope["data"];
    assert_eq!(data["command"], "mcp_call");
    assert_eq!(data["tool"], "execute_sql");
    assert_eq!(data["args"]["sql"], "select 1");
    assert_eq!(data["endpoint"], "http://127.0.0.1:10825/mcp");
    assert_eq!(data["risk"]["level"], "high-risk-write");
    assert_eq!(data["confirmation_required"], false);
}

#[test]
fn login_help_describes_login_command() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["login", "--help"])
        .assert()
        .success()
        .stdout(contains("Open Chat2DB and wait until login is detected"))
        .stdout(contains("chat2db login --edition local"))
        .stdout(contains("chat2db login --edition pro --json"));
}

#[test]
fn logout_help_describes_logout_command() {
    let mut command = Command::cargo_bin("chat2db").unwrap();

    command
        .args(["logout", "--help"])
        .assert()
        .success()
        .stdout(contains("Clear the saved Chat2DB login state"))
        .stdout(contains("chat2db logout --edition local"))
        .stdout(contains("chat2db logout --edition pro --json"));
}
