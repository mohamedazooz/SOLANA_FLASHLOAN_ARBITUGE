# Phase 3 — Meteora DLMM Forensic Audit

## 3.1 Protocol Version Identification

### 3.1.1 Program ID
- **Meteora DLMM Program ID**: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
- **Verified against**: On-chain deployment, official Meteora Ag/dlmm-sdk source
- **SDK version**: `lb_clmm` crate version matching on-chain program

### 3.1.2 Key Bin Array Constants (from `arb-constants`)
| Constant | Value | Meaning |
|---|---|---|
| `METEORA_BINS_PER_BIN_ARRAY` | `70` | Bins per BinArray account — drives PDA index calculation |
| `METEORA_BIN_STEP` | configurable per pool | Basis points fee tier (e.g. 4 = 0.04%) |
| `METEORA_SWAP2_DISCRIMINATOR` | `[65, 75, 63, 76, 235, 91, 91, 136]` | `sha256("global:swap2")[..8]` |

### 3.1.3 Account Layouts Verified
- **LbPair account**: Discriminator + parameters + volatility + bin arrays
- **BinArray account**: 70 bins × (bin_id i32 + amount_x u128 + amount_y u128) + bitmap
- **Reserve vaults**: Derived PDA `[lb_pair, token_mint]` — verified against SDK pda.rs
- **Oracle account**: Derived PDA `[oracle, lb_pair]` — verified against SDK pda.rs
- **Event authority**: Derived PDA `[__event_authority]` — verified against SDK pda.rs

### 3.1.4 Bin Array Index Derivation
- **Formula**: `bin_array_index = bin_id.div_euclid(70)` (floor division toward negative infinity)
- **SDK verification**: `derive_meteora_bin_array_pda(pool, bin_array_index)` uses seeds `[b"bin_array", lb_pair, &idx_bytes]` where `idx_bytes = bin_array_index.to_le_bytes()` (i64, not i32)
- **On-chain verification**: PDA derivation must match the BinArray accounts referenced by `swap2` instruction

## 3.2 Bin Array Correctness

### 3.2.1 Bin Array Index Calculation
The code correctly uses `div_euclid` for floor division:
```rust
pub fn bin_array_index_for_bin(bin_id: i32) -> i64 {
    (bin_id as i64).div_euclid(METEORA_BINS_PER_BIN_ARRAY)
}
```
This ensures negative bin IDs map correctly (e.g., bin -1 → array index -1, bin -70 → array index -1, bin -71 → array index -2).

### 3.2.2 Negative Bin Handling
- Bin IDs range from -443,636 to +443,636 (MIN_BIN_ID/MAX_BIN_ID)
- `div_euclid` ensures correct array indexing across the full range
- `advance_active_id` decrements/increments by 1 and validates bounds

### 3.2.3 Bins Per Array Verification
- **Constant**: `METEORA_BINS_PER_BIN_ARRAY = 70`
- **Test**: `test_bin_price_at_zero` confirms P_0 = 1.0 in Q64.64 (ONE_Q64)
- **Test**: `test_bin_price_positive_exponent` and `test_bin_price_negative_exponent` verify price progression
- **Test**: Reciprocal product `P_1 × P_{-1}` must be within 100 units of 1.0 in Q64.64

### 3.2.4 Bitmap Extension
- BinArray accounts include a bitmap for initialized bin tracking
- The code checks `bin.price_q64 != 0` before using cached price; otherwise derives from `get_price_from_id`
- Missing bin arrays are handled via `BinNotFound` error

## 3.3 Quote Engine Audit

### 3.3.1 Exact-in and Exact-out Validation

The quote engine implements both swap directions:
- `swap_for_y = true`: User deposits Token X (SOL), receives Token Y (USDC)
- `swap_for_y = false`: User deposits Token Y (USDC), receives Token X (SOL)

**Validation checks**:
- `amount_in == 0 → Err(ZeroInput)` — fail-closed
- `pool.status != 0 → Err(PoolDisabled)` — pool must be active
- Bin found in provided slice → `BinNotFound` if missing
- Zero liquidity in target bin → handled gracefully (0 output)

