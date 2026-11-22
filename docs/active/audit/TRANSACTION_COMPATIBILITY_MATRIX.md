# Transaction Compatibility Matrix — Phase 5 (Mission §10)

Companion to `TRANSACTION_COMPILER_AUDIT.md`. Status per (format × transport × runtime).

## Formats

- **Legacy**: supported everywhere (tests only; never used by the flash loan loop).
- **V0**: production default. 1,232B packet gate, ALT compression, `maxSupportedTransactionVersion: 1` in RPC poll calls (must be ≥0 to never break node replies).
- **V1 (SIMD-0385)**: implemented to spec (0x81, signed-message config mask/values, 4,096B, ≤64 ixs/accounts, no ALTs, CB strip + `borrow_instruction_index` rewrite fail-closed). Activation-gated per the matrix below.

## Transport × Version × Runtime

| Transport | V0 | V1 (Agave ≥ 4.2.2) | V1 (older Agave) |
|---|---|---|---|
| Direct RPC | ✅ | env `SOLANA_TX_V1_DIRECT_ENABLED` (default OFF) | capability probe → hard error, no downgrade |
| Jito sendBundle | ✅ | env `SOLANA_TX_V1_JITO_ENABLED` (default OFF) | hard error, no downgrade |
| SWQoS | ✅ | same class as direct RPC | hard error, no downgrade |
| Preflight simulation | ✅ | ✅ (arbitrary cluster exposes V1 via RPC) | — |

## Rules enforced in code

1. **No silent format downgrade** — capability mismatch is an error, never a fallback (`selection.rs`).
2. **No ALT in V1** — inline-only; over-limit → `SanitizationError`, not recompilation.
3. **CB strip + index rewrite** — every compiled V1 flash-bundle passes `atomic_integrity` re-verification for its format.
4. **Packet gates** — V0 ≤1,232B; V1 ≤4,096B; else `TransactionTooLarge`.

## Verification evidence (2026-11-22)

`cargo test -p arb-tx` → 434 passed / 0 failed (incl. `v1_property_fuzz`, `solana_v1_boundary_tests`, `v1_size_ladder_tests`, `dynamic_v1_boundary`, `adaptive_compiler_tests`).

Open item: live on-cluster V1 landing observation before enabling the V1 env toggles (see TXC-F-2026-11-22-N1).
