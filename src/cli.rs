use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

use crate::completions::Shell;
use crate::edition::{Edition, EditionSelector};
use crate::logs::LogsKind;

fn redacted_option(value: Option<&String>) -> Option<&'static str> {
    value.map(|_| "<redacted>")
}

#[derive(Debug, Parser)]
#[command(
    name = "chat2db",
    version,
    disable_version_flag = true,
    arg_required_else_help = true,
    about = "Chat2DB CLI",
    after_help = "Options summary:\n  -v, --version           [--json]\n  version                 [--json]\n  status                  [--json]\n  login | logout          [--edition community|pro|local|all] [--json]\n  start | restart         [--edition community|pro|local|all] [--json]\n  stop                    [--edition community|pro|local|all] [--all] [--json]\n  install                 [--edition community|pro|local|all] [--force] [--yes] [--dry-run] [--background] [--json]\n  uninstall               [--edition community|pro|local|all] [--cli] [--target <PATH>] [--yes] [--dry-run] [--json]\n  logs                    <install> [--edition community|pro|local] [--follow]\n  update                  [--target <PATH>] [--json]\n  completions             [--shell zsh|bash|fish] [--install] [--json]\n  skill                   install/uninstall/update [--client claude-code|codex] [--force] [--yes] [--dry-run] [--json]\n  runtime                 status/start/stop [--edition community|pro|local] [--json]\n  db                      datasources/datasource/connection-test/datasource-create/datasource-update/datasource-delete/databases/schemas/tables/table [--edition community|pro|local] [--json]\n  sql query               --data-source-id <ID> --database <DB> --sql <SQL> [--schema <SCHEMA>] [--page-no N] [--page-size N] [--result-set-id N] [--no-row-number] [--json]\n  mcp status              [--edition community|pro|local|all] [--json]\n  mcp enable|disable|restart|tools [--edition community|pro|local] [--json]\n  mcp call                <tool-name> [--args-json <JSON_OBJECT>] [--edition community|pro|local] [--dry-run] [--json]\n\nExamples:\n  chat2db -v --json\n  chat2db status\n  chat2db install --background\n  chat2db runtime start --json\n  chat2db db datasources --json\n  chat2db db connection-test --db-type MYSQL --host localhost --port 3306 --database test --user root --password '<password>' --json\n  chat2db db datasource-create --db-type MYSQL --host localhost --port 3306 --database test --user root --password '<password>' --environment-id 1 --json\n  chat2db sql query --data-source-id 179431 --database postgres --schema public --sql 'select 1' --page-no 1 --page-size 100 --json\n  chat2db mcp status --edition all --json\n  chat2db skill install\n\nRun `chat2db <command> --help` for full per-command help."
)]
pub struct Cli {
    #[arg(
        short = 'v',
        long = "version",
        action = ArgAction::SetTrue,
        help = "Print the CLI version"
    )]
    pub version: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        requires = "version",
        help = "Print a machine-readable JSON response with -v/--version"
    )]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Cli {
    pub fn parse_from_env() -> Self {
        Self::parse_from(normalize_args(std::env::args_os()))
    }

    #[cfg(test)]
    fn try_parse_normalized_from<I, T>(args: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString>,
    {
        Self::try_parse_from(normalize_args(args))
    }

    pub fn wants_json(&self) -> bool {
        if self.json {
            return true;
        }

        match &self.command {
            Some(Command::Version(args)) => args.json,
            Some(Command::Status(args)) => args.json,
            Some(Command::Login(args)) => args.json,
            Some(Command::Logout(args)) => args.json,
            Some(Command::Start(args)) => args.json,
            Some(Command::Stop(args)) => args.json,
            Some(Command::Restart(args)) => args.json,
            Some(Command::Update(args)) => args.json,
            Some(Command::Install(args)) => args.json,
            Some(Command::Uninstall(args)) => args.json,
            Some(Command::Completions(args)) => args.json,
            Some(Command::Mcp(args)) => match &args.op {
                McpOp::Status(a) => a.json,
                McpOp::Enable(a) => a.json,
                McpOp::Disable(a) => a.json,
                McpOp::Restart(a) => a.json,
                McpOp::Tools(a) => a.json,
                McpOp::Call(a) => a.json,
                McpOp::Config(a) => a.json,
            },
            Some(Command::Runtime(args)) => match &args.op {
                RuntimeOp::Status(a) => a.json,
                RuntimeOp::Start(a) => a.json,
                RuntimeOp::Stop(a) => a.json,
            },
            Some(Command::Db(args)) => match &args.op {
                DbOp::Datasources(a) => a.json,
                DbOp::Datasource(a) => a.json,
                DbOp::ConnectionTest(a) => a.json,
                DbOp::DatasourceCreate(a) => a.json,
                DbOp::DatasourceUpdate(a) => a.json,
                DbOp::DatasourceDelete(a) => a.json,
                DbOp::Databases(a) => a.json,
                DbOp::Schemas(a) => a.json,
                DbOp::Tables(a) => a.json,
                DbOp::Table(a) => a.json,
            },
            Some(Command::Sql(args)) => match &args.op {
                SqlOp::Query(a) => a.json,
            },
            Some(Command::Skill(args)) => match &args.op {
                SkillOp::Install(a) => a.json,
                SkillOp::Uninstall(a) => a.json,
                SkillOp::Update(a) => a.json,
            },
            Some(Command::Logs(_)) | None => false,
        }
    }
}

fn normalize_args<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    args.into_iter()
        .map(|arg| {
            let arg = arg.into();
            if arg == "-version" {
                OsString::from("-v")
            } else {
                arg
            }
        })
        .collect()
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Print the CLI version")]
    Version(VersionArgs),

    #[command(about = "Show app, CLI runtime, and MCP status for every Chat2DB edition")]
    Status(StatusArgs),

    #[command(about = "Open Chat2DB and wait until login is detected")]
    Login(LoginArgs),

    #[command(about = "Clear the saved Chat2DB login state")]
    Logout(LogoutArgs),

    #[command(about = "Open a Chat2DB desktop app")]
    Start(StartArgs),

    #[command(about = "Quit a Chat2DB desktop app gracefully")]
    Stop(StopArgs),

    #[command(about = "Restart a Chat2DB desktop app")]
    Restart(RestartArgs),

    #[command(about = "Update this Chat2DB CLI binary from the official CDN")]
    Update(UpdateArgs),

    #[command(about = "Install or update Chat2DB on macOS or Windows")]
    Install(InstallArgs),

    #[command(about = "Uninstall a Chat2DB app or this CLI binary")]
    Uninstall(UninstallArgs),

    #[command(about = "Print or install shell completion for chat2db")]
    Completions(CompletionsArgs),

    #[command(about = "Show Chat2DB logs (install log; more kinds in the future)")]
    Logs(LogsArgs),

    #[command(about = "Manage the bundled Chat2DB agent skill")]
    Skill(SkillArgs),

    #[command(about = "Manage the Chat2DB desktop MCP endpoint")]
    Mcp(McpArgs),

    #[command(about = "Manage the headless Chat2DB runtime used by CLI commands")]
    Runtime(RuntimeArgs),

    #[command(about = "Read datasources and database metadata through the Chat2DB runtime")]
    Db(DbArgs),

    #[command(about = "Run SQL through the Chat2DB runtime")]
    Sql(SqlArgs),
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db runtime status --edition pro --json\n  chat2db runtime start --edition pro --json\n  chat2db runtime stop --edition pro --json"
)]
pub struct RuntimeArgs {
    #[command(subcommand)]
    pub op: RuntimeOp,
}

