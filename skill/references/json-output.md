# JSON Output

Use `--json` whenever command output will be parsed.

## Envelope

Success:

```json
{"ok": true, "data": {}}
```

Failure:

```json
{"ok": false, "error": {"code": "...", "message": "...", "hint": "...", "next_action": "...", "retryable": false, "risk": {"level": "...", "action": "..."}}}
```

Parse `error.code`, `error.hint`, `error.next_action`, `error.retryable`, and `error.risk`. Treat `error.message` as display text.

## Raw Output Exceptions

Do not JSON-parse:

```bash
chat2db logs install --edition community
chat2db logs install --edition community --follow
chat2db completions --shell zsh
chat2db completions --shell bash
chat2db completions --shell fish
```

`chat2db completions --install --json` does return JSON.

## Common Fields

### `status`

Read:

- `data.cli_version`
- `data.summary.community`
- `data.summary.pro`
- `data.summary.local`
- `data.summary.<edition>.app.state`
- `data.summary.<edition>.app.pids`
- `data.summary.<edition>.cli.state`
- `data.summary.<edition>.cli.pid`
- `data.summary.<edition>.cli.port`
- `data.summary.<edition>.cli.url`
- `data.summary.<edition>.mcp.enabled`
- `data.summary.<edition>.mcp.endpoint`
- `data.summary.<edition>.mcp.needs_restart`
- `data.summary.<edition>.background_install.state` when a background install is running or has completed
- `data.summary.<edition>.background_install.pid` while running
- `data.summary.<edition>.background_install.exit_code` after completion
- `data.summary.<edition>.background_install.log_path`
- `data.summary.<edition>.safe_to_exit`
- `data.summary.<edition>.next_action`

### `install`

Read:

- `data.edition`
- `data.status`
- `data.app_path`
- `data.log_path` for background install
- `data.pid` for background install

### `runtime`

Read:

- `data.command`
- `data.edition`
- `data.status`
- `data.pid`
- `data.healthy`
- `data.state_path`
- `data.app_path`

Do not print token-bearing runtime state file contents.

### `db` and `sql`

Read:

- `data.command`
- `data.edition`
- `data.data`

`data.data` is the command result payload.

Known direct command names include:

- `db_datasources`
- `db_datasource`
- `db_connection_test`
- `db_datasource_create`
- `db_datasource_update`
- `db_datasource_delete`
- `db_databases`
- `db_schemas`
- `db_tables`
- `db_table`
- `sql_query`

For successful `db_connection_test`, read `data.data.mode`, `data.data.canConnect`, and `data.data.durationMs`; `canConnect` should be `true`. Failed connection tests return `ok:false` with `error.code=datasource_connection_failed`. For datasource create/update, `data.data` is the datasource payload returned by Chat2DB. For datasource delete, `data.data` is the delete result. Do not print password fields.

For `sql_query`, read the returned result payload fields when present:

- `columns` / `rows`
- `rowCount`
- `updateCount`
- `pageNo`
- `pageSize`
- `hasNextPage`
- `resultSetId`

When `hasNextPage` is true, rerun `chat2db sql query` with the next `--page-no` and the same SQL arguments. When `--no-row-number` is not used, Chat2DB may include a synthetic row-number column in `columns` and row values.

### `mcp status`

Read:

- `data.command`
- `data.editions[]`
- `data.editions[].edition`
- `data.editions[].enabled`
- `data.editions[].token_state`
- `data.editions[].app_running`
- `data.editions[].endpoint_state`
- `data.editions[].needs_restart`
- `data.editions[].next_action`

Do not print token values.

### `mcp tools`

Read:

- `data.tools[]`
- `data.tools[].name`
- `data.tools[].description`
- `data.tools[].input_schema`
- `data.tools[].example_args`
- `data.tools[].risk_hint`
- `data.tools[].call_example`

Use this before `mcp call`.

### `mcp call`

Read:

- `data.tool_name`
- `data.result`

### `skill`

Read:

- `data.operation`
- `data.status`
- `data.client`
- `data.target_path`
- `data.skill_version`
- `data.manifest_url`
- `data.artifact_url`

### `login`

Read:

- `data.edition`
- `data.status`
- `data.login_state`
- `data.waited_seconds`

For Community, `data.status` is `not_applicable` and no GUI authentication
flow is started.

### `logout`

Read:

- `data.edition`
- `data.status`
- `data.login_state`

For Community, `data.status` is `not_applicable` and no account state is
changed.

### `start`, `stop`, `restart`

Read:

- `data.edition`
- `data.status` for `start` and `stop`
- `data.stop_status` and `data.start_status` for `restart`
- `data.app_path`

For `chat2db stop --all --json`, read:

- `data.command`
- `data.mode`
- `data.editions[]`
- `data.editions[].edition`
- `data.editions[].status`
- `data.editions[].steps.mcp.status`
- `data.editions[].steps.cli.status`
- `data.editions[].steps.app.status`
- `data.editions[].final_summary.safe_to_exit`

### `update`

Read:

- `data.status`
- `data.target_path`
- `data.version`

`data.status` can be `up_to_date` when the installed CLI already matches the latest manifest.

### `completions --install`

Read:

- `data.status`
- `data.shell`
- `data.script_path`
- `data.rc_path`
- `data.rc_action`
