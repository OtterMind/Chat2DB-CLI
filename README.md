# Chat2DB CLI

English | [French](README_FR.md) | [Japanese](README_JA.md) | [Spanish](README_ES.md) | [Korean](README_KO.md)

Chat2DB CLI is the official command-line and agent integration toolkit for
[Chat2DB](https://github.com/OtterMind/Chat2DB). It provides a Rust CLI for
desktop lifecycle, datasource metadata, SQL, and MCP workflows, plus the
Chat2DB Agent Skill in the same repository. Chat2DB Community is the default
edition.

## Included

- `chat2db`: a native CLI for macOS, Linux, and Windows.
- `skill/`: the Chat2DB Agent Skill for Codex and Claude Code.
- Machine-readable `--json` output for scripts and agents.
- One version and one release for the CLI and Skill.

## Quick Start With Community

Install the CLI on macOS or Linux:

```bash
curl -fsSL https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.sh | sh
```

On Windows PowerShell:

```powershell
iwr https://github.com/OtterMind/Chat2DB-CLI/releases/latest/download/install-cli.ps1 -UseBasicParsing | iex
```

On macOS or Windows, install and inspect Chat2DB Community:

```bash
chat2db install --edition community
chat2db status
```

`community` is the default edition, so the explicit `--edition community`
option may be omitted. Community is local-first and does not require login.
On Linux, the CLI and Agent Skill are supported, while desktop lifecycle and
runtime commands require a supported Chat2DB desktop release.

## System Requirements

- macOS 11 or later on arm64 or x86_64.
- Windows 10 or 11, 64-bit only. Windows on ARM64 runs the x86_64 binary
  through the operating system's x64 compatibility layer. 32-bit Windows is
  not supported.
- GNU/Linux on arm64 or x86_64 with glibc 2.35 or later. Published Linux
  artifacts use GNU libc; musl-only systems, including Alpine Linux, are not
  supported.

## Editions

| Capability | Community | Pro | Local |
| --- | :---: | :---: | :---: |
| Install and desktop lifecycle on macOS/Windows | Yes | Yes | Yes |
| Headless CLI runtime | Yes | Yes | Yes |
| Datasource CRUD and metadata | Yes | Yes | Yes |
| SQL execution | Yes | Yes | Yes |
| MCP management and tool calls | Yes | Yes | Yes |
| Account login and logout | Not applicable | Yes | Yes |
| JSON output | Yes | Yes | Yes |

Community runtime, datasource, metadata, and SQL commands require a compatible
Chat2DB Community 5.3.0 or later build. The CLI validates the runtime edition,
API version, and capabilities before sending business data and fails closed on
an incompatible server instead of falling back to Local or Pro.

Use `--edition community`, `--edition pro`, or `--edition local` to target one
product. Commands that support multiple products also accept `--edition all`.
Run `chat2db <command> --help` for the exact options available to a command.

## CLI Workflows

Manage the Community desktop app:

```bash
chat2db status
chat2db start --edition community
chat2db stop --edition community
chat2db restart --edition community
```

The runtime, datasource, and SQL examples below require a compatible Chat2DB
Community 5.3.0 or later build. An older or mismatched runtime is rejected
before the CLI sends business data.

Inspect the local runtime:

```bash
chat2db runtime status --edition community --json
chat2db runtime start --edition community --json
chat2db runtime stop --edition community --json
```

List datasources and metadata:

```bash
chat2db db datasources --edition community --json
chat2db db databases --edition community --data-source-id 123 --json
chat2db db schemas --edition community --data-source-id 123 --database postgres --json
chat2db db tables --edition community --data-source-id 123 --database postgres --schema public --json
chat2db db table --edition community --data-source-id 123 --database postgres --schema public --table users --json
```

Test and create a datasource:

```bash
chat2db db connection-test --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --json
chat2db db datasource-create --edition community --db-type MYSQL --host 127.0.0.1 --port 3306 --database demo --user root --password '<password>' --environment-id 1 --json
```

Prefer reusing a saved datasource so a password does not enter shell history.
Never publish passwords, tokens, private hosts, or production data in logs or
issues.

Run SQL:

```bash
chat2db sql query --edition community --data-source-id 123 --database postgres --schema public --sql 'select 1' --json
```

The CLI starts or reuses the selected edition's loopback-only runtime when a
database command needs it.

## MCP

Enable MCP, inspect its tools, and print a client configuration:

```bash
chat2db mcp enable --edition community --json
chat2db mcp tools --edition community --json
chat2db mcp config --edition community
```

Call a tool directly:

```bash
chat2db mcp call list_all_datasources --edition community --args-json '{}' --json
```

`mcp config` intentionally contains an authentication token. Treat its output
as a secret and do not paste it into public logs.

## Agent Skill

Install the bundled Skill for Codex:

```bash
chat2db skill install --client codex
```

Or for Claude Code:

```bash
chat2db skill install --client claude-code
```

Update or remove it with:

```bash
chat2db skill update --client codex
chat2db skill uninstall --client codex
```

The Skill is maintained in [`skill/`](skill/) and released with the matching
CLI version. It uses the CLI instead of duplicating Chat2DB runtime logic.

## Automation

At startup, interactive commands with human-readable output perform a
best-effort update check. The network manifest is refreshed at most once every
24 hours; other starts reuse the private cache and continue to show a known
newer version. The check only prints a notice and never installs automatically.
It is skipped for `--json`, non-interactive sessions, background processes, and
the `update`, `completions`, and `logs` commands. Set
`CHAT2DB_NO_UPDATE_CHECK=1` to disable it.

Use `--json` when output is consumed by a script or agent:

```bash
chat2db status --json
chat2db version --json
```

Mutating or destructive commands may require `--yes`. Supported operations can
be inspected with `--dry-run`. JSON responses never intentionally expose
datasource passwords or runtime bearer tokens.

## Install From Source

Install the stable Rust toolchain, then run:

```bash
git clone https://github.com/OtterMind/Chat2DB-CLI.git
cd Chat2DB-CLI
cargo build --release
cargo test
```

The binary is written to `target/release/chat2db` (or
`target/release/chat2db.exe` on Windows). The default developer command builds
with the lockfile and transactionally installs both the CLI and the Codex Skill.
Use the second form for Claude Code or the third to refresh only the Codex
Skill:

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

These development installers require Bash. On Windows, run them from Git Bash,
MSYS2, or Cygwin; a native PowerShell development installer is not provided.
The combined command commits the CLI and Skill together, restores both after a
failed or interrupted install, and requires `--force` before taking ownership
of targets that do not have matching installer provenance.

## Development

Before opening a pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

When CLI behavior or commands change, update the Skill and all five README
files in the same pull request. See [CONTRIBUTING.md](CONTRIBUTING.md).

## Repository Layout

```text
src/          Rust CLI
tests/        integration tests
skill/        Chat2DB Agent Skill
scripts/      local install and packaging helpers
.github/      CI and release automation
```

## Releases

The manually dispatched release workflow has two modes. `cdn-only` builds and
uploads immutable versioned objects without changing `latest`. `release`
requires a pre-existing signed `vX.Y.Z` Tag at the protected `main` HEAD, then
promotes CDN aliases and publishes the CLI binaries and Skill bundle together to
[GitHub Releases](https://github.com/OtterMind/Chat2DB-CLI/releases).

Each GitHub Release includes a CycloneDX SBOM and GitHub build-provenance
attestation. Every CLI and Skill archive includes `LICENSE` and
`THIRD_PARTY_LICENSES.txt`; both files are also top-level Release assets. CDN
credentials and the Feishu notification webhook are held only
in GitHub Actions secrets; they are never stored in this repository.
Maintainers should follow [docs/RELEASING.md](docs/RELEASING.md).

CLI and Skill installers read authoritative manifests from GitHub Releases;
those independently hosted manifests pin the checksum of each CDN artifact.
Desktop install metadata is reviewed in `desktop-artifacts.json` and compiled
into the CLI. Community entries pin formal GitHub Release assets with SHA-256;
Pro and Local entries pin exact versioned CDN artifacts with size and Base64
SHA-512. The CLI never trusts a mutable CDN update manifest at install time.
Update the CLI to obtain a newer reviewed desktop artifact registry.

## Security

Do not report vulnerabilities in public Issues. Follow
[SECURITY.md](SECURITY.md) and use GitHub private vulnerability reporting.
Chat2DB Community is a single-user, local-first product; do not expose its
local service directly to untrusted networks.

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md)
and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

The CLI and Agent Skill in this repository are licensed under the
[Apache License 2.0](LICENSE). This repository's license is independent from
the license applied to the Chat2DB Community server source.
