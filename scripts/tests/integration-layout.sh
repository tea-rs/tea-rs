#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname "$0")/../.." && pwd)
cd "$repository_root"

test_root=$(mktemp -d "${TMPDIR:-/tmp}/tea-integration-layout.XXXXXX")
trap 'rm -rf "$test_root"' EXIT HUP INT TERM

failed=0
suite_count=0

for suite in crates/*/tests/integration.rs; do
  crate_dir=${suite%/tests/integration.rs}
  manifest="$crate_dir/Cargo.toml"
  crate_name=${crate_dir##*/}
  expected="$test_root/$crate_name.expected"
  actual="$test_root/$crate_name.actual"
  suite_count=$((suite_count + 1))

  find "$crate_dir/tests" -maxdepth 1 -type f -name '*.rs' \
    ! -name integration.rs \
    -exec basename {} .rs \; | sort >"$expected"
  if [ "$crate_name" = "tea-cli" ]; then
    sed '/^mcp_integration$/d' "$expected" >"$expected.filtered"
    mv "$expected.filtered" "$expected"
    if ! grep -q 'path = "tests/mcp_integration.rs"' "$manifest"; then
      echo "$manifest must retain the feature-gated MCP integration target" >&2
      failed=1
    fi
  fi
  sed -n 's/^#[[]path = "\([^"]*\)\.rs"[]]$/\1/p' "$suite" \
    | sed '/\//d' | sort >"$actual"

  if ! diff -u "$expected" "$actual"; then
    echo "$crate_name integration harness does not cover every test source" >&2
    failed=1
  fi
  if ! grep -Eq '^autotests[[:space:]]*=[[:space:]]*false$' "$manifest"; then
    echo "$manifest must disable automatic per-file test targets" >&2
    failed=1
  fi
  if ! grep -q 'path = "tests/integration.rs"' "$manifest"; then
    echo "$manifest must declare the aggregate integration target" >&2
    failed=1
  fi
done

if [ "$suite_count" -ne 18 ]; then
  echo "expected 18 aggregate integration suites, found $suite_count" >&2
  failed=1
fi

if [ "$failed" -ne 0 ]; then
  exit 1
fi

echo "aggregate integration layout: PASS ($suite_count suites)"
