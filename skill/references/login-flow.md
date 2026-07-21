# Login

Community has no account authentication. Pro and Local use the desktop GUI for
login.

## Community

```bash
chat2db login --edition community --json
chat2db logout --edition community --json
```

Both commands return a successful structured result with
`data.status: "not_applicable"`. They do not open the app, wait for a user, or
change account state. Do not retry them against another edition.

## Pro and Local

```bash
chat2db login --edition pro --json
chat2db login --edition local --json
chat2db logout --edition pro --json
chat2db logout --edition local --json
```

- Run login only when the user explicitly asks for it or confirms it in the
  current turn.
- Do not use `login --edition all`.
- Do not auto-retry `login_timed_out`.
- If login times out, report that GUI login was not completed.
- Use the login/logout JSON result as the account-state source.
