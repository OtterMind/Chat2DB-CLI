# Releasing Chat2DB CLI And Skill

The Rust CLI and bundled Agent Skill have one version and one release. CDN
credentials and provider commands are owned by GitHub Actions; a maintainer
must not publish release objects from a local workstation.

## Publication Modes

The manually dispatched workflow has two explicit modes:

- `cdn-only` requires a stable `X.Y.Z` version, builds and attests the complete
  bundle, and uploads only create-only versioned CDN objects. It does not create
  a GitHub Release and does not update any `latest` manifest or installer alias.
- `release` requires a pre-existing signed `vX.Y.Z` Tag at the exact protected
  `main` HEAD. It runs the same build and validation, creates a Draft GitHub
  Release, uploads or verifies the immutable CDN objects, promotes all `latest`
  aliases, verifies the Draft assets, and then publishes the GitHub Release
  inside the CDN rollback boundary.

Both modes must be dispatched from the protected `main` branch, and the source
commit must equal the remote `main` HEAD. Pushing a Tag never starts a workflow
or exposes a release secret. A CDN-only object can be reused by a later release
only when its SHA256 is identical. If the source changes after a CDN-only
upload, increment the version; versioned objects are never overwritten.

## One-Time Public Genesis

Deleting the local `.git` directory does not remove remote commits, tags, or
Release assets. The private `OtterMind/Chat2DB-CLI` repository was deleted and
recreated empty during genesis preparation; the superseded standalone Skill
repository was deleted. This repository
recreation is stronger than a force-push because old commit object URLs can
remain reachable until GitHub garbage collection even after visible refs are
deleted.

Before the first genesis push:

1. Confirm the new local repository has no inherited commits, refs, objects,
   nested repositories, or generated artifacts.
2. Run Gitleaks against the complete publish tree.
3. Confirm the empty private remote has no branch, Tag, Release, or Git object.
4. Record the required branch protection, Environment, Secrets, Variables,
   vulnerability reporting, and Dependabot settings so they can be created
   after the genesis branch exists.
5. Keep the replacement repository private while the genesis commit, CI, and
   release configuration are reviewed.
6. Verify the recreated remote has only the intended `main` history and no old
   tags or Releases before changing repository visibility.

## Required Repository Configuration

Protect the `main` branch and create a `v*` Tag ruleset that restricts creation
to release maintainers and blocks Tag updates and deletion. The workflow also
requires a signed annotated Tag whose signature GitHub reports as verified for
full releases. Each release maintainer must register the corresponding GPG or
SSH signing key with GitHub before tagging.

Protect the `cdn-production` Environment. It must require reviewer approval,
prevent self-review, and allow deployments only from the protected `main`
branch for both publication modes.

Create a separate `release-notifications` Environment for the Feishu webhook.
Allow deployments only from the protected `main` branch, require reviewer
approval, and prevent self-review. Notification delivery is non-blocking, but
the webhook is still a credential and must not be exposed to unreviewed refs.

Configure these `cdn-production` Environment secrets:

- `ALIYUN_OSS_ACCESS_KEY_ID`
- `ALIYUN_OSS_ACCESS_KEY_SECRET`

Configure these `cdn-production` Environment variables:

- `ALIYUN_OSS_ENDPOINT`
- `ALIYUN_OSS_BUCKET`

The canonical CDN origin is fixed in the workflow as
`https://cdn.chat2db-ai.com`; it is not a configurable secret or variable.

Configure this `release-notifications` Environment secret:

- `FEISHU_RELEASE_NOTIFY_WEBHOOK`

The Feishu webhook is used for start and final-state notifications in both
publication modes. Notification delivery is retried and reported as a warning;
it never changes a successful publication into a failed one.

The OSS principal should have only the object and CDN refresh permissions used
by the release Action. Enable Bucket Versioning or WORM retention as an
additional provider-side recovery layer.

Secret scanning uses the open-source Gitleaks CLI downloaded at a fixed version
and verified by SHA256. It does not require a Gitleaks organization license or
an additional repository secret.