#[derive(Debug, Subcommand)]
pub enum RuntimeOp {
    #[command(about = "Show headless runtime status")]
    Status(RuntimeCommandArgs),
    #[command(about = "Start or reuse the headless runtime")]
    Start(RuntimeCommandArgs),
    #[command(about = "Stop the headless runtime")]
    Stop(RuntimeCommandArgs),
}

#[derive(Debug, Parser)]
pub struct RuntimeCommandArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db db datasources --edition pro --json\n  chat2db db connection-test --data-source-id 179431 --json\n  chat2db db connection-test --db-type MYSQL --host localhost --port 3306 --database test --user root --password '<password>' --json\n  chat2db db connection-test --db-type MYSQL --url jdbc:mysql://127.0.0.1:3306/test --user root --password '<password>' --json\n  chat2db db datasource-create --db-type MYSQL --url jdbc:mysql://127.0.0.1:3306/test --user root --password '<password>' --environment-id 1 --json\n  chat2db db datasource-create --db-type MYSQL --host localhost --port 3306 --database test --user root --password '<password>' --environment-id 1 --json\n  chat2db db datasource-update --data-source-id 179431 --alias prod-mysql --json\n  chat2db db datasource-delete --data-source-id 179431 --yes --json\n  chat2db db databases --data-source-id 179431\n  chat2db db schemas --data-source-id 179431 --database postgres\n  chat2db db tables --data-source-id 179431 --database postgres --schema public\n  chat2db db table --data-source-id 179431 --database postgres --schema public --table users"
)]
pub struct DbArgs {
    #[command(subcommand)]
    pub op: DbOp,
}

#[derive(Debug, Subcommand)]
pub enum DbOp {
    #[command(about = "List Chat2DB datasources")]
    Datasources(DbListArgs),
    #[command(about = "Show one datasource")]
    Datasource(DbDatasourceArgs),
    #[command(about = "Test an existing datasource or temporary connection parameters")]
    ConnectionTest(DbConnectionTestArgs),
    #[command(about = "Create a datasource")]
    DatasourceCreate(DbDatasourceCreateArgs),
    #[command(about = "Update a datasource")]
    DatasourceUpdate(DbDatasourceUpdateArgs),
    #[command(about = "Delete a datasource")]
    DatasourceDelete(DbDatasourceDeleteArgs),
    #[command(about = "List databases for a datasource")]
    Databases(DbConnectionArgs),
    #[command(about = "List schemas for a database")]
    Schemas(DbDatabaseArgs),
    #[command(about = "List tables for a database/schema")]
    Tables(DbTablesArgs),
    #[command(about = "Show one table schema")]
    Table(DbTableArgs),
}

#[derive(Debug, Parser)]
pub struct DbListArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, default_value_t = 1, help = "Page number")]
    pub page_no: u32,

    #[arg(long, default_value_t = 100, help = "Page size")]
    pub page_size: u32,

    #[arg(long, help = "Refresh metadata cache")]
    pub refresh: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct DbDatasourceArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Parser)]
#[command(
    after_help = "Examples:\n  chat2db db connection-test --data-source-id 195652 --json\n  chat2db db connection-test --db-type MYSQL --host localhost --port 3306 --database test --user root --password '<password>' --json\n  chat2db db connection-test --db-type MYSQL --url jdbc:mysql://localhost:3306/test --user root --password '<password>' --json"
)]
pub struct DbConnectionTestArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Existing datasource id to test")]
    pub data_source_id: Option<u64>,

    #[arg(long, help = "Datasource type, e.g. MYSQL or POSTGRESQL")]
    pub db_type: Option<String>,

    #[arg(long, help = "JDBC URL")]
    pub url: Option<String>,

    #[arg(long, help = "Host")]
    pub host: Option<String>,

    #[arg(long, help = "Port")]
    pub port: Option<String>,

    #[arg(long, help = "Default database/service name")]
    pub database: Option<String>,

    #[arg(long, help = "Datasource user")]
    pub user: Option<String>,

    #[arg(long, help = "Datasource password")]
    pub password: Option<String>,

    #[arg(long, help = "Service name; overrides --database when both are set")]
    pub service_name: Option<String>,

    #[arg(long, help = "Service type")]
    pub service_type: Option<String>,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

impl fmt::Debug for DbConnectionTestArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbConnectionTestArgs")
            .field("edition", &self.edition)
            .field("data_source_id", &self.data_source_id)
            .field("db_type", &self.db_type)
            .field("url", &self.url)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("user", &self.user)
            .field("password", &redacted_option(self.password.as_ref()))
            .field("service_name", &self.service_name)
            .field("service_type", &self.service_type)
            .field("json", &self.json)
            .finish()
    }
}

