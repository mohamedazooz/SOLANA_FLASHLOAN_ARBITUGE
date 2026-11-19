# Phase 2 — Architecture and Economic Audit

## 2.1 Data Flow Audit

### 2.1.1 Yellowstone gRPC → Account Decoding → Pool Registry → State Integrity

**Input**: gSSE subscription messages containing account update packets from Yellowstone gRPC.

**Output**: Decoded `PoolSnapshot` structures wrapped in the ingest layer's internal state.

**Data Owner**: `arb-ingest` crate — Yellowstone gRPC client and account decoder.

**Freshness Requirement**: 
- `PoolSnapshot.slot` must be ≥ last seen slot for the same pool
- `PoolSnapshot.write_version` must be ≥ last seen write_version for the same slot
- Out-of-order updates (older slot or lower write_version) are rejected

**Failure Behavior**:
- Malformed account data → deserialization error → account skipped, logged
- Generation change → filter re-derivation required → pending until rebuilt
- Dead slot → account invalidated, tracked in slot integrity

**Retry Behavior**:
- Connection drop → auto-resubscribe to gRPC
- Generation change → re-derive account filters, continue from fresh state
- No backpressure on gRPC stream; dropped messages are simply missed

**Observability**:
- `messages_received` (counter)
- `decode_errors` (counter)
- `stale_slot_rejected` (counter)
- `provider_lag` (gauge: current slot vs last seen slot per provider)

**Invariants**:
- All accounts in the registry have `write_version` monotonically increasing per bank
- No account carries a `slot` older than the provider's reported slot
- Every decoded pool has a non-empty `protocol` field

**Tests**: 
- Yellowstone mock server with deterministic account updates
- Deserialization fixtures from real mainnet account snapshots
- Generation change simulation (inject generation change, verify filter re-derivation)

### 2.1.2 Provider Supervisor → Quote Engine

**Input**: Provider health pings, slot lag measurements, freshness timestamps.

**Output**: Selected primary + fallback provider endpoints with health status.

**Data Owner**: `arb-ingest::provider_supervisor` module.

**Freshness Requirement**:
- `max_slot_lag` (default 8): provider's slot must lag current by at most 8
- `stale_after` (default 3s): tip floor snapshot older than 3s treated as unavailable
- Provider must respond to health ping within expected latency

**Failure Behavior**:
- Single failure → record, continue with next provider
- 3 consecutive failures → Open Circuit + 2s cooldown, then test fallback
- All providers fail → trading Halted (mode → Shadow)

**Retry Behavior**:
- Exponential backoff on provider health checks
- After cooldown, test fallback provider before marking all unhealthy
- Never automatically resume live trading after halt without operator intervention

**Observability**:
- `provider_health_status` (gauge: healthy/unhealthy/cooldown)
- `provider_lag` (gauge: slot difference from current)
- `provider_switches` (counter: number of provider switches)
- `circuit_breaker_state` (state: Closed/Open/HalfOpen)

**Invariants**:
- Primary and fallback must be distinct URLs/endpoints
- After Open Circuit, cooldown must complete before fallback testing
- No provider may have slot_lag > max_slot_lag while selected

**Tests**:
- Provider health check with simulated lag injection
- Failover sequence: primary fails 3× → cooldown → fallback tested
- Circuit breaker cooldown timing verification
- Concurrent health checks don't corrupt state

### 2.1.3 Quote Engine → Route Discovery → Profitability Evaluation

**Input**: Pool map (fresh state age ≤ max_state_age_slots) + route request (input amount, base mint).

**Output**: `SizedOpportunity` with full cost breakdown and profit calculation.

**Data Owner**: `arb-engine` route discovery + `arb-math` economic model.

**Freshness Requirement**:
- All pool snapshots must have `slot` within `max_state_age_slots` (default 2)
- Pool `write_version` must be current
- Stale pools → route rejected, re-discovered on next refresh

**Failure Behavior**:
- Missing pool → hop rejected, route pruned
- Insufficient liquidity → `min_amount_out` too low → route rejected
- Invalid fee (denominator == 0) → pool skipped entirely
- Price impact > max → route rejected

**Retry Behavior**:
- Re-quote on next data refresh (every `discovery_refresh_secs` = 15s)
- Route cache TTL (default 15s) — same route not re-evaluated within TTL
- If route rejected, try alternative paths through different pools

**Observability**:
- `quote_latency` (histogram: seconds)
- `quote_errors` (counter by error type)
- `routes_discovered` (counter)
- `routes_rejected` (counter by rejection reason)
- `profitability_evaluated` (counter)
- `profitability_passed` (counter)
- `profitability_rejected` (counter)

