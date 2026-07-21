#!/usr/bin/env bash
# shellcheck disable=SC2030,SC2031 # Each installer case intentionally isolates HOME and PATH.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
TEST_ROOT="$(mktemp -d)"

cleanup() {
  local status=$?
  trap - EXIT
  if [ "$status" -ne 0 ]; then
    for log in "$TEST_ROOT"/*.log; do
      [ -f "$log" ] || continue
      echo "--- $log" >&2
      sed -n '1,180p' "$log" >&2
    done
  fi
  rm -rf "$TEST_ROOT"
  exit "$status"
}
trap cleanup EXIT

fail() {
  echo "dev install test failed: $*" >&2
  exit 1
}

assert_file() {
  [ -f "$1" ] || fail "missing file: $1"
}

assert_dir() {
  [ -d "$1" ] || fail "missing directory: $1"
}

assert_absent() {
  if [ -e "$1" ] || [ -L "$1" ]; then
    fail "unexpected path: $1"
  fi
}

assert_contains() {
  grep -F "$2" "$1" >/dev/null || fail "$1 does not contain: $2"
}

assert_not_contains() {
  if grep -F "$2" "$1" >/dev/null; then
    fail "$1 unexpectedly contains: $2"
  fi
}

new_fixture() {
  local fixture="$TEST_ROOT/$1"
  mkdir -p "$fixture/scripts" "$fixture/skill"
  cp "$ROOT_DIR/scripts/dev-install.sh" "$fixture/scripts/dev-install.sh"
  cp "$ROOT_DIR/scripts/dev-install-skill.sh" "$fixture/scripts/dev-install-skill.sh"
  cp "$ROOT_DIR/scripts/dev-install-common.sh" "$fixture/scripts/dev-install-common.sh"
  cp "$ROOT_DIR/Cargo.toml" "$fixture/Cargo.toml"
  cp -R "$ROOT_DIR/skill/." "$fixture/skill/"
  printf '%s' "$fixture"
}

make_fake_tools() {
  local directory="$1"
  mkdir -p "$directory"
  cat > "$directory/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [ "$*" != "build --release --locked" ]; then
  echo "unexpected cargo arguments: $*" >&2
  exit 64
fi
if [ -n "${CHAT2DB_TEST_CARGO_SENTINEL:-}" ]; then
  : > "$CHAT2DB_TEST_CARGO_SENTINEL"
fi
: "${CARGO_TARGET_DIR:?CARGO_TARGET_DIR must be set by the development installer}"
if [ "${CHAT2DB_TEST_CARGO_NO_OUTPUT:-0}" = "1" ]; then
  exit 0
fi
if [ -n "${CHAT2DB_TEST_CARGO_DELAY:-}" ]; then
  sleep "$CHAT2DB_TEST_CARGO_DELAY"
fi
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) suffix=.exe ;; *) suffix= ;; esac
mkdir -p "$CARGO_TARGET_DIR/release"
cat > "$CARGO_TARGET_DIR/release/chat2db$suffix" <<SCRIPT
#!/usr/bin/env bash
# build-marker: ${CHAT2DB_TEST_BUILD_MARKER:-initial}
case "\${1:-}" in
  version) echo "chat2db 0.2.0" ;;
  completions) exit 0 ;;
  *) exit 0 ;;
esac
SCRIPT
chmod 0755 "$CARGO_TARGET_DIR/release/chat2db$suffix"
EOF
  chmod 0755 "$directory/cargo"
  cat > "$directory/cygpath" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[ "${1:-}" = "-u" ] || exit 64
: "${CHAT2DB_TEST_CYGPATH_RESULT:?CHAT2DB_TEST_CYGPATH_RESULT must be set}"
printf '%s\n' "$CHAT2DB_TEST_CYGPATH_RESULT"
EOF
  chmod 0755 "$directory/cygpath"
}

run_combined() {
  local fixture="$1" home="$2" bin_dir="$3" skill_target="$4" output="$5"
  shift 5
  (
    export HOME="$home"
    export PATH="${CHAT2DB_TEST_PATH:-$TEST_ROOT/tools:/usr/bin:/bin}"
    cd "$fixture"
    bash scripts/dev-install.sh --bin-dir "$bin_dir" --skill-target "$skill_target" "$@"
  ) > "$output" 2>&1
}

run_cli_only() {
  local fixture="$1" home="$2" bin_dir="$3" output="$4"
  shift 4
  (
    export HOME="$home"
    export PATH="${CHAT2DB_TEST_PATH:-$TEST_ROOT/tools:/usr/bin:/bin}"
    cd "$fixture"
    bash scripts/dev-install.sh --bin-dir "$bin_dir" --no-skill "$@"
  ) > "$output" 2>&1
}

run_skill() {
  local fixture="$1" home="$2" target="$3" output="$4"
  shift 4
  (
    export HOME="$home"
    export PATH="${CHAT2DB_TEST_PATH:-$TEST_ROOT/tools:/usr/bin:/bin}"
    cd "$fixture"
    bash scripts/dev-install-skill.sh --target "$target" "$@"
  ) > "$output" 2>&1
}

wait_for_path() {
  local path="$1" attempts=0
  while [ ! -e "$path" ] && [ ! -L "$path" ]; do
    attempts=$((attempts + 1))
    [ "$attempts" -lt 100 ] || fail "timed out waiting for path: $path"
    sleep 0.02
  done
}

make_restore_failure_tools() {
  local directory="$1"
  mkdir -p "$directory"
  cat > "$directory/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
for argument in "$@"; do
  case "$argument" in
    *.chat2db-restore.*/*.pending|*/recovery/*.pending|*/recovery-cli/*.pending) exit 74 ;;
  esac
done
exec /bin/mv "$@"
EOF
  chmod 0755 "$directory/mv"
}

make_fake_tools "$TEST_ROOT/tools"

# One command installs both artifacts, writes provenance, and remains idempotent.
fixture="$(new_fixture combined)"
home="$TEST_ROOT/home-combined"
bin_dir="$home/bin"
skill_target="$home/skills/chat2db-cli"
cli_provenance="$bin_dir/.chat2db.chat2db-dev-install.provenance"
skill_provenance="$home/skills/.chat2db-cli.chat2db-dev-install.provenance"
mkdir -p "$home"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-1.log"
assert_file "$bin_dir/chat2db"
assert_file "$cli_provenance"
assert_file "$skill_target/SKILL.md"
assert_file "$skill_provenance"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-2.log"
assert_contains "$TEST_ROOT/combined-2.log" "CLI is up to date"
assert_contains "$TEST_ROOT/combined-2.log" "Skill is up to date"
assert_absent "$bin_dir/.chat2db-dev-install.tx"
assert_absent "$home/skills/.chat2db-cli.chat2db-dev-install.tx"

# Tampering invalidates provenance and requires explicit takeover.
printf '\nupdated\n' >> "$fixture/skill/references/installation.md"
printf 'stale\n' > "$skill_target/references/stale.txt"
if CHAT2DB_TEST_BUILD_MARKER=updated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/tamper-refused.log"; then
  fail "tampered Skill target was accepted without --force"
fi
assert_contains "$TEST_ROOT/tamper-refused.log" "pass --force"
CHAT2DB_TEST_BUILD_MARKER=updated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-update.log" --force
assert_contains "$bin_dir/chat2db" "build-marker: updated"
assert_contains "$skill_target/references/installation.md" "updated"
assert_absent "$skill_target/references/stale.txt"

# HOME must fail closed before cargo unless every required path is explicit.
fixture="$(new_fixture home-unset)"
cargo_sentinel="$TEST_ROOT/home-unset-cargo-ran"
if (
  unset HOME
  export PATH="$TEST_ROOT/tools:/usr/bin:/bin" CHAT2DB_TEST_CARGO_SENTINEL="$cargo_sentinel"
  cd "$fixture"
  bash scripts/dev-install.sh
) > "$TEST_ROOT/home-unset.log" 2>&1; then
  fail "missing HOME selected an implicit CLI path"
fi
assert_contains "$TEST_ROOT/home-unset.log" "HOME is not set"
assert_absent "$cargo_sentinel"
if (
  unset HOME
  export PATH="$TEST_ROOT/tools:/usr/bin:/bin" CHAT2DB_TEST_CARGO_SENTINEL="$cargo_sentinel"
  cd "$fixture"
  bash scripts/dev-install.sh --bin-dir "$TEST_ROOT/explicit-bin"
) > "$TEST_ROOT/home-unset-skill.log" 2>&1; then
  fail "missing HOME selected an implicit Skill path"
fi
assert_contains "$TEST_ROOT/home-unset-skill.log" "pass --skill-target"
assert_absent "$cargo_sentinel"
(
  unset HOME
  export PATH="$TEST_ROOT/tools:/usr/bin:/bin"
  cd "$fixture"
  bash scripts/dev-install.sh --bin-dir "$TEST_ROOT/explicit-cli-only" --no-skill
) > "$TEST_ROOT/home-unset-cli-only.log" 2>&1
assert_file "$TEST_ROOT/explicit-cli-only/chat2db"
(
  unset HOME
  export PATH="$TEST_ROOT/tools:/usr/bin:/bin"
  cd "$fixture"
  bash scripts/dev-install.sh \
    --bin-dir "$TEST_ROOT/explicit-pair-bin" \
    --skill-target "$TEST_ROOT/explicit-pair-skill"
) > "$TEST_ROOT/home-unset-explicit-pair.log" 2>&1
assert_file "$TEST_ROOT/explicit-pair-bin/chat2db"
assert_file "$TEST_ROOT/explicit-pair-skill/SKILL.md"

# Raw internal and trailing newlines are rejected before command substitution can trim them.
if CHAT2DB_TEST_CARGO_SENTINEL="$TEST_ROOT/newline-cargo-ran" run_cli_only \
  "$fixture" "$TEST_ROOT/newline-home" "$TEST_ROOT/newline-bin"$'\n' "$TEST_ROOT/newline-bin.log"; then
  fail "trailing newline in CLI path was accepted"
fi
assert_contains "$TEST_ROOT/newline-bin.log" "must not contain a newline"
assert_absent "$TEST_ROOT/newline-cargo-ran"
if run_skill "$fixture" "$TEST_ROOT/newline-home" \
  "$TEST_ROOT/newline"$'\n'"skill" "$TEST_ROOT/newline-skill.log"; then
  fail "internal newline in Skill path was accepted"
fi
assert_contains "$TEST_ROOT/newline-skill.log" "must not contain a newline"

# Windows drive paths are converted by cygpath instead of being prefixed with PWD.
fixture="$(new_fixture windows-drive-paths)"
home="$TEST_ROOT/home-windows-drive-paths"
drive_bin="$home/drive-bin"
drive_skill="$home/drive-skill"
mkdir -p "$home"
CHAT2DB_TEST_CYGPATH_RESULT="$drive_bin" run_cli_only \
  "$fixture" "$home" 'C:/Users/test/bin' "$TEST_ROOT/windows-drive-cli.log"
assert_file "$drive_bin/chat2db"
CHAT2DB_TEST_CYGPATH_RESULT="$drive_skill" run_skill \
  "$fixture" "$home" 'D:\Users\test\skill' "$TEST_ROOT/windows-drive-skill.log"
assert_file "$drive_skill/SKILL.md"

# A stale or tampered checkout target artifact is never executed or installed.
fixture="$(new_fixture stale-build-artifact)"
home="$TEST_ROOT/home-stale-build"
stale_build_sentinel="$TEST_ROOT/stale-build-executed"
mkdir -p "$fixture/target/release"
cat > "$fixture/target/release/chat2db" <<EOF
#!/usr/bin/env bash
: > "$stale_build_sentinel"
echo "chat2db 0.2.0"
EOF
chmod 0755 "$fixture/target/release/chat2db"
run_combined "$fixture" "$home" "$home/bin" "$home/skill" "$TEST_ROOT/stale-build.log"
assert_absent "$stale_build_sentinel"
assert_contains "$home/bin/chat2db" "build-marker: initial"

# The installer-owned target also removes its final executable before Cargo,
# so a successful no-output build cannot reuse a stale binary.
fixture="$(new_fixture stale-dev-build-artifact)"
home="$TEST_ROOT/home-stale-dev-build"
stale_dev_sentinel="$TEST_ROOT/stale-dev-build-executed"
mkdir -p "$fixture/target/dev-install/release"
cat > "$fixture/target/dev-install/release/chat2db" <<EOF
#!/usr/bin/env bash
: > "$stale_dev_sentinel"
echo "chat2db 0.2.0"
EOF
chmod 0755 "$fixture/target/dev-install/release/chat2db"
if CHAT2DB_TEST_CARGO_NO_OUTPUT=1 run_combined \
  "$fixture" "$home" "$home/bin" "$home/skill" "$TEST_ROOT/stale-dev-build.log"; then
  fail "stale installer-owned build artifact was reused"
fi
assert_absent "$stale_dev_sentinel"
assert_contains "$TEST_ROOT/stale-dev-build.log" "cargo built no executable"

# Different install targets share incremental artifacts but receive their own
# verified staged binary after the repository build lock serializes Cargo.
fixture="$(new_fixture concurrent-builds)"
home="$TEST_ROOT/home-concurrent-builds"
mkdir -p "$home"
(
  CHAT2DB_TEST_BUILD_MARKER=first CHAT2DB_TEST_CARGO_DELAY=0.2 \
    run_cli_only "$fixture" "$home" "$home/bin-first" "$TEST_ROOT/build-first.log"
) &
first_build_pid=$!
sleep 0.03
(
  CHAT2DB_TEST_BUILD_MARKER=second \
    run_cli_only "$fixture" "$home" "$home/bin-second" "$TEST_ROOT/build-second.log"
) &
second_build_pid=$!
wait "$first_build_pid"
wait "$second_build_pid"
assert_contains "$home/bin-first/chat2db" "build-marker: first"
assert_contains "$home/bin-second/chat2db" "build-marker: second"

# Existing CLI targets are never executed before provenance proves ownership.
fixture="$(new_fixture unknown-cli)"
home="$TEST_ROOT/home-unknown-cli"
bin_dir="$home/bin"
skill_target="$home/skill"
execution_sentinel="$TEST_ROOT/unknown-cli-executed"
mkdir -p "$bin_dir"
cat > "$bin_dir/chat2db" <<EOF
#!/usr/bin/env bash
: > "$execution_sentinel"
echo "chat2db 0.2.0"
EOF
chmod 0755 "$bin_dir/chat2db"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/unknown-cli.log"; then
  fail "unknown CLI was accepted without --force"
fi
assert_absent "$execution_sentinel"
assert_contains "$TEST_ROOT/unknown-cli.log" "not owned"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/unknown-cli-force.log" --force
assert_absent "$execution_sentinel"
cli_sidecar="$bin_dir/.chat2db.chat2db-dev-install.provenance"
cp "$cli_sidecar" "$TEST_ROOT/valid-cli-provenance"
sed 's#^target=.*#target=/wrong/target#' "$TEST_ROOT/valid-cli-provenance" > "$cli_sidecar"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/wrong-cli-provenance-target.log"; then
  fail "CLI provenance for a different target was accepted"
fi
assert_contains "$TEST_ROOT/wrong-cli-provenance-target.log" "pass --force"
cp "$TEST_ROOT/valid-cli-provenance" "$cli_sidecar"
sed 's/^version=.*/version=9.9.9/' "$TEST_ROOT/valid-cli-provenance" > "$cli_sidecar"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/wrong-cli-provenance-version.log"; then
  fail "CLI provenance for a different version was accepted"
fi
assert_contains "$TEST_ROOT/wrong-cli-provenance-version.log" "pass --force"
cp "$TEST_ROOT/valid-cli-provenance" "$cli_sidecar"
cp "$bin_dir/chat2db" "$TEST_ROOT/valid-cli-binary"
printf '\ntampered\n' >> "$bin_dir/chat2db"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/wrong-cli-provenance-hash.log"; then
  fail "CLI provenance with a stale hash was accepted"
fi
assert_contains "$TEST_ROOT/wrong-cli-provenance-hash.log" "pass --force"
cp "$TEST_ROOT/valid-cli-binary" "$bin_dir/chat2db"
rm "$cli_sidecar"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/missing-cli-provenance.log"; then
  fail "CLI bytes without provenance were accepted"
fi
assert_contains "$TEST_ROOT/missing-cli-provenance.log" "pass --force"
printf 'do not follow\n' > "$TEST_ROOT/provenance-link-target"
ln -s "$TEST_ROOT/provenance-link-target" "$cli_sidecar"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/symlink-cli-provenance.log" --force; then
  fail "symlink CLI provenance was accepted with --force"
fi
assert_contains "$TEST_ROOT/symlink-cli-provenance.log" "refusing symlink"

# A copied Skill bundle without installer provenance is also unowned.
fixture="$(new_fixture unknown-skill)"
home="$TEST_ROOT/home-unknown-skill"
bin_dir="$home/bin"
skill_target="$home/skill"
mkdir -p "$skill_target"
cp -R "$fixture/skill/." "$skill_target/"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/unknown-skill.log"; then
  fail "Skill frontmatter without provenance was accepted"
fi
assert_absent "$bin_dir/chat2db"
assert_contains "$TEST_ROOT/unknown-skill.log" "pass --force"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/unknown-skill-force.log" --force
assert_file "$skill_target/../.skill.chat2db-dev-install.provenance"
skill_sidecar="$skill_target/../.skill.chat2db-dev-install.provenance"
sed -i.bak 's/^version=.*/version=9.9.9/' "$skill_sidecar"
rm -f "$skill_sidecar.bak"
if run_skill "$fixture" "$home" "$skill_target" "$TEST_ROOT/wrong-skill-provenance-version.log"; then
  fail "Skill provenance for a different version was accepted"
fi
assert_contains "$TEST_ROOT/wrong-skill-provenance-version.log" "pass --force"
run_skill "$fixture" "$home" "$skill_target" "$TEST_ROOT/wrong-skill-provenance-version-force.log" --force
assert_contains "$skill_sidecar" "version=0.2.0"

# Client defaults resolve to the documented Codex and Claude Code locations.
fixture="$(new_fixture clients)"
home="$TEST_ROOT/home-clients"
mkdir -p "$home"
(
  export HOME="$home" PATH="$TEST_ROOT/tools:/usr/bin:/bin"
  cd "$fixture"
  bash scripts/dev-install-skill.sh --client codex
  bash scripts/dev-install-skill.sh --client claude-code
) > "$TEST_ROOT/clients.log" 2>&1
assert_file "$home/.codex/skills/chat2db-cli/SKILL.md"
assert_file "$home/.claude/skills/chat2db-cli/SKILL.md"

# Git metadata, target symlinks, internal symlinks, and provenance symlinks stay protected.
fixture="$(new_fixture protected)"
home="$TEST_ROOT/home-protected"
mkdir -p "$home/git-file/references" "$home/git-dir/references" "$home/real-target"
cp "$fixture/skill/SKILL.md" "$home/git-file/SKILL.md"
cp "$fixture/skill/SKILL.md" "$home/git-dir/SKILL.md"
printf 'gitdir: elsewhere\n' > "$home/git-file/.git"
mkdir "$home/git-dir/.git"
if run_skill "$fixture" "$home" "$home/git-file" "$TEST_ROOT/git-file.log" --force; then
  fail "a .git file target was overwritten"
fi
if run_skill "$fixture" "$home" "$home/git-dir" "$TEST_ROOT/git-dir.log" --force; then
  fail "a .git directory target was overwritten"
fi
ln -s "$home/real-target" "$home/symlink-target"
if run_skill "$fixture" "$home" "$home/symlink-target/" "$TEST_ROOT/symlink.log" --force; then
  fail "a symlink target was overwritten"
fi
mkdir -p "$home/internal-link-target"
printf 'foreign\n' > "$home/internal-link-target/README"
ln -s "$home/real-target" "$home/internal-link-target/link"
if run_skill "$fixture" "$home" "$home/internal-link-target" "$TEST_ROOT/internal-symlink.log" --force; then
  fail "a target containing a symlink was overwritten"
fi
mkdir -p "$home/nested-repo-container/project/.git"
printf 'tracked\n' > "$home/nested-repo-container/project/README"
if run_skill "$fixture" "$home" "$home/nested-repo-container" "$TEST_ROOT/nested-git.log" --force; then
  fail "a target containing a nested Git worktree was overwritten"
fi
assert_contains "$home/nested-repo-container/project/README" "tracked"
assert_file "$home/git-file/.git"
assert_file "$home/git-dir/SKILL.md"

# Canonical HOME aliases cannot bypass the destructive-target guard.
if run_skill "$fixture" "$home" "$home/../$(basename "$home")" "$TEST_ROOT/home-alias.log" --force; then
  fail "HOME alias was accepted as a Skill target"
fi
assert_contains "$TEST_ROOT/home-alias.log" "refusing to replace HOME"

# Standalone Skill activation failures restore both the bundle and provenance.
fixture="$(new_fixture skill-rollback)"
home="$TEST_ROOT/home-skill-rollback"
target="$home/skills/chat2db-cli"
mkdir -p "$home"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-rollback-first.log"
old_skill_provenance="$(cat "$home/skills/.chat2db-cli.chat2db-dev-install.provenance")"
printf '\nnew-source\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_FAIL_AT=skill-activated run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-rollback-failed.log"; then
  fail "injected Skill activation failure succeeded"
fi
assert_not_contains "$target/references/installation.md" "new-source"
[ "$(cat "$home/skills/.chat2db-cli.chat2db-dev-install.provenance")" = "$old_skill_provenance" ] || \
  fail "Skill provenance was not restored"
assert_absent "$home/skills/.chat2db-cli.chat2db-dev-install.tx"

# A Skill failure in the combined command restores the old CLI and old Skill.
fixture="$(new_fixture combined-rollback)"
home="$TEST_ROOT/home-combined-rollback"
bin_dir="$home/bin"
skill_target="$home/skill"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-rollback-first.log"
printf '\ncombined-new\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_BUILD_MARKER=combined-new CHAT2DB_TEST_FAIL_AT=skill-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-rollback-failed.log"; then
  fail "combined transaction accepted a failed Skill activation"
fi
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_not_contains "$skill_target/references/installation.md" "combined-new"
assert_absent "$bin_dir/.chat2db-dev-install.tx"
assert_absent "$home/.skill.chat2db-dev-install.tx"

# A failed first combined install leaves neither artifact behind.
fixture="$(new_fixture combined-first-failure)"
home="$TEST_ROOT/home-combined-first-failure"
bin_dir="$home/bin"
skill_target="$home/skill"
if CHAT2DB_TEST_FAIL_AT=skill-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-first-failure.log"; then
  fail "failed first combined install succeeded"
fi
assert_absent "$bin_dir/chat2db"
assert_absent "$bin_dir/.chat2db.chat2db-dev-install.provenance"
assert_absent "$skill_target"
assert_absent "$home/.skill.chat2db-dev-install.provenance"

# Controlled interrupts after either activation boundary restore the whole pair.
fixture="$(new_fixture combined-interrupt)"
home="$TEST_ROOT/home-combined-interrupt"
bin_dir="$home/bin"
skill_target="$home/skill"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-interrupt-first.log"
printf '\ninterrupt-new\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_BUILD_MARKER=interrupt-new CHAT2DB_TEST_INTERRUPT_AT=combined-cli-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/interrupt-cli.log"; then
  fail "CLI-boundary interrupt succeeded"
fi
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_not_contains "$skill_target/references/installation.md" "interrupt-new"
if CHAT2DB_TEST_BUILD_MARKER=interrupt-new CHAT2DB_TEST_INTERRUPT_AT=combined-both-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/interrupt-both.log"; then
  fail "combined-boundary interrupt succeeded"
fi
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_not_contains "$skill_target/references/installation.md" "interrupt-new"

# Hard crashes leave a reentrant journal; the next run restores then installs one matching pair.
fixture="$(new_fixture combined-crash)"
home="$TEST_ROOT/home-combined-crash"
bin_dir="$home/bin"
skill_target="$home/skill"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-crash-first.log"
printf '\ncrash-new\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_BUILD_MARKER=crash-new CHAT2DB_TEST_CRASH_AT=combined-cli-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/crash-cli.log"; then
  fail "CLI-boundary crash succeeded"
fi
assert_dir "$bin_dir/.chat2db-dev-install.tx"
CHAT2DB_TEST_BUILD_MARKER=crash-new run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/crash-cli-recover.log"
assert_contains "$bin_dir/chat2db" "build-marker: crash-new"
assert_contains "$skill_target/references/installation.md" "crash-new"
if CHAT2DB_TEST_BUILD_MARKER=crash-second CHAT2DB_TEST_CRASH_AT=combined-both-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/crash-both.log"; then
  fail "combined-boundary crash succeeded"
fi
assert_dir "$bin_dir/.chat2db-dev-install.tx"
CHAT2DB_TEST_BUILD_MARKER=crash-second run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/crash-both-recover.log"
assert_contains "$bin_dir/chat2db" "build-marker: crash-second"
assert_contains "$skill_target/references/installation.md" "crash-new"
assert_absent "$bin_dir/.chat2db-dev-install.tx"

# Recovery uses the journal's own Skill target and runs before a new Cargo build.
fixture="$(new_fixture recovery-before-build)"
home="$TEST_ROOT/home-recovery-before-build"
bin_dir="$home/bin"
skill_target="$home/skill"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/recovery-before-build-first.log"
printf '\nrecovery-before-build\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_BUILD_MARKER=recovery-before-build CHAT2DB_TEST_CRASH_AT=combined-both-activated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/recovery-before-build-crash.log"; then
  fail "recovery-before-build setup crash succeeded"
fi
if CHAT2DB_TEST_CARGO_NO_OUTPUT=1 run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/recovery-before-build-failed-build.log"; then
  fail "no-output Cargo build succeeded"
fi
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_not_contains "$skill_target/references/installation.md" "recovery-before-build"
assert_contains "$TEST_ROOT/recovery-before-build-failed-build.log" "recovered previous CLI and Skill transaction"
assert_absent "$bin_dir/.chat2db-dev-install.tx"

# Once a completed or rolled-back journal is atomically retired, interruption
# during cleanup cannot leave a malformed active journal behind.
fixture="$(new_fixture journal-retirement)"
home="$TEST_ROOT/home-journal-retirement"
bin_dir="$home/bin"
run_cli_only "$fixture" "$home" "$bin_dir" "$TEST_ROOT/journal-retirement-first.log"
if CHAT2DB_TEST_BUILD_MARKER=committed CHAT2DB_TEST_CRASH_AT=combined-committed-retired run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/committed-retirement-crash.log"; then
  fail "committed journal retirement crash succeeded"
fi
assert_contains "$bin_dir/chat2db" "build-marker: committed"
assert_absent "$bin_dir/.chat2db-dev-install.tx.committed"
CHAT2DB_TEST_BUILD_MARKER=committed run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/committed-retirement-retry.log"
if CHAT2DB_TEST_BUILD_MARKER=rollback CHAT2DB_TEST_FAIL_AT=combined-cli-activated \
   CHAT2DB_TEST_CRASH_AT=combined-rollback-retired run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/rollback-retirement-crash.log"; then
  fail "rolled-back journal retirement crash succeeded"
fi
assert_contains "$bin_dir/chat2db" "build-marker: committed"
assert_absent "$bin_dir/.chat2db-dev-install.tx"
run_cli_only "$fixture" "$home" "$bin_dir" "$TEST_ROOT/rollback-retirement-retry.log"

# Recovery workspace cleanup is also retired before recursive deletion.
fixture="$(new_fixture restore-workspace-retirement)"
home="$TEST_ROOT/home-restore-workspace-retirement"
bin_dir="$home/bin"
skill_target="$home/skill"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/restore-workspace-first.log"
printf '\nrestore-workspace-new\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_BUILD_MARKER=restore-workspace-new CHAT2DB_TEST_FAIL_AT=combined-cli-activated \
   CHAT2DB_TEST_CRASH_AT=restore-workspace-retired run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/restore-workspace-retirement-crash.log"; then
  fail "recovery workspace retirement crash succeeded"
fi
assert_dir "$bin_dir/.chat2db-dev-install.tx"
if CHAT2DB_TEST_CARGO_NO_OUTPUT=1 run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/restore-workspace-retirement-retry.log"; then
  fail "no-output Cargo build after recovery workspace crash succeeded"
fi
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_not_contains "$skill_target/references/installation.md" "restore-workspace-new"
assert_absent "$bin_dir/.chat2db-dev-install.tx"

# Recovery failures preserve CLI backups and become recoverable after the injected fault is removed.
fixture="$(new_fixture cli-recovery-preserve)"
home="$TEST_ROOT/home-cli-recovery-preserve"
bin_dir="$home/bin"
run_cli_only "$fixture" "$home" "$bin_dir" "$TEST_ROOT/cli-recovery-first.log"
if CHAT2DB_TEST_BUILD_MARKER=recovery-new CHAT2DB_TEST_CRASH_AT=combined-cli-activated run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/cli-recovery-crash.log"; then
  fail "CLI recovery crash succeeded"
fi
make_restore_failure_tools "$TEST_ROOT/failing-restore-tools"
CHAT2DB_TEST_PATH="$TEST_ROOT/failing-restore-tools:$TEST_ROOT/tools:/usr/bin:/bin"
export CHAT2DB_TEST_PATH
if run_cli_only "$fixture" "$home" "$bin_dir" "$TEST_ROOT/cli-recovery-blocked.log"; then
  fail "injected CLI recovery failure succeeded"
fi
unset CHAT2DB_TEST_PATH
assert_file "$bin_dir/.chat2db-dev-install.tx/cli-backup"
run_cli_only "$fixture" "$home" "$bin_dir" "$TEST_ROOT/cli-recovery-retry.log"
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_absent "$bin_dir/.chat2db-dev-install.tx"

# CLI activation is recoverable if killed after the old target is atomically quarantined.
if CHAT2DB_TEST_BUILD_MARKER=quarantine-new CHAT2DB_TEST_CRASH_AT=combined-cli-target-quarantined run_cli_only \
  "$fixture" "$home" "$bin_dir" "$TEST_ROOT/cli-quarantine-crash.log"; then
  fail "CLI quarantine-boundary crash succeeded"
fi
assert_absent "$bin_dir/chat2db"
run_cli_only "$fixture" "$home" "$bin_dir" "$TEST_ROOT/cli-quarantine-recover.log"
assert_contains "$bin_dir/chat2db" "build-marker: initial"
assert_absent "$bin_dir/.chat2db-dev-install.tx"

# Recovery failures preserve Skill backups and are independently retryable.
fixture="$(new_fixture skill-recovery-preserve)"
home="$TEST_ROOT/home-skill-recovery-preserve"
target="$home/skill"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-recovery-first.log"
printf '\nrecovery-new\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_CRASH_AT=skill-activated run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-recovery-crash.log"; then
  fail "Skill recovery crash succeeded"
fi
CHAT2DB_TEST_PATH="$TEST_ROOT/failing-restore-tools:$TEST_ROOT/tools:/usr/bin:/bin"
export CHAT2DB_TEST_PATH
if run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-recovery-blocked.log" --recover-only; then
  fail "injected Skill recovery failure succeeded"
fi
unset CHAT2DB_TEST_PATH
assert_dir "$home/.skill.chat2db-dev-install.tx/target-backup"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-recovery-retry.log" --recover-only
assert_not_contains "$target/references/installation.md" "recovery-new"
assert_absent "$home/.skill.chat2db-dev-install.tx"

# Standalone Skill journal cleanup has the same atomic retirement boundary.
fixture="$(new_fixture skill-journal-retirement)"
home="$TEST_ROOT/home-skill-journal-retirement"
target="$home/skill"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-journal-retirement-first.log"
printf '\nskill-committed\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_CRASH_AT=skill-committed-retired run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-committed-retirement-crash.log"; then
  fail "committed Skill journal retirement crash succeeded"
fi
assert_contains "$target/references/installation.md" "skill-committed"
assert_absent "$home/.skill.chat2db-dev-install.tx.committed"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-committed-retirement-retry.log" --recover-only
printf '\nskill-rollback\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_FAIL_AT=skill-activated CHAT2DB_TEST_CRASH_AT=skill-rollback-retired run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-rollback-retirement-crash.log"; then
  fail "rolled-back Skill journal retirement crash succeeded"
fi
assert_not_contains "$target/references/installation.md" "skill-rollback"
assert_absent "$home/.skill.chat2db-dev-install.tx"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-rollback-retirement-retry.log" --recover-only

# Skill activation and recovery both survive SIGKILL at their rename boundaries.
fixture="$(new_fixture skill-quarantine-boundaries)"
home="$TEST_ROOT/home-skill-quarantine"
target="$home/skill"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-quarantine-first.log"
printf '\nquarantine-new\n' >> "$fixture/skill/references/installation.md"
if CHAT2DB_TEST_CRASH_AT=skill-target-quarantined run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-quarantine-crash.log"; then
  fail "Skill quarantine-boundary crash succeeded"
fi
assert_absent "$target"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-quarantine-recover.log" --recover-only
assert_not_contains "$target/references/installation.md" "quarantine-new"
if CHAT2DB_TEST_CRASH_AT=skill-activated run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-restore-setup-crash.log"; then
  fail "Skill restore setup crash succeeded"
fi
if CHAT2DB_TEST_CRASH_AT=restore-component-quarantined run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/skill-restore-boundary-crash.log" --recover-only; then
  fail "Skill restore quarantine-boundary crash succeeded"
fi
assert_absent "$target"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/skill-restore-boundary-recover.log" --recover-only
assert_not_contains "$target/references/installation.md" "quarantine-new"
assert_absent "$home/.skill.chat2db-dev-install.tx"

# Lock metadata is published atomically, live owners serialize, and only complete stale locks are reclaimed.
fixture="$(new_fixture locks)"
home="$TEST_ROOT/home-locks"
target="$home/skill"
lock_file="$home/.skill.chat2db-dev-install.lock"
mkdir -p "$home"
(
  export HOME="$home" PATH="$TEST_ROOT/tools:/usr/bin:/bin"
  export CHAT2DB_TEST_HOLD_LOCK=Skill CHAT2DB_TEST_HOLD_LOCK_SECONDS=0.35
  cd "$fixture"
  bash scripts/dev-install-skill.sh --target "$target"
) > "$TEST_ROOT/lock-first.log" 2>&1 &
first_pid=$!
wait_for_path "$lock_file"
assert_contains "$lock_file" "owner=chat2db-dev-install"
assert_contains "$lock_file" "token="
[ "$(wc -l < "$lock_file" | tr -d '[:space:]')" = "3" ] || fail "lock metadata was not atomic"
lock_owner_pid="$(sed -n 's/^pid=//p' "$lock_file")"
case "$lock_owner_pid" in ''|*[!0-9]*) fail "lock owner PID is invalid" ;; esac
kill -0 "$lock_owner_pid" 2>/dev/null || fail "lock owner PID is not live"
(
  export HOME="$home" PATH="$TEST_ROOT/tools:/usr/bin:/bin"
  cd "$fixture"
  bash scripts/dev-install-skill.sh --target "$target"
) > "$TEST_ROOT/lock-second.log" 2>&1 &
second_pid=$!
wait "$first_pid"
wait "$second_pid"
assert_file "$target/SKILL.md"
assert_absent "$lock_file"
assert_absent "$lock_file.guard"

