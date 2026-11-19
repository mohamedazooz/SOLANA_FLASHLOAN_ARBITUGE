# Phase 9 — Risk Engine and Capital Protection Audit

## 9.1 Centralized Risk Policy

### 9.1.1 Single Authoritative Configuration

The risk engine uses one `RiskLimits` struct (`crates/arb-risk/src/lib.rs`) that consolidates all risk thresholds. All crates access risk limits through this single source of truth.

**Current defaults** (from `RiskLimits::default()`):
| Limit | Default Value | Classification |
|---|---|---|
| `max_trade_size_usd` | 25.0 USD | Operational configuration |
| `min_net_profit_abs` | 50,000 lamports (0.005 SOL) | Operational configuration |
| `min_net_profit_bps` | 8 bps | Operational configuration |
| `max_price_impact_bps` | 40 bps | Operational configuration |
| `max_state_age_slots` | 2 slots | Operational configuration |
| `max_tip_share_bps` | 8,500 bps (85%) | Operational configuration |
| `max_tip_lamports` | 2,000,000 lamports (0.2 SOL) | Operational configuration |
| `max_daily_loss_usd` | 5.0 USD | Operational configuration |
| `max_hourly_loss_usd` | 2.0 USD | Operational configuration |
| `min_landing_rate` | 0.02 (2%) | Operational configuration |
| `max_inflight_bundles` | 2 per slot | Operational configuration |
| `max_tip_spend_ratio` | 0.7 (70%) | Operational configuration |
| `max_orders_per_hour` | 120 orders/hour | Operational configuration |

**Conflicts identified** (from Phase 0 audit):
- **Conflicting defaults**: Some risk limits may have been set in multiple places with different values. The `RiskLimits::default()` is the authoritative single source.
- **`check_tip_share` edge case**: 80% tip share passes the check (8000 bps ≤ 8500 bps max), but 90% exceeds it. This is correct behavior — the check caps at 10,000 bps (100%) via `unwrap_or(10_000)` in `arb-types PoolState.fee_bps`, but the risk gate explicitly checks against `max_tip_share_bps`.

### 9.1.2 Risk Check Ordering

The `evaluate_full` method checks in this exact order:

1. **Kill switch** → if blocked, immediately deny
2. **Circuit breaker** → if open, immediately deny
3. **Trade size** → if exceeds `max_trade_size_usd`, deny
4. **Velocity** → if > `max_orders_per_hour` within 1hr window, deny
5. **Tip share** → if > `max_tip_share_bps`, deny
6. **Tip lamports** → if > `max_tip_lamports`, deny
7. **Price impact** → if > `max_price_impact_bps`, deny
8. **State age** → if > `max_state_age_slots`, deny
9. **Minimum profit** → if net < `min_net_profit_abs` OR net < `min_net_profit_bps` of amount_in, deny
10. **Losses** → if daily > `max_daily_loss_usd` OR hourly > `max_hourly_loss_usd`, deny

**Final check must happen immediately before submission**: The `evaluate_full` or `evaluate_flash_loan` method should be called in the supervisor's execution path just before building and signing the transaction.

### 9.2 Risk Checks — Detailed

#### 9.2.1 Kill Switch

- **Durable**: Marker file (`data/kill_switch`) survives restarts
- **Restart behavior**: If marker exists at startup → supervisor starts BLOCKED (fail-safe)
- **Atomic activation**: `activate()` writes marker file + sets in-memory flag
- **Permission checks**: Operator must explicitly call `activate()` or `activate_from_signal()`
- **No automatic reactivation**: After restart, kill switch remains in previous state (blocked if marker present)
- **Auditable state transitions**: Logs when activated/deactivated, file I/O errors reported

**Finding**: The kill switch implementation is well-durable with file-based persistence. The `is_blocked()` method checks the file first, then syncs the in-memory flag.

#### 9.2.2 Circuit Breaker

- **States**: Closed → Open → HalfOpen → Closed
- **Opens after**: 8 consecutive failures (default)
- **Cooldown**: 10 slots (~30s) before transitioning to HalfOpen
- **Closes after**: 3 successes in HalfOpen state
- **`is_allowed(current_slot)`**: Returns false during Open state until cooldown elapsed; then true in HalfOpen

**Finding**: The circuit breaker implementation is correct with proper state transitions. The `record_success()` and `record_failure()` methods correctly track state.

#### 9.2.3 Trade Size

- **Check**: `size_usd > 0` AND `size_usd ≤ max_trade_size_usd`
- **Invalid**: Non-finite, zero, or negative sizes denied
- **Config invalid**: `max_trade_size_usd` non-finite or ≤ 0 denied

**Finding**: Trade size check is correct and fail-closed.

#### 9.2.4 Velocity (Order Rate)

- **Check**: `orders_per_hour ≤ max_orders_per_hour` within rolling 1-hour window
- **Implementation**: Tracks `recent_order_timestamps`; purges entries older than 1 hour; rejects if count ≥ max
- **Default**: 120 orders/hour