#[derive(Parser)]
pub struct DbDatasourceCreateArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource alias")]
    pub alias: Option<String>,

    #[arg(long, help = "Datasource type, e.g. MYSQL or POSTGRESQL")]
    pub db_type: String,

    #[arg(
        long,
        help = "JDBC URL; mutually exclusive with --host/--port/--database"
    )]
    pub url: Option<String>,

    #[arg(long, help = "Datasource user")]
    pub user: Option<String>,

    #[arg(long, help = "Datasource password")]
    pub password: String,

    #[arg(long, help = "Environment id")]
    pub environment_id: u64,

    #[arg(long, help = "Host for automatic JDBC URL construction")]
    pub host: Option<String>,

    #[arg(long, help = "Port for automatic JDBC URL construction")]
    pub port: Option<String>,

    #[arg(
        long,
        help = "Default database/service name for automatic JDBC URL construction"
    )]
    pub database: Option<String>,

    #[arg(long, help = "Service name; overrides --database when both are set")]
    pub service_name: Option<String>,

    #[arg(long, help = "Service type")]
    pub service_type: Option<String>,

    #[arg(long, help = "Storage type")]
    pub storage_type: Option<String>,

    #[arg(long, help = "Space id")]
    pub space_id: Option<u64>,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

impl fmt::Debug for DbDatasourceCreateArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbDatasourceCreateArgs")
            .field("edition", &self.edition)
            .field("alias", &self.alias)
            .field("db_type", &self.db_type)
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &"<redacted>")
            .field("environment_id", &self.environment_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("service_name", &self.service_name)
            .field("service_type", &self.service_type)
            .field("storage_type", &self.storage_type)
            .field("space_id", &self.space_id)
            .field("json", &self.json)
            .finish()
    }
}

#[derive(Parser)]
pub struct DbDatasourceUpdateArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Datasource alias")]
    pub alias: Option<String>,

    #[arg(long, help = "Datasource type, e.g. MYSQL or POSTGRESQL")]
    pub db_type: Option<String>,

    #[arg(long, help = "JDBC URL")]
    pub url: Option<String>,

    #[arg(long, help = "Datasource user")]
    pub user: Option<String>,

    #[arg(long, help = "Datasource password")]
    pub password: Option<String>,

    #[arg(long, help = "Environment id")]
    pub environment_id: Option<u64>,

    #[arg(long, help = "Host")]
    pub host: Option<String>,

    #[arg(long, help = "Port")]
    pub port: Option<String>,

    #[arg(long, help = "Default database/service name")]
    pub database: Option<String>,

    #[arg(long, help = "Service name; overrides --database when both are set")]
    pub service_name: Option<String>,

    #[arg(long, help = "Service type")]
    pub service_type: Option<String>,

    #[arg(long, help = "Storage type")]
    pub storage_type: Option<String>,

    #[arg(long, help = "Space id")]
    pub space_id: Option<u64>,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

impl fmt::Debug for DbDatasourceUpdateArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbDatasourceUpdateArgs")
            .field("edition", &self.edition)
            .field("data_source_id", &self.data_source_id)
            .field("alias", &self.alias)
            .field("db_type", &self.db_type)
            .field("url", &self.url)
            .field("user", &self.user)
            .field("password", &redacted_option(self.password.as_ref()))
            .field("environment_id", &self.environment_id)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("service_name", &self.service_name)
            .field("service_type", &self.service_type)
            .field("storage_type", &self.storage_type)
            .field("space_id", &self.space_id)
            .field("json", &self.json)
            .finish()
    }
}

#[derive(Debug, Parser)]
pub struct DbDatasourceDeleteArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Confirm datasource deletion")]
    pub yes: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct DbConnectionArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Refresh metadata cache")]
    pub refresh: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct DbDatabaseArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Database name")]
    pub database: String,

    #[arg(long, help = "Refresh metadata cache")]
    pub refresh: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct DbTablesArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Database name")]
    pub database: String,

    #[arg(long, help = "Schema name")]
    pub schema: Option<String>,

    #[arg(long, default_value_t = 1, help = "Page number")]
    pub page_no: u32,

    #[arg(long, default_value_t = 100, help = "Page size")]
    pub page_size: u32,

    #[arg(long, help = "Refresh metadata cache")]
    pub refresh: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct DbTableArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Database name")]
    pub database: String,

    #[arg(long, help = "Schema name")]
    pub schema: Option<String>,

    #[arg(long, help = "Table name")]
    pub table: String,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db sql query --data-source-id 179431 --database postgres --schema public --sql 'select 1' --page-no 1 --page-size 100 --json\n  chat2db sql query --data-source-id 179431 --database postgres --schema public --sql 'select 1; select 2' --result-set-id 2 --no-row-number --json"
)]
pub struct SqlArgs {
    #[command(subcommand)]
    pub op: SqlOp,
}

#[derive(Debug, Subcommand)]
pub enum SqlOp {
    #[command(about = "Run SQL")]
    Query(SqlQueryArgs),
}

#[derive(Debug, Parser)]
pub struct SqlQueryArgs {
    #[arg(long, value_enum, default_value_t = Edition::Community, help = "Choose which Chat2DB edition to use")]
    pub edition: Edition,

    #[arg(long, help = "Datasource id")]
    pub data_source_id: u64,

    #[arg(long, help = "Database name")]
    pub database: String,

    #[arg(long, help = "Schema name")]
    pub schema: Option<String>,

    #[arg(long, help = "SQL text")]
    pub sql: String,

    #[arg(long, default_value_t = 1, help = "Page number")]
    pub page_no: u32,

    #[arg(long, default_value_t = 100, help = "Page size")]
    pub page_size: u32,

    #[arg(
        long,
        help = "Return one result set by 1-based result-set id for multi-result SQL"
    )]
    pub result_set_id: Option<u32>,

    #[arg(long, help = "Do not include the synthetic Chat2DB row-number column")]
    pub no_row_number: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db mcp status --edition all --json\n  chat2db mcp enable --edition pro --json\n  chat2db mcp config --edition pro --json\n  chat2db mcp tools --edition pro\n  chat2db mcp call list_all_datasources --args-json '{}' --json\n  chat2db mcp call execute_sql --args-json '{\"dataSourceId\":179431,\"databaseName\":\"postgres\",\"schemaName\":\"public\",\"sql\":\"select 1\",\"pageSize\":100}' --json"
)]
pub struct McpArgs {
    #[command(subcommand)]
    pub op: McpOp,
}

