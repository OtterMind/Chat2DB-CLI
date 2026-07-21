use serde::Serialize;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::SystemPaths;
use crate::mcp::{McpChangeRequest, McpFrontendSyncStatus, mcp_disable_without_restart};
use crate::runtime::{RuntimeRequest, runtime_stop};
use crate::status::{EditionRuntimeSummary, StatusRequest, check_status};
use crate::stop::{StopRequest, stop};

#[derive(Debug, Clone)]
pub struct StopAllRequest {
    pub editions: Vec<Edition>,
    pub paths: SystemPaths,
}

impl StopAllRequest {
    pub fn production(editions: &[Edition]) -> Self {
        Self {
            editions: editions.to_vec(),
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StopAllOutcome {
    pub command: &'static str,
    pub mode: &'static str,
    pub editions: Vec<StopAllEditionOutcome>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopAllEditionOutcome {
    pub edition: Edition,
    pub status: &'static str,
    pub steps: StopAllSteps,
    pub final_summary: EditionRuntimeSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopAllSteps {
    pub mcp: StepStatus,
    pub cli: StepStatus,
    pub app: StepStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepStatus {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl StepStatus {
    fn ok(status: &'static str) -> Self {
        Self {
            status,
            error: None,
        }
    }

    fn failed(error: impl ToString) -> Self {
        Self {
            status: "failed",
            error: Some(error.to_string()),
        }
    }
}

pub fn stop_all(request: StopAllRequest) -> Result<StopAllOutcome> {
    let mut editions = Vec::new();
    for edition in request.editions {
        editions.push(stop_all_edition(edition, &request.paths)?);
    }

    Ok(StopAllOutcome {
        command: "stop",
        mode: "all",
        editions,
    })
}

fn stop_all_edition(edition: Edition, paths: &SystemPaths) -> Result<StopAllEditionOutcome> {
    stop_all_edition_with_status(edition, paths, || {
        let status = check_status(StatusRequest {
            paths: paths.clone(),
        });
        status.summary.for_edition(edition).clone()
    })
}

fn stop_all_edition_with_status<F>(
    edition: Edition,
    paths: &SystemPaths,
    final_summary: F,
) -> Result<StopAllEditionOutcome>
where
    F: FnOnce() -> EditionRuntimeSummary,
{
    let mcp = match mcp_disable_without_restart(McpChangeRequest {
        edition,
        paths: paths.clone(),
        wait_timeout: std::time::Duration::ZERO,
        wait_interval: std::time::Duration::ZERO,
        settle_duration: std::time::Duration::ZERO,
        progress: false,
    }) {
        Ok(McpFrontendSyncStatus::Synced) => StepStatus::ok("disabled"),
        Ok(McpFrontendSyncStatus::SkippedLocked) => {
            StepStatus::ok("disabled_with_ui_sync_skipped_locked")
        }
        Err(error) => StepStatus::failed(error),
    };

    let cli = match runtime_stop(RuntimeRequest {
        edition,
        paths: paths.clone(),
    }) {
        Ok(_) => StepStatus::ok("stopped"),
        Err(error) => StepStatus::failed(error),
    };

    let app = match stop(StopRequest {
        edition,
        paths: paths.clone(),
    }) {
        Ok(_) => StepStatus::ok("stopped"),
        Err(Chat2DbError::AppNotInstalled { .. }) => StepStatus::ok("skipped"),
        Err(error) => StepStatus::failed(error),
    };

    let final_summary = final_summary();
    let mcp = normalize_mcp_step(mcp, &final_summary);
    let status = if final_summary.safe_to_exit {
        "stopped"
    } else {
        "partial"
    };

    Ok(StopAllEditionOutcome {
        edition,
        status,
        steps: StopAllSteps { mcp, cli, app },
        final_summary,
    })
}

fn normalize_mcp_step(step: StepStatus, final_summary: &EditionRuntimeSummary) -> StepStatus {
    if step.status != "failed" {
        return step;
    }
    if final_summary.mcp.enabled || final_summary.mcp.endpoint.is_alive() {
        return step;
    }
    StepStatus::ok("disabled")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn stop_all_skips_uninstalled_edition_and_returns_safe_summary() {
        let temp = tempfile::tempdir().unwrap();
        let paths = test_paths(temp.path());

        let local =
            stop_all_edition_with_status(Edition::Local, &paths, || EditionRuntimeSummary {
                app: crate::status::StatusAppSummary {
                    state: crate::status::StatusAppState::NotInstalled,
                    pids: vec![],
                },
                cli: crate::status::StatusCliRuntimeSummary {
                    state: crate::status::StatusCliRuntimeState::Stopped,
                    pid: None,
                    port: None,
                    url: None,
                    error: None,
                },
                mcp: crate::status::StatusMcpSummary {
                    enabled: false,
                    endpoint: crate::mcp::EndpointState::NotListening,
                    needs_restart: false,
                    error: None,
                },
                background_install: None,
                safe_to_exit: true,
                next_action: Some("chat2db install --edition local --json".to_string()),
            })
            .unwrap();

        assert_eq!(local.edition, Edition::Local);
        assert_eq!(local.status, "stopped");
        assert_eq!(local.steps.mcp.status, "disabled");
        assert_eq!(local.steps.cli.status, "stopped");
        assert_eq!(local.steps.app.status, "skipped");
        assert!(local.final_summary.safe_to_exit);
        assert_eq!(
            local.final_summary.next_action.as_deref(),
            Some("chat2db install --edition local --json")
        );
    }

    #[test]
    fn normalizes_failed_mcp_step_when_final_mcp_state_is_disabled() {
        let step = StepStatus::failed("database lock is held by another instance");
        let final_summary = EditionRuntimeSummary {
            app: crate::status::StatusAppSummary {
                state: crate::status::StatusAppState::Stopped,
                pids: vec![],
            },
            cli: crate::status::StatusCliRuntimeSummary {
                state: crate::status::StatusCliRuntimeState::Stopped,
                pid: None,
                port: None,
                url: None,
                error: None,
            },
            mcp: crate::status::StatusMcpSummary {
                enabled: false,
                endpoint: crate::mcp::EndpointState::NotListening,
                needs_restart: false,
                error: None,
            },
            background_install: None,
            safe_to_exit: true,
            next_action: None,
        };

        let normalized = normalize_mcp_step(step, &final_summary);

        assert_eq!(normalized.status, "disabled");
        assert!(normalized.error.is_none());
    }

    fn test_paths(root: &Path) -> SystemPaths {
        let applications_dir = root.join("Applications");
        let home_dir = root.join("home");
        let background_state_dir = root.join("state");
        fs::create_dir_all(&applications_dir).unwrap();
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&background_state_dir).unwrap();
        SystemPaths {
            applications_dir,
            home_dir,
            background_state_dir,
        }
    }
}