### 3.3.2 Raw Integer Token Amounts

All computations use U256 for intermediate results, with final try_into to u64:
- `pow_q64`: U256 math, overflow check, result == 0 → Err (underflow)
- `consume_bin_liquidity`: U256 division with ceiling/floor rounding
- `compute_variable_fee`: U256 `div_rem` with explicit ceiling
- `quote_dlmm_multi_bin`: All additions use `checked_add`, propagating Overflow

**No floating point** in raw token accounting. `f64` may only be used for UI display after conversion.

### 3.3.3 Price Impact Calculation

`calculate_price_impact_bps` (dlmm.rs lines 777-806):
- Ideal output computed from `amount_in × start_price_q64 >> 64` (for X→Y) or `amount_in << 64 / start_price_q64` (for Y→X)
- If `ideal_out ≤ amount_out` → 0 bps impact (price moved favorably or not at all)
- Otherwise: `bps = (diff × 10_000) / ideal_out`, clamped to max 10,000 (100%)
- **Correct**: Uses exact integer arithmetic, no floating point

### 3.3.4 Variable Fee Dynamics

`compute_variable_fee` correctly:
- Returns 0 when `variable_fee_control == 0` or `volatility_accumulator == 0`
- Uses U256 `square = prod * prod` then `div_rem` by 100_000_000_000
- Ceiling division: if remainder ≠ 0, add 1
- Clamps result to u64 range

### 3.3.5 Bucket Traversal Correctness

`quote_dlmm_multi_bin` (dlmm.rs lines 661-755):
- Validates pool status == 0 (enabled)
- Updates volatility references with decay based on elapsed timestamp
- For each bin: gets/derives price_q64, computes total fee rate, consumes liquidity
- Advances active bin ID via `advance_active_id` (direction-aware)
- Tracks `bins_crossed`, `end_active_id`, `end_volatility_accumulator`
- Price impact calculated against starting spot price

**Critique**: The traversal correctly walks bins in the direction of the swap (positive bin index for Y→X, negative for X→Y). Each bin's liquidity is consumed in sequence.

## 3.4 The 92% Safety Factor Issue

### 3.4.1 Location
The 92% scaling factor appears in `scale_dlmm_three_hop_input` (dlmm.rs lines 292-316):
```rust
let safe_optimal_in = (bottleneck_in * 92) / 100;
```

### 3.4.2 Origin and Purpose
This factor scales down the probe input to "avoid crossing into suboptimal bins." It was hardcoded as a safety margin rather than being based on actual bin traversal computation.

### 3.4.3 Audit Findings from SAFETY_AND_HARDCODED_AUDIT.md
> **14. Meteora safety factor 92%**: Hardcoded to avoid crossing suboptimal bins; may be overly conservative.

### 3.4.4 Directive Position
> **Remove approximations only after differential tests prove the replacement correct.**

### 3.4.5 Analysis

The 92% factor is an **approximation/replacement** for actual bin traversal. The correct approach is:

1. **Do not use 92% as a replacement for actual bin traversal.** Instead, run the full `quote_dlmm_multi_bin` simulation across the actual bin arrays and determine the bottleneck from real liquidity consumption.

2. **If 92% is retained as a conservative cap**, it must be validated against differential testing:
   - Compare `scale_dlmm_three_hop_input` output against full `quote_dlmm_multi_bin` simulation
   - Test across many bin steps, active bins, liquidity configurations
   - Measure the actual output difference and relative difference

3. **Current test coverage**: The existing test `test_scale_dlmm_three_hop_input` verifies the 92% math internally but does NOT compare against `quote_dlmm_multi_bin` results.

### 3.4.6 Required Action

**Option A — Remove 92%, use actual traversal:**
- Replace `scale_dlmm_three_hop_input` with a call to `quote_dlmm_multi_bin` across the three hops
- Determine bottleneck from actual min `active_bin_max_in` across hops
- No arbitrary scaling factor

