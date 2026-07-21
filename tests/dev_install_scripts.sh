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
      sed -n '1,160p' "$log" >&2
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

assert_absent() {
  [ ! -e "$1" ] && [ ! -L "$1" ] || fail "unexpected path: $1"
}

assert_contains() {
  grep -F "$2" "$1" >/dev/null || fail "$1 does not contain: $2"
}

new_fixture() {
  local fixture="$TEST_ROOT/$1"
  mkdir -p "$fixture/scripts" "$fixture/skill"
  cp "$ROOT_DIR/scripts/dev-install.sh" "$fixture/scripts/dev-install.sh"
  cp "$ROOT_DIR/scripts/dev-install-skill.sh" "$fixture/scripts/dev-install-skill.sh"
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
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) suffix=.exe ;; *) suffix= ;; esac
mkdir -p target/release
cat > "target/release/chat2db$suffix" <<SCRIPT
#!/usr/bin/env bash
# build-marker: ${CHAT2DB_TEST_BUILD_MARKER:-initial}
case "\${1:-}" in
  version) echo "chat2db 0.2.0" ;;
  completions) exit 0 ;;
  *) exit 0 ;;
esac
SCRIPT
chmod 0755 "target/release/chat2db$suffix"
EOF
  chmod 0755 "$directory/cargo"
}

