use std::io::IsTerminal;
use std::time::Duration;

use chat2db::background;
use chat2db::cli::{Cli, Command, McpOp, RuntimeOp, SkillOp, SqlOp};
use chat2db::completions::{self, CompletionsRequest};
use chat2db::db::run_db_op;
use chat2db::direct::{DirectRequest, sql_query};
use chat2db::dry_run;
use chat2db::edition::Edition;
use chat2db::error::{Chat2DbError, Result};
use chat2db::http::ReqwestHttpClient;
use chat2db::installer::{InstallRequest, Installer, SystemPaths, UninstallRequest};
use chat2db::login::{LoginRequest, login};
use chat2db::logout::{LogoutRequest, logout};
use chat2db::logs::{LogsRequest, logs};
use chat2db::mcp::{
    McpChangeRequest, McpStatusRequest, mcp_disable, mcp_enable, mcp_restart, mcp_status,
};
use chat2db::mcp_client::{McpCallRequest, McpToolsRequest, mcp_call, mcp_tools, parse_args_json};
use chat2db::mcp_config::{McpConfigRequest, mcp_config};
use chat2db::output::{Outcome, VersionOutcome, print_error};
use chat2db::platform::{ensure_desktop_supported, home_dir};
use chat2db::restart::{RestartRequest, restart};
use chat2db::runtime::{RuntimeRequest, runtime_start, runtime_status, runtime_stop};
use chat2db::runtime_client::SqlQueryRequest;
use chat2db::self_update::{UninstallCliRequest, UpdateRequest, uninstall_cli, update_cli};
use chat2db::skill::{
    self, SkillInstallRequest, SkillTargetRequest, SkillUninstallRequest, SkillUpdateRequest,
};
use chat2db::start::{StartRequest, start};
use chat2db::status::{StatusRequest, check_status};
use chat2db::stop::{StopRequest, stop};
use chat2db::stop_all::{StopAllRequest, stop_all};
use chat2db::update_check::{self, UpdateCheckPolicy};

const LOGIN_TIMEOUT_SECONDS: u64 = 600;
const LOGIN_INTERVAL_SECONDS: u64 = 5;

fn main() {
    let cli = Cli::parse_from_env();
    let json_output = cli.wants_json();
    let update_check_policy = UpdateCheckPolicy {
        json_output,
        interactive: std::io::stdout().is_terminal() && std::io::stderr().is_terminal(),
        background_child: background::is_child(),
        excluded_command: update_check_excluded(&cli),
    };

    if update_check::enabled(update_check_policy)
        && let Some(notice) = update_check::check_for_update()
    {
        eprintln!(
            "A new chat2db CLI version is available: {} -> {}. Run `chat2db update`.",
            notice.current_version, notice.latest_version
        );
    }

    let exit_code = match run(cli) {
        Ok(outcomes) => {
            let mut code = 0;
            for outcome in &outcomes {
                if let Err(error) = outcome.print(std::io::stdout(), json_output) {
                    eprintln!("error: {error}");
                    code = 1;
                    break;
                }
            }
            code
        }
        Err(error) => {
            let _ = print_error(std::io::stderr(), &error, json_output);
            1
        }
    };

    // When this process is the detached child of `install --background`,
    // record the final exit code so the parent / future `chat2db status`
    // calls can tell the install finished.
    background::mark_done_if_child(exit_code);

    std::process::exit(exit_code);
}

fn update_check_excluded(cli: &Cli) -> bool {
    cli.version
        || matches!(
            &cli.command,
            Some(
                Command::Version(_)
                    | Command::Update(_)
                    | Command::Completions(_)
                    | Command::Logs(_)
            )
        )
}

