# Provider Resilience, Endpoint Auditing & Transport Failover

## 1. External Endpoints Audit

The bot interfaces with external endpoints across four critical infrastructure layers:

1. **Jito Block Engine (Multi-Region)**:
   - Regions: `frankfurt`, `amsterdam`, `ny`.
   - Protocol: REST / JSON-RPC / WebSocket.
   - Endpoint: `https://{region}.mainnet.block-engine.jito.wtf/api/v1/bundles`.
   - Rate Limits: 1 request/second per IP on tip-floor endpoint (`tip_floor_poll_ms >= 1000`).
   - Timeout: 400ms request timeout; exponential backoff with jitter on HTTP 429 / 5xx.
2. **Yellowstone gRPC Stream**:
   - Primary: Helius Yellowstone Geyser gRPC stream (`confirmed` commitment).
   - Secondary / Failover: Shyft gRPC stream.
   - Keepalive: Sub-second ping/pong replies enabled (`ping_reply = true`).
   - Freshness: Quotes rejected if slot lag > `max_state_age_slots` (2 slots).
3. **Validator Registry (Kobe)**:
   - Endpoint: Optional registry endpoint; default falls back to internal validator map.
   - Refresh: 60-second periodic poll (`leader_refresh_secs = 60`).
   - Resiliency: Accepts both camelCase (`absoluteSlot`, `slotIndex`, `slotsInEpoch`) and snake_case JSON schemas.
4. **DEX REST APIs (Orca / Raydium / DexScreener / Pump.fun)**:
   - Gated **off by default** (`http_discovery_enabled = false`). All active pool states are ingested via WebSocket account streaming, preventing blocking HTTP I/O in the low-latency hot path.

---

## 2. Resiliency Mechanisms & Circuit Breakers

1. **Exponential Backoff with Jitter**:
   - Used for all RPC retries:
     $$T_{\text{wait}} = \min\left(T_{\text{max}}, T_{\text{base}} \times 2^{\text{attempt}}\right) + \text{rand}(0, J)$$
   - Prevents thundering herd problems during validator restarts or network congestion.
2. **Circuit Breakers**:
   - `max_consecutive_failures = 8`: After 8 consecutive transaction/bundle failures, the circuit breaker trips (`arb_metrics::set_breaker_state(1)`), pausing all execution until manual reset or cooldown expiration.
   - `max_hourly_loss_usd = 2.0` and `max_daily_loss_usd = 5.0`: Immediate shutdown if cumulative realized loss exceeds safety limits.
3. **Fail-Closed Market Data Policy**:
   - If Jito tip oracle snapshot is older than `tip_floor_max_age_ms` (5,000ms), the bidder refuses with `RefuseReason::MarketUnavailable`.
   - The engine never invents a fictional price when live data is missing.
