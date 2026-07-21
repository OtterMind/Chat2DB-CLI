# chat2db Commands

Use this file only to look up command shape and flags. Prefer `chat2db <command> --help` when available.

## General

- Use `--json` when parsing command output.
- `logs` streams text and does not accept `--json`.
- `completions --shell <shell>` streams script text unless `--install` is used.
- Default edition is `community` when a command accepts `--edition`, except
  `mcp status`, which defaults to `all`.
- Use `--edition pro` or `--edition local` only when the user asks for that product.
- Use `--edition all` only on commands that list it below.
- Prefer headless runtime commands for database work. Do not run App lifecycle commands unless the user explicitly asks to open, install, restart, login, or stop the desktop app.

## version

```bash
chat2db version [--json]
chat2db -v [--json]
chat2db --version [--json]
chat2db -version [--json]
```

Use to confirm the installed CLI version.

## status

```bash
chat2db status [--json]
```

Read this first. It reports Community, Pro, and Local in one response with
separate App, CLI runtime, and MCP state. Use
`data.summary.<edition>.next_action` when the requested edition is missing or
not safe to exit.

## install

```bash
chat2db install [--edition community|pro|local|all] [--force] [--yes] [--dry-run] [--background] [--json]
```

Run only when the user explicitly asks to install or update the desktop app.

Common forms:

```bash
chat2db install --edition community --json
chat2db install --edition all --background --json
chat2db install --edition community --force --dry-run --json
chat2db install --edition community --force --yes --json
```

Rules:

- `--force` real writes require `--yes`.
- Prefer `--dry-run --json` before unclear replacement.
- If a background install is already running, use `logs install`; do not start another one.

## uninstall

```bash
chat2db uninstall [--edition community|pro|local|all] [--yes] [--dry-run] [--json]
chat2db uninstall --cli [--target <path>] [--yes] [--dry-run] [--json]
```

Common forms:

```bash
chat2db uninstall --edition community --dry-run --json
chat2db uninstall --edition community --yes --json
chat2db uninstall --cli --yes --json
```

## login

```bash
chat2db login [--edition community|pro|local|all] [--json]
```

Rules:

- Community returns `not_applicable` without opening an authentication flow.
- For Pro and Local, confirm the user is ready before running it unless they
  explicitly asked to log in during the current turn.
- Do not use `--edition all`.
- Pro and Local open the app and wait for the user to finish login.

## logout

```bash
chat2db logout [--edition community|pro|local|all] [--json]
```

Community returns `not_applicable`. Pro and Local sign out when the user asks.

## start

```bash
chat2db start [--edition community|pro|local|all] [--json]
```

Opens the selected app edition. Run only when the user explicitly asks to open the GUI.

## stop

```bash
chat2db stop [--edition community|pro|local|all] [--all] [--json]
```

Without `--all`, this quits only the selected desktop App. With `--all`, it disables MCP, stops the CLI runtime, stops the desktop App, and returns final summary state.

Common forms:

```bash
chat2db stop --edition community --json
chat2db stop --edition community --all --json
chat2db stop --edition all --all --json
```

## restart

```bash
chat2db restart [--edition community|pro|local|all] [--json]
```

Runs stop then start for the selected edition. Run only when the user explicitly asks to restart the desktop app.

## runtime

```bash
chat2db runtime status [--edition community|pro|local] [--json]
chat2db runtime start  [--edition community|pro|local] [--json]
chat2db runtime stop   [--edition community|pro|local] [--json]
```

Use `runtime status` for diagnostics. `db` and `sql` commands start the headless runtime when needed, so `runtime start` is usually optional. Starting the runtime is not a GUI operation.

## db

