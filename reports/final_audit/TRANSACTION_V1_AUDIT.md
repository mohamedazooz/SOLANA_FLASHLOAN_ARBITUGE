# Transaction V1 Audit (PHASE I)

## Compiler — REAL, not a stub

`arb-tx/src/transport/adaptive.rs::compile_v1_message` implements the V1 wire format:
message-header config (priority fee lamports, CU limit, loaded-accounts-data-size limit),
inline addresses, per-instruction fixed headers, contiguous payloads. Verified by
`crates/arb-tx/tests/v1_header_differential.rs` (byte-level differential vs an independent
encoder) and `v1_mainnet_sim_envelope.rs` (live mainnet simulation of a real signed
Kamino flash borrow/repay envelope).

## Limits enforced (SIMD-0160 / SIMD-0385)

| Limit | Constant | Enforcement |
|---|---|---|
| 4,096 bytes | `SOLANA_PACKET_V1_MTU` | post-serialization check → reject, never downgrade |
| 64 addresses | `MAX_V1_ADDRESSES` | pre-flight + post-serialization |
| 64 instructions | `MAX_V1_INSTRUCTIONS` | pre-flight |
| No ALT | V1 design | accounts inline only |
| 12 signatures | single-signer routes | signer_count asserted in tests |

## Priority fee — ceil, verified

```rust
total_priority_fee_lamports = ceil(cu_limit × micro_lamports / 1_000_000)
                            = (cu.saturating_mul(micro) + 999_999) / 1_000_000
```

(`flash/compiler.rs:1046-1049`). The incorrect `micro / 1_000_000` form does not exist in the codebase (grep-verified).

## ComputeBudget interaction

V1 messages strip legacy ComputeBudget instructions (they are 150-CU no-ops in V1) and
re-encode limits natively. Klend's `flash_repay.borrow_instruction_index` is remapped to
the EXECUTED instruction list so borrow/repay pairing survives the strip
(`adaptive.rs:418-443`) — a subtle correctness trap, pinned by tests.

## Version selection — no silent downgrade

`compile_3leg_meme_dlmm_arb`: V0+ALT first (≤1232) → V1 only when `v1_transport_supported`
→ otherwise `Rejected(V0Oversized|V1TransportUnsupported)`. Cross-DEX and 4-leg builders
fail closed at >1232 with `TransactionTooLarge` (grep-verified, no downgrade branch).

## Transport compatibility — HONEST BLOCKER (BUG-004)

* RPC/Jito/BAM/Helius-Sender V1 ingress has NOT been canary-verified end-to-end.
* Therefore the production 3-leg path hardcodes `v1_transport_supported: false` — V1 is
  fully implemented and tested at the compiler level but DISABLED for broadcast.
* Enabling requires a harmless non-financial canary accepted and confirmed through the
  configured transport first. Until then V0+ALT (proven: 942-byte wire, live sim) is the
  only broadcast format. The 1232-byte limit remains binding for live routes.

## Boundary tests

`v1_header_differential.rs` covers 1,231/1,232/1,233 and 4,095/4,096/4,097-byte regimes,
63/64/65 addresses, and signature-count boundaries. PASS.
