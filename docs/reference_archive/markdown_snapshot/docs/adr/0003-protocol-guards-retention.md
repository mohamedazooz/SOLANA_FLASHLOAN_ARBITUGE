# ADR-0003: Retain MTU/blockhash/min-out protocol guards

**Date**: 2026-11-09
**Status**: accepted
**Deciders**: Senior Reverse Engineering Lead, Protocol Engineer

## Context

Several guards enforce Solana protocol invariants and Kamino flash loan atomicity:
- MTU 1232 check (`dispatch.rs:171-199`) — Solana validators drop transactions >1232 bytes
- Zero blockhash check (`run.rs:1083-1084`) — Validators reject transactions with zero blockhash
- Min-out floor (economic invariant) — Ensures swap output covers borrow + fees + repay; without it, Kamino repay fails → atomic revert

These are not "safety features" — they are protocol requirements. Removing them produces invalid transactions that fail on-chain, wasting fees and potentially leaving state inconsistent.

## Decision

**Retain all three guards unchanged.** Do not weaken, bypass, or remove.

## Alternatives Considered

### Alternative 1: Make MTU check configurable
- **Cons**: Transactions >1232 bytes will be dropped by validators — no recovery

### Alternative 2: Make zero blockhash a warning
- **Cons**: Transaction will fail on-chain — waste of priority fee + tip

### Alternative 3: Remove min-out floor, rely on on-chain revert
- **Cons**: Atomic revert still burns all fees (priority + tip + base); min-out prevents waste

## Consequences

### Positive
- Zero invalid transactions sent to network
- Fees only spent on transactions that can succeed
- Atomicity preserved: borrow→swap→repay either all succeed or all revert

### Negative
- Some legitimate opportunities rejected if tx >1232 bytes (rare)
- Blockhash fetch adds ~50ms latency

### Risks
- None — these are protocol invariants, not policy choices