```bash
chat2db db datasources [--edition community|pro|local] [--page-no N] [--page-size N] [--refresh] [--json]
chat2db db datasource  --data-source-id ID [--edition community|pro|local] [--json]
chat2db db connection-test --data-source-id ID [--edition community|pro|local] [--json]
chat2db db connection-test --db-type TYPE --url JDBC_URL --user USER --password PASSWORD [--edition community|pro|local] [--json]
chat2db db connection-test --db-type TYPE --host HOST --port PORT --database DB --user USER --password PASSWORD [--edition community|pro|local] [--json]
chat2db db datasource-create --db-type TYPE --url JDBC_URL --password PASSWORD --environment-id ID [--edition community|pro|local] [--alias ALIAS] [--user USER] [--service-name NAME] [--service-type TYPE] [--storage-type TYPE] [--space-id ID] [--json]
chat2db db datasource-create --db-type TYPE --host HOST --port PORT --database DB --password PASSWORD --environment-id ID [--edition community|pro|local] [--alias ALIAS] [--user USER] [--service-name NAME] [--service-type TYPE] [--storage-type TYPE] [--space-id ID] [--json]
chat2db db datasource-update --data-source-id ID [--edition community|pro|local] [--alias ALIAS] [--db-type TYPE] [--url JDBC_URL] [--user USER] [--password PASSWORD] [--environment-id ID] [--host HOST] [--port PORT] [--database DB] [--service-name NAME] [--service-type TYPE] [--storage-type TYPE] [--space-id ID] [--json]
chat2db db datasource-delete --data-source-id ID [--edition community|pro|local] --yes [--json]
chat2db db databases   --data-source-id ID [--edition community|pro|local] [--refresh] [--json]
chat2db db schemas     --data-source-id ID --database DB [--edition community|pro|local] [--refresh] [--json]
chat2db db tables      --data-source-id ID --database DB [--schema SCHEMA] [--edition community|pro|local] [--page-no N] [--page-size N] [--refresh] [--json]
chat2db db table       --data-source-id ID --database DB [--schema SCHEMA] --table TABLE [--edition community|pro|local] [--json]
```

Datasource CRUD:

```bash
chat2db db connection-test --db-type MYSQL --host localhost --port 3306 --database test --user root --password '<password>' --json
chat2db db connection-test --db-type MYSQL --url 'jdbc:mysql://127.0.0.1:3306/test' --user root --password '<password>' --json
chat2db db datasource-create --db-type MYSQL --url 'jdbc:mysql://127.0.0.1:3306/test' --user root --password '<password>' --environment-id 1 --json
chat2db db datasource-create --db-type MYSQL --host 127.0.0.1 --port 3306 --database test --user root --password '<password>' --environment-id 1 --json
chat2db db connection-test --data-source-id <id> --json
chat2db db datasource-update --data-source-id <id> --alias <alias> --json
chat2db db datasource-delete --data-source-id <id> --yes --json
```

Rules:

- Run `connection-test` before `datasource-create` when the user provides new connection fields and the password is non-empty.
- `connection-test --data-source-id` tests an existing saved datasource and must not include temporary connection fields.
- Temporary `connection-test` requires non-empty `--db-type`, `--user`, `--password`, and either `--url` or `--host --port --database`.
- `connection-test --url` works for any Chat2DB datasource type when the user supplies the full JDBC URL.
- `connection-test --host --port --database` supports automatic URL construction for: `MYSQL`, `MARIADB`, `TIDB`, `DORIS`, `STARROCKS`, `POSTGRESQL`, `POSTGRES`, `COCKROACHDB`, `GAUSSDB`, `OPENGAUSS`, `SQLSERVER`, `ORACLE`, `DB2`, `DM`, `KINGBASE`, `CLICKHOUSE`, `PRESTO`, `HIVE`, `OCEANBASE`, `OCEANBASE_ORACLE`, `MONGODB`, `REDIS`, `H2`, `REDSHIFT`, `KYLIN`, `OSCAR`, `SUNDB`, `XUGUDB`, and `TDENGINE`.
- Use `connection-test --url` instead of host mode for `BIGQUERY`, `DUCKDB`, `ELASTICSEARCH`, `SQLITE`, `SNOWFLAKE`, `GBASE8S`, `INFORMIX`, custom drivers, or any special JDBC URL that cannot be represented by host, port, and database.
- `datasource-create` requires `--db-type`, `--password`, `--environment-id`, and either `--url` or `--host --port --database`.
- `datasource-create --url` is mutually exclusive with `--host`, `--port`, and `--database`.
- `datasource-create --host --port --database` supports the same automatic JDBC URL construction list as temporary host-mode `connection-test`.
- `datasource-create --password ''` is valid for an intentionally empty datasource password. The flag must still be present.
- Environment ids are `1=TEST` and `2=RELEASE`; default to `1` when the user does not specify one.
- `datasource-update` requires `--data-source-id`; all other datasource fields are optional.
- `datasource-delete` requires `--data-source-id` and `--yes`.
- Redact `--password` values in user-facing output.

Normal discovery order:

