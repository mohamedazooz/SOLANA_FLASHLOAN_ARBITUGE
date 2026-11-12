# ADR-0008: Enable Jupiter live path with quote parity

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Protocol Engineer

## Context

Jupiter execution is hardcoded disabled in `run.rs:912-915`:
```rust
let route_via_jupiter = false;
// Jupiter execution is disabled in this remediation build. A Jupiter
// quote/route is not proof that our locally sized route has equivalent
// fees, account metas, min-out and flash-loan repayment semantics.
```

Jupiter quote ≠ local execution parity. Differences:
- Fee calculation (Jupiter includes platform fee)
- Account metas (Jupiter uses different ATA resolution)
- Min-out derivation (Jupiter uses slippage; we use conservative floor)
- Flash loan repay semantics (Jupiter doesn't know our Kamino params)

## Decision

**Do not enable until full parity proven.** Required:
1. Jupiter quote → local re-quote for each hop
2. Verify fee/account meta/min-out match
3. Integrate Kamino borrow/repay into Jupiter route (or use Jupiter's flash loan)
4. End-to-end fork simulation

## Alternatives Considered

### Alternative 1: Use Jupiter for quote, local for execution
- **Cons**: Quote/execution divergence → failed transactions

### Alternative 2: Use Jupiter entirely (including flash loan)
- **Cons**: Different risk profile; not our stack

## Consequences

### Positive
- No execution divergence
- Clear acceptance criteria

### Negative
- Exotic routes (3-hop+) remain unsupported
- Jupiter's broader DEX coverage unused

### Risks
- **Mitigation**: Shadow-mode Jupiter quotes logged for comparison
- **Mitigation**: Native builder covers 95%+ of current opportunities