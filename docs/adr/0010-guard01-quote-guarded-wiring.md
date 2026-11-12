# ADR-0010: Wire guarded quote as sole builder/dispatch path

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Architecture Review

## Context

GUARD-01 (OPEN): `quote_route_guarded`, `revalidate_before_dispatch` in `arb-engine` have **no callers** outside the crate. Supervisor uses direct quote path (`MarketGraph::quote_route` → `build_swap_hop_instruction`).

Guarded quote provides:
- Single `EconomicQuote` with hash
- Revalidation at risk/build/pre-dispatch with hash comparison
- Fail-closed on any mismatch

Without it: quote at ingest → risk eval → build → dispatch can diverge if state changes.

## Decision

**Refactor supervisor to use guarded quote as ONLY path.** Required:
1. `MarketGraph::quote_route_guarded` at ingest → returns `GuardedRouteQuote`
2. `RiskGate::evaluate` takes `GuardedRouteQuote` (not raw opportunity)
3. `TxBuilder::assemble` takes `GuardedRouteQuote`
4. `dispatch_opportunity` calls `revalidate_before_dispatch` with same quote
5. Assert `quote_hash == revalidated_hash` at each stage

## Alternatives Considered

### Alternative 1: Keep direct path, add optional guard
- **Cons**: Two paths → divergence risk; guard not enforced

### Alternative 2: Remove guarded quote entirely
- **Cons**: Loses defense-in-depth; GUARD-01 stays OPEN forever

## Consequences

### Positive
- Single source of truth for quote economics
- Hash comparison catches state drift at every stage
- Meets Gate D requirement

### Negative
- Significant refactor across supervisor, risk, tx, dispatch
- All callers must migrate

### Risks
- **Mitigation**: Phase migration: shadow mode first, then live
- **Mitigation**: Keep direct path as fallback behind feature flag during transition