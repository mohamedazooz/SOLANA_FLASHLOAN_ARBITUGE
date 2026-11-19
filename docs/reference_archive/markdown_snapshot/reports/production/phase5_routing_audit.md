# Phase 5 — Routing and Arbitrage Engine Audit

## 5.1 Route Correctness

### 5.1.1 Protocol Verification

Support only routes whose protocols have verified adapters. Every hop must pass the `execution_capability` check before reaching the transaction builder.

**For every hop verify**:
- **Input mint**: Must match pool's `base_mint` or `quote_mint` (depending on `a_to_b` direction)
- **Output mint**: Must connect to the next hop's input mint (mint continuity)
- **Pool identity**: `pool_id` must match a known, registered pool
- **Program owner**: Must match the protocol's expected program ID (from `arb-constants`)
- **Fee**: Must be valid (`fee_denominator != 0`, `is_fee_valid() == true`)
- **Liquidity**: Reserves must > 0 (per-protocol checks)
- **State slot**: Must be within `max_state_age_slots` (default 2)
- **Token program**: Must be SPL Token or Token-2022 (per-adapter validation)
- **Supported execution capability**: Must return `Ok(())` from `execution_capability()`

### 5.1.2 Mint Continuity

**Full mint continuity check**:
- Hop 1 output mint == Hop 2 input mint
- Hop 2 output mint == Hop 3 input mint
- Final output mint must match the original input mint for a triangular cycle

**Code verification**: In `quote_route` (adapters.rs lines 217-268):
```rust
let a_to_b = pool.base_mint == hop.mint_in;
// mint continuity check
if !a_to_b && pool.quote_mint != hop.mint_in {
    return Err(ArbError::InvalidInput(
        "hop input mint does not match pool".into(),
    ));
}
```

**Gap**: The continuity check only validates hop_i input against pool_i. It does NOT automatically verify that hop_i.output_mint == hop_{i+1}.input_mint. This must be checked by the caller.

### 5.1.3 Triangular Cycle Validation

For a 3-hop triangular arbitrage cycle:
- Hop 1: input_mint₀ → output_mint₁
- Hop 2: input_mint₁ → output_mint₂
- Hop 3: input_mint₂ → output_mint₀ (must match original input)

**Verification**: After `quote_route` completes, the caller must verify:
```rust
assert_eq!(route.hops[0].mint_in, route.hops[2].mint_out_equivalent);
// or explicitly: final_output_mint == initial_input_mint
```

**Current code does not enforce this**: The `RouteQuote` struct carries `amount_in` and `amount_out` but not mint continuity verification.

## 5.2 Route Enumeration

### 5.2.1 Supported Protocols

Currently supported protocols and their adapter status:

| Protocol | Execution Capability | Status |
|---|---|---|
| Raydium V4 | `execution_capability` validates fee + reserves | Always Ok |
| Raydium CPMM | `execution_capability` validates fee + reserves > 0 | Always Ok |
| Raydium CLMM | `execution_capability` validates CLMM state decoded | Requires `pool.clmm.is_some()` |
| Orca Whirlpool | `execution_capability` requires CLMM state | Requires `pool.clmm.is_some()` |
| Meteora DLMM | `execution_capability` validates bins + params | Requires `pool.dlmm.is_some()` + bins not empty + params Some |
| Pump.fun | `execution_capability` returns `Err` (quote-only) | Cannot reach transaction builder |
| PumpSwap | `execution_capability` returns `Err` (quote-only) | Cannot reach transaction builder |

### 5.2.2 Route Enumeration Constraints

**No repeated pool where prohibited**: 
- A route must not use the same pool twice in succession without a valid economic reason
- The `quote_route` function actually handles this by mutating pool state sequentially — using the same pool twice will quote against its already-advanced state, losing to fees

**No invalid cycles**:
- Cycles that don't close (final output mint ≠ initial input mint) must be rejected
- The onus is on the caller to verify triangular closure

**No zero-value hops**:
- `amount_in == 0` → rejected by `quote_hop` → `InvalidInput("zero input")`

**No unsupported protocol**:
- Any protocol whose `execution_capability` returns `Err` → route rejected at hop level

**No stale pool**:
- Pool snapshot slot must be within `max_state_age_slots`

**No invalid token extension**:
- `has_risky_token || transfer_fee_bps != 0` → rejected per-hop (execution_capability + quote_hop)

**No duplicate route suppression of valid orientations**:
- Both swap directions (X→Y and Y→X) should be enumerable as separate routes
- The engine should not drop reverse-direction opportunities

### 5.2.3 Directed Graph Construction

The route discovery builds a directed graph of pool connections. Key invariants:

1. **Edge direction**: Each pool edge has a direction (base→quote or quote→base), determined by `a_to_b = pool.base_mint == hop.mint_in`
2. **Mint continuity**: Path through the graph must maintain mint continuity (output of one hop = input of next)
3. **No self-loops**: A pool should not connect to itself in a way that creates a zero-length cycle
4. **Graph connectivity**: The graph must have at least one path from input mint to output mint that forms a triangular cycle

**Gap**: The code in `arb-engine/src/bellman_ford.rs` (if it exists) or the route discovery logic must ensure these graph invariants. The current `quote_route` mutates pool state sequentially but doesn't validate the full cycle closure.

## 5.3 Route Depth

### 5.3.1 Depth Constraints

The system supports 2-hop and 3-hop routes:

| Hop Count | Compute Budget | Max Pools |
|---|---|---|
| 2-hop | `COMPUTE_BUDGET_LIMIT_2HOP = 260,000` | Configurable (default 1000) |
| 3-hop | `COMPUTE_BUDGET_LIMIT_3HOP = 320,000` | Configurable (default 1000) |

**Do not estimate route profitability from pool reserves alone.**

**Must account for**:
- Actual executable liquidity (reserves > 0, fee valid)
- Bin traversal (for DLMM hops)
- Tick traversal (for CLMM hops)
- Fee tiers (per-pool fee_bps)
- Price impact (computed across hops)
- Slippage (min_amount_out derived from expected_out)
- Account availability (instruction account limits)
- Transaction account limits (MTU 1232 bytes)
- Compute unit consumption (per-hop estimates + safety buffer)
- Flash loan availability (if use_flash_loan = true)

### 5.3.2 Profitability Calculation

Profitability must be calculated using the **full economic model**:

```
gross_profit = final_received - borrowed_principal
total_costs = flash_fee + dex_fees + transfer_fees + base_fee + priority_fee + jito_tip + rent + other
net_profit = gross_profit - total_costs
```

**Never authorize execution using gross profit.** Only `minimum_net_profit_signed_raw > 0` after ALL cost deductions clears the gate.

### 5.3.3 Route Depth Recommendations

1. **2-hop routes**: Prefer when price impact is low and liquidity is deep. Fewer instructions = lower CU cost and lower landing risk.

2. **3-hop routes**: Use when 2-hop cannot find profitable opportunities within the capital constraints. Account for the additional CU cost (~35k per hop) and additional tip/priority fee.

3. **Do not split routes simply because price impact exceeds a static threshold**. Split only when:
   - Parallel pool independence is verified (no shared writable account conflicts)
   - Capital allocation across splits is well-defined
   - Combined price impact + additional costs still yields positive expected net profit

## 5.4 Split Routing

### 5.4.1 Current Implementation

The `maybe_split_allocation` function in `arb-engine/src/route_worker.rs` implements split routing with these conditions:

- Only splits when `price_impact_bps > max_price_impact_bps` (default 40 bps)
- Requires ≥ 2 parallel pools
- Skips zero-reserve/fee-invalid pools

### 5.4.2 Economic and Execution Model Verification

**Must verify**:
1. **Parallel pool independence**: No shared writable account conflicts between the split routes
2. **Capital allocation**: The input capital is divided among splits with well-defined allocation ratio
3. **Combined price impact**: The total price impact of all splits must be evaluated, not just individual impacts
4. **Additional instruction cost**: Each split adds instructions → more CU consumption → may require higher CU budget
5. **Additional account count**: Each split adds accounts → may exceed transaction MTU (1232 bytes) or account limit
6. **Final net profit**: After ALL additional costs (tips, priority fees, CU costs), the split must still yield positive expected net profit

**Do not split a route simply because price impact exceeds a static threshold**. The split must be economically justified.

### 5.4.3 Split Routing Test Coverage

**Existing tests** (arb-engine/tests/pipeline.rs):
- ✓ `fees_turn_a_flat_market_into_a_zero_profit_cycle` — cycle detection
- ✓ `graph_detects_the_spread_and_reports_net_below_gross` — spread reporting
- ✓ `risky_and_inactive_pools_never_reach_the_graph` — pool filtering
- ✓ `graph_without_economic_config_fails_closed` — config-gated routing
- ✓ `cycle_profit_peaks_at_an_interior_size` — profit peak analysis

**Missing tests**:
- ❌ Split routing economic validation (does split actually improve net profit after additional costs?)
- ❌ Split routing CU impact (does split exceed compute budget?)
- ❌ Split routing account count (does split exceed MTU?)
- ❌ Split routing with Token-2022 mints
- ❌ Split routing with DLMM hops (bin traversal complexity)

## 5.5 Route Continuity and Mint Tracking

### 5.5.1 Gap Analysis

The `quote_route` function validates that each hop's input mint matches its pool, but does **not** automatically verify that:

```
hop[0].mint_in → hop[0].pool → hop[0].mint_out == hop[1].mint_in → hop[1].pool → hop[1].mint_out == ... → hop[n].mint_out == initial_mint_for_triangular
```