run_combined() {
  local fixture="$1" home="$2" bin_dir="$3" skill_target="$4" output="$5"
  shift 5
  (
    export HOME="$home"
    export PATH="$TEST_ROOT/tools:/usr/bin:/bin"
    cd "$fixture"
    bash scripts/dev-install.sh --bin-dir "$bin_dir" --skill-target "$skill_target" "$@"
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

make_fake_tools "$TEST_ROOT/tools"

# One command installs both artifacts and remains idempotent.
fixture="$(new_fixture combined)"
home="$TEST_ROOT/home-combined"
bin_dir="$home/bin"
skill_target="$home/skills/chat2db-cli"
mkdir -p "$home"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-1.log"
assert_file "$bin_dir/chat2db"
assert_file "$skill_target/SKILL.md"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-2.log"
assert_contains "$TEST_ROOT/combined-2.log" "CLI is up to date"
assert_contains "$TEST_ROOT/combined-2.log" "Skill is up to date"
assert_absent "$bin_dir/.chat2db-dev-install.tx"
assert_absent "$skill_target/../.chat2db-cli.chat2db-dev-install.tx"

# A changed local binary and Skill replace the installed copies and remove stale files.
printf '\nupdated\n' >> "$fixture/skill/references/installation.md"
printf 'stale\n' > "$skill_target/references/stale.txt"
CHAT2DB_TEST_BUILD_MARKER=updated run_combined \
  "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/combined-update.log"
assert_contains "$bin_dir/chat2db" "build-marker: updated"
assert_contains "$skill_target/references/installation.md" "updated"
assert_absent "$skill_target/references/stale.txt"

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

# Foreign CLI and Skill targets require --force.
fixture="$(new_fixture foreign)"
home="$TEST_ROOT/home-foreign"
bin_dir="$home/bin"
skill_target="$home/skills/chat2db-cli"
mkdir -p "$bin_dir" "$skill_target"
printf '#!/usr/bin/env bash\necho foreign\n' > "$bin_dir/chat2db"
chmod 0755 "$bin_dir/chat2db"
printf 'foreign\n' > "$skill_target/README"
if run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/foreign-refused.log"; then
  fail "foreign targets were replaced without --force"
fi
assert_contains "$skill_target/README" "foreign"
run_combined "$fixture" "$home" "$bin_dir" "$skill_target" "$TEST_ROOT/foreign-force.log" --force
assert_file "$skill_target/SKILL.md"
assert_absent "$skill_target/README"

# Git metadata and symlinks remain protected even with --force.
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
assert_file "$home/git-file/.git"
assert_file "$home/git-dir/SKILL.md"

# Canonical HOME aliases cannot bypass the destructive-target guard.
if run_skill "$fixture" "$home" "$home/../$(basename "$home")" "$TEST_ROOT/home-alias.log" --force; then
  fail "HOME alias was accepted as a Skill target"
fi
assert_contains "$TEST_ROOT/home-alias.log" "refusing to replace HOME"

# Activation errors roll back the previous Skill.
fixture="$(new_fixture rollback)"
home="$TEST_ROOT/home-rollback"
target="$home/skills/chat2db-cli"
mkdir -p "$home"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/rollback-first.log"
printf '\nnew-source\n' >> "$fixture/skill/references/installation.md"
mkdir -p "$TEST_ROOT/failing-tools"
cat > "$TEST_ROOT/failing-tools/mv" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in */pending) exit 73 ;; esac
exec /bin/mv "$@"
EOF
chmod 0755 "$TEST_ROOT/failing-tools/mv"
CHAT2DB_TEST_PATH="$TEST_ROOT/failing-tools:$TEST_ROOT/tools:/usr/bin:/bin"
export CHAT2DB_TEST_PATH
if run_skill "$fixture" "$home" "$target" "$TEST_ROOT/rollback-failed.log"; then
  fail "injected Skill activation failure succeeded"
fi
unset CHAT2DB_TEST_PATH
if grep -F "new-source" "$target/references/installation.md" >/dev/null; then
  fail "failed Skill activation did not restore the previous bundle"
fi
assert_absent "$home/skills/.chat2db-cli.chat2db-dev-install.tx"

# A deterministic interrupted transaction restores the backup on the next run.
fixture="$(new_fixture recovery)"
home="$TEST_ROOT/home-recovery"
target="$home/skills/chat2db-cli"
mkdir -p "$home"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/recovery-first.log"
parent="$(cd -P "$(dirname "$target")" && pwd -P)"
canonical_target="$parent/$(basename "$target")"
tx="$parent/.chat2db-cli.chat2db-dev-install.tx"
mkdir "$tx"
printf '%s\n' "$canonical_target" > "$tx/target"
mv "$canonical_target" "$tx/backup"
mkdir "$tx/pending"
printf 'partial\n' > "$tx/pending/partial"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/recovery-second.log"
assert_contains "$TEST_ROOT/recovery-second.log" "recovered previous Skill"
assert_file "$target/SKILL.md"
assert_absent "$tx"

# A process killed before writing lock ownership leaves only an empty reserved
# directory; a later install reclaims it after the grace period.
fixture="$(new_fixture stale-lock)"
home="$TEST_ROOT/home-stale-lock"
target="$home/skills/chat2db-cli"
mkdir -p "$home/skills/.chat2db-cli.chat2db-dev-install.lock"
run_skill "$fixture" "$home" "$target" "$TEST_ROOT/stale-lock.log"
assert_file "$target/SKILL.md"
assert_absent "$home/skills/.chat2db-cli.chat2db-dev-install.lock"

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

# Two installs of the same target serialize and leave one complete bundle.
fixture="$(new_fixture concurrent)"
home="$TEST_ROOT/home-concurrent"
target="$home/skills/chat2db-cli"
mkdir -p "$home" "$TEST_ROOT/slow-tools"
cat > "$TEST_ROOT/slow-tools/cp" <<'EOF'
#!/usr/bin/env bash
sleep 0.2
exec /bin/cp "$@"
EOF
chmod 0755 "$TEST_ROOT/slow-tools/cp"
(
  export HOME="$home" PATH="$TEST_ROOT/slow-tools:$TEST_ROOT/tools:/usr/bin:/bin"
  cd "$fixture"
  bash scripts/dev-install-skill.sh --target "$target"
) > "$TEST_ROOT/concurrent-1.log" 2>&1 &
first_pid=$!
(
  export HOME="$home" PATH="$TEST_ROOT/slow-tools:$TEST_ROOT/tools:/usr/bin:/bin"
  cd "$fixture"
  bash scripts/dev-install-skill.sh --target "$target"
) > "$TEST_ROOT/concurrent-2.log" 2>&1 &
second_pid=$!
wait "$first_pid"
wait "$second_pid"
assert_file "$target/SKILL.md"
assert_absent "$home/skills/.chat2db-cli.chat2db-dev-install.tx"
assert_absent "$home/skills/.chat2db-cli.chat2db-dev-install.lock"

echo "development installer tests passed"
