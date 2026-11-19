# ADR-0007: Build PumpSwap adapter with IDL parity

**Date**: 2026-11-09
**Status**: proposed
**Deciders**: Senior Reverse Engineering Lead, Protocol Engineer

## Context

PumpSwap adapter currently returns hard error (`crates/arb-tx/src/lib.rs:1443`):
```rust
DexProtocol::PumpSwap => Err(TxError::UnsupportedProtocol("PumpSwap transaction adapter is not enabled; shadow-only".into()))
```

This is intentional — the adapter lacks IDL parity with on-chain program. Enabling without parity produces invalid transactions.

## Decision

**Do not enable until IDL parity achieved.** Required work:
1. Obtain official PumpSwap IDL
2. Decode pool accounts (reserves, fees, config)
3. Build swap instruction with correct account metas
4. Validate against on-chain quotes (differential testing)
5. Add to `build_swap_hop_instruction` match arm

## Alternatives Considered

### Alternative 1: Enable with placeholder accounts
- **Cons**: Will fail on-chain; wastes fees

### Alternative 2: Route via Jupiter for PumpSwap
- **Cons**: Adds dependency; Jupiter may not support all PumpSwap pairs

## Consequences

### Positive
- Safe: no invalid transactions
- Clear acceptance criteria: IDL parity + differential tests

### Negative
- PumpSwap opportunities remain shadow-only
- Missed revenue during development

### Risks
- **Mitigation**: Track PumpSwap TVL/volume; prioritize if significant
- **Mitigation**: Shadow-mode logging captures missed opportunities