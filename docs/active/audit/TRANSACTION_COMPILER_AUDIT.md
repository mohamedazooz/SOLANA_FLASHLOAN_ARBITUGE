# Transaction Compiler Audit — Phase 5 (Mission §10)

- **Date:** 2026-11-22 · Source files: `crates/arb-tx/src/transport/` (adaptive 1,131 L / alt / master_alt / selection / template), `crates/arb-tx/src/primitives/builder.rs` (V0), wire constants.

## 1. Verified facts (SIMD-0385 layout, re-checked against the raw spec 2026-11-22)

| Requirement | Repo implementation | Evidence |
|---|---|---|
| 0x81 version discriminator | `V1_VERSION_DISCRIMINATOR = 0x81` first byte (`adaptive.rs:556`) | `v1_header_unit`, `v1_header_differential` tests |
| Legacy 3-byte header | pushed verbatim (`num_required_signatures / readonly signed / readonly unsigned`) | same |
| Resourced config in signed message | `V1ConfigParams::compute_mask_and_values()` → mask u32 LE + popcount-sized values (CU limit, CU price, loaded-accounts-data-size, total lamports fee) | `v1_loaded_data_limit_regression`, `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT = 67_108_864` |
| Limits 4,096B / 64 ixs / 64 accounts | `SOLANA_PACKET_V1_MTU=4096`, `MAX_V1_INSTRUCTIONS=64` (SIMD-0160), `MAX_V1_ADDRESSES=64`, ≤255 accounts/ix, ≤10 KiB ix data | `solana_v1_boundary_tests`, `v1_size_ladder_tests` |
| No Address Lookup Tables in V1 | `compile_v1_only` — "all addresses inline"; error on excess instead of ALT fallback | `adaptive.rs:633+` |
| CB no-op stripping + borrow-index rewrite | `compile_v1_message:415–450` — strips CB ixs, remaps, **fail-closed** `SanitizationError` if a stripped index is referenced; rewrites klend repay `borrow_instruction_index` (data[16]) | Phase 2 official `flash_repay_checks` verification |
| Never downgrade V1→V0 silently | transport capability gates: `supports_v1=false` → hard error `UnsupportedVersionForTransport`; direct-V1 and Jito-V1 each gated by explicit env toggles | `selection.rs:292–336` |

## 2. V0 path

- `compile_v0_message` (builder.rs): payer-first writable-signer ordering, BTreeMap dedup, compact-u16 encoding, ALT compression via `master_alt.rs`/`alt.rs`, MTU 1,232B gate, base64 wire output, `maxSupportedTransactionVersion: 1` in RPC submissions.
- Blockhash: lifetime spec in message; recovery layer persists signature + last-valid block height and only declares expiration with block-height evidence (`recovery.rs` / ISSUE-P1-02).

## 3. Audit results

| Property | Status | Notes |
|---|---|---|
| Serialization round-trip (V1) | ✅ | `v1_property_fuzz` (proptest) + differential header tests |
| Signature verification | ✅ | `verify_signatures` API + signed-tx tests |
| Instruction index remapping under CB strip | ✅ fail-closed | V1 CB no-ops consume 150 CU each per runtime behavior — strip required |
| Account key ordering | ✅ | payer-first, dedup, overflow-checked index space (≤64 → u8 safe) |
| Message size ladders | ✅ | `v1_size_ladder_tests`, `estimate_v0_transaction_size` mirrors V0 with ALT compression |
| RPC/runtime compatibility | ⚠️ conditional | Agave ≥ 4.2.2 required for V1 (SIMD-0385 activated epoch 1035); feature-gated by env toggles `SOLANA_TX_V1_DIRECT_ENABLED` / `SOLANA_TX_V1_JITO_ENABLED` — operator-controlled, default-off (fail-closed) |
| Wire format checks (base64, size, version) | ✅ | dispatch preflight + boundary tests |

## 4. Findings

- **TXC-F-2026-11-22-N1 (P3):** V1 support remains behind operator toggles pending an on-cluster compatibility re-verification (requires owned RPC node ≥ Agave 4.2.2 — not available in sandbox). Gate is correct and fail-closed; do not enable until a live-V1 landing is observed.
- No other defects found in the compiler paths during this audit.

## 5. Compatibility matrix (transport × version)

| Transport \ Version | V0 | V1 | Notes |
|---|---|---|---|
| Direct RPC | ✅ default-ready | ⚠️ gated by `SOLANA_TX_V1_DIRECT_ENABLED` + Agave ≥4.2.2 | `maxSupportedTransactionVersion: 1` sent |
| Jito bundle | ✅ | ⚠️ gated by `SOLANA_TX_V1_JITO_ENABLED` | non-atomic Jito mode rejected by atomic-integrity |
| SWQoS direct | ✅ | ⚠️ same toggle class as direct RPC | capability-aware selection |
| Simulation (arb-sim) | ✅ | ✅ | LiteSVM harness version-aware |

## 6. Evidence

```text
cargo test -p arb-tx → 434 passed / 0 failed
```
Compiler/round-trip/boundary suites: `adaptive_compiler_tests`, `solana_v1_boundary_tests`, `three_leg_meme_compiler_tests`, `v1_header_unit`, `v1_header_differential`, `v1_loaded_data_limit_regression`, `v1_property_fuzz`, `v1_size_ladder_tests`, `cross_dex_dlmm_cpmm_v1_atomic_tests`.
