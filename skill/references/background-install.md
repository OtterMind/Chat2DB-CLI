# Background Install

Use background install when the user wants the terminal back while the app downloads and installs.

## Start

```bash
chat2db install --edition community --background --json
chat2db install --edition pro --background --json
chat2db install --edition local --background --json
```

The command returns a PID and log path immediately.

## Logs

```bash
chat2db logs install --edition community
chat2db logs install --edition community --follow
```

`--follow` tails the log. Stopping the tail does not stop the install. Use the
PID and log path returned by the background install command as the primary
progress handles.

## Rules

- Do not start a second background install for an edition that already has one running.
- If `background_install_already_running` appears, use `logs install`.
- Poll once or twice, then report the state.
- Use foreground install when the user did not ask for background behavior.