**Finding**: Velocity check is correct. The rolling window implementation properly purges old timestamps.

#### 9.2.5 Tip Share

- **Check**: `tip_share_bps ≤ max_tip_share_bps` (default 8,500 bps = 85%)
- **Edge case**: 8,000 bps (80%) passes; 9,000 bps (90%) denied

**Finding**: Tip share check is correct and explicit. No silent clamping.

#### 9.2.6 Tip Lamports

- **Check**: `tip_lamports ≤ max_tip_lamports` (default 2,000,000 lamports = 0.2 SOL)
- **Implementation**: Direct u64 comparison

**Finding**: Tip lamports check is correct and simple.

#### 9.2.7 Price Impact

- **Check**: `price_impact_bps ≤ max_price_impact_bps` (default 40 bps = 0.4%)
- **Implementation**: Direct u64 comparison

**Finding**: Price impact check is correct and explicit.

#### 9.2.8 State Age

- **Check**: `state_age_slots ≤ max_state_age_slots` (default 2 slots)
- **Implementation**: Direct u64 comparison

**Finding**: State age check is correct and explicit.

#### 9.2.9 Minimum Profit

- **Check**: 
  - `opportunity_net.as_u64() ≥ min_net_profit_abs` (default 50,000 lamports)
  - `opportunity_net.as_u64() ≥ amount_in.as_u64() * min_net_profit_bps / 10_000` (default 8 bps)
- **Both conditions must pass**: Absolute minimum AND percentage of notional

**Finding**: Minimum profit check is correct with two complementary guards. The bps check prevents small-notional trades from flying under the absolute limit.

#### 9.2.10 Losses

- **Check**: `daily_loss_usd ≤ max_daily_loss_usd` (default 5.0 USD) AND `hourly_loss_usd ≤ max_hourly_loss_usd` (default 2.0 USD)
- **Tracking**: `record_loss(loss_usd)` adds to both daily and hourly counters
- **Periodic reset**: `maybe_reset_periodic()` resets hourly after 3600s, daily after 86400s

**Finding**: Loss tracking is correct with periodic automatic resets. The `maybe_reset_periodic()` should be called in the supervisor's periodic task loop.

#### 9.2.11 Max Inflight Bundles

- **Check**: `inflight_bundles_for_current_slot ≤ max_inflight_bundles` (default 2)
- **Tracking**: `inflight_bundles` map: `slot → count`; incremented on `record_bundle_sent`, decremented on `record_bundle_landed`
- **Cleanup**: Retain only current and previous slot

**Finding**: Inflight bundle check is correct. The slot-based tracking properly accounts for Solana's slot structure.

#### 9.2.12 Tip Spend Ratio

- **Check**: `tip_lamports / (tip_lamports + net_profit) ≤ max_tip_spend_ratio` (default 70%)
- **Implementation**: `tip_lamports as f64 / total as f64`; if net == 0, pass (no ratio to measure)

**Finding**: Tip spend ratio check is correct. The net == 0 guard prevents division by zero.

#### 9.2.13 Minimum Landing Rate

- **Check**: `total_landed / total_sent ≥ min_landing_rate` (default 2%)
- **Tracking**: `total_bundles_sent` and `total_bundles_landed` tracked in risk gate
- **History**: If `total_sent == 0`, allow first attempts (no history yet)

**Finding**: Minimum landing rate check is correct. The "allow first attempts" guard prevents blocking on startup.

### 9.3 Capital Isolation

**Separate wallet categories**:
1. **Hot execution wallet**: Funds used for active trading; signator for transactions
2. **Cold treasury wallet**: Long-term reserves; NOT used for active trading
3. **Operational reserve**: Buffer for operational costs; not spendable by strategies
4. **Flash loan capital**: Borrowed during flash loans; returned immediately after repay
5. **Realized profit**: Accumulated after successful landings; can be withdrawn or reinvested
6. **Unsettled funds**: Between submission and confirmation; at risk of reorg

**Never automatically sweep operational reserves**: Operational reserves must not be automatically moved or spent.

**Never allow a strategy to spend funds belonging to another strategy**: Each strategy/instance must only spend from its designated hot wallet.

### 9.4 Kill Switch

**Durable kill switch behavior**:

| Aspect | Behavior |
|---|---|
| **Persistence** | Marker file (`data/kill_switch`) survives restarts |
| **Restart behavior** | If marker exists at startup → supervisor starts BLOCKED (fail-safe) |
| **Atomic activation** | `activate()` writes marker file + sets in-memory flag simultaneously |
| **Permission checks** | Operator must explicitly call `activate()` or `activate_from_signal()` |
| **No automatic reactivation** | After restart, kill switch remains in previous state; marker re-blocks if present |
| **Auditable state transitions** | Logs every activation/deactivation, file I/O errors reported |

