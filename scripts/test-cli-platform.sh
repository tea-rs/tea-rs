#!/usr/bin/env sh
set -eu

platform=$(uname -s 2>/dev/null || echo unknown)

case "$platform" in
  Darwin|Linux|FreeBSD|NetBSD|OpenBSD)
    echo "CLI PTY platform: Unix ($platform)"
    ;;
  CYGWIN*|MINGW*|MSYS*)
    echo "CLI PTY platform: Windows ($platform)"
    echo "SIGINT process-group test: SKIPPED (safe console control injection is unavailable without platform-specific unsafe code)"
    ;;
  *)
    echo "CLI PTY process tests: SKIPPED (unsupported native PTY platform: $platform)"
    echo "Running platform-neutral acceptance tests only"
    cargo test -p tea-cli --test integration 'cross_mode::'
    cargo test -p tea-cli --test integration 'tui::terminal_guard::'
    cargo test -p tea-cli --all-features --test mcp_integration
    exit 0
    ;;
esac

cargo test -p tea-cli --test integration 'pty::' -- --test-threads=1
cargo test -p tea-cli --test integration 'cross_mode::'
cargo test -p tea-cli --test integration 'tui::terminal_guard::'
cargo test -p tea-cli --all-features --test mcp_integration
