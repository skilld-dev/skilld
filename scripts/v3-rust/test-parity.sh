#!/usr/bin/env bash

set -euo pipefail

repository_root=$(git rev-parse --show-toplevel)
fixture="$repository_root/tests/fixtures/v3-rust/local-skill"
runner="$repository_root/packages/cli-wasm32-wasi/run-component.mjs"
native="$repository_root/target/release/skilld"

bash "$repository_root/scripts/v3-rust/build-component.sh"
cargo build --manifest-path "$repository_root/Cargo.toml" --release -p skilld-native

native_project=$(mktemp -d)
wasi_project=$(mktemp -d)
trap 'rm -rf "$native_project" "$wasi_project"' EXIT
cp -R "$fixture" "$native_project/local-skill"
cp -R "$fixture" "$wasi_project/local-skill"

(
  cd "$native_project"
  "$native" install ./local-skill
) > "$native_project/install.out"

set +e
(
  cd "$wasi_project"
  node "$runner" install ./local-skill
) > "$wasi_project/install.out" 2> "$wasi_project/install.err"
wasi_install_status=$?
set -e

test "$wasi_install_status" -eq 2
rg -q 'cannot open the transaction journal: Not supported' "$wasi_project/install.err"
cp -R "$fixture" "$wasi_project/.skills/local-skill"

(
  cd "$native_project"
  "$native" list
) > "$native_project/list.out"
(
  cd "$wasi_project"
  node "$runner" list
) > "$wasi_project/list.out"

diff -u "$native_project/list.out" "$wasi_project/list.out"
diff -u "$native_project/.skills/local-skill/SKILL.md" "$wasi_project/.skills/local-skill/SKILL.md"

"$native" --version > "$native_project/version.out"
node "$runner" --version > "$wasi_project/version.out"
diff -u "$native_project/version.out" "$wasi_project/version.out"

printf 'stdio-proof\n' | SKILLD_PROBE_STDIO=1 node "$runner" \
  > "$wasi_project/stdio.out" 2> "$wasi_project/stdio.err"
test "$(cat "$wasi_project/stdio.out")" = 'stdin:stdio-proof'
test "$(cat "$wasi_project/stdio.err")" = 'stderr:probe'

set +e
node "$runner" auth status > "$wasi_project/auth.out" 2> "$wasi_project/auth.err"
auth_status=$?
node "$runner" auth login > "$wasi_project/login.out" 2> "$wasi_project/login.err"
login_status=$?
set -e

test "$auth_status" -eq 2
test "$login_status" -eq 2
rg -q '^UNSUPPORTED_HOST: credential capability' "$wasi_project/auth.err"
rg -q '^UNSUPPORTED_HOST: process capability' "$wasi_project/login.err"

SKILLD_WASI_MEMORY_CREDENTIAL=1 node "$runner" auth status \
  > "$wasi_project/auth-memory.out"
test "$(cat "$wasi_project/auth-memory.out")" = 'Authenticated.'

set +e
SKILLD_PROBE_GIT=1 node "$runner" \
  > "$wasi_project/git-missing.out" 2> "$wasi_project/git-missing.err"
git_missing_status=$?
set -e
test "$git_missing_status" -eq 2
rg -q '^UNSUPPORTED_HOST: Git process capability failed' "$wasi_project/git-missing.err"

SKILLD_PROBE_GIT=1 SKILLD_WASI_ENABLE_GIT=1 node "$runner" \
  > "$wasi_project/git.out"
rg -q '^git version ' "$wasi_project/git.out"

echo 'Native and WASIp2 read behavior matches.'
echo 'WASIp2 write parity is blocked by preview2-shim file creation.'
