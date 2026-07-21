# Troubleshooting

Use this file when the symptom is clearer than the error code. If JSON output exists, prefer `error.code` and [error-codes.md](error-codes.md).

## `chat2db: command not found`

Check:

```bash
ls -l "${CHAT2DB_CLI_BIN_DIR:-$HOME/.local/bin}/chat2db"
```

If the binary exists, add it to `PATH`. If it does not exist, tell the user the install command:

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

On Windows:

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

## Unsupported Platform

Desktop app lifecycle and runtime operations require macOS or Windows:

```bash
chat2db install
chat2db start
chat2db stop
chat2db restart
```

Pro and Local login/logout also require their supported desktop environment.
Community login/logout are authentication-free `not_applicable` operations and
do not open an app.

## App Already Exists

Do not add `--force` automatically.

Use:

```bash
chat2db install --edition community --force --dry-run --json
```

Then ask before:

```bash
chat2db install --edition community --force --yes --json
```

## Checksum Mismatch

Stop. Surface expected and actual hashes. Do not retry automatically.

## Login Timed Out

This applies only to Pro and Local. Community login returns `not_applicable`.
Do not retry automatically. Tell the user the GUI login was not completed,
then run login again only when they are ready:

```bash
chat2db login --edition pro --json
```

## Background Install Already Running

Do not start another install.

Use:

```bash
chat2db logs install --edition community --follow
```

## No Install Log Found

`chat2db logs install` only works after a background install.

For CLI runtime diagnostics, use `chat2db runtime status --edition <edition> --json`. `chat2db status --json` now reports App, CLI runtime, and MCP state only.

## Runtime Shows `stale`

`runtime status` reports `stale` when the saved runtime pid or port no longer answers the health check.

Use the direct runtime path first, not MCP:

```bash
chat2db runtime status --edition community --json
chat2db runtime stop --edition community --json
chat2db db datasources --edition community --json
```

`db` and `sql` commands start the runtime automatically when needed. If the direct command still fails, inspect `error.code` and `chat2db runtime status --edition <edition> --json`. Do not switch to `mcp` unless the user explicitly asked for MCP or the runtime/db/sql path cannot cover the request.

## Completion Shell Not Detected

Pass the shell explicitly:

```bash
chat2db completions --shell zsh
chat2db completions --install --shell zsh --json
```

Supported shells: `zsh`, `bash`, `fish`.

## Datasource Delete Requires Confirmation

Without `--yes`, datasource deletion returns `confirmation_required`.

Use only after the user clearly wants deletion:

```bash
chat2db db datasource-delete --data-source-id <id> --yes --json
```

Before deletion, inspect the datasource when useful:

```bash
chat2db db datasource --data-source-id <id> --json
```

Do not print password fields.

## Invalid Installed App

If `status` reports `data.summary.<edition>.app.state: "invalid"`, do not use `install --force` blindly.

Try:

```bash
chat2db uninstall --edition community --json
```

If uninstall is refused as unsafe, surface the reason and stop.

## JSON Parse Fails

Do not parse raw-output commands as JSON:

```bash
chat2db logs install --edition community
chat2db logs install --edition community --follow
chat2db completions --shell zsh
```

Use JSON-capable forms instead:

```bash
chat2db status --json
chat2db completions --install --shell zsh --json
```