```bash
chat2db db datasources --edition community --json
chat2db db datasource --data-source-id <id> --json
chat2db db databases --data-source-id <id> --json
chat2db db schemas --data-source-id <id> --database <db> --json
chat2db db tables --data-source-id <id> --database <db> --schema <schema> --json
chat2db db table --data-source-id <id> --database <db> --schema <schema> --table <table> --json
```

## sql

```bash
chat2db sql query --data-source-id ID --database DB [--schema SCHEMA] --sql SQL [--edition community|pro|local] [--page-no N] [--page-size N] [--result-set-id N] [--no-row-number] [--json]
```

Executes SQL through the CLI runtime. The command does not start the GUI; execution behavior is governed by Chat2DB, the datasource, and database permissions.

```bash
chat2db sql query --data-source-id <id> --database <db> --schema <schema> --sql 'select 1' --json
chat2db sql query --data-source-id <id> --database <db> --schema <schema> --sql 'select * from users' --page-no 2 --page-size 100 --json
chat2db sql query --data-source-id <id> --database <db> --schema <schema> --sql 'select 1; select 2' --result-set-id 2 --no-row-number --json
```

Rules:

- Default paging is `--page-no 1 --page-size 100`. For large result sets, rerun the same query with the next `--page-no`.
- `--result-set-id` selects one result set from multi-result SQL. Use the id returned in the SQL response when available.
- `--no-row-number` suppresses Chat2DB's synthetic row-number column for script/ETL style consumers.

## mcp

```bash
chat2db mcp status  [--edition community|pro|local|all] [--json]
chat2db mcp enable  [--edition community|pro|local] [--json]
chat2db mcp disable [--edition community|pro|local] [--json]
chat2db mcp restart [--edition community|pro|local] [--json]
chat2db mcp config  [--edition community|pro|local] [--json]
chat2db mcp tools   [--edition community|pro|local] [--json]
chat2db mcp call <tool-name> [--args-json <json-object>] [--edition community|pro|local] [--dry-run] [--json]
```

Use `mcp status` before changing MCP. Use `mcp config` when the user asks for client config. Use `mcp tools` before `mcp call`. `mcp enable` and `mcp restart` can relaunch the desktop app, so run them only when the user explicitly asks for MCP startup or restart.

Common forms:

```bash
chat2db mcp status --edition all --json
chat2db mcp enable --edition community --json
chat2db mcp disable --edition community --json
chat2db mcp restart --edition community --json
chat2db mcp config --edition community
chat2db mcp tools --edition community --json
chat2db mcp call list_all_datasources --args-json '{}' --edition community --json
chat2db mcp call <tool> --args-json '<json-object>' --edition community --dry-run --json
```

`mcp config` prints the generic JSON config itself, not an `ok/data` envelope. It intentionally includes the selected edition MCP token:

```json
{
  "mcpServers": {
    "chat2db": {
      "transport": "streamable_http",
      "url": "http://127.0.0.1:10825/mcp",
      "headers": {
        "x-chat2db-mcp-token": "<token>"
      }
    }
  }
}
```

Community uses `http://127.0.0.1:10825/mcp`. Pro and Local use
`http://127.0.0.1:11924/mcp`. Always use the URL returned by `mcp config`
instead of substituting a port manually.

For datasource-scoped MCP tools, start with `list_all_datasources`, then pass the returned `dataSourceId` to later tool calls.

## update

```bash
chat2db update [--target <path>] [--json]
```

Updates the CLI binary. Use `--target` only when the user asks for a specific binary path.

## skill

```bash
chat2db skill install   [--client claude-code|codex] [--force] [--yes] [--dry-run] [--json]
chat2db skill update    [--client claude-code|codex] [--json]
chat2db skill uninstall [--client claude-code|codex] [--json]
```

Target paths:

| Selector | Path |
| --- | --- |
| no `--client` | `<cwd>/chat2db-cli/` |
| `--client claude-code` | `~/.claude/skills/chat2db-cli/` |
| `--client codex` | `~/.codex/skills/chat2db-cli/` |

Rules:

- `install --force` real writes require `--yes`.
- Git-managed targets are refused.
- `uninstall` has no force mode.

## completions

```bash
chat2db completions [--shell zsh|bash|fish] [--install] [--json]
```

Common forms:

```bash
chat2db completions --shell zsh > _chat2db
chat2db completions --install --shell zsh --json
```

## logs

```bash
chat2db logs install [--edition community|pro|local] [--follow]
```

Use only for background install logs.
