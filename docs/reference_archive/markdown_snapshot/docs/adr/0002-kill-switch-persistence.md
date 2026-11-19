# ADR-0002: Modify KillSwitch to not auto-block on marker file

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Operations

## Context

`KillSwitch::with_file()` in `crates/arb-risk/src/lib.rs:131-141` checks for a marker file (`data/kill_switch` by default) at startup. If the file exists, the switch starts in BLOCKED state (fail-safe). The file is created on `activate()` and removed on `deactivate()`. This provides durable pause across restarts.

Current behavior: `is_blocked()` re-checks file existence on every call, so a manually created file blocks immediately.

## Decision

Modify `KillSwitch::with_file()` to start unblocked regardless of marker file. Add env var `KILL_SWITCH_DISABLED=1` to bypass file check entirely in `is_blocked()`.

## Alternatives Considered

### Alternative 1: Remove file persistence entirely
- **Pros**: Simple
- **Cons**: Loses durable pause for operations team

### Alternative 2: Keep file check but add grace period
- **Pros**: Allows restart recovery
- **Cons**: Complexity; still blocks on marker

## Consequences

### Positive
- No accidental block on restart if marker file persists
- Env var allows complete disable for testing

### Negative
- Durable pause loses its "fail-safe on restart" property
- Operations must use `/pause` endpoint instead of file touch

### Risks
- **Mitigation**: Document that `/pause` + `/resume` are now the only pause mechanism
- **Mitigation**: Keep file write on `activate()` for audit trail