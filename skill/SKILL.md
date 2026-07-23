---
name: chat2db-cli
description: "Operate Chat2DB Community, Pro, and Local through the chat2db CLI: inspect status, manage the app and headless runtime, work with datasources and SQL, configure MCP, manage the Skill, and update the CLI. Use for CLI-driven Chat2DB workflows, not GUI interaction."
metadata:
  version: "0.2.1"
  compatible-cli-versions: ">=0.2, <0.3"
---

# Chat2DB CLI

Use the `chat2db` command directly. Prefer live command output and
`chat2db <command> --help` over static examples.

## Start Here

```bash
command -v chat2db && chat2db version --json
chat2db status --json
```

If `chat2db` is missing, do not install it silently. Tell the user the
appropriate install command from [references/installation.md](references/installation.md).

This Skill supports CLI versions `>=0.2, <0.3`. Outside that range, say that
the Skill may be stale and continue from `chat2db --help` and
`chat2db <command> --help`.

## Edition Selection

- Use `community` when the user does not specify an edition.
- Use `pro` or `local` only when the user requests that product or the
  current state clearly belongs to it.
- Use `all` only for commands that explicitly support it. The order is
  Community, Pro, then Local.
- Community has no account authentication. `login` and `logout` with
  `--edition community` return `not_applicable` and must not open an
  authentication flow.

Community `runtime`, `db`, and `sql` commands require a compatible Chat2DB
Community 5.3.0 or later build. The CLI validates the runtime edition, API
version, and capabilities before sending business data. Report any contract
error; never retry against Local or Pro.

Read [references/editions.md](references/editions.md) for the complete edition
matrix and paths.

## Priority

1. Run `chat2db status --json` before choosing follow-up commands. Read
   `data.summary.<edition>`, `safe_to_exit`, and `next_action`.
2. Prefer `chat2db runtime`, `chat2db db`, and `chat2db sql` for database
   work. These commands may start the headless runtime but should not open the
   GUI.
3. Start, install, stop, or restart the desktop app only when the user asks for
   that lifecycle action.
4. Use `chat2db mcp` only for an explicit MCP request or when direct
   runtime/database/SQL commands cannot perform the task.
5. Use `--json` whenever command output will be parsed.

## Common Commands

| Task | Command |
| --- | --- |
| Inspect all editions | `chat2db status --json` |
| Community runtime status | `chat2db runtime status --edition community --json` |
| List Community datasources | `chat2db db datasources --edition community --json` |
| Show a datasource | `chat2db db datasource --data-source-id <id> --edition community --json` |
| Test a saved datasource | `chat2db db connection-test --data-source-id <id> --edition community --json` |
| List databases | `chat2db db databases --data-source-id <id> --edition community --json` |
| List schemas | `chat2db db schemas --data-source-id <id> --database <db> --edition community --json` |
| List tables | `chat2db db tables --data-source-id <id> --database <db> --schema <schema> --edition community --json` |
| Inspect a table | `chat2db db table --data-source-id <id> --database <db> --schema <schema> --table <table> --edition community --json` |
| Query SQL | `chat2db sql query --data-source-id <id> --database <db> --schema <schema> --sql '<sql>' --edition community --json` |
| MCP status | `chat2db mcp status --edition community --json` |
| MCP tools | `chat2db mcp tools --edition community --json` |
| Install Skill for Codex | `chat2db skill install --client codex --json` |
| Update CLI | `chat2db update --json` |

Read [references/commands.md](references/commands.md) for complete command
shapes and flags.

## Datasource Flow

1. List existing datasources for the selected edition.
2. Test user-supplied connection fields before creating a datasource when the
   password is non-empty.
3. Use either `--url` or `--host --port --database`; do not mix them.
4. Keep `--password` present for datasource creation, including an explicitly
   empty password.
5. Create only after a successful connection test, except for an intentionally
   empty-password datasource that cannot use the temporary test form.
6. Treat environment ids as `1=TEST` and `2=RELEASE`; default to `1` when
   the user does not specify one.

## SQL Flow

1. Use `--page-no` and `--page-size` for result paging.
2. If `hasNextPage` is true, rerun the same SQL with the next page number.
3. Use `--result-set-id` for a specific result from multi-result SQL.
4. Use `--no-row-number` when downstream consumers need only database
   columns.

## Safety

- Parse `ok`, `data`, `error.code`, `error.details`, `error.hint`,
  `error.next_action`, `error.retryable`, and `error.risk`. Do not branch
  on prose messages.
- Do not pass `--json` to `chat2db logs`; it streams log text.
- Run login only when explicitly requested. Never turn a Community
  `not_applicable` result into a Pro or Local login attempt.
- Do not install a missing edition unless the user asked for installation.
- Use `--dry-run --json` for writes with unclear impact. Use `--yes` only
  for explicit destructive intent.
- Do not retry an unchanged command when `error.retryable` is false.
- Do not auto-retry integrity, official-source, or unsafe-path errors.
- Poll long operations once or twice, then report their state.
- Redact passwords, tokens, cookies, and bearer values from user-facing output.
- `chat2db mcp config` intentionally includes a selected edition's MCP token;
  show it only when the user explicitly requests MCP client configuration.

## References

Read a reference only when the live command help is insufficient.

| Need | Reference |
| --- | --- |
| Full command surface | [references/commands.md](references/commands.md) |
| Edition behavior and paths | [references/editions.md](references/editions.md) |
| JSON envelopes and fields | [references/json-output.md](references/json-output.md) |
| Error recovery | [references/error-codes.md](references/error-codes.md) |
| Background installation | [references/background-install.md](references/background-install.md) |
| Login behavior | [references/login-flow.md](references/login-flow.md) |
| CLI and Skill installation | [references/installation.md](references/installation.md) |
| Symptom troubleshooting | [references/troubleshooting.md](references/troubleshooting.md) |
