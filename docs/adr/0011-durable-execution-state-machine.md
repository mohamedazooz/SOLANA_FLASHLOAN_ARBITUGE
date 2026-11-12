# ADR-0011: Build durable execution state machine with reconciliation

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Architecture Review

## Context

Current execution tracking (`arb-land`, `arb-store`):
- Records: `sent` / `simulated` / `canary` outcomes
- `BundleTracker` polls Jito for terminal status
- **No durable state machine**: crash after send = bundle accepted but untracked
- No idempotent reconciliation: restart doesn't recover in-flight bundles

Gap 14.5 OPEN: "Durable execution state machine + idempotent reconciliation"

## Decision

**Implement persistent execution state machine.** Required states:
```
Created → Quoted → RiskApproved → Built → Simulated → Dispatched 
  → Accepted (bundle_id/sig) → Landed (on-chain confirmed) → Finalized
  → Reconciled (PnL realized)
```

Each state transition persisted to DB (SQLite/Postgres). On restart:
1. Load all `Dispatched`/`Accepted` records
2. Poll Jito/RPC for actual status
3. Reconcile: update state to actual terminal state
4. Resume polling for non-terminal

## Alternatives Considered

### Alternative 1: External orchestrator (Temporal, Cadence)
- **Cons**: New dependency; overkill for single-process bot

### Alternative 2: File-based state (current kill-switch pattern)
- **Cons**: No query/index; manual recovery

## Consequences

### Positive
- Zero lost bundles on crash/restart
- Audit trail for every execution
- Enables live with confidence

### Negative
- New DB schema/migrations
- Complexity in state transitions

### Risks
- **Mitigation**: Start with SQLite (existing store); migrate to Postgres later
- **Mitigation**: Idempotent polling: `get_status(bundle_id)` safe to call repeatedly