Enable GitHub Private Vulnerability Reporting, Dependabot alerts, secret
scanning, and push protection after the repository becomes public.

## Supported Binary Baseline

Published CLI binaries have this minimum operating-system contract:

- macOS 11 or later on arm64 or x86_64. Release builds set
  `MACOSX_DEPLOYMENT_TARGET=11.0`.
- Windows 10 or 11, 64-bit only. Windows on ARM64 uses the x86_64 binary
  through the operating system's x64 compatibility layer. 32-bit Windows is
  unsupported.
- GNU/Linux on arm64 or x86_64 with glibc 2.35 or later. Linux artifacts are
  linked on Ubuntu 22.04 against GNU libc. musl-only systems, including Alpine
  Linux, are unsupported.

Do not raise these minimums silently. Any runner, target, linker, or dependency
change that changes the binary baseline must update all five README files and
this release contract in the same pull request.

## Before Tagging

1. Use an exact stable version `X.Y.Z`; prerelease and build metadata are not
   accepted by this workflow.
2. Set the same version in `Cargo.toml`, `Cargo.lock`, and `skill/SKILL.md`.
3. Update `skill/CHANGELOG.md` and all five README files when behavior changes.
4. Review every changed entry in `desktop-artifacts.json` against the formal
   product release. Require exactly one Community, Pro, and Local entry for
   macOS arm64, macOS x86_64, and Windows x86_64. Do not copy Community metadata
   from a gray CDN namespace or accept a mutable CDN manifest as authority.
5. Ensure CI is green on the current protected `main` HEAD.
6. Confirm the new version is strictly greater than every published stable
   GitHub Release and the six CDN `latest` manifests agree.
7. Confirm GitHub marks the release maintainer's signing key as verified; a
   signature-looking Tag message is not accepted as proof.

When `Cargo.lock` changes, regenerate and verify the committed third-party
notices with the pinned generator:

```bash
cargo install cargo-about --version 0.9.1 --locked --features cli
cargo about generate --frozen --fail --locked --all-features --output-file THIRD_PARTY_LICENSES.txt about.hbs
python3 .github/scripts/check-third-party-licenses.py
```

Run the `Release` workflow from the protected `main` branch with mode
`cdn-only` when versioned artifacts need validation without a public Release.
Create and push the signed Tag only when the version is ready for public
release, then manually run the same workflow from `main` with mode `release`
and the matching version:

```bash
git tag -s vX.Y.Z -m "Chat2DB CLI vX.Y.Z"
git push origin vX.Y.Z
```

## Automated Release Contract

The `Release` workflow uses one global `release-production` concurrency group.
Both manually selected modes perform these stages first:

1. Reuse the complete CI quality workflow: MSRV, format, Clippy, Rust tests on
   Linux and Windows, package reproducibility, documentation, workflow pins,
   `cargo-deny`, and full-history Gitleaks.
2. Require the workflow source to equal `origin/main`, fail closed if the
   published-Release inventory cannot be read, reject version downgrade, and,
   in release mode, require GitHub's Tag API to report a cryptographically
   verified signed annotated Tag resolving to that exact commit.
3. Build five CLI targets, package the Skill, generate third-party license
   notices, and generate a CycloneDX SBOM.
4. Assemble one allowlisted release bundle containing archives, versioned and
   stable manifest aliases, versioned and stable installers, SBOM, consolidated
   `LICENSE`, `THIRD_PARTY_LICENSES.txt`, `SHA256SUMS`, and GitHub build
   provenance. Every CLI archive and the Skill archive also contain both legal
   files.

CDN-only mode then uploads all versioned CDN objects with OSS server-side
overwrite prevention and verifies every object from OSS and the public CDN. It
stops without creating a Draft Release or changing mutable aliases.

Release mode continues with these stages:

