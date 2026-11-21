# TRANSACTION_V1_VERIFICATION — SIMD-0385 Compliance

**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4`
**Verification date**: 2026-11-21
**Reference status**: SIMD-0385 **Activated**; mainnet epoch 1035 (slot 447,120,000, 2026-09-15T01:00 UTC), carried by SIMD-0296 (4,096-byte envelope).

---

## 1. Spec-item matrix (SIMD-0385 → implementation)

Implementation under test: `crates/arb-tx/src/transport/adaptive.rs`,
`crates/arb-tx/src/flash/compiler.rs`, `crates/arb-tx/src/primitives/`.

| Spec item | Spec value | Implementation | Verdict |
|---|---|---|---|
| VersionByte | `129` = 0x81, message first, trailing signatures | `V1_VERSION_DISCRIMINATOR = 0x81` (`adaptive.rs:42`); header parse + tail-extracted signature | ✅ |
| TransactionConfigMask bits | [0,1] priority fee (u64 LE, **absolute lamports**, both bits mandatory); [2] CU limit u32; [3] loaded-accounts-data u32; [4] heap u32 | `CONFIG_MASK_PRIORITY_FEE = 0x03`, `CONFIG_MASK_CU_LIMIT = 0x04`, `CONFIG_MASK_LOADED_DATA_LIMIT = 0x08`, `CONFIG_MASK_HEAP_SIZE = 0x10`; `compute_mask_and_values` emits contiguous 4-byte chunks | ✅ |
| Unset requests | priority 0; CU 0 (fatal); loaded-data 0 (32 KiB cost-model page); heap 32 KiB | `build_signed_v1_transaction` refuses `None`/0 CU and `None`/0 loaded-data before signing; permissive only for header differential tests at the message level | ✅ fail-closed |
| Heap bounds | 1 KiB multiple ∈ [32 KiB, 256 KiB] else sanitization failure | heap validation in `build_signed_v1_transaction` | ✅ |
| Limits | 4,096 B wire; 12 sigs; 64 accounts; 64 ixs; ≤255 accounts/ix | `SOLANA_PACKET_V1_MTU`, `MAX_V1_ADDRESSES = 64`, `MAX_V1_INSTRUCTIONS = 64` (SIMD-0160), per-ix account bound | ✅ |
| ALTs | absent in v1 | `v1_capable()` profile disables ALTs; V1 message has no lookup section; `v1_profile_never_downgrades_to_v0_alt` regression | ✅ |
| Priority-fee units | total lamports | `total_priority_fee_lamports` converts legacy micro-lamports/CU → `ceil(µlam/CU · CU / 1e6)`, checked (`None` on overflow, never saturating) | ✅ |
| ComputeBudget ixs | no-ops in v1, still consume 150 CU | stripped at compile; klend `borrow_instruction_index` rewritten against executed list; stripping a borrow *target* errors | ✅ |
| RPC reads | `maxSupportedTransactionVersion: 1`, base64 | `TransactionVersion::as_rpc_version`, `rpc_max_supported_version_from_wire`; simulation path uses sigVerify:false zero-filled envelope | ✅ |
| Loaded-data limit | requested 64 MiB cap for flash routes (klend spine measured ≈10.7 MB on Agave 4.4.1) | `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT = 67_108_864` with measurement note | ✅ (measurement PARTIALLY VERIFIED, conservative) |

## 2. Executed evidence (this session)

- `cargo test -p arb-tx --all-targets` (part of workspace run): V1 wire-layout,
  discriminator, resource-config bitmask, sanitization, oversized-rejection,
  scenario matrix A–H, cryptographic verification tests — all green. Named:
  `test_v1_wire_layout_and_discriminator`,
  `test_v1_resource_config_bitmask_and_values`,
  `test_v1_strips_compute_budget_and_requires_resource_limits`,
  `test_v1_sanitization_rules`,
  `test_parse_real_compiled_v1_wire_header_regression`,
  `test_matrix_11_v1_packet_size_boundary`,
  `test_matrix_15_v1_transaction_structure_cryptographic_verification_and_config_mask`,
  `test_scenario_a/b/c/d/h_*`,
  `test_compile_result_rejected_v1_oversized`.
- Workspace-wide: 1,398 passed / 0 failed / 2 ignored.

## 3. Articulated risk notes

1. **SIMD-0596 (96 accounts)** is draft — the code is correct to keep a 64-account limit; do not relax until activation.
2. V1 read-side compatibility of third-party RPC providers must be re-verified per provider during the Shadow soak (provider scenario tests exist; live provider sweep BLOCKED here).
3. Zero-CU mistake class is structurally prevented: all producing paths funnel through `build_signed_v1_transaction` / `build_unsigned_v1_simulation_transaction`, both of which enforce explicit resource config.
