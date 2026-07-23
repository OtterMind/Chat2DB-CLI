# Installation

This file is for installing, updating, and removing the `chat2db` CLI binary and the skill bundle.

The bundled Skill version is `0.2.1` and supports CLI versions
`>=0.2, <0.3`.

## CLI Binary

Install or update:

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

Windows PowerShell:

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

Verify:

```bash
command -v chat2db && chat2db version --json
```

If `chat2db` is not on `PATH`, add the install directory:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

On Windows the default install path is `%LocalAppData%\Programs\chat2db-cli\chat2db.exe`; open a new terminal after PATH changes.

Useful installer variables:

| Variable | Purpose |
| --- | --- |
| `CHAT2DB_CLI_VERSION` | Pin a CLI version instead of `latest` |
| `CHAT2DB_CLI_BIN_DIR` | Install directory for the binary |
| `CHAT2DB_CLI_NO_COMPLETIONS=1` | Skip completion install |
| `CHAT2DB_CLI_NO_PATH_WARN=1` | Suppress PATH warning |

`CHAT2DB_CLI_MANIFEST_BASE` and `CHAT2DB_CLI_CDN_BASE` exist for isolated
installer tests. Do not override them in normal installations: GitHub Releases
is the manifest trust root and the Chat2DB CDN is the artifact mirror.

PowerShell parameters: `-Version`, `-BinDir`, `-Force`, and `-NoModifyPath`.

Desktop installation uses the reviewed registry compiled into the CLI.
Community entries bind formal GitHub Release assets with SHA-256. Pro and Local
entries bind exact versioned CDN artifacts with size and Base64 SHA-512. Update
the CLI when a newer desktop release is required; do not bypass a registry
failure with a direct mutable artifact URL.

## CLI Update

```bash
chat2db update --json
chat2db update --target "$HOME/.local/bin/chat2db" --json
```

Use `--target` only when the user asks for a specific binary path.
The default update and an existing target accept only a strictly newer release.
An explicit nonexistent target may install the current or a newer release. If
an explicit existing target is not the running binary, run that target's own
`update` command; the CLI refuses to replace a binary whose version it cannot
verify safely.

## CLI Uninstall

```bash
chat2db uninstall --cli --dry-run --json
chat2db uninstall --cli --yes --json
```

This removes the binary only. It does not remove shell completions, state files, app data, or installed skill bundles.

## Skill Bundle

Install with the CLI:

```bash
chat2db skill install --json
chat2db skill install --client claude-code --json
chat2db skill install --client codex --json
```

Update:

```bash
chat2db skill update --json
chat2db skill update --client codex --json
```

Uninstall:

```bash
chat2db skill uninstall --json
chat2db skill uninstall --client codex --json
```

Download without the CLI:

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-skill.sh | bash
```

That command writes `./chat2db-cli/`. Move it into the client skill directory yourself if needed.

Skill target paths:

| Selector | Path |
| --- | --- |
| no `--client` | `<cwd>/chat2db-cli/` |
| `--client claude-code` | `~/.claude/skills/chat2db-cli/` |
| `--client codex` | `~/.codex/skills/chat2db-cli/` |

Rules:

- Do not overwrite a foreign target without explicit user confirmation.
- Git-managed targets are refused; manage them with git.
- If `skill_version_incompatible` appears, update the CLI or install a compatible skill version.

## Repository Development Install

From a `Chat2DB-CLI` source checkout, install the current CLI build and Skill
together:

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

The first command targets Codex, the second targets Claude Code, and the third
refreshes only the Codex Skill. These development installers build with
`Cargo.lock`, require the Cargo and Skill versions to match, reject symlink and
Git-managed targets, and recover interrupted same-filesystem replacements.