**Option B — Keep 92% but validate:**
- Run differential tests: `quote_dlmm_multi_bin` vs `scale_dlmm_three_hop_input` for many configurations
- Document the maximum output difference and its frequency
- If difference is small and conservative (92% always ≤ actual), retain with documentation
- If difference is large or non-conservative, replace with actual traversal

### 3.4.7 Recommendation
**Replace the 92% scaling with actual `quote_dlmm_multi_bin` traversal.** The 92% factor is an approximation that may incorrectly limit trade sizes. The proper production approach is:

1. For three-hop routes, compute `active_bin_max_in` for each hop via `dlmm_active_bin_max_input`
2. Take the minimum across hops as the bottleneck
3. Use that bottleneck as the probe input to `quote_dlmm_multi_bin`
4. The quote will naturally account for actual bin traversal, price impact, and fee dynamics

**If 92% is kept as an additional safety cap** on top of actual traversal, it should be:
- Documented as a conservative overlay
- Validated against on-chain behavior
- Applied AFTER the actual traversal quote, not instead of it

## 3.5 SDK Differential Testing

### 3.5.1 Recommended Test Framework

Use the official Meteora SDK or an independently verified reference implementation. Create fixtures from real mainnet account snapshots.

### 3.5.2 Test Matrix

| Dimension | Values to Test |
|---|---|
| Swap directions | X→Y, Y→X |
| Bin steps | 1, 4, 10, 25, 100 bps (0.01%, 0.04%, 0.1%, 0.25%, 1%) |
| Active bins | Positive (0 to 443636), negative (-1 to -443636), across array boundaries |
| Liquidity | Sparse (few bins with liquidity), Deep (many bins filled), Empty (zero liquidity) |
| Fee variations | Different protocol shares, variable fees, capped at 10% |
| Input size | Small (near dust), Medium, Large (spans multiple bins) |
| Boundary conditions | Bin at array edge (index calculation), Bin at min/max (±443636), Single bin, Multi-bin spanning |
| Sparse liquidity | Gaps between bins, isolated liquidity pools |
| Deep liquidity | 20+ bins filled, progressive price impact |

### 3.5.3 Metrics to Measure

For each test fixture, compare arb-math DLMM output against the reference SDK:

| Metric | Target |
|---|---|
| Absolute output difference | Should be 0 when models and input snapshots are equivalent |
| Relative output difference | Should be 0 for equivalent inputs |
| Fee difference | Should be 0 (both use on-chain fee params) |
| Price impact difference | Should be 0 (same price Q64.64 math) |
| Rounding difference | Should be explainable (ceiling vs floor choices) |

**Target**: Exact raw-unit agreement where the models and input snapshots are equivalent.

### 3.5.4 Current Status

- **DLMM math tests**: 23 tests pass within arb-math package
- **SDK differential**: Not yet implemented in this codebase
- **Reference comparison**: No independent reference implementation integrated
- **Gap**: Must implement differential testing before claiming DLMM production readiness

## 3.6 Meteora Transaction Construction

### 3.6.1 Instruction Discriminator
- `swap2` discriminator: `[65, 75, 63, 76, 235, 91, 91, 136]` ✓ (verified against official SDK)

### 3.6.2 Account Ordering
The `build_meteora_dlmm_swap_instruction` function produces the following account order (verified from `dlmm::client::accounts::Swap2` and `cli/src/instructions/swap_exact_in.rs`):

| Index | Account | Writable | Signer |
|---|---|---|---|
| 0 | lb_pair | writable | No |
| 1 | bin_array_bitmap_extension | readonly | No (program_id placeholder) |
| 2 | reserve_x | writable | No |
| 3 | reserve_y | writable | No |
| 4 | token_x_mint | readonly | No |
| 5 | token_y_mint | readonly | No |
| 6 | token_x_program (SPL Token) | readonly | No |
| 7 | token_y_program (SPL Token) | readonly | No |
| 8 | user | readonly | Yes (signer) |
| 9 | user_token_in | writable | No |
| 10 | user_token_out | writable | No |
| 11 | oracle | readonly | No |
| 12 | host_fee_in | readonly | No (program_id placeholder) |
| 13 | event_authority | readonly | No |
| 14 | program (self-reference) | readonly | No |
| 15 | memo_program | readonly | No |
| 16..16+N | bin_arrays (2..=4) | writable | No |

