> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Meteora DLMM Protocol Correctness & Verification Specification

## 1. Protocol Architecture & Deployed Identifiers
- **Program ID**: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (`METEORA_DLMM_PROGRAM_ID`)
- **Protocol Version**: `lb_clmm` 0.12.0
- **Anchor Discriminators**:
  - `LbPair`: 8 bytes
  - `BinArray`: 8 bytes (`BIN_ARRAY_INDEX_OFFSET = 8`, `BIN_ARRAY_INDEX_LEN = 8`)
  - `BinArrayBitmapExtension`: 8 bytes
- **Bins Per Array**: 70 (`METEORA_BINS_PER_BIN_ARRAY = 70`)
- **Bin Struct Size**: 144 bytes per bin

## 2. PDA Derivations (Official Seeds)
- **Oracle PDA**: `[b"oracle", lb_pair]`
- **Reserve Vaults**: `[lb_pair, token_mint]`
- **BinArray PDA**: `[b"bin_array", lb_pair, index_le_i64]`
- **BinArrayBitmapExtension PDA**: `[b"bitmap", lb_pair]`

## 3. Bin Array Indexing & Math Correctness
The mapping from `bin_id` ($[-443636, 443636]$) to `bin_array_index` handles signed integers and negative values strictly:
```rust
pub fn bin_id_to_array_index(bin_id: i32) -> i64 {
    let b = i64::from(bin_id);
    let n = METEORA_BINS_PER_BIN_ARRAY;
    if b >= 0 { b / n } else { (b - (n - 1)) / n }
}
```
This ensures zero index skew across negative array boundaries and matches Meteora's on-chain Rust contract.

## 4. Multi-Bin Traversal & Liquidity Aggregation
- **Limit-Order Liquidity Folding**: The on-chain `Bin` struct maintains limit-order liquidity (`open_order_amount`, `processed_order_remaining_amount`). The adapter folds this executable liquidity directly into the swap calculation.
- **Fail-Closed Availability**: If the active bin or adjacent traversal crosses into an unobserved or unverified bin array, the quote engine immediately terminates without extrapolating synthetic liquidity.
- **Dynamic Fee Model**: Real-time volatility accumulator, index reference, and dynamic fee computation are incorporated into the swap quote.

## 5. lb_clmm 0.12.0 Compliance Audit (mainnet-active since 2026-05-21)

Audited against the official Meteora docs (`meteora_docs_complete.md`, Changelog
0.12.0 + Accounts + CPI/Rust Integration + Formulas sections).

| 0.12.0 change | Our status | Evidence |
|---|---|---|
| Bitmap extension must be **writable** in `swap`/`swap2` remaining accounts | Implemented | `crates/arb-tx/src/protocols/meteora_dlmm.rs`: `to_account_metas` emits `AccountMeta::new(ext, false)` (writable) at index 1 |
| Max bins per swap reduced **280 -> 260** | Implemented (fail-closed) | `arb_constants::METEORA_MAX_BINS_PER_SWAP = 260`; `plan_bin_arrays` returns `DepthAboveSwapLimit`; evaluator rejects with `BINS_ABOVE_SWAP_LIMIT`; 3-leg `quote_single_hop` returns `None`; regression test `traversal_above_the_0_12_0_swap_bin_limit_fails_closed` |
| `CollectFeeMode` (InputOnly=0, OnlyY=1) | Implemented (fail-closed) | Decoded into pool params; `quote()` rejects `collect_fee_mode > 1`; fee formulas branch on mode in `arb-math/src/dlmm.rs` |
| Limit-order liquidity (function_type 2) | Implemented | `open_order_amount` / `processed_order_remaining_amount` folded into `bin_max_amount_out` |
| Fee formulas: base = base_factor x bin_step x 10 x 10^base_fee_power_factor (1e9); variable = ceil(vfc x (va x bin_step)^2 / 1e11); MAX_FEE_RATE = 10% | Implemented | `arb-math/src/dlmm.rs`: `compute_total_fee_rate` (cap), `dlmm_fee_on_input` / `compute_fee_to_add` (ceil, both denominators) |
| `AccountsType::TransferHookReferral` when `host_fee_in` is Token-2022-with-hook | N/A | We always build with `host_fee_in = None` |
| LM-pool bin-field packing | N/A (read-side unaffected) | We only read `amount_x` / `amount_y` / `open_order` / `processed` from BinArray V2 (144B bins) |
| `swap2` used everywhere | Implemented | All builders route through `build_swap2_exact_in*` |

## 6. Differential Testing Evidence
- Differential parity confirmed against official Meteora test vectors in:
  - `crates/arb-engine/tests/meteora_sdk_differential_tests.rs`
  - `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`
  - `crates/arb-math/tests/dlmm_bin_array_boundary_tests.rs`
- Result: **0 unit/differential test failures** across positive, negative, and edge-boundary bin arrays.