#[derive(Debug, Subcommand)]
pub enum McpOp {
    #[command(about = "Show MCP settings, app process, and endpoint status")]
    Status(McpStatusArgs),
    #[command(about = "Enable MCP for one edition, relaunch it, and wait for /mcp")]
    Enable(McpChangeArgs),
    #[command(about = "Disable MCP for one edition, relaunch it, and wait for /mcp to stop")]
    Disable(McpChangeArgs),
    #[command(about = "Relaunch one MCP-enabled edition and wait for /mcp")]
    Restart(McpChangeArgs),
    #[command(about = "List tools from the Chat2DB desktop MCP endpoint")]
    Tools(McpToolsArgs),
    #[command(about = "Call one tool on the Chat2DB desktop MCP endpoint")]
    Call(McpCallArgs),
    #[command(about = "Print a generic JSON MCP client config")]
    Config(McpConfigArgs),
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db mcp status\n  chat2db mcp status --edition pro --json\n  chat2db mcp status --edition all --json"
)]
pub struct McpStatusArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::All,
        help = "Choose which edition to inspect (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct McpChangeArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = Edition::Community,
        help = "Choose which Chat2DB edition to manage"
    )]
    pub edition: Edition,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db mcp tools\n  chat2db mcp tools --edition local --json"
)]
pub struct McpToolsArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = Edition::Community,
        help = "Choose which Chat2DB edition token to use"
    )]
    pub edition: Edition,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db mcp call list_all_datasources --args-json '{}'\n  chat2db mcp call list_all_databases --args-json '{\"dataSourceId\":179431}' --json\n  chat2db mcp call execute_sql --args-json '{\"dataSourceId\":179431,\"databaseName\":\"postgres\",\"schemaName\":\"public\",\"sql\":\"select 1\",\"pageSize\":100}' --json\n  chat2db mcp call execute_sql --args-json '{\"dataSourceId\":179431,\"databaseName\":\"postgres\",\"schemaName\":\"public\",\"sql\":\"select 1\"}' --dry-run --json"
)]
pub struct McpCallArgs {
    pub tool_name: String,

    #[arg(long, default_value = "{}", help = "Tool arguments as a JSON object")]
    pub args_json: String,

    #[arg(
        long,
        value_enum,
        default_value_t = Edition::Community,
        help = "Choose which Chat2DB edition token to use"
    )]
    pub edition: Edition,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,

    #[arg(long, help = "Describe the MCP call without sending it")]
    pub dry_run: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db mcp config --edition pro\n  chat2db mcp config --edition local --json"
)]
pub struct McpConfigArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = Edition::Community,
        help = "Choose which Chat2DB edition token to print"
    )]
    pub edition: Edition,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db skill install\n  chat2db skill install --client claude-code\n  chat2db skill install --client codex --force --yes --json\n  chat2db skill install --client codex --dry-run --json\n  chat2db skill uninstall --json\n  chat2db skill update"
)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub op: SkillOp,
}

#[derive(Debug, Subcommand)]
pub enum SkillOp {
    #[command(about = "Install the Chat2DB skill bundle into the current directory or a client")]
    Install(SkillInstallArgs),
    #[command(about = "Remove the Chat2DB skill bundle from the current directory or a client")]
    Uninstall(SkillUninstallArgs),
    #[command(about = "Update the installed Chat2DB skill bundle to the latest version")]
    Update(SkillUpdateArgs),
}

#[derive(Debug, Parser)]
pub struct SkillInstallArgs {
    #[arg(
        long,
        value_enum,
        help = "Target AI client. Omit to install into ./chat2db-cli"
    )]
    pub client: Option<crate::skill::SkillClient>,

    #[arg(long, help = "Replace an existing bundle directly")]
    pub force: bool,

    #[arg(long, help = "Confirm a high-risk write such as --force replacement")]
    pub yes: bool,

    #[arg(long, help = "Describe the install without writing files")]
    pub dry_run: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct SkillUninstallArgs {
    #[arg(
        long,
        value_enum,
        help = "Target AI client. Omit to remove ./chat2db-cli"
    )]
    pub client: Option<crate::skill::SkillClient>,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
pub struct SkillUpdateArgs {
    #[arg(
        long,
        value_enum,
        help = "Target AI client. Omit to update ./chat2db-cli"
    )]
    pub client: Option<crate::skill::SkillClient>,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(after_help = "Examples:\n  chat2db status\n  chat2db status --json")]
pub struct StatusArgs {
    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db login\n  chat2db login --edition local\n  chat2db login --edition pro --json"
)]
pub struct LoginArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to open and check (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db logout\n  chat2db logout --edition local\n  chat2db logout --edition pro --json"
)]
pub struct LogoutArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to log out (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db start\n  chat2db start --edition local\n  chat2db start --json"
)]
pub struct StartArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to open (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db stop\n  chat2db stop --edition local\n  chat2db stop --edition pro --all\n  chat2db stop --edition all --all --json"
)]
pub struct StopArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to quit (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Stop MCP, headless CLI runtime, and desktop app")]
    pub all: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db restart\n  chat2db restart --edition local\n  chat2db restart --json"
)]
pub struct RestartArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to restart (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(after_help = "Examples:\n  chat2db version\n  chat2db version --json")]
pub struct VersionArgs {
    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db update\n  chat2db update --json\n  chat2db update --target ~/.local/bin/chat2db"
)]
pub struct UpdateArgs {
    #[arg(long, help = "Binary path to replace [default: current executable]")]
    pub target: Option<PathBuf>,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db install\n  chat2db install --edition local\n  chat2db install --edition pro --force --yes --json\n  chat2db install --edition pro --force --dry-run --json\n  chat2db install --edition all --background"
)]
pub struct InstallArgs {
    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to install (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(long, help = "Replace an existing verified Chat2DB app")]
    pub force: bool,

    #[arg(long, help = "Confirm a high-risk write such as --force replacement")]
    pub yes: bool,

