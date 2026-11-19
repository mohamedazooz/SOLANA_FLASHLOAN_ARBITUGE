# ADR-0009: Implement split routing with per-leg re-quote

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Quant Engineer

## Context

Split routing is hardcoded disabled in `run.rs:812-814`:
```rust
let split: Option<arb_engine::SplitAllocation> = None;
// Split routing is disabled until per-leg AMM quote, fee, slippage,
// execution instruction and aggregate min-out equivalence are proven.
```

Previous implementation (SPLIT-01) reused first-hop mints/fees for all legs and slippage-only return hop — economically incorrect.

## Decision

**Do not enable until per-leg re-quote implemented.** Required:
1. For each leg: fetch pool reserves → compute exact output for allocated input
2. Per-leg fee calculation from on-chain pool state
3. Per-leg slippage-adjusted min-out
4. Aggregate min-out = Σ leg min-outs - return hop slippage
5. Verify Σ input = flash loan borrow amount
6. Fork simulation of complete split route

## Alternatives Considered

### Alternative 1: Equal split with first-hop params
- **Cons**: Economically wrong; fails on heterogeneous pools

### Alternative 2: Convex optimization (RouteArchitectWorker)
- **Status**: Worker exists but disabled (`route_worker.rs:310-318`)
- **Cons**: Requires live reserve streaming + on-chain fee decode

## Consequences

### Positive
- No economically unsound splits executed
- Clear path: per-leg re-quote → worker enablement

### Negative
- High price-impact opportunities limited to single venue
- Worker code exists but unused

### Risks
- **Mitigation**: Worker revival tracked as separate epic
- **Mitigation**: Single-venue routes cover majority of current volume