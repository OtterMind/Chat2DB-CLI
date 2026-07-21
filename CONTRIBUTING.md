# Contributing to Chat2DB CLI

Thank you for helping improve the Chat2DB CLI and Agent Skill.

This is a single repository for two release artifacts:

- the Rust CLI in `src/`
- the Agent Skill in `skill/`

Changes should keep both artifacts compatible and useful for Chat2DB Community.

## Before Starting

Search the existing [Issues](https://github.com/OtterMind/Chat2DB-CLI/issues)
before opening a new one. For a substantial behavior or command change, open
or comment on an Issue before implementation so the public contract can be
agreed first.

Do not use a public Issue for a vulnerability. Follow [SECURITY.md](SECURITY.md).

## Development Setup

Install the stable Rust toolchain, then clone and test the repository:

```bash
git clone https://github.com/OtterMind/Chat2DB-CLI.git
cd Chat2DB-CLI
cargo test --all-targets --all-features
```

Build the release binary with:

```bash
cargo build --release
```

Install the current checkout for local development with:

```bash
bash scripts/dev-install.sh
bash scripts/dev-install.sh --client claude-code
bash scripts/dev-install-skill.sh --client codex
```

The first command installs both the CLI and Codex Skill. The second installs
both for Claude Code, and the third refreshes only the Codex Skill. The scripts
reject symlink and Git-managed targets, use rollback-capable replacement, and
accept `--target`/`--skill-target` plus `--force` for explicit custom targets.

## Pull Requests

Keep each pull request focused on one problem. A useful description includes:

- the problem and intended behavior
- the related Issue
- the affected editions and platforms
- tests that were added or run
- any compatibility or security impact

Before opening the pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
bash tests/dev_install_scripts.sh
```

Update tests for behavior changes. Platform-specific installer changes should
include coverage for the affected operating system or a clear explanation of
the remaining manual verification.

## CLI And Skill Stay Together

The CLI and Skill use one repository version and are released together. A
command, option, output contract, edition behavior, or error-code change must
be reflected in all affected places in the same pull request:

- Rust implementation and tests
- `skill/SKILL.md` and relevant Skill references
- `README.md`, `README_FR.md`, `README_JA.md`, `README_ES.md`, and
  `README_KO.md`
- package or release metadata when the version changes

Do not add a Skill workflow that bypasses the CLI and independently reimplements
Chat2DB runtime behavior.

## Community-First Compatibility

Chat2DB Community is the default edition and the public compatibility baseline.
New shared commands should work with Community unless the underlying capability
is intentionally product-specific. Keep edition differences explicit and
covered by tests; do not silently fall back to a different edition.

## Documentation

Keep commands and code blocks identical across the five README files. Translate
the explanation, not the command contract. Verify relative links whenever a
document or directory moves.

## Secrets And Release Changes

Never commit credentials, tokens, private keys, signing material, customer
data, internal hostnames, or production configuration. Do not place secret
values in examples, fixtures, test snapshots, logs, or workflow output.

CDN credentials belong only in protected GitHub Actions secrets. Pull request
workflows must not receive release secrets, and workflows must not use
`pull_request_target` to execute untrusted repository code. Packaging scripts
must produce local artifacts without requiring CDN credentials.

Release automation changes should preserve the protected flow documented in
[`docs/RELEASING.md`](docs/RELEASING.md). Do not add manual provider upload
steps to the repository documentation.

## License

By contributing, you agree that your contributions are licensed under the
repository's [Apache License 2.0](LICENSE).

## Conduct

All contributors must follow the [Code of Conduct](CODE_OF_CONDUCT.md).
