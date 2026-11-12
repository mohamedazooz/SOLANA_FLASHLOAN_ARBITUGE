# ADR-0004: Allow risk gate bypass via env (already implemented)

**Date**: 2026-11-09
**Status**: accepted
**Deciders**: Senior Reverse Engineering Lead

## Context

The risk gate in `run.rs:829-880` evaluates:
- Chronos EV gate
- State age limits
- Max trade size USD
- Max daily/hourly loss
- Consecutive failures
- Landing rate
- Tip spend ratio

Currently, `BYPASS_RISK_GATE=1` env var bypasses all checks except kill-switch. This is already implemented and tested.

## Decision

**Keep existing bypass mechanism.** Document it as the official "flow" path for testing.

## Alternatives Considered

### Alternative 1: Remove risk gate entirely
- **Cons**: No safety net for live; financial risk

### Alternative 2: Granular bypass per check
- **Pros**: Fine-grained control
- **Cons**: Complexity; current bypass is sufficient for testing

## Consequences

### Positive
- Single env var enables full flow for devnet testing
- Kill-switch remains as final backstop

### Negative
- Bypass removes all economic guards
- Must be used with small capital only

### Risks
- **Mitigation**: Never use on mainnet with significant capital
- **Mitigation**: Kill-switch + max trade size still apply