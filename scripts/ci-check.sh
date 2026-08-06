#!/usr/bin/env sh
set -eu
set -f

metadata_loaded=0
metadata_checked=0
workspace_has_packages=0
package_arguments=

usage() {
  cat >&2 <<EOF
usage: $0 [quick [package ...]|metadata|format|clippy|test|docs|dependencies|rust|all]

  quick          fast local feedback (default); name packages for Clippy + tests
  metadata       workspace metadata
  format         workspace formatting
  clippy         exhaustive workspace/all-feature Clippy
  test           ordinary workspace/all-feature tests (excludes doctests)
  docs           doctests and warning-denied API documentation
  dependencies   cargo-deny dependency policy
  rust           format + exhaustive Clippy + ordinary tests
  all            release-only aggregate of every gate

Examples:
  $0 quick tea-cli
  $0 quick tea-model tea-provider-openai
  $0 docs
EOF
}

require_no_arguments() {
  if [ "$#" -ne 0 ]; then
    usage
    exit 2
  fi
}

prepare_package_arguments() {
  package_arguments=
  for package in "$@"; do
    case "$package" in
      '' | *[!A-Za-z0-9_-]*)
        echo "invalid Cargo package name: $package" >&2
        usage
        exit 2
        ;;
    esac
    package_arguments="$package_arguments -p $package"
  done
}

load_metadata() {
  if [ "$metadata_loaded" -eq 1 ]; then
    return
  fi

  workspace_metadata_cache=$(cargo metadata --no-deps --format-version 1) || return $?
  metadata_loaded=1
  case "$workspace_metadata_cache" in
    *'"packages":[]'*) workspace_has_packages=0 ;;
    *) workspace_has_packages=1 ;;
  esac
}

check_metadata() {
  if [ "$metadata_checked" -eq 1 ]; then
    return
  fi

  run_stage "workspace metadata" run_metadata_policy || return $?
  metadata_checked=1
}

run_metadata_policy() {
  load_metadata || return $?
  echo "cargo metadata: PASS"
}

run_stage() {
  stage_name=$1
  shift
  started_at=$(date +%s)
  echo "==> $stage_name"
  if "$@"; then
    finished_at=$(date +%s)
    echo "<== $stage_name: PASS ($((finished_at - started_at))s)"
  else
    status=$?
    finished_at=$(date +%s)
    echo "<== $stage_name: FAIL ($((finished_at - started_at))s)" >&2
    return "$status"
  fi
}

configure_build_parallelism() {
  if [ "$(uname -s)" = "Darwin" ]; then
    CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
    export CARGO_BUILD_JOBS
  fi
}

configure_heavy_build() {
  configure_build_parallelism
  CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0}
  export CARGO_INCREMENTAL
}

run_format() {
  cargo fmt --all --check
}

run_workspace_check() {
  cargo check --workspace --all-targets
}

run_selected_clippy() {
  # Package names are validated before this intentionally split expansion.
  # shellcheck disable=SC2086
  cargo clippy $package_arguments --all-targets -- -D warnings
}

run_selected_tests() {
  # Package names are validated before this intentionally split expansion.
  # shellcheck disable=SC2086
  cargo test $package_arguments --lib --bins --tests
}

run_workspace_clippy() {
  cargo clippy --workspace --all-targets --all-features -- -D warnings
}

run_workspace_tests() {
  cargo test --workspace --all-features --lib --bins --examples --tests
}

run_doctests() {
  cargo test --workspace --all-features --doc
}

run_api_docs() {
  RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-D warnings" \
    cargo doc --workspace --all-features --no-deps
}

require_cargo_deny() {
  if ! command -v cargo-deny >/dev/null 2>&1; then
    echo "cargo-deny is required once workspace packages exist" >&2
    return 1
  fi
}

run_dependency_policy() {
  cargo deny check
}

prepare_workspace() {
  skip_message=$1
  check_metadata || exit $?
  if [ "$workspace_has_packages" -eq 0 ]; then
    echo "$skip_message: SKIPPED (workspace has no packages yet)"
    return 1
  fi
}

check_quick() {
  prepare_package_arguments "$@"
  if ! prepare_workspace "Quick checks"; then
    return
  fi

  configure_build_parallelism
  run_stage "format" run_format
  if [ -z "$package_arguments" ]; then
    run_stage "workspace check" run_workspace_check
  else
    run_stage "selected-package Clippy" run_selected_clippy
    run_stage "selected-package tests" run_selected_tests
  fi
}

check_format() {
  if ! prepare_workspace "Formatting"; then
    return
  fi
  run_stage "format" run_format
}

check_clippy() {
  if ! prepare_workspace "Clippy"; then
    return
  fi
  configure_heavy_build
  run_stage "workspace Clippy" run_workspace_clippy
}

check_tests() {
  if ! prepare_workspace "Rust tests"; then
    return
  fi
  configure_heavy_build
  run_stage "workspace tests" run_workspace_tests
}

check_docs() {
  if ! prepare_workspace "Rust documentation"; then
    return
  fi
  configure_heavy_build
  run_stage "doctests" run_doctests
  run_stage "API documentation" run_api_docs
}

check_dependencies() {
  if ! prepare_workspace "Dependency policy"; then
    return
  fi
  require_cargo_deny
  run_stage "dependency policy" run_dependency_policy
}

check_rust() {
  if ! prepare_workspace "Rust quality gates"; then
    return
  fi
  configure_heavy_build
  run_stage "format" run_format
  run_stage "workspace Clippy" run_workspace_clippy
  run_stage "workspace tests" run_workspace_tests
}

check_all() {
  if ! prepare_workspace "Release quality gates"; then
    return
  fi
  require_cargo_deny
  configure_heavy_build
  run_stage "format" run_format
  run_stage "workspace Clippy" run_workspace_clippy
  run_stage "workspace tests" run_workspace_tests
  run_stage "doctests" run_doctests
  run_stage "API documentation" run_api_docs
  run_stage "dependency policy" run_dependency_policy
}

mode=${1:-quick}
if [ "$#" -gt 0 ]; then
  shift
fi

case "$mode" in
  quick)
    check_quick "$@"
    ;;
  metadata)
    require_no_arguments "$@"
    check_metadata
    ;;
  format)
    require_no_arguments "$@"
    check_format
    ;;
  clippy)
    require_no_arguments "$@"
    check_clippy
    ;;
  test)
    require_no_arguments "$@"
    check_tests
    ;;
  docs)
    require_no_arguments "$@"
    check_docs
    ;;
  dependencies)
    require_no_arguments "$@"
    check_dependencies
    ;;
  rust)
    require_no_arguments "$@"
    check_rust
    ;;
  all)
    require_no_arguments "$@"
    check_all
    ;;
  *)
    usage
    exit 2
    ;;
esac
