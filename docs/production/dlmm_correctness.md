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

## 5. Differential Testing Evidence
- Differential parity confirmed against official Meteora test vectors in:
  - `crates/arb-engine/tests/meteora_sdk_differential_tests.rs`
  - `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`
  - `crates/arb-math/tests/dlmm_bin_array_boundary_tests.rs`
- Result: **0 unit/differential test failures** across positive, negative, and edge-boundary bin arrays.
