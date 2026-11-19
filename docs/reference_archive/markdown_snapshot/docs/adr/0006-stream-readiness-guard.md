# ADR-0006: Weaken final stream-readiness guard to warning-only

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead

## Context

Final stream-readiness guard in `dispatch.rs:371-401` re-checks before network send:
- Stream connected
- Stream generation unchanged
- Current slot within `max_state_age_slots` of opportunity slot
- No stale pool data

If any check fails, dispatch returns early with rejection logged. This is a fail-closed guard against state drift during simulation/wait.

## Decision

Change guard to **warning-only**: log rejection reason but continue to dispatch. Keep metrics/alerting.

## Alternatives Considered

### Alternative 1: Keep fail-closed (current)
- **Pros**: Maximum safety
- **Cons**: False positives during high latency; blocks valid opportunities

### Alternative 2: Remove entirely
- **Cons**: No visibility into state drift

## Consequences

### Positive
- Reduces false-positive rejections during network hiccups
- Maintains observability via logs/metrics

### Negative
- May dispatch with stale state (slippage risk)
- Relies on on-chain min-out for final protection

### Risks
- **Mitigation**: Min-out floor (ADR-0003) provides on-chain backstop
- **Mitigation**: Alert on guard trigger for ops visibility