printf 'owner=chat2db-dev-install\npid=99999999\ntoken=stale\n' > "$lock_file"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/stale-lock.log"
assert_absent "$lock_file"
printf 'owner=chat2db-dev-install\n' > "$lock_file"
if CHAT2DB_DEV_INSTALL_LOCK_ATTEMPTS=3 CHAT2DB_DEV_INSTALL_LOCK_SLEEP=0.01 run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/malformed-lock.log"; then
  fail "malformed lock was reclaimed"
fi
assert_contains "$lock_file" "owner=chat2db-dev-install"
rm "$lock_file"

# A crash before or after guard metadata publication cannot create a permanent deadlock.
fixture="$(new_fixture guard-recovery)"
home="$TEST_ROOT/home-guard-recovery"
target="$home/skill"
guard="$home/.skill.chat2db-dev-install.lock.guard"
mkdir -p "$home"
if CHAT2DB_TEST_CRASH_AT=lock-guard-acquired run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/guard-empty-crash.log"; then
  fail "empty guard crash succeeded"
fi
assert_dir "$guard"
CHAT2DB_DEV_INSTALL_GUARD_GRACE_ATTEMPTS=3 CHAT2DB_DEV_INSTALL_LOCK_SLEEP=0.01 run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/guard-empty-recover.log"
assert_file "$target/SKILL.md"
assert_absent "$guard"
if CHAT2DB_TEST_CRASH_AT=lock-guard-published run_skill \
  "$fixture" "$home" "$target" "$TEST_ROOT/guard-metadata-crash.log"; then
  fail "published guard crash succeeded"
