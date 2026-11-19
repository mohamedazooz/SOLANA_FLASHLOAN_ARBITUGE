# Centralized Risk Policy & Capital Protection

## 1. Authoritative Risk Policy
Risk evaluation is decoupled from route discovery and execution assembly. The 20-point risk check sequence in `crates/arb-risk/src/lib.rs` and `arb-bot/src/supervisor/` must pass in full immediately prior to bundle submission.

## 2. The 20 Pre-Execution Checks

| Check # | Name | Verification Method | Failure Outcome |
|---|---|---|---|
| **1** | Kill Switch | In-memory atomic + durable marker file check | REJECT (`KillSwitchActive`) |
| **2** | Global Live Authorization | Checks `ARB_LIVE=1` and `execution.live_enabled == true` | REJECT (`LiveDisabled`) |
| **3** | Circuit Breaker | Consecutive failure counter vs threshold; cooldown slots | REJECT (`CircuitBreakerOpen`) |
| **4** | Hot Wallet Reserve | Minimum operational SOL balance (default 0.1 SOL reserved) | REJECT (`InsufficientWalletBalance`) |
| **5** | Trade Size Ceiling | Borrow size $\le$ `MAX_TRADE_SIZE_USD` (Canary: $5-$10) | REJECT (`TradeSizeExceeded`) |
| **6** | Route Freshness | State age $\le$ `MAX_STATE_AGE_SLOTS` (max 2 slots) | REJECT (`StaleState`) |
| **7** | State Coherence | Update banks across hops belong to the same generation | REJECT (`SplitBrainState`) |
| **8** | Price Impact Ceiling | Combined price impact $\le$ `MAX_PRICE_IMPACT_BPS` | REJECT (`PriceImpactExceeded`) |
| **9** | Minimum Net Profit | Projected net profit $\ge$ `MIN_NET_PROFIT_USD` floor | REJECT (`BelowProfitHurdle`) |
| **10** | Tip Budget Cap | Tip $\le$ 30% of net profit AND $\le$ 0.01 SOL hard ceiling | REJECT (`TipCeilingExceeded`) |
| **11** | Total Cost Ratio | Total execution costs $\le$ `MAX_COST_RATIO` of gross | REJECT (`CostRatioExceeded`) |
| **12** | Daily Loss Limit | Cumulative 24h loss $\le$ daily limit (Canary: $15) | REJECT (`DailyLossLimitHit`) |
| **13** | Hourly Loss Limit | Cumulative 1h loss $\le$ hourly limit (Canary: $5) | REJECT (`HourlyLossLimitHit`) |
| **14** | Inflight Bundles | Concurrent pending bundles $\le$ `MAX_INFLIGHT_BUNDLES` | REJECT (`InflightCapacityReached`) |
| **15** | Provider Health | Ingestion slot lag $\le$ `MAX_SLOT_LAG` (max 3 slots) | REJECT (`ProviderLagged`) |
| **16** | Landing Probability | Expected probability $P(\text{land}) \ge \text{MIN\_PROBABILITY}$ | REJECT (`LowLandingProbability`) |
| **17** | Transaction Wire Size | Serialized packet $\le$ MTU (1,232 for V0; 4,096 for V1) | REJECT (`PacketOversized`) |
| **18** | Duplicate Execution | Identical route/slot hash in flight rejected | REJECT (`DuplicateOpportunity`) |
| **19** | Account Collision | Writable account locks do not conflict with inflight txs | REJECT (`AccountCollision`) |
| **20** | Final Dispatch Gate | Atomic re-check of blockhash freshness and balance | REJECT (`DispatchGateFailed`) |

## 3. Durable Kill Switch Invariant
- **Persistent State**: Persisted via marker file on disk (default: `/var/run/arb-bot/paused` or configured marker path).
- **Restart Immunity**: If the service restarts while the marker file exists, the bot initializes in a halted, fail-closed state.
- **Zero Hallucination Override**: Can only be disarmed by an authenticated operator via `/resume` endpoint with `WEB_ADMIN_PASSWORD` or explicit removal of the marker file.
