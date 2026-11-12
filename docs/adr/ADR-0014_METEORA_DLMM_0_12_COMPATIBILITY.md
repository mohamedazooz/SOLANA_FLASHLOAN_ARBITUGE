# ADR-0014: Meteora DLMM 0.12.0 Protocol Compatibility, Byte Offsets & Sizing Parity

## Status
Accepted

## Context
SENTINEL executes high-frequency, atomic cross-pool arbitrage involving Meteora DLMM pools (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`).
Previous decoders and quote approximations suffered from:
1. Mislabeled field byte offsets in `LbPair` account deserialization (such as reading `min_bin_id` at offset 24 instead of `base_fee_power_factor` at offset 34).
2. Unhandled `CollectFeeMode` introduced in DLMM 0.12.0, causing fee token denomination confusion.
3. Lack of discrete exact-out quoting math matching on-chain integer ceil division.
4. Rigid 3-BinArray assumptions causing quote failures near bin boundaries.
5. Unchecked cross-currency arithmetic (e.g. adding raw USDC units directly to SOL lamports).

## Decision

### 1. Pinned Dependencies
We pin the official TypeScript SDK and reference Anchor bindings for differential test generation:
- `@meteora-ag/dlmm`: `1.9.14`
- `@coral-xyz/anchor`: `^0.30.1`
- `@solana/web3.js`: `^1.98.0`

The authoritative on-chain interface is pinned by the official IDL
(`MeteoraAg/dlmm-sdk` → `idls/dlmm.json`), whose metadata is:
- `name`: `lb_clmm`
- `version`: `0.12.0`
- address: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`

Two different things are pinned here and they must not be conflated:
the **program** interface (IDL 0.12.0) and the **TypeScript client**
(`@meteora-ag/dlmm` 1.9.14). The client version does not imply program
version; the IDL does.

### 2. Canonical `LbPair` Byte Layout & Decoders
In Meteora DLMM 0.12.0, `LbPair` account data starts after the 8-byte Anchor discriminator:
- **`StaticParameters` (Offsets 8..40)**:
  - `base_factor`: `u16` @ 8..10
  - `filter_period`: `u16` @ 10..12
  - `decay_period`: `u16` @ 12..14
  - `reduction_factor`: `u16` @ 14..16
  - `variable_fee_control`: `u32` @ 16..20
  - `max_volatility_accumulator`: `u32` @ 20..24
  - `min_bin_id`: `i32` @ 24..28
  - `max_bin_id`: `i32` @ 28..32
  - `protocol_share`: `u16` @ 32..34
  - `base_fee_power_factor`: `u8` @ 34
  - `concrete_function_type`: `u8` @ 35 (0 = Undetermined, 1 = LiquidityMining, 2 = LimitOrder)
  - `collect_fee_mode`: `u8` @ 36 (0 = InputOnly, 1 = OnlyY)
  - `padding`: 37..40
- **`VariableParameters` (Offsets 40..72)**:
  - `volatility_accumulator`: `u32` @ 40..44
  - `volatility_reference`: `u32` @ 44..48
  - `index_reference`: `i32` @ 48..52
  - `padding`: 52..56
  - `last_update_timestamp`: `i64` @ 56..64
  - `padding`: 64..72
- **`bump_seed` @ 72, `bin_step_seed` (u16) @ 73..75, `pair_type` @ 75**
- **Active State (Offsets 76..83)**:
  - `active_id`: `i32` @ 76..80
  - `bin_step`: `u16` @ 80..82
  - `status`: `u8` @ 82 (0 = enabled)
- **Mints and reserves (88..216)**: `token_x_mint` @ 88..120, `token_y_mint` @ 120..152,
  `reserve_x` @ 152..184, `reserve_y` @ 184..216
- **Protocol fees (216..232)**: `protocol_fee.amount_x` @ 216..224, `protocol_fee.amount_y` @ 224..232
- **`_padding_1`**: 232..264
- **`reward_infos`**: `[RewardInfo; 2]` @ 264..552 (144 bytes each)
- **Oracle (552..584)**: `oracle: Pubkey` @ 552..584

> Correction: an earlier revision of this ADR placed `oracle` at 232..264 and
> omitted `_padding_1` and `reward_infos`. That was wrong; the decoder in
> `crates/arb-dex/src/meteora_dlmm.rs` has always read `oracle` at 552, which is
> the correct IDL offset. Any consumer using 232 would read padding as a pubkey.

### 3. CollectFeeMode Semantics
Fee collection denomination is determined by `collect_fee_mode`:
- **`0 (InputOnly)`**: Swap fee is charged on the inbound token.
  - When `swap_for_y` (X -> Y): Fee is denominated in Token X.
  - When `!swap_for_y` (Y -> X): Fee is denominated in Token Y.
- **`1 (OnlyY)`**: Swap fee is always collected in Token Y.
  - When `swap_for_y` (X -> Y): Fee is deducted from the output Token Y.
  - When `!swap_for_y` (Y -> X): Fee is deducted from the input Token Y.

### 4. Discrete Bin Quoting (Exact-In & Exact-Out)
- Exact-In (`quote_dlmm_view`): Computes output using floor division and net input after fee deduction per bin.
- Exact-Out (`quote_dlmm_exact_out`): Computes required input using ceil division and adds fees backward per bin.
- Transfer Fees: Applied to token transfer amounts via `apply_transfer_fee_ceil` matching Token-2022 extension math.

### 5. Bin Array Traversal Beyond 3-Array Fast Path
Bin array indices are Euclidean multiples of 70:
```rust
pub fn bin_array_index_for_bin(bin_id: i32) -> i64 {
    bin_id.div_euclid(METEORA_BINS_PER_ARRAY as i32) as i64
}
```
Dynamic traversal collects all required Bin Array PDAs across boundaries (`-71, -70, -1, 0, 69, 70`) without any unconditional 3-array execution ceiling.

### 6. Dimensional Profitability Accounting
All amounts and fees are strictly wrapped in `TypedTokenAmount` and evaluated via `DimensionalProfitabilityReport`:
- Cross-currency additions (e.g. USDC + lamports) are rejected by compile-time marker types and runtime checks.
- Network transaction fees (SOL lamports) are converted to route quote currency using conservative, timestamped conversion rates.
- Embedded DEX fees are itemized without double-counting against output amounts.

## Consequences
- The decoder and quote engine match the pinned DLMM 0.12.0 **interface layout**, so
  offsets are verified against the IDL rather than inferred from SDK behaviour.
- Boundary traversal is validated at bins `-71, -70, -1, 0, 69, 70` and across more
  than three bin arrays, with no execution ceiling.
- Fee denomination follows `CollectFeeMode` and is no longer assumed to be input-side.
- Dimensional accounting rejects cross-currency arithmetic at compile time and runtime.

### What is NOT claimed
- **No profitability claim.** Correct arithmetic does not imply a profitable route.
- **No live readiness.** Live execution remains disabled.
- **Differential tests are not all SDK differential tests.** Two distinct classes
  exist and must stay labelled separately:
  - *Official-SDK differential*: `crates/arb-engine/tests/meteora_sdk_differential_tests.rs`
    compares our numbers against values produced by the pinned `@meteora-ag/dlmm`
    client from the fixture in `fixtures/meteora_official_sdk_comparison.json`.
  - *Invariant / regression*: tests that assert internal consistency (round-trip
    exact-in vs exact-out, conservation of amounts, boundary indexing). These are
    valuable but they are **not** evidence of SDK parity and are not described as such.
- Parity is limited to the fixture's inputs. A different pool, bin step, or fee mode
  is covered only if the fixture contains that case.
