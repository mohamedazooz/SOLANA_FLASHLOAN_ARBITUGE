# ADR-0012: Integrate fork simulation for complete route

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Protocol Engineer

## Context

Current simulation:
- LiteSVM in-memory (feature-gated, no bank state)
- RPC pre-flight (current bank, but single-slot)

Missing (14.1/14.3/14.4 OPEN):
- Fork simulation of **complete Kamino route**: borrow → swap(s) → repay
- Coherent bank state across all instructions
- Profit guard instruction semantics verification
- Exact account meta validation against on-chain program versions

## Decision

**Integrate fork simulation (Solana `simulateTransaction` with `replaceRecentBlockhash` + specific bank) as mandatory pre-dispatch step in Live.** Required:
1. Use `solana-program-test` or `litesvm` with forked bank from RPC
2. Execute full transaction (borrow + swaps + repay + tip)
3. Verify:
   - All instructions succeed
   - Repay amount = borrow + fee
   - Net profit ≥ min-out floor
   - No account meta errors
   - Compute units within limit
4. Cache simulation result for revalidation hash (ADR-0010)

## Alternatives Considered

### Alternative 1: Keep RPC pre-flight only
- **Cons**: Current bank may differ from leader's bank at dispatch; no multi-instruction atomicity proof

### Alternative 2: Skip simulation, rely on on-chain revert
- **Cons**: Burns fees on every failure; no pre-flight visibility

## Consequences

### Positive
- True atomicity proof before send
- Catches account meta mismatches, compute overflow, repay failures
- Meets Gate D requirements

### Negative
- Latency: fork sim ~100-500ms vs RPC sim ~50ms
- Requires RPC with fork support (not all providers)
- Complexity in bank management

### Risks
- **Mitigation**: Run fork sim in parallel with RPC pre-flight; use faster result
- **Mitigation**: Cache fork sim results for revalidation
- **Mitigation**: Start with `litesvm` fork feature; migrate to RPC fork when available