**Invariants**:
- `hop_inputs[i]` must equal the expected output of hop `i-1` (continuity check)
- Final output mint must match input mint for triangular cycles
- `net_profit ≥ min_net_profit_abs (50,000 lamports) AND net_profit ≥ min_net_profit_bps × amount_in (8 bps)`
- No double-counted fees: each cost entry in ledger has exactly one `included_in_quote` flag

**Tests**:
- Differential tests against Meteora SDK and Kamino reference implementation
- Property-based tests for route continuity (mint matching across hops)
- Stale pool rejection: inject old snapshots, verify route rejection
- Fee validity check: pool with fee_denominator == 0 → route rejected

## 2.2 Economic Correctness

### 2.2.1 Single Authoritative Net-Profit Model

The economic model in `crates/arb-math/src/economic.rs` provides the single authoritative net-profit calculation. The model follows this exact formula:

```
gross = final_received - borrow_principal
total_costs = flash_fee + dex_fees + transfer_fees + account_creation_cost + other_verified_costs + network_fee + priority_fee + jito_tip

if gross >= 0:
    net = gross - total_costs
    if net >= 0:
        signed_net = net
    else:
        signed_net = -(gross + total_costs)  // loss
else:
    gross is loss
    signed_net = -(gross + total_costs)  // total loss including gross loss

net_profit_raw = max(0, signed_net)  // clamped to 0 if negative
minimum_net_profit_signed_raw = signed net using minimum_final_amount_raw
```

**Key Properties**:
1. **Exact integer arithmetic**: All token amounts in raw atomic units (u64/u128). No floating point in raw token accounting.
2. **Checked operations**: `checked_add`, `checked_sub`, `checked_mul` used throughout. Overflow → `Err(ArbError::Overflow)` → quote rejected.
3. **Explicit conversion**: SOL-denominated costs (network, priority fee, Jito tip) converted to base-mint units via verified rational rate `rate_num / rate_denom`.
4. **Rounding policy**: 
   - `lamports_to_base_raw`: `ceil(lamports * rate_num / rate_denom)` — remainder adds 1
   - `calculate_flash_fee_raw`: `ceil(borrow * sf / 2^64)` — ceiling division so repayment never understated
5. **No silent clamping**: Invalid inputs return `Err()` instead of saturating to max values.

**Cost Tracking**:
Each cost is tracked in the `CostLedger` with `included_in_quote` and `deducted_already` flags to prevent double-counting and double deduction.

**Separate Tracking**:
- `gross_profit` = final_received - borrowed_principal (before costs)
- `expected_net_profit` = gross - all costs (using expected output)
- `minimum_net_profit` = gross_min - all costs (using minimum output, more conservative)
- `realized_net_profit` = after actual execution (recorded in ledger)

### 2.2.2 Cost Fields per Route

For every route, the following costs are calculated exactly once:

| Cost | Source | Unit | Arithmetic |
|------|--------|------|------------|
| Dex swap fees | Read from pool `fee_numerator/fee_denominator` | Token raw | `amount * fee_numerator / fee_denominator` |
| Flash loan fee | Read from Kamino `ReserveConfig.fees.flash_loan_fee_sf` (Q64.64) | Token raw | `ceil(borrow * sf / 2^64)` |
| Base transaction fee | RPC-estimated or fixed | Lamports → converted via rate | `lamports_to_base_raw(lamports, rate_num, rate_denom)` |
| Priority fee | Dynamic, based on CU price | Lamports → converted via rate | Same conversion |
| Jito tip | From tip oracle at bid time | Lamports → converted via rate | Same conversion |
| Rent ATA creation/closure | Derived from account data size | Token raw or lamports | Conversion as above |
| Other verified costs | Protocol-specific (e.g. transfer fees) | Token raw | Read from on-chain data |

**Never double-counted**: Each cost has exactly one entry in the `CostLedger` with `included_in_quote` flag. The `CostLedger.total_included_not_deducted()` sums these exactly once.

### 2.2.3 What Must NOT Happen

1. **Do not use floating point in raw token accounting**: All raw amounts are u64/u128. `f64` may only be used for UI display after conversion.
2. **Do not silently use synthetic liquidity**: All reserves must come from on-chain decodes.
3. **Do not use arbitrary 92% scaling as replacement for actual bin traversal**: Meteora DLMM price must be computed from actual bin prices.
4. **Do not authorize execution using gross profit**: Only `minimum_net_profit_signed_raw > 0` after ALL costs clears the gate.
5. **Do not double-count flash loan fees**: The fee is calculated once from `calculate_flash_fee_raw(borrow, sf)` and appears in exactly one `CostLedger` entry.