fn run(cli: Cli) -> Result<Vec<Outcome>> {
    if cli.version {
        return Ok(vec![version_outcome()]);
    }

    match cli
        .command
        .expect("clap requires a subcommand unless -v is used")
    {
        Command::Version(_args) => Ok(vec![Outcome::Version(VersionOutcome {
            command: "version",
            version: env!("CARGO_PKG_VERSION"),
        })]),
        Command::Status(_args) => Ok(vec![Outcome::Status(check_status(
            StatusRequest::production(),
        ))]),
        Command::Login(args) => for_each_edition(args.edition.editions(), |edition| {
            if edition.requires_login() {
                ensure_desktop_supported()?;
            }
            login(LoginRequest {
                edition,
                paths: SystemPaths::production(),
                timeout: Duration::from_secs(LOGIN_TIMEOUT_SECONDS),
                interval: Duration::from_secs(LOGIN_INTERVAL_SECONDS),
                progress: !args.json,
            })
            .map(Outcome::Login)
        }),
        Command::Logout(args) => for_each_edition(args.edition.editions(), |edition| {
            if edition.requires_login() {
                ensure_desktop_supported()?;
            }
            logout(LogoutRequest {
                edition,
                paths: SystemPaths::production(),
            })
            .map(Outcome::Logout)
        }),
        Command::Start(args) => {
            ensure_desktop_supported()?;
            for_each_edition(args.edition.editions(), |edition| {
                start(StartRequest {
                    edition,
                    paths: SystemPaths::production(),
                })
                .map(Outcome::Start)
            })
        }
        Command::Stop(args) => {
            ensure_desktop_supported()?;
            if args.all {
                return stop_all(StopAllRequest::production(args.edition.editions()))
                    .map(|outcome| vec![Outcome::StopAll(outcome)]);
            }
            for_each_edition(args.edition.editions(), |edition| {
                stop(StopRequest {
                    edition,
                    paths: SystemPaths::production(),
                })
                .map(Outcome::Stop)
            })
        }
        Command::Restart(args) => {
            ensure_desktop_supported()?;
            for_each_edition(args.edition.editions(), |edition| {
                restart(RestartRequest {
                    edition,
                    paths: SystemPaths::production(),
                })
                .map(Outcome::Restart)
            })
        }
        Command::Update(args) => update_cli(UpdateRequest {
            target_path: args.target,
            progress: !args.json,
        })
        .map(|outcome| vec![Outcome::Update(outcome)]),
        Command::Install(args) => {
            let paths = SystemPaths::production();
            if args.dry_run {
                return Ok(args
                    .edition
                    .editions()
                    .iter()
                    .map(|edition| {
                        Outcome::DryRun(dry_run::install_dry_run(
                            *edition, args.force, args.yes, &paths,
                        ))
                    })
                    .collect());
            }
            if args.force && !args.yes {
                return Err(Chat2DbError::ConfirmationRequired {
                    action: "install --force",
                });
            }
            ensure_desktop_supported()?;
            if args.background {
                return for_each_edition(args.edition.editions(), |edition| {
                    background::spawn_install(edition, args.force, args.yes, args.json)
                        .map(Outcome::InstallStarted)
                });
            }
            let installer = Installer::new(ReqwestHttpClient, paths);
            for_each_edition(args.edition.editions(), |edition| {
                installer
                    .install(InstallRequest {
                        edition,
                        force: args.force,
                        progress: !args.json,
                    })
                    .map(Outcome::Install)
            })
        }
        Command::Uninstall(args) => {
            let paths = SystemPaths::production();
            if args.dry_run {
                if args.cli {
                    return Ok(vec![Outcome::DryRun(dry_run::uninstall_cli_dry_run(
                        args.yes,
                        args.target,
                    ))]);
                }
                return Ok(args
                    .edition
                    .editions()
                    .iter()
                    .map(|edition| {
                        Outcome::DryRun(dry_run::uninstall_dry_run(*edition, args.yes, &paths))
                    })
                    .collect());
            }
            if !args.yes {
                return Err(Chat2DbError::ConfirmationRequired {
                    action: if args.cli {
                        "uninstall cli"
                    } else {
                        "uninstall app"
                    },
                });
            }
            if args.cli {
                return uninstall_cli(UninstallCliRequest {
                    target_path: args.target,
                })
                .map(|outcome| vec![Outcome::UninstallCli(outcome)]);
            }

            ensure_desktop_supported()?;
            let installer = Installer::new(ReqwestHttpClient, paths);
            for_each_edition(args.edition.editions(), |edition| {
                installer
                    .uninstall(UninstallRequest { edition })
                    .map(Outcome::Uninstall)
            })
        }
        Command::Completions(args) => {
            completions::run(CompletionsRequest::production(args.shell, args.install))
                .map(|outcome| vec![Outcome::Completions(outcome)])
        }
        Command::Logs(args) => logs(LogsRequest {
            kind: args.kind,
            edition: args.edition,
            follow: args.follow,
            paths: SystemPaths::production(),
        })
        .map(|outcome| vec![Outcome::Logs(outcome)]),
        Command::Mcp(args) => match args.op {
            McpOp::Status(a) => mcp_status(McpStatusRequest::production(a.edition))
                .map(|outcome| vec![Outcome::Mcp(outcome)]),
            McpOp::Enable(a) => {
                ensure_desktop_supported()?;
                mcp_enable(McpChangeRequest::production(a.edition, !a.json))
                    .map(|outcome| vec![Outcome::Mcp(outcome)])
            }
            McpOp::Disable(a) => {
                ensure_desktop_supported()?;
                mcp_disable(McpChangeRequest::production(a.edition, !a.json))
                    .map(|outcome| vec![Outcome::Mcp(outcome)])
            }
            McpOp::Restart(a) => {
                ensure_desktop_supported()?;
                mcp_restart(McpChangeRequest::production(a.edition, !a.json))
                    .map(|outcome| vec![Outcome::Mcp(outcome)])
            }
            McpOp::Tools(a) => mcp_tools(McpToolsRequest::production(a.edition))
                .map(|outcome| vec![Outcome::McpTools(outcome)]),
            McpOp::Call(a) => {
                if a.dry_run {
                    return Ok(vec![Outcome::DryRun(dry_run::mcp_call_dry_run(
                        a.edition,
                        a.tool_name,
                        parse_args_json(&a.args_json)?,
                    ))]);
                }
                mcp_call(McpCallRequest::production(
                    a.edition,
                    a.tool_name,
                    &a.args_json,
                )?)
                .map(|outcome| vec![Outcome::McpCall(outcome)])
            }
            McpOp::Config(a) => mcp_config(McpConfigRequest::production(a.edition))
                .map(|outcome| vec![Outcome::McpConfig(outcome)]),
        },
        Command::Runtime(args) => {
            ensure_desktop_supported()?;
            match args.op {
                RuntimeOp::Status(a) => runtime_status(RuntimeRequest::production(a.edition))
                    .map(|outcome| vec![Outcome::Runtime(outcome)]),
                RuntimeOp::Start(a) => runtime_start(RuntimeRequest::production(a.edition))
                    .map(|outcome| vec![Outcome::Runtime(outcome)]),
                RuntimeOp::Stop(a) => runtime_stop(RuntimeRequest::production(a.edition))
                    .map(|outcome| vec![Outcome::Runtime(outcome)]),
            }
        }
        Command::Db(args) => {
            ensure_desktop_supported()?;
            run_db_op(args.op)
        }
        Command::Sql(args) => {
            ensure_desktop_supported()?;
            match args.op {
                SqlOp::Query(a) => sql_query(
                    DirectRequest::production(a.edition),
                    SqlQueryRequest {
                        data_source_id: a.data_source_id,
                        database_name: a.database,
                        schema_name: a.schema,
                        sql: a.sql,
                        page_no: a.page_no,
                        page_size: a.page_size,
                        result_set_id: a.result_set_id,
                        include_row_number: !a.no_row_number,
                    },
                )
                .map(|outcome| vec![Outcome::Direct(outcome)]),
            }
        }
        Command::Skill(args) => {
            let home_dir = home_dir();
            let current_dir = std::env::current_dir()?;
            let cli_version = env!("CARGO_PKG_VERSION").to_string();
            match args.op {
                SkillOp::Install(a) => {
                    let target = SkillTargetRequest {
                        client: a.client,
                        home_dir,
                        current_dir,
                    };
                    if a.dry_run {
                        return Ok(vec![Outcome::DryRun(dry_run::skill_install_dry_run(
                            target, a.force, a.yes,
                        ))]);
                    }
                    if a.force && !a.yes {
                        return Err(Chat2DbError::ConfirmationRequired {
                            action: "skill install --force",
                        });
                    }
                    skill::install(
                        &ReqwestHttpClient,
                        SkillInstallRequest {
                            target,
                            force: a.force,
                            progress: !a.json,
                            cli_version,
                        },
                    )
                    .map(|o| vec![Outcome::Skill(o)])
                }
                SkillOp::Uninstall(a) => skill::uninstall(SkillUninstallRequest {
                    target: SkillTargetRequest {
                        client: a.client,
                        home_dir,
                        current_dir,
                    },
                })
                .map(|o| vec![Outcome::Skill(o)]),
                SkillOp::Update(a) => skill::update(
                    &ReqwestHttpClient,
                    SkillUpdateRequest {
                        target: SkillTargetRequest {
                            client: a.client,
                            home_dir,
                            current_dir,
                        },
                        progress: !a.json,
                        cli_version,
                    },
                )
                .map(|o| vec![Outcome::Skill(o)]),
            }
        }
    }
}

fn version_outcome() -> Outcome {
    Outcome::Version(VersionOutcome {
        command: "version",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Runs `op` for each edition in `editions`. Stops at the first error so a
/// later edition's success doesn't mask an earlier failure (matches what the
/// user would have written manually with `&&`).
fn for_each_edition<F>(editions: &[Edition], mut op: F) -> Result<Vec<Outcome>>
where
    F: FnMut(Edition) -> Result<Outcome>,
{
    let mut outcomes = Vec::with_capacity(editions.len());
    for edition in editions {
        outcomes.push(op(*edition)?);
    }
    Ok(outcomes)
}