**Required**: Caller must add explicit mint continuity verification after `quote_route` returns.

### 5.5.2 Mint Tracking Mechanism

The `ExecutionOpportunity` struct carries:
```rust
pub hop_inputs: Vec<u64>, // Per-hop expected input amounts
// hop_inputs[hops.len()] is the final expected output
```

**This is used to derive a correct `min_amount_out`** for each swap leg. However, the mint continuity check is separate and must be implemented by the caller.

## 5.6 Summary of Phase 5 Findings

### Correctness
- Route enumeration correctly supports supported protocols with execution capability checks
- Mint continuity is partially checked (per-hop input validation) but not automatically verified across the full cycle
- Compounded price impact calculated as `1 - prod(1 - impact_i)` — correct formula
- Split routing implemented with price-impact threshold but lacks economic validation
- CPMM/CLMM/DLMM quote functions all use exact integer arithmetic with checked operations
- Token-2022 rejection without verified per-mint schedule (intentional fail-closed)

### Issues Identified

1. **No automatic mint continuity verification** across full route cycle
   - `quote_route` validates per-hop input but not hop_i.output_mint == hop_{i+1}.input_mint
   - Caller must add explicit verification

2. **Hardcoded borrow index = 2** in Kamino (Phase 4 issue carries over)
   - Not specific to routing but affects flash loan routes

3. **Split routing lacks economic validation**
   - `maybe_split_allocation` splits on price impact threshold alone
   - Must verify combined net profit after additional costs

4. **Route depth not accounted for in profitability gate**
   - Economic model checks net profit but doesn't differentiate 2-hop vs 3-hop CU costs

5. **Pump.fun and PumpSwap quote-only** (intentional, but documented)

6. **Orca/CLMM requires decoded state** in pool snapshot

7. **No property-based tests** for route enumeration edge cases

### Required Actions

1. **Add mint continuity verification** after `quote_route` — verify final output mint == initial input mint for triangular cycles
2. **Economic validation for split routing** — test that split routes improve net profit after additional CU/tip/fee costs
3. **Dynamic borrow index determination** — fix hardcoded index 2 in Kamino flash loan
4. **Property-based tests** for route enumeration: mint continuity, depth constraints, split economics
5. **Document all hardcoded values** with classification

## 5.7 Hardcoded Value Classification (Routing/Engine-specific)

| Value | Classification | justification |
|---|---|---|
| `COMPUTE_BUDGET_LIMIT_2HOP = 260_000` | Operational configuration | Safety buffer for 2-hop routes; not protocol invariant |
| `COMPUTE_BUDGET_LIMIT_3HOP = 320_000` | Operational configuration | Safety buffer for 3-hop routes; not protocol invariant |
| `MAX_COMPUTE_UNIT_LIMIT = 350_000` | Solana protocol cap | Hard limit; cannot be exceeded |
| `CU_BUFFER_PCT = 15` | Operational configuration | Safety buffer percentage |
| `SOLANA_PACKET_MTU_LIMIT = 1232` | Solana protocol cap | Packet size limit; hard constraint |
| `BASE_PRIORITY_FEE_MICRO_LAMPORTS = 1_000` | Operational configuration | Default priority fee; configurable |
| `DEFAULT_HOP_SLIPPAGE_BPS = 10` | Operational configuration | Per-hop slippage tolerance |
| `max_price_impact_bps = 40` | Operational configuration | Risk gate threshold; configurable |
| `max_state_age_slots = 2` | Operational configuration | State freshness gate |
| `min_net_profit_abs = 50000` | Operational configuration | Minimum absolute profit in lamports |
| `min_net_profit_bps = 5` | Operational configuration | Minimum profit in bps of amount_in |
| `max_trade_size_usd = 10000.0` | Operational configuration | Max trade size for risk gates |
| `max_tip_lamports = 2_000_000` | Operational configuration | Max Jito tip ceiling |
| `max_tip_share_bps = 7000` | Operational configuration | Max tip share of edge |
| `max_tip_spend_ratio = 0.7` | Risk gate configuration | Tip vs net proportion |
| `min_landing_rate = 0.02` | Risk gate configuration | Minimum historical landing rate |
| `max_inflight_bundles = 2` | Risk gate configuration | Concurrent bundle limit |
| `max_consecutive_failures = 8` | Circuit breaker configuration | Failure threshold before breaker opens |
| `cooldown_slots = 75` | Circuit breaker configuration | Cooldown window in slots |
| `graph_max_age_slots = 8` | Discovery configuration | Pool graph refresh horizon |
| `discovery_refresh_secs = 15` | Ingestion configuration | Pool discovery refresh interval |
| `discovery_min_tvl_usd = 25000.0` | Ingestion configuration | Minimum TVL floor for pool inclusion |