    #[arg(
        long,
        help = "Describe the install without downloading or writing files"
    )]
    pub dry_run: bool,

    #[arg(
        long,
        help = "Detach the install into a background process; logs to ~/.local/state/chat2db/"
    )]
    pub background: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db uninstall\n  chat2db uninstall --edition local --yes --json\n  chat2db uninstall --edition all --dry-run --json\n  chat2db uninstall --cli --yes"
)]
pub struct UninstallArgs {
    #[arg(
        long,
        help = "Remove the Chat2DB CLI binary itself instead of the desktop app"
    )]
    pub cli: bool,

    #[arg(
        long,
        value_enum,
        default_value_t = EditionSelector::Community,
        help = "Choose which Chat2DB edition to uninstall (`all` = all three)"
    )]
    pub edition: EditionSelector,

    #[arg(
        long,
        requires = "cli",
        help = "CLI binary path to remove [default: current executable]"
    )]
    pub target: Option<PathBuf>,

    #[arg(long, help = "Confirm a high-risk write")]
    pub yes: bool,

    #[arg(long, help = "Describe the uninstall without removing files")]
    pub dry_run: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db logs install\n  chat2db logs install --edition local\n  chat2db logs install --follow\n\nThe install log is written by `chat2db install --background --edition <e>`."
)]
pub struct LogsArgs {
    #[arg(value_enum, help = "Which log to show (currently: install)")]
    pub kind: LogsKind,

    #[arg(
        long,
        value_enum,
        default_value_t = Edition::Community,
        help = "Which Chat2DB edition's log to show"
    )]
    pub edition: Edition,

    #[arg(long, help = "Stream new lines as they are appended (like `tail -f`)")]
    pub follow: bool,
}