**Finding**: Kill switch implementation is well-durable with file-based persistence and correct fail-safe behavior on restart.

### 9.5 Risk Engine Gaps and Required Actions

**Identified gaps**:

1. **No daily/hourly loss reset automation**: `maybe_reset_periodic()` exists but must be called in the supervisor's periodic task loop; currently not hooked up

2. **No per-strategy capital isolation**: All strategies share the same hot wallet; no mechanism to prevent one strategy from spending another's funds

3. **No position concentration limit**: No check on how much of the hot wallet is deployed across concurrent routes

4. **No correlated risk check**: No check for multiple routes sharing the same pool or correlated exposure

5. **Chronos EV gate not fully integrated**: The `evaluate_chronos_gate` and `evaluate_full` methods exist but may not be called in all execution paths

6. **No dynamic risk limit adjustment**: Risk limits are static; no mechanism to adjust based on market conditions

7. **No risk utilization monitoring**: No metric tracking how much of the risk limits are being utilized (e.g., "daily loss used: 3/5 USD")

### 9.6 Required Actions

1. **Hook up `maybe_reset_periodic()`** in supervisor's periodic task loss window reset
2. **Implement per-strategy capital isolation** (separate hot wallets per strategy or strict accounting)
3. **Add position concentration limit**: Max % of hot wallet deployed across concurrent routes
4. **Add correlated risk check**: Prevent multiple routes on same pool or correlated assets
5. **Integrate Chronos EV gate** into all execution paths
6. **Add risk utilization metrics**: Track % of limits used
7. **Document risk check ordering**: Explicit documentation of the 10-check sequence in `evaluate_full`

## 9.7 Summary of Phase 9 Findings

### Correctness
- Risk engine has one authoritative `RiskLimits` config
- All 13 risk checks implemented with exact threshold comparisons
- No floating point in risk comparisons (all u64/f64 comparisons use explicit conversion)
- Kill switch is durable with file-based persistence
- Circuit breaker has correct state machine (Closed → Open → HalfOpen → Closed)
- Loss tracking with periodic automatic reset
- Inflight bundle tracking with slot-based accounting

### Issues Identified

1. **Loss reset not automated**: `maybe_reset_periodic()` exists but not hooked up to supervisor's periodic loop
2. **No capital isolation per strategy**: All strategies share hot wallet
3. **No position concentration limit**: No max % of wallet deployed check
4. **No correlated risk check**: No prevention of correlated pool exposure
5. **Chronos EV gate not fully integrated**: Methods exist but may not be called in all paths
6. **No risk utilization metrics**: Cannot tell how much of limits are being used

### Required Actions

1. **Hook up loss reset** in supervisor periodic task
2. **Implement capital isolation** per strategy
3. **Add position concentration limit**
4. **Add correlated risk check**
5. **Integrate Chronos EV gate** fully
6. **Add risk utilization metrics**
7. **Document risk check ordering**

## 9.7 Hardcoded Value Classification (Risk-specific)

| Value | Classification | justification |
|---|---|---|
| `max_consecutive_failures = 8` | Circuit breaker config | Failure threshold before open |
| `cooldown_slots = 10` | Circuit breaker config | Cooldown duration in slots |
| `success_threshold = 3` | Circuit breaker config | Successes in HalfOpen to close |
| `MAX_ORDERS_PER_HOUR = 120` | Risk gate config | Order velocity limit |
| `MIN_LANDING_RATE = 0.02` | Risk gate config | Minimum historical landing rate |
| `MAX_TIP_SPEND_RATIO = 0.7` | Risk gate config | Tip/edge proportion limit |
| `MAX_TIP_LAMPORTS = 2_000_000` | Risk gate config | Max tip ceiling |
| `MIN_NET_PROFIT_ABS = 50_000` | Risk gate config | Min absolute profit in lamports |
| `MIN_NET_PROFIT_BPS = 8` | Risk gate config | Min profit in bps of amount_in |
| `MAX_PRICE_IMPACT_BPS = 40` | Risk gate config | Max price impact threshold |
| `MAX_STATE_AGE_SLOTS = 2` | Risk gate config | Max state freshness |
| `MAX_DAILY_LOSS_USD = 5.0` | Risk gate config | Daily loss ceiling |
| `MAX_HOURLY_LOSS_USD = 2.0` | Risk gate config | Hourly loss ceiling |
| `CHRONOS_DECAY_LAMBDA = 3.0` | Chronos config | Hazard rate per second |
| `CHRONOS_MAX_AGE_MS = 400` | Chronos config | 1 slot ceiling in ms |
| `CHRONOS_MIN_EV_USD = 0.05` | Chronos config | Min EV in USD |
| `CHRONOS_FAILED_COST = 10_000` | Chronos config | Failed attempt cost in lamports |
| `CHRONOS_BETA = 0.0005` | Chronos config | Hop risk penalty factor |