## 2.3 Independent Economics Verification

### 2.3.1 Second Implementation Reference

Build independent tests that calculate the same route using a second implementation or reference SDK.

**Comparison Points** (for each quote compare):
- Input amount (base_mint raw units)
- Expected output (expected_final_amount_raw)
- Minimum output (minimum_final_amount_raw)
- Flash loan fee (flash_loan_fee_raw)
- Dex fees (dex_fees_raw)
- Transfer fees (transfer_fees_raw)
- Network fee (network_fee_lamports → converted)
- Priority fee (priority_fee_lamports → converted)
- Jito tip (jito_tip_lamports → converted)
- Rounding at each stage
- Price impact at each hop
- Intermediate account balances after each swap
- Final profit (net_profit_raw)
- Sign of net profit (positive/negative)

### 2.3.2 Quote Mismatch Policy

A quote mismatch must be **explained, not hidden** behind a safety haircut. The policy is:

1. **Mismatch detected** → log detailed comparison (all fields above)
2. **Mismatch explained** → document reason (different SDK version, different bin array traversal, different fee reading, different rounding)
3. **Mismatch NOT explained** → flag as protocol compatibility blocker, gate execution
4. **Safety haircut applied** → only after explicit operator review and documented rationale

**Never** use a "conservative margin" to mask a mismatch. If the two implementations disagree on output amount, the mismatch must be resolved by:
- Verifying both against on-chain data
- Checking which SDK version is newer/more accurate
- Documenting the difference and choosing the more reliable source
- Filing a protocol compatibility issue if the difference stems from a protocol upgrade

### 2.3.3 Independent Verification Tests

The following test categories must exist and pass:

| Test Category | Description |
|---|---|
| `dlmm_differential` | Compare arb-math DLMM quote against official Meteora SDK or independently verified reference. Test both swap directions, different bin steps, positive/negative bin arrays, sparse/deep liquidity, fee variations, large/small inputs, boundary conditions, insufficient liquidity. |
| `kamino_differential` | Compare Kamino flash loan fee calculation against Kamino reference implementation. Test missing reserve, wrong owner, wrong discriminator, wrong mint, inactive reserve, invalid fee, insufficient liquidity, overflow, wrong borrow index, incorrect instruction order, missing repay instruction. |
| `econ_differential` | Compare `build_economic_quote` against a second economic model implementation. Test basic positive profit, below-floor, double counting, USDC rational rate conversion, extreme loss signedness, min profit in base units, conversion rounding. |
| `route_continuity` | Verify mint continuity across hops: hop_1.output_mint == hop_2.input_mint; triangular cycle: final output mint == input mint. |
| `edge_case_numerical` | Test numeric edge cases: u64::MAX inputs, zero amounts, overflow boundaries, exact boundary conditions (gamma = D0 in CPMM), tick array boundaries in CLMM. |

### 2.3.4 Current Verification Status

- **DLMM differential**: Framework exists (test fixtures planned), actual SDK comparison not yet implemented in this codebase.
- **Kamino differential**: Flash fee `calculate_flash_fee_raw` verified against Q64.64 math (5 test cases pass: 5% sf, sf=0, mainnet sf=11_529_215_046_068, full-scale ceilings to loan, identity).
- **Economic differential**: All 11 arb-math test suites pass, including double-counting detection, signed-loss handling, WSOL identity conversion, and base/raw rounding.
- **Route continuity**: Not yet automated; manual verification recommended for every discovered route.

## 2.4 Summary of Phase 2 Findings

### Correctness
- Economic model is well-structured with exact integer arithmetic and checked operations.
- Flash fee calculation verified against Q64.64 mainnet values.
- No floating point in raw token accounting.
- Good separation of gross/profit/minimum_net.

### Gaps Identified
1. **No independent SDK differential testing** for Meteora DLMM or Orca CLMM quotes.
2. **No automated route continuity tests** (mint matching across hops).
3. **No Kamino flash loan instruction sequence verification** against deployed program (instruction ordering, borrow index consistency).
4. **No property-based tests** for numeric edge cases across all math modules.
5. **No integration tests** (require RPC infrastructure).

### Required Actions
1. Implement DLMM differential testing against Meteora SDK.
2. Implement Kamino instruction sequence verification.
3. Add property-based tests for all math modules.
4. Set up mock RPC infrastructure for integration tests.
5. Document all hardcoded values and classify them (Protocol invariant / Operational config / Dynamic market observation / Security secret / Mathematical constant / Test fixture / Deprecated/unused).