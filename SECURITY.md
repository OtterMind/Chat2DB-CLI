# Security Policy

## Reporting A Vulnerability

Do not report security vulnerabilities through public Issues, Discussions, or
pull requests.

Use GitHub private vulnerability reporting for this repository:

<https://github.com/OtterMind/Chat2DB-CLI/security/advisories/new>

Include the affected CLI and Skill version, Chat2DB edition, operating system,
impact, reproduction steps, and any relevant logs. Remove passwords, API keys,
access tokens, datasource credentials, private URLs, and production data.

## Supported Versions

Security fixes are applied to the latest released version. Please verify a
report against the latest release when practical. Maintainers may ask for an
upgrade before investigating behavior that is already fixed.

## Scope

This policy covers:

- the Rust CLI and its local runtime interactions
- the Agent Skill distributed from `skill/`
- official CLI and Skill release artifacts
- installer, updater, package verification, and GitHub Actions workflows
- credential or token exposure caused by this repository

Vulnerabilities in the Chat2DB Community server itself should be reported to
the [Chat2DB repository](https://github.com/OtterMind/Chat2DB/security/advisories/new).
Reports involving Pro or Local may be redirected to the appropriate private
product channel after initial triage.

## Security Boundaries

Chat2DB Community is a single-user, local-first product. The operating-system
user who starts Chat2DB is the trusted operator. Community does not provide a
multi-user authorization boundary. Do not expose its local HTTP, runtime, or
MCP endpoints directly to other users or untrusted networks.

The CLI runtime binds to loopback and uses a per-run bearer token. MCP client
configuration intentionally contains an authentication token. Treat runtime
state files and `chat2db mcp config` output as secrets.

Database commands execute with the permissions of the selected datasource.
Installing a custom JDBC driver or Skill is equivalent to installing executable
code; use only trusted sources.

## Supply Chain

CLI and Skill installers read their authoritative manifests from GitHub
Release assets, then download checksum-pinned artifacts from the CDN. Desktop
installation uses the reviewed `desktop-artifacts.json` registry compiled into
the CLI: Community entries bind formal GitHub Release assets, while Pro and
Local entries bind exact CDN objects by size and checksum. The CLI never accepts
the CDN's mutable desktop update manifests as an authority. Changing desktop
metadata therefore requires repository and CLI Release permissions, not only a
CDN credential. A report is in scope if an untrusted party can bypass artifact
verification, redirect an official download to an untrusted source, obtain
release credentials, or modify a release without the required permissions.

CDN credentials are stored only in the protected `cdn-production` GitHub
Actions environment. They must never be added to source files, local packaging
scripts, workflow logs, or pull request artifacts. Versioned CDN objects are
create-only; all CLI and Skill `latest` pointers are promoted together and are
rolled back together while the GitHub Release remains Draft. Create-only OSS
promotion journals preserve the previous aliases across runner failures. Once a
GitHub Release is public, recovery completes its CDN promotion instead of
rolling the published version back.
