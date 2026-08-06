#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
test_root=$(mktemp -d "${TMPDIR:-/tmp}/tea-ci-check.XXXXXX")
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

fake_bin="$test_root/bin"
log_file="$test_root/cargo.log"
mkdir -p "$fake_bin"

cat >"$fake_bin/cargo" <<'EOF'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >>"$TEA_TEST_CARGO_LOG"
if [ "${1:-}" = "metadata" ]; then
  printf '%s\n' "${TEA_TEST_METADATA:-{\"packages\":[{}]}}"
fi
if [ -n "${TEA_TEST_FAIL_COMMAND:-}" ] && [ "${1:-}" = "$TEA_TEST_FAIL_COMMAND" ]; then
  exit 17
fi
EOF
chmod +x "$fake_bin/cargo"

cat >"$fake_bin/cargo-deny" <<'EOF'
#!/usr/bin/env sh
exit 0
EOF
chmod +x "$fake_bin/cargo-deny"

export TEA_TEST_CARGO_LOG="$log_file"
test_path="$fake_bin:/usr/bin:/bin"
failures=0

reset_log() {
  : >"$log_file"
}

run_check() {
  PATH="$test_path" "$repository_root/scripts/ci-check.sh" "$@"
}

assert_log() {
  name=$1
  expected=$2
  actual=$(cat "$log_file")
  if [ "$actual" != "$expected" ]; then
    echo "not ok - $name" >&2
    printf 'expected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    failures=$((failures + 1))
  else
    echo "ok - $name"
  fi
}

assert_fails() {
  name=$1
  expected_status=$2
  shift 2
  set +e
  "$@" >"$test_root/stdout" 2>"$test_root/stderr"
  status=$?
  set -e
  if [ "$status" -ne "$expected_status" ]; then
    echo "not ok - $name (expected $expected_status, got $status)" >&2
    sed 's/^/stdout: /' "$test_root/stdout" >&2
    sed 's/^/stderr: /' "$test_root/stderr" >&2
    failures=$((failures + 1))
  else
    echo "ok - $name"
  fi
}

metadata='metadata --no-deps --format-version 1'

reset_log
run_check >/dev/null
assert_log "no arguments select quick workspace check" "$metadata
fmt --all --check
check --workspace --all-targets"

reset_log
run_check quick tea-cli tea-model >/dev/null
assert_log "quick checks only selected packages" "$metadata
fmt --all --check
clippy -p tea-cli -p tea-model --all-targets -- -D warnings
test -p tea-cli -p tea-model --lib --bins --tests"

reset_log
run_check rust >/dev/null
assert_log "rust excludes docs and dependency policy" "$metadata
fmt --all --check
clippy --workspace --all-targets --all-features -- -D warnings
test --workspace --all-features --lib --bins --examples --tests"

reset_log
run_check docs >/dev/null
assert_log "docs owns doctests and API docs" "$metadata
test --workspace --all-features --doc
doc --workspace --all-features --no-deps"

reset_log
run_check all >/dev/null
assert_log "all resolves metadata once and runs every gate" "$metadata
fmt --all --check
clippy --workspace --all-targets --all-features -- -D warnings
test --workspace --all-features --lib --bins --examples --tests
test --workspace --all-features --doc
doc --workspace --all-features --no-deps
deny check"

reset_log
export TEA_TEST_METADATA='{"packages":[]}'
run_check all >/dev/null
unset TEA_TEST_METADATA
assert_log "empty workspace skips compilation and policy" "$metadata"

reset_log
mv "$fake_bin/cargo-deny" "$fake_bin/cargo-deny.disabled"
assert_fails "dependencies requires cargo-deny" 1 run_check dependencies
assert_log "missing cargo-deny fails before dependency check" "$metadata"
mv "$fake_bin/cargo-deny.disabled" "$fake_bin/cargo-deny"

reset_log
assert_fails "unknown mode returns usage status" 2 run_check unknown
assert_log "unknown mode does not invoke Cargo" ""

reset_log
assert_fails "non-quick modes reject extra arguments" 2 run_check test tea-cli
assert_log "invalid arguments do not invoke Cargo" ""

reset_log
assert_fails "quick rejects invalid package names" 2 run_check quick 'tea cli'
assert_log "invalid package does not invoke Cargo" ""

reset_log
export TEA_TEST_FAIL_COMMAND=clippy
assert_fails "stage failures preserve Cargo status" 17 run_check clippy
unset TEA_TEST_FAIL_COMMAND

reset_log
export TEA_TEST_FAIL_COMMAND=metadata
assert_fails "metadata failures are not reported as an empty workspace" 17 run_check quick
unset TEA_TEST_FAIL_COMMAND

if [ "$failures" -ne 0 ]; then
  echo "$failures ci-check tests failed" >&2
  exit 1
fi

echo "ci-check script tests: PASS"
