# ADR-0001: Remove ARB_LIVE env gate and default shadow mode

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Architecture Review

## Context

The codebase currently has a hardcoded `ARB_LIVE` environment variable gate that prevents live execution unless explicitly set to a truthy value (`1`, `true`, `TRUE`, `yes`, `on`). The default config sets `mode = "shadow"` with a comment "live is gated off in this build". This is a policy lock, not a protocol requirement. The gate exists at two levels:
1. Startup check in `main.rs:599-602` — exits with error if `--mode live` but `ARB_LIVE` not truthy
2. Re-check in `dispatch.rs:127` — `if crate::live_execution_enabled()` before any network send

Both are pure policy gates. Removing them does not affect transaction validity.

## Decision

Remove the `ARB_LIVE` gate entirely:
1. Delete the startup check in `main.rs:599-602`
2. Delete the re-check in `dispatch.rs:127` 
3. Change default config `mode = "live"` in `config.example.toml`
4. Simplify `live_execution_enabled()` to always return `true`

## Alternatives Considered

### Alternative 1: Keep gate but default to live
- **Pros**: Preserves explicit opt-in for production
- **Cons**: Still requires env var; doesn't achieve "flow"

### Alternative 2: Make gate a warning instead of error
- **Pros**: Logs intent but allows execution
- **Cons**: Adds noise; still a gate

## Consequences

### Positive
- Single-command execution: `cargo run -- --mode live` works without env setup
- Removes friction for devnet/testnet iteration

### Negative
- Accidental live execution on mainnet becomes possible
- No policy barrier between shadow and live

### Risks
- **Mitigation**: Require `ARB_CONFIG` pointing to explicit config; use separate config files per environment
- **Mitigation**: Keep kill-switch (ADR-0002) as separate safety layer