5. Create a Draft GitHub Release containing exactly that bundle.
6. Upload or verify every immutable CDN object.
7. Snapshot all nine mutable objects into create-only OSS objects under
   `release-journal/chat2db-cli/<version>/`, persist the journal before the first
   alias write, validate the six existing `latest` versions, promote CLI and
   Skill manifests plus installers together, refresh the CDN, and verify every
   promoted object.
8. While that promotion snapshot and rollback trap are still active, download
   the Draft assets again, verify their names and `SHA256SUMS`, and publish the
   GitHub Release as the latest stable release. Only a successful publication
   commits the CDN promotion.

GitHub Release manifests are the independent trust root used by the CLI and
Skill installers, `chat2db update`, and `chat2db skill update`. CDN objects
contain the binary and Skill payloads referenced by those manifests.

Desktop installation uses `desktop-artifacts.json`, which is compiled into the
reviewed CLI binary. Community entries bind exact formal GitHub Release assets
and SHA-256. Pro and Local entries bind exact versioned CDN DMG/EXE objects,
nonzero size, and Base64 SHA-512. Runtime installation does not fetch or trust
the product CDN's mutable Electron manifests. A newer desktop release requires
a reviewed registry change and a new CLI Release.

## Failure And Rollback

- In release mode, a packaging, validation, immutable upload, or readback
  failure leaves the GitHub Release in Draft and does not change `latest`.
- A CDN-only failure may leave digest-identical immutable objects from completed
  steps. They are not discoverable through `latest` and are safe to reuse only
  from the same source and version.
- A promotion, refresh, public readback, or Draft asset verification failure
  restores every previous mutable object, removes objects that did not exist
  before the attempt, then refreshes and verifies the rollback while no GitHub
  publication request has been sent.
- GitHub publication is the commit point. If a publication response is lost or
  the job is interrupted after the Release becomes public, cleanup confirms the
  public state and never rolls aliases back. After a publication request has
  been sent, an uncertain or still-Draft response also defers rollback rather
  than risking a later public Release with old aliases. A rerun resolves the
  final state: Draft restores the journal before retrying, while public verifies
  the Release assets and completes every CDN alias from the canonical bundle.
- Promotion snapshots and backups are durable, create-only OSS objects. A rerun
  with a Draft Release restores the recorded pre-promotion state before trying
  again, so a runner crash cannot make a split alias set the new baseline. If
  GitHub state cannot be determined, the job fails without guessing and leaves
  the journal for a later reconciliation run.
- Use **Re-run failed jobs** on the original workflow run after a runner failure;
  its canonical bundle is retained for 30 days. Do not use **Re-run all jobs**
  after the Release is public because the draft-creation guard intentionally
  rejects an existing public Release. Do not start a higher version while an
  earlier promotion journal still needs reconciliation.
- Versioned objects are create-only. If an object already exists, publication
  proceeds only when its SHA256 is identical.
- A platform expansion may start with missing `latest` manifests only when all
  present manifests agree on one strictly older version. Split, equal, or newer
  versions fail closed; inspect and restore the last verified bundle instead of
  guessing a version.
- Never reuse a failed version. Fix the source and publish a higher patch
  version after credentials and provider state are known to be trustworthy.

## Post-Release Verification

Verify the release assets and provenance with GitHub CLI, then install into an
isolated directory from the public entry point. Verify one CLI update and one
Skill install. Run a Community headless runtime, datasource, metadata, and SQL
smoke test against a compatible Chat2DB Community 5.3.0 or later build.

## npm Distribution

npm publication is not implemented yet. The intended package is a small
dependency-free launcher, preferably `@chat2db/cli`, that resolves the exact
GitHub Release manifest, downloads the checksum-bound CDN payload, verifies it,
and then runs the native CLI. GitHub Releases remain the trust root.

Add npm only after the first production GitHub and CDN transaction succeeds.
Use npm Trusted Publishing with OIDC and provenance in a separate downstream
workflow triggered by a published GitHub Release. CDN-only runs must never
publish an npm version or move an npm dist-tag, and npm failure must not roll
back an already valid GitHub/CDN release.
