# ADR-0005: Keep at least one simulation (LiteSVM) before send

**Date**: 2026-11-09
**Status**: accepted
**Deciders**: Senior Reverse Engineering Lead, Protocol Engineer

## Context

Two simulation layers exist:
1. **LiteSVM in-memory** (`dispatch.rs:201-216`) — sub-millisecond, feature-gated, returns `success=false` with "litesvm feature disabled" when disabled
2. **RPC pre-flight** (`dispatch.rs:218-342`) — full RPC simulation, configurable via `ENABLE_PREFLIGHT_SIMULATION` / `FAST_PATH_SKIP_RPC_SIM`

Currently: Live mode requires at least one simulation (RPC pre-flight enabled by default). Shadow/canary optional.

## Decision

**Keep LiteSVM as mandatory minimum simulation in Live mode.** If `litesvm` feature disabled, RPC pre-flight becomes mandatory. Never send without at least one simulation pass.

## Alternatives Considered

### Alternative 1: Remove all simulation in Live
- **Cons**: Zero pre-flight validation; transactions fail on-chain wasting fees

### Alternative 2: Make both optional
- **Cons**: Same as above

## Consequences

### Positive
- Catches instruction errors, account missing, compute unit overflow before send
- LiteSVM adds ~1ms; negligible latency impact

### Negative
- Requires `litesvm` feature or RPC access
- Feature flag complexity

### Risks
- **Mitigation**: Keep `litesvm` feature enabled in release builds
- **Mitigation**: Fallback to RPC sim if LiteSVM fails for non-feature reasons