fi
assert_file "$guard/metadata"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/guard-metadata-recover.log"
assert_absent "$guard"

# Cargo and Skill versions are one coupled development contract.
fixture="$(new_fixture version-mismatch)"
home="$TEST_ROOT/home-version"
mkdir -p "$home"
sed -i.bak 's/^version = "0.2.0"$/version = "0.2.1"/' "$fixture/Cargo.toml"
rm -f "$fixture/Cargo.toml.bak"
if run_skill "$fixture" "$home" "$home/skill" "$TEST_ROOT/version-mismatch.log"; then
  fail "Cargo and Skill version mismatch was accepted"
fi
assert_contains "$TEST_ROOT/version-mismatch.log" "Cargo and Skill identities must match"

# Overlapping CLI and Skill targets are rejected before cargo or installation writes.
fixture="$(new_fixture overlap)"
home="$TEST_ROOT/home-overlap"
mkdir -p "$home"
if CHAT2DB_TEST_CARGO_SENTINEL="$TEST_ROOT/overlap-cargo-ran" run_combined \
  "$fixture" "$home" "$home/shared" "$home/shared" "$TEST_ROOT/overlap.log"; then
  fail "overlapping targets were accepted"
fi
assert_contains "$TEST_ROOT/overlap.log" "must not overlap"
assert_absent "$TEST_ROOT/overlap-cargo-ran"
if CHAT2DB_TEST_CARGO_SENTINEL="$TEST_ROOT/source-overlap-cargo-ran" run_combined \
  "$fixture" "$home" "$home/bin" "$fixture/skill" "$TEST_ROOT/source-overlap.log" --force; then
  fail "source Skill directory was accepted as an install target"
fi
assert_contains "$TEST_ROOT/source-overlap.log" "overlaps the source checkout"
assert_absent "$TEST_ROOT/source-overlap-cargo-ran"
mkdir -p "$home/worktree/nested"
printf 'gitdir: external\n' > "$home/worktree/.git"
if CHAT2DB_TEST_CARGO_SENTINEL="$TEST_ROOT/git-ancestor-cargo-ran" run_combined \
  "$fixture" "$home" "$home/bin" "$home/worktree/nested/skill" "$TEST_ROOT/git-ancestor.log" --force; then
  fail "target below an ancestor .git marker was accepted"
fi
assert_contains "$TEST_ROOT/git-ancestor.log" "inside a Git worktree"
assert_absent "$TEST_ROOT/git-ancestor-cargo-ran"
mkdir -p "$home/cli-worktree/bin"
printf 'gitdir: external\n' > "$home/cli-worktree/.git"
printf 'tracked\n' > "$home/cli-worktree/bin/chat2db"
if CHAT2DB_TEST_CARGO_SENTINEL="$TEST_ROOT/cli-git-ancestor-cargo-ran" run_cli_only \
  "$fixture" "$home" "$home/cli-worktree/bin" "$TEST_ROOT/cli-git-ancestor.log" --force; then
  fail "CLI target below an ancestor .git marker was accepted"
fi
assert_contains "$TEST_ROOT/cli-git-ancestor.log" "inside a Git worktree"
assert_contains "$home/cli-worktree/bin/chat2db" "tracked"
assert_absent "$TEST_ROOT/cli-git-ancestor-cargo-ran"
if CHAT2DB_TEST_CARGO_SENTINEL="$TEST_ROOT/cli-source-overlap-cargo-ran" run_cli_only \
  "$fixture" "$home" "$fixture/local-bin" "$TEST_ROOT/cli-source-overlap.log" --force; then
  fail "CLI binary directory inside the source checkout was accepted"
fi
assert_contains "$TEST_ROOT/cli-source-overlap.log" "overlaps the source checkout"
assert_absent "$TEST_ROOT/cli-source-overlap-cargo-ran"

echo "development installer tests passed"