### 3.6.3 Bin Array Calculation
- `active_idx = bin_array_index_for_bin(active_bin_id)` — uses `div_euclid(70)`
- `bin_array_count` dynamically calculated based on price impact estimation:
  - `est_bins >= 140 → 4 arrays`
  - `est_bins >= 70 ∨ bin_step <= 5 → 3 arrays`
  - Else → 2 arrays
- Each bin array PDA derived via `derive_meteora_bin_array_pda(&accounts.lb_pair, idx)` with i64 index

### 3.6.4 Remaining Accounts
- `MemoProgram` for log messages
- `event_authority` for DLMM events
- `host_fee_in` placeholder (program_id, not used in fee calculation)
- `bin_array_bitmap_extension` placeholder (program_id as readonly)

### 3.6.5 Correctness Verification
- All program IDs sourced from `arb-constants` (single source of truth)
- Discriminator matches `METEORA_SWAP2_DISCRIMINATOR`
- Account flags: only `user` is signer; all others readonly
- Bin arrays are dynamically 2..=4 contiguous arrays
- No hardcoded reserve values: on-chain program reads live vault balances

## 3.7 Summary of Phase 3 Findings

### Correctness
- DLMM math is well-structured with exact integer arithmetic (U256, Q64.64)
- All overflow paths return typed errors (`DlmmError`)
- No floating point in raw token accounting
- Bin array indexing correctly uses `div_euclid(70)`
- Price calculation `get_price_from_id` verified with test fixtures
- Fee computation capped at 10% `MAX_FEE_RATE`
- Multi-bin traversal correctly walks bins in swap direction

### Issues Identified

1. **92% safety factor in `scale_dlmm_three_hop_input`**: 
   - Currently an approximation replacing actual bin traversal
   - Must be validated against `quote_dlmm_multi_bin` differential tests
   - Recommendation: replace with actual traversal, or keep as documented conservative cap

2. **No independent SDK differential testing**:
   - No comparison against official Meteora SDK
   - Required before production readiness

3. **No property-based tests** for numeric edge cases across full DLMM range

4. **Bin array version skew risk**: `METEORA_BINS_PER_BIN_ARRAY=70` must match on-chain program; mismatch silently targets wrong BinArrays

### Required Actions

1. **Implement DLMM differential testing** against Meteora SDK or reference implementation
2. **Validate or replace the 92% safety factor** with actual bin traversal
3. **Add property-based tests** for DLMM numeric edge cases
4. **Verify METEORA_BINS_PER_BIN_ARRAY** against deployed program deployment data
5. **Document all hardcoded values** with classification (invariant vs config vs deprecated)

## 3.8 Hardcoded Value Classification (DLMM-specific)

| Value | Classification | justification |
|---|---|---|
| `METEORA_BINS_PER_BIN_ARRAY = 70` | Protocol invariant | Must match on-chain `BINS_PER_ARRAY`; wrong value silently breaks swaps |
| `METEORA_BIN_STEP` (per-pool) | Operational configuration | Read from on-chain LbPair; varies per pool |
| `METEORA_SWAP2_DISCRIMINATOR` | Protocol invariant | Fixed by program discriminator; must match exactly |
| `MAX_FEE_RATE = 100_000_000` (10%) | Protocol invariant | On-enforced cap by Meteora program |
| `92%` safety factor | Deprecated/To-be-replaced | Approximation; should be replaced with actual traversal |
| `ONE_Q64 = 1 << 64` | Mathematical constant | Fixed-point representation base |
| `BASIS_POINT_MAX = 10_000` | Mathematical constant | 100% = 10_000 bps |
| `FEE_PRECISION = 1_000_000_000` | Mathematical constant | 10^9 units for fee rates |