#[derive(Debug, Parser)]
#[command(
    after_help = "Examples:\n  chat2db completions --install\n  chat2db completions --shell zsh\n  chat2db completions --shell bash\n  chat2db completions --install --shell fish"
)]
pub struct CompletionsArgs {
    #[arg(
        long,
        value_enum,
        help = "Target shell [default: detected from $SHELL]"
    )]
    pub shell: Option<Shell>,

    #[arg(
        long,
        help = "Write the completion script to the standard location and wire it into the shell rc"
    )]
    pub install: bool,

    #[arg(long, help = "Print a machine-readable JSON response")]
    pub json: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edition::Edition;
    use clap::CommandFactory;

    fn parsed_command(cli: Cli) -> Command {
        cli.command.expect("expected command")
    }

    #[test]
    fn install_defaults_to_community() {
        let cli = Cli::try_parse_from(["chat2db", "install"]).unwrap();
        match parsed_command(cli) {
            Command::Install(args) => assert_eq!(args.edition, EditionSelector::Community),
            _ => panic!("expected install command"),
        }
    }

    #[test]
    fn uninstall_defaults_to_community() {
        let cli = Cli::try_parse_from(["chat2db", "uninstall"]).unwrap();
        match parsed_command(cli) {
            Command::Uninstall(args) => assert_eq!(args.edition, EditionSelector::Community),
            _ => panic!("expected uninstall command"),
        }
    }

    #[test]
    fn parses_local_install_json() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "install",
            "--edition",
            "local",
            "--force",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Install(args) => {
                assert_eq!(args.edition, EditionSelector::Local);
                assert!(args.force);
                assert!(!args.yes);
                assert!(!args.dry_run);
                assert!(args.json);
            }
            _ => panic!("expected install command"),
        }
    }

    #[test]
    fn parses_edition_all() {
        let cli = Cli::try_parse_from(["chat2db", "stop", "--edition", "all"]).unwrap();
        match parsed_command(cli) {
            Command::Stop(args) => {
                assert_eq!(args.edition, EditionSelector::All);
                assert_eq!(args.edition.editions(), &Edition::ALL);
                assert!(!args.all);
            }
            _ => panic!("expected stop command"),
        }
    }

    #[test]
    fn parses_stop_all_json() {
        let cli = Cli::try_parse_from(["chat2db", "stop", "--edition", "all", "--all", "--json"])
            .unwrap();
        match parsed_command(cli) {
            Command::Stop(args) => {
                assert_eq!(args.edition, EditionSelector::All);
                assert!(args.all);
                assert!(args.json);
            }
            _ => panic!("expected stop command"),
        }
    }

    #[test]
    fn parses_update_json() {
        let cli = Cli::try_parse_from(["chat2db", "update", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Update(args) => assert!(args.json),
            _ => panic!("expected update command"),
        }
    }

    #[test]
    fn update_rejects_removed_source_dir_option() {
        assert!(Cli::try_parse_from(["chat2db", "update", "--source-dir", "."]).is_err());
    }

    #[test]
    fn parses_version_json() {
        let cli = Cli::try_parse_from(["chat2db", "version", "--json"]).unwrap();
        match cli.command.as_ref().expect("expected command") {
            Command::Version(args) => assert!(args.json),
            _ => panic!("expected version command"),
        }
        assert!(cli.wants_json());
    }

    #[test]
    fn parses_root_short_version() {
        let cli = Cli::try_parse_from(["chat2db", "-v"]).unwrap();
        assert!(cli.version);
        assert!(cli.command.is_none());
        assert!(!cli.wants_json());
    }

    #[test]
    fn parses_root_long_version() {
        let cli = Cli::try_parse_from(["chat2db", "--version"]).unwrap();
        assert!(cli.version);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_root_version_json() {
        let cli = Cli::try_parse_from(["chat2db", "-v", "--json"]).unwrap();
        assert!(cli.version);
        assert!(cli.json);
        assert!(cli.command.is_none());
        assert!(cli.wants_json());
    }

    #[test]
    fn root_json_requires_root_version() {
        assert!(Cli::try_parse_from(["chat2db", "--json"]).is_err());
    }

    #[test]
    fn normalizes_single_dash_version() {
        let cli = Cli::try_parse_normalized_from(["chat2db", "-version"]).unwrap();
        assert!(cli.version);
        assert!(cli.command.is_none());
    }

    #[test]
    fn normalizes_single_dash_version_with_json() {
        let cli = Cli::try_parse_normalized_from(["chat2db", "-version", "--json"]).unwrap();
        assert!(cli.version);
        assert!(cli.json);
        assert!(cli.command.is_none());
        assert!(cli.wants_json());
    }

    #[test]
    fn parses_status_json() {
        let cli = Cli::try_parse_from(["chat2db", "status", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Status(args) => assert!(args.json),
            _ => panic!("expected status command"),
        }
    }

    #[test]
    fn parses_login_args() {
        let cli =
            Cli::try_parse_from(["chat2db", "login", "--edition", "local", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Login(args) => {
                assert_eq!(args.edition, EditionSelector::Local);
                assert!(args.json);
            }
            _ => panic!("expected login command"),
        }
    }

    #[test]
    fn parses_logs_install_with_follow() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "logs",
            "install",
            "--edition",
            "local",
            "--follow",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Logs(args) => {
                assert_eq!(args.kind, LogsKind::Install);
                assert_eq!(args.edition, Edition::Local);
                assert!(args.follow);
            }
            _ => panic!("expected logs command"),
        }
    }

    #[test]
    fn logs_requires_kind() {
        // No positional kind → clap error
        assert!(Cli::try_parse_from(["chat2db", "logs"]).is_err());
    }

    #[test]
    fn parses_logout_args() {
        let cli =
            Cli::try_parse_from(["chat2db", "logout", "--edition", "local", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Logout(args) => {
                assert_eq!(args.edition, EditionSelector::Local);
                assert!(args.json);
            }
            _ => panic!("expected logout command"),
        }
    }

    #[test]
    fn rejects_login_timing_options() {
        assert!(Cli::try_parse_from(["chat2db", "login", "--timeout", "300"]).is_err());
        assert!(Cli::try_parse_from(["chat2db", "login", "--interval", "5"]).is_err());
    }

    #[test]
    fn rejects_unsupported_source_option() {
        let result = Cli::try_parse_from([
            "chat2db",
            "install",
            "--source",
            "https://example.com/app.zip",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn root_help_has_useful_descriptions() {
        let mut help = Vec::new();
        Cli::command().write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(help.contains("Chat2DB CLI"));
        assert!(help.contains("Show app, CLI runtime, and MCP status for every Chat2DB edition"));
        assert!(help.contains("Open Chat2DB and wait until login is detected"));
        assert!(help.contains("Clear the saved Chat2DB login state"));
        assert!(help.contains("Open a Chat2DB desktop app"));
        assert!(help.contains("Quit a Chat2DB desktop app gracefully"));
        assert!(help.contains("Restart a Chat2DB desktop app"));
        assert!(help.contains("Update this Chat2DB CLI binary from the official CDN"));
        assert!(help.contains("version                 [--json]"));
        assert!(help.contains("update                  [--target <PATH>] [--json]"));
        assert!(!help.contains("--source-dir"));
        assert!(help.contains("Install or update Chat2DB on macOS or Windows"));
        assert!(help.contains("Uninstall a Chat2DB app or this CLI binary"));
        assert!(help.contains("Manage the bundled Chat2DB agent skill"));
        assert!(help.contains("Manage the Chat2DB desktop MCP endpoint"));
        assert!(help.contains("Manage the headless Chat2DB runtime used by CLI commands"));
        assert!(
            help.contains("Read datasources and database metadata through the Chat2DB runtime")
        );
        assert!(help.contains("Run SQL through the Chat2DB runtime"));
        assert!(help.contains("--client claude-code|codex"));
        assert!(help.contains(
            "runtime                 status/start/stop [--edition community|pro|local] [--json]"
        ));
        assert!(help.contains(
            "stop                    [--edition community|pro|local|all] [--all] [--json]"
        ));
        assert!(help.contains(
            "db                      datasources/datasource/connection-test/datasource-create/datasource-update/datasource-delete/databases/schemas/tables/table [--edition community|pro|local] [--json]"
        ));
        assert!(help.contains(
            "sql query               --data-source-id <ID> --database <DB> --sql <SQL> [--schema <SCHEMA>] [--page-no N] [--page-size N] [--result-set-id N] [--no-row-number] [--json]"
        ));
        assert!(
            help.contains("mcp status              [--edition community|pro|local|all] [--json]")
        );
        assert!(
            help.contains(
                "mcp enable|disable|restart|tools [--edition community|pro|local] [--json]"
            )
        );
        assert!(help.contains(
            "mcp call                <tool-name> [--args-json <JSON_OBJECT>] [--edition community|pro|local] [--dry-run] [--json]"
        ));
        assert!(!help.contains("--from-dir"));
        assert!(help.contains("Examples:"));
        assert!(!help.contains("Chat2DB macOS installer CLI"));
    }

    #[test]
    fn parses_runtime_status_json() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "runtime",
            "status",
            "--edition",
            "local",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Runtime(args) => match args.op {
                RuntimeOp::Status(a) => {
                    assert_eq!(a.edition, Edition::Local);
                    assert!(a.json);
                }
                _ => panic!("expected runtime status op"),
            },
            _ => panic!("expected runtime command"),
        }
    }

    #[test]
    fn parses_db_datasources_defaults_to_community() {
        let cli = Cli::try_parse_from(["chat2db", "db", "datasources", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::Datasources(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.page_no, 1);
                    assert_eq!(a.page_size, 100);
                    assert!(!a.refresh);
                    assert!(a.json);
                }
                _ => panic!("expected datasources op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_table_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "table",
            "--data-source-id",
            "179431",
            "--database",
            "postgres",
            "--schema",
            "public",
            "--table",
            "users",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::Table(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.data_source_id, 179431);
                    assert_eq!(a.database, "postgres");
                    assert_eq!(a.schema.as_deref(), Some("public"));
                    assert_eq!(a.table, "users");
                    assert!(a.json);
                }
                _ => panic!("expected table op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_datasource_create_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "datasource-create",
            "--db-type",
            "MYSQL",
            "--url",
            "jdbc:mysql://127.0.0.1:3306/test",
            "--user",
            "root",
            "--password",
            "secret",
            "--environment-id",
            "1",
            "--alias",
            "local mysql",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::DatasourceCreate(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.db_type, "MYSQL");
                    assert_eq!(a.url.as_deref(), Some("jdbc:mysql://127.0.0.1:3306/test"));
                    assert_eq!(a.user.as_deref(), Some("root"));
                    assert_eq!(a.password, "secret");
                    assert_eq!(a.environment_id, 1);
                    assert_eq!(a.alias.as_deref(), Some("local mysql"));
                    assert_eq!(a.database, None);
                    assert!(a.json);
                }
                _ => panic!("expected datasource-create op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_datasource_create_host_mode_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "datasource-create",
            "--db-type",
            "MYSQL",
            "--host",
            "127.0.0.1",
            "--port",
            "3306",
            "--database",
            "test",
            "--user",
            "root",
            "--password",
            "secret",
            "--environment-id",
            "1",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::DatasourceCreate(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.db_type, "MYSQL");
                    assert_eq!(a.url, None);
                    assert_eq!(a.host.as_deref(), Some("127.0.0.1"));
                    assert_eq!(a.port.as_deref(), Some("3306"));
                    assert_eq!(a.database.as_deref(), Some("test"));
                    assert!(a.json);
                }
                _ => panic!("expected datasource-create op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_connection_test_existing_datasource_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "connection-test",
            "--data-source-id",
            "195652",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::ConnectionTest(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.data_source_id, Some(195652));
                    assert!(a.db_type.is_none());
                    assert!(a.json);
                }
                _ => panic!("expected connection-test op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_connection_test_temporary_host_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "connection-test",
            "--db-type",
            "MYSQL",
            "--host",
            "localhost",
            "--port",
            "3306",
            "--database",
            "test",
            "--user",
            "root",
            "--password",
            "secret",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::ConnectionTest(a) => {
                    assert_eq!(a.db_type.as_deref(), Some("MYSQL"));
                    assert_eq!(a.host.as_deref(), Some("localhost"));
                    assert_eq!(a.port.as_deref(), Some("3306"));
                    assert_eq!(a.database.as_deref(), Some("test"));
                    assert_eq!(a.user.as_deref(), Some("root"));
                    assert_eq!(a.password.as_deref(), Some("secret"));
                    assert!(a.json);
                }
                _ => panic!("expected connection-test op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_connection_test_temporary_url_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "connection-test",
            "--db-type",
            "MYSQL",
            "--url",
            "jdbc:mysql://localhost:3306/test",
            "--user",
            "root",
            "--password",
            "secret",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::ConnectionTest(a) => {
                    assert_eq!(a.db_type.as_deref(), Some("MYSQL"));
                    assert_eq!(a.url.as_deref(), Some("jdbc:mysql://localhost:3306/test"));
                    assert_eq!(a.user.as_deref(), Some("root"));
                    assert_eq!(a.password.as_deref(), Some("secret"));
                    assert!(a.json);
                }
                _ => panic!("expected connection-test op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn redacts_db_passwords_from_debug_output() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "connection-test",
            "--db-type",
            "MYSQL",
            "--host",
            "localhost",
            "--port",
            "3306",
            "--database",
            "test",
            "--user",
            "root",
            "--password",
            "secret",
        ])
        .unwrap();

        let debug = format!("{cli:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret"));
    }

    #[test]
    fn parses_db_datasource_update_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "datasource-update",
            "--data-source-id",
            "179431",
            "--alias",
            "prod mysql",
            "--password",
            "new-secret",
            "--environment-id",
            "2",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::DatasourceUpdate(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.data_source_id, 179431);
                    assert_eq!(a.alias.as_deref(), Some("prod mysql"));
                    assert_eq!(a.password.as_deref(), Some("new-secret"));
                    assert_eq!(a.environment_id, Some(2));
                    assert!(a.json);
                }
                _ => panic!("expected datasource-update op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_db_datasource_delete_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "db",
            "datasource-delete",
            "--data-source-id",
            "179431",
            "--yes",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Db(args) => match args.op {
                DbOp::DatasourceDelete(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.data_source_id, 179431);
                    assert!(a.yes);
                    assert!(a.json);
                }
                _ => panic!("expected datasource-delete op"),
            },
            _ => panic!("expected db command"),
        }
    }

    #[test]
    fn parses_sql_query_args() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "sql",
            "query",
            "--edition",
            "local",
            "--data-source-id",
            "179431",
            "--database",
            "postgres",
            "--schema",
            "public",
            "--sql",
            "select 1",
            "--page-no",
            "2",
            "--page-size",
            "10",
            "--result-set-id",
            "2",
            "--no-row-number",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Sql(args) => match args.op {
                SqlOp::Query(a) => {
                    assert_eq!(a.edition, Edition::Local);
                    assert_eq!(a.data_source_id, 179431);
                    assert_eq!(a.database, "postgres");
                    assert_eq!(a.schema.as_deref(), Some("public"));
                    assert_eq!(a.sql, "select 1");
                    assert_eq!(a.page_no, 2);
                    assert_eq!(a.page_size, 10);
                    assert_eq!(a.result_set_id, Some(2));
                    assert!(a.no_row_number);
                    assert!(a.json);
                }
            },
            _ => panic!("expected sql command"),
        }
    }

    #[test]
    fn direct_commands_set_json_intent() {
        assert!(
            Cli::try_parse_from(["chat2db", "runtime", "start", "--json"])
                .unwrap()
                .wants_json()
        );
        assert!(
            Cli::try_parse_from([
                "chat2db",
                "db",
                "databases",
                "--data-source-id",
                "1",
                "--json"
            ])
            .unwrap()
            .wants_json()
        );
        assert!(
            Cli::try_parse_from([
                "chat2db",
                "sql",
                "query",
                "--data-source-id",
                "1",
                "--database",
                "postgres",
                "--sql",
                "select 1",
                "--json",
            ])
            .unwrap()
            .wants_json()
        );
    }

    #[test]
    fn parses_skill_install_default_target_and_json() {
        let cli = Cli::try_parse_from(["chat2db", "skill", "install", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Skill(args) => match args.op {
                SkillOp::Install(a) => {
                    assert_eq!(a.client, None);
                    assert!(a.json);
                    assert!(!a.force);
                    assert!(!a.yes);
                    assert!(!a.dry_run);
                }
                _ => panic!("expected install op"),
            },
            _ => panic!("expected skill command"),
        }
    }

    #[test]
    fn parses_skill_install_with_client_and_json() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "skill",
            "install",
            "--client",
            "claude-code",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Skill(args) => match args.op {
                SkillOp::Install(a) => {
                    assert_eq!(a.client, Some(crate::skill::SkillClient::ClaudeCode));
                    assert!(a.json);
                    assert!(!a.force);
                    assert!(!a.yes);
                    assert!(!a.dry_run);
                }
                _ => panic!("expected install op"),
            },
            _ => panic!("expected skill command"),
        }
    }

    #[test]
    fn parses_skill_update_with_codex_client() {
        let cli = Cli::try_parse_from(["chat2db", "skill", "update", "--client", "codex"]).unwrap();
        match parsed_command(cli) {
            Command::Skill(args) => match args.op {
                SkillOp::Update(a) => {
                    assert_eq!(a.client, Some(crate::skill::SkillClient::Codex));
                    assert!(!a.json);
                }
                _ => panic!("expected update op"),
            },
            _ => panic!("expected skill command"),
        }
    }

    #[test]
    fn parses_skill_uninstall_json() {
        let cli = Cli::try_parse_from(["chat2db", "skill", "uninstall", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Skill(args) => match args.op {
                SkillOp::Uninstall(a) => {
                    assert!(a.json);
                    assert_eq!(a.client, None);
                }
                _ => panic!("expected uninstall op"),
            },
            _ => panic!("expected skill command"),
        }
        assert!(
            Cli::try_parse_from(["chat2db", "skill", "uninstall", "--json"])
                .unwrap()
                .wants_json()
        );
    }

    #[test]
    fn skill_requires_subcommand() {
        assert!(Cli::try_parse_from(["chat2db", "skill"]).is_err());
    }

    #[test]
    fn parses_mcp_status_default_all_and_json() {
        let cli = Cli::try_parse_from(["chat2db", "mcp", "status", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Mcp(args) => match args.op {
                McpOp::Status(a) => {
                    assert_eq!(a.edition, EditionSelector::All);
                    assert!(a.json);
                }
                _ => panic!("expected status op"),
            },
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn parses_mcp_enable_pro_json() {
        let cli = Cli::try_parse_from(["chat2db", "mcp", "enable", "--edition", "pro", "--json"])
            .unwrap();
        match parsed_command(cli) {
            Command::Mcp(args) => match args.op {
                McpOp::Enable(a) => {
                    assert_eq!(a.edition, Edition::Pro);
                    assert!(a.json);
                }
                _ => panic!("expected enable op"),
            },
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn rejects_mcp_change_edition_all() {
        assert!(Cli::try_parse_from(["chat2db", "mcp", "enable", "--edition", "all"]).is_err());
        assert!(Cli::try_parse_from(["chat2db", "mcp", "disable", "--edition", "all"]).is_err());
        assert!(Cli::try_parse_from(["chat2db", "mcp", "restart", "--edition", "all"]).is_err());
    }

    #[test]
    fn parses_mcp_tools_pro_json() {
        let cli =
            Cli::try_parse_from(["chat2db", "mcp", "tools", "--edition", "pro", "--json"]).unwrap();
        match parsed_command(cli) {
            Command::Mcp(args) => match args.op {
                McpOp::Tools(a) => {
                    assert_eq!(a.edition, Edition::Pro);
                    assert!(a.json);
                }
                _ => panic!("expected tools op"),
            },
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn parses_mcp_config_pro_json() {
        let cli = Cli::try_parse_from(["chat2db", "mcp", "config", "--edition", "pro", "--json"])
            .unwrap();
        match parsed_command(cli) {
            Command::Mcp(args) => match args.op {
                McpOp::Config(a) => {
                    assert_eq!(a.edition, Edition::Pro);
                    assert!(a.json);
                }
                _ => panic!("expected config op"),
            },
            _ => panic!("expected mcp command"),
        }
        assert!(
            Cli::try_parse_from(["chat2db", "mcp", "config", "--edition", "pro", "--json"])
                .unwrap()
                .wants_json()
        );
    }

    #[test]
    fn parses_mcp_call_default_community() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "mcp",
            "call",
            "execute_sql",
            "--args-json",
            r#"{"sql":"select 1"}"#,
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Mcp(args) => match args.op {
                McpOp::Call(a) => {
                    assert_eq!(a.edition, Edition::Community);
                    assert_eq!(a.tool_name, "execute_sql");
                    assert_eq!(a.args_json, r#"{"sql":"select 1"}"#);
                    assert!(!a.json);
                    assert!(!a.dry_run);
                }
                _ => panic!("expected call op"),
            },
            _ => panic!("expected mcp command"),
        }
    }

    #[test]
    fn skill_rejects_removed_or_renamed_options() {
        assert!(Cli::try_parse_from(["chat2db", "skill", "install", "--from-dir", "."]).is_err());
        assert!(
            Cli::try_parse_from(["chat2db", "skill", "install", "--client", "claude"]).is_err()
        );
    }

    #[test]
    fn uninstall_help_documents_modes() {
        let mut command = Cli::command();
        let uninstall = command.find_subcommand_mut("uninstall").unwrap();
        let mut help = Vec::new();
        uninstall.write_long_help(&mut help).unwrap();
        let help = String::from_utf8(help).unwrap();

        assert!(help.contains("Remove the Chat2DB CLI binary itself"));
        assert!(help.contains("chat2db uninstall --cli"));
        assert!(help.contains("chat2db uninstall --edition local"));
    }

    #[test]
    fn parses_high_risk_confirmation_and_dry_run_flags() {
        let cli = Cli::try_parse_from([
            "chat2db",
            "install",
            "--edition",
            "pro",
            "--force",
            "--yes",
            "--dry-run",
            "--json",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Install(args) => {
                assert_eq!(args.edition, EditionSelector::Pro);
                assert!(args.force);
                assert!(args.yes);
                assert!(args.dry_run);
                assert!(args.json);
            }
            _ => panic!("expected install command"),
        }

        let cli =
            Cli::try_parse_from(["chat2db", "uninstall", "--cli", "--yes", "--dry-run"]).unwrap();
        match parsed_command(cli) {
            Command::Uninstall(args) => {
                assert!(args.cli);
                assert!(args.yes);
                assert!(args.dry_run);
            }
            _ => panic!("expected uninstall command"),
        }

        let cli = Cli::try_parse_from([
            "chat2db",
            "skill",
            "install",
            "--client",
            "codex",
            "--force",
            "--yes",
            "--dry-run",
        ])
        .unwrap();
        match parsed_command(cli) {
            Command::Skill(args) => match args.op {
                SkillOp::Install(a) => {
                    assert_eq!(a.client, Some(crate::skill::SkillClient::Codex));
                    assert!(a.force);
                    assert!(a.yes);
                    assert!(a.dry_run);
                }
                _ => panic!("expected install op"),
            },
            _ => panic!("expected skill command"),
        }

        let cli =
            Cli::try_parse_from(["chat2db", "mcp", "call", "execute_sql", "--dry-run"]).unwrap();
        match parsed_command(cli) {
            Command::Mcp(args) => match args.op {
                McpOp::Call(a) => assert!(a.dry_run),
                _ => panic!("expected call op"),
            },
            _ => panic!("expected mcp command"),
        }
    }
}
