#!/usr/bin/env bash
# =============================================================================
# Linux verification entrypoint (Gate B).
#
# The workspace cannot build natively on Windows because yellowstone-grpc-client
# uses Unix sockets. This script is the single source of truth for a Linux run
# and is invoked both by CI and by the Docker helper below:
#
#   docker run --rm --network host -v "$PWD:/work" -w /work \
#     -v arb-target-cache:/target -e CARGO_TARGET_DIR=/target \
#     rust:1.99.1-slim-bookworm bash scripts/verify-linux.sh
#
# It always runs every stage, records one log per stage under target/ci-logs,
# and exits non-zero if any stage failed.
# =============================================================================
set -uo pipefail

LOG_DIR="${CI_LOG_DIR:-target/ci-logs}"
mkdir -p "$LOG_DIR"
status=0

run() {
    local name="$1"
    shift
    echo "==> $name"
    if "$@" >"$LOG_DIR/$name.log" 2>&1; then
        echo "    PASS $name"
    else
        local code=$?
        echo "    FAIL $name (exit=$code) — last 40 lines:"
        tail -n 40 "$LOG_DIR/$name.log" || true
        status=1
    fi
}

# Formatting is pure syntax, so it can also be checked on Windows.
run fmt cargo fmt --all -- --check
run check cargo check --workspace --all-targets --locked
run clippy cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
run test cargo test --workspace --lib --bins --tests --no-fail-fast --locked

# Dependency policy is optional locally; CI installs cargo-deny explicitly.
if command -v cargo-deny >/dev/null 2>&1; then
    run deny cargo deny check
else
    echo "==> deny SKIPPED (cargo-deny not installed)"
fi

echo "==> logs in $LOG_DIR"
exit "$status"
