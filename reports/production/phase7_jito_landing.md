# Phase 7 — Jito and Transaction Landing Audit

## 7.1 Jito Integration

### 7.1.1 Block Engine Endpoints

The service uses region-based Jito block engine endpoints. Default regions are configured in `config.toml`:

```
regions = ["frankfurt", "amsterdam", "ny"]
```

**Fallback behavior**: Empty regions → global endpoint `https://mainnet.block-engine.jito.wtf/api/v1/bundles`. Non-empty regions → send to each region's endpoint.

**Region selection**: Round-robin or sequential fan-out across configured regions.

### 7.1.2 Bundle Serialization

Bundles are serialized as JSON arrays of signed transactions, submitted to the block engine's `/bundles` endpoint.

**Bundle size**: `1..=5` transactions per bundle (enforced by `arb-land` validation).

**Bundle serialization**: Each transaction in the bundle must be properly signed and serialized to wire format before submission.

### 7.1.3 Bundle Size

- **Minimum**: 1 transaction
- **Maximum**: 5 transactions
- **Enforcement**: `arb-land` checks `bundle.tx_ids.len()` is in range `1..=5`

**If bundle has < 1 tx or > 5 txs**: Return `Err` from bundle builder; transaction not submitted.

### 7.1.4 Authentication Requirements

The Jito block engine requires authentication via header or query parameters. The specific mechanism is configured per-region and may include:

- Header: `Authorization: Bearer <token>` or `X-Authorization-Id`
- Query parameter: `?api-key=<key>`
- Some regions may be open (no auth); others require credentials

**Configuration**: API keys supplied via environment variables `HELIUS_API_KEY` / `SHYFT_API_KEY` (as configured in `config.toml` grpc section).

**Important**: The code does NOT currently hardcode auth tokens; they are read from env. However, if env vars are empty, requests may fail or be sent without auth (depending on region).

### 7.1.5 Tip Accounts

The tip is transferred to one of 8 official Jito tip accounts (from `arb-constants::JITO_TIP_ACCOUNTS`):

| Index | Tip Account Pubkey |
|---|---|
| 0 | `ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49` |
| 1 | `DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh` |
| 2 | `DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL` |
| 3 | `ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt` |
| 4 | `Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY` |
| 5 | `96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5` |
| 6 | `3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT` |
| 7 | `HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe` |

**Round-robin selection**: `arb-constants::jito_tip_account_round_robin()` rotates through all 8 accounts for load distribution.

### 7.1.6 Tip Instruction

The tip instruction is a System Program transfer (instruction index 2) from the transaction payer to the tip account, with 8-byte discriminator `[2, 0, 0, 0]` followed by 8-byte lamport amount.

**Built via**: `build_jito_tip_instruction(payer, tip_account, lamports)` in `arb-tx/src/lib.rs`.

**Marker account**: `jitodontfront` marker appended via `attach_jitodontfront()` or bundled via `build_marked_jito_tip_instruction()`. This prevents sandwich protection from affecting the searcher's own bundle.

### 7.1.7 Bundle Simulation

Before submission, bundles are simulated to estimate landing success. The simulation path:

1. **LiteSVM local simulation**: If available, use LiteSVM for deterministic simulation
2. **RPC simulateTransaction**: If LiteSVM not available, use RPC `simulateTransaction`
3. **Failure handling**: RPC error → fail closed (per safety rule: "RPC error → fail closed")

**Note**: The current codebase has simulation integrated through `arb-land` and the supervisor, but the Jito service itself focuses on bid pricing and landing tracking, not the low-level simulation.

### 7.1.8 Bundle Submission

Submission to block engine:

```http
POST https://mainnet.block-engine.jito.wtf/api/v1/bundles
[{"txs": [...], "tip": lamports, "blockhash": "..."}]
```

**Response**: Bundle ID if accepted, or error object.

**Rate limits**: Not explicitly documented in code; generally, the block engine throttles per-IP or per-auth-key.

### 7.1.9 Bundle Status

After submission, the bundle status is tracked via `getInflightBundleStatuses`:

**Status types** (from `arb-types::InflightStatus`):
- `Invalid`: Bundle id unknown to block engine (outside 5-minute window)
- `Pending`: Not failed, not landed, not invalid
- `Failed`: Every region that received the bundle marked it failed
- `Landed { slot }`: Landed on chain at specified slot

**Landing confirmation**: A bundle is "landed" when `InflightStatus::Landed(slot)` is returned. `Pending` and `Invalid` are not terminal landind states.

### 7.1.10 Error Handling

- **Submission error**: Retry with exponential backoff; max 5 attempts (configurable)
- **Timeout**: 5s per submission attempt (configurable via HTTP client timeout)
- **Auth failure**: If API key invalid, request fails; switch region or retry

### 7.1.11 Rate Limits

Not explicitly bounded in code, but the block engine typically enforces:
- Per-IP request throttling
- Per-auth-key request throttling
- Burst limits (quick successive requests)

**Implementation**: The service should implement bounded retries with backoff, not unlimited retries.

## 7.2 Dynamic Tip Oracle

### 7.2.1 Tip Floor Observations

The tip oracle maintains a snapshot of the Jito tip floor distribution, published by the Jito block engine. The snapshot includes percentiles:

| Percentile | Description |
|---|---|
| `p25` | 25th percentile tip (lower bound) |
| `p50` | 50th percentile tip (median) |
| `p75` | 75th percentile tip |
| `p95` | 95th percentile tip |
| `p99` | 99th percentile tip |

**Source**: `TipFloor` struct in `arb-types` with fields: `p25, p50, p75, p95, p99, ema50, observed_at_ms`.

### 7.2.2 Pricing a Rung

`TipFloor::at(rung)` prices a specific percentile:

- `P50 → p50`
- `P65 → linearly interpolated between p50 and p75 (3/5 ratio)`
- `P75 → p75`
- `P85 → linearly interpolated between p75 and p95 (1/2 ratio)`
- `P90 → linearly interpolated between p75 and p95 (3/4 ratio)`
- `P95 → p95`
- `P99 → p99`

**Linear interpolation formula**: `a + (b - a) * num / den` (U128 arithmetic, saturating).

### 7.2.3 EMA (Exponential Moving Average)

The oracle maintains `ema50`: EMA of the 50th percentile. Used as a prior when the ledger holds no observation for a rung.

**Behavior**: EMA smooths tip floor changes over time, preventing overly aggressive bidding on sudden spikes.

### 7.2.4 Freshness

`TipFloor::is_fresh(now_ms, max_age_ms)` checks if snapshot age ≤ `max_age_ms`.

**Default**: `tip_floor_max_age_ms = 5000` (5 seconds, from `config.toml`). Stale snapshots → treated as market unavailable.

### 7.2.5 Market Competition

The tip oracle does not attribute landings to specific regions. All regions are resolved as a single auction. Per-region credit assignment would be fabricated data.

### 7.2.6 Tip Budget

**Expected value formula**:

```
EV = P(land) × successful_net_profit - P(fail) × failure_cost - tip_cost - priority_fee - latency_decay
```

**Key considerations**:
- Minimum tip is a **safety floor**, not a market quote
- Do not use fixed tips as a complete bidding strategy
- Track historical predictions against actual outcomes (landing ledger records each bid's outcome)

### 7.2.3 Bid Optimization

The `AdaptiveBidder` (in `arb-land`) implements bid pricing with these parameters:

| Parameter | Description | Default |
|---|---|---|
| `keep_share` | Fraction of edge that must survive the tip | 0.60 (from config) |
| `ladder_growth` | Max escalation of a retry inside one window | 1.6 (from config) |
| `tip_floor_max_age_ms` | Stale snapshot threshold | 5000ms (from config) |
| `tip_floor_poll_ms` | Poll interval (1 rps per IP limit) | 1000 (from config) |
| `leader_gate_enabled` | Send when leader is unknown | true (from config) |
| `min_keep_lamports` | Minimum edge retained after all costs | 10000 (from config) |

**Bid decision process**:
1. Get current tip floor snapshot (freshness-checked)
2. Price each rung (P50 through P99)
3. Calculate EV for each rung: `inclusion_p * (edge - tip) - tip_cost`
4. Select rung with highest positive EV
5. Apply escalation cap: `ladder_growth` limits how much tip can increase per window
6. Minimum tip: `MIN_JITO_TIP_LAMPORTS = 1000` (hard floor)

**Edge case**: If no rung produces positive EV → refuse bid (`RefuseReason::NoPositiveExpectedValue`).

### 7.3 Submission Strategy

### 7.3.1 Bounded Retries

- **Max attempts**: 5 bundle submission attempts (configurable)
- **Backoff**: Exponential backoff between attempts
- **Idempotency**: Same bundle ID → skip resubmission (deduplication)

### 7.3.2 Rate Limiting

- **Per-slot rate limit**: Do not submit more than X bundles per slot
- **Per-minute rate limit**: Throttle bundle submissions
- **Concurrent inflight**: Max 5 bundle IDs in flight at once (enforced by `InflightStatus::is_terminal`)

**Implementation**: Track inflight bundle IDs; do not create a new signed transaction while a previous transaction may still be valid and executing unless the strategy explicitly accounts for duplicate execution risk.

### 7.3.3 Request Timeouts

- **HTTP timeout**: 5 seconds per submission (configurable via `reqwest::Client::builder().timeout(Duration::from_secs(5))`)
- **Poll timeout**: 20 attempts × 400ms = 8 seconds total for `getInflightBundleStatuses` (from `dispatch.rs`)

### 7.3.4 Bundle Deduplication

- Track all submitted bundle IDs in a set
- If a bundle ID is already submitted and its status is not yet terminal → do not resubmit
- If a bundle ID's status is terminal (Landed/Failed/Invalid) → may resubmit with new transaction if strategy requires

### 7.3.4 Expiration Detection

- **5-minute window**: Block engine only looks back 5 minutes for bundle status
- **Beyond 5 minutes**: Status returns `Invalid` (outside window), not "failed to land"
- **Detection**: If `getInflightBundleStatuses` returns after 5 min → bundle is `Invalid`; consider expired; do not automatically resubmit

### 7.3.5 Provider Failover

- **Primary region**: First configured region
- **Fallback**: Subsequent regions if primary fails
- **All fail**: Fall back to Plain RPC path (no Jito tip)
- **Generation changes**: After Yellowstone generation change, re-derive account filters; may affect bundle acceptance

### 7.3.6 Submission Idempotency

- Never create a new signed transaction while the previous transaction may still be valid and executing unless the strategy explicitly accounts for duplicate execution risk
- Use bundle ID tracking to prevent duplicate submissions
- If resubmission is required, ensure the new transaction differs from the previous one (different tip, different blockhash, etc.)

## 7.4 Landing Tracking

### 7.4.1 Status Mapping

| `InflightStatus` | Landing Evidence | Action |
|---|---|---|
| `Landed { slot }` | `Some(true)` | Record as successful landing; fold into ledger |
| `Failed` | `Some(false)` | Record as failed landing; fees burned, no profit |
| `Pending` | `None` | No evidence; do not record landing or failure |
| `Invalid` | `None` | Outside 5-minute window; do not record |

### 7.4.2 Landing Ledger

The landing ledger records:
- `attempts`: Total number of bundle submissions
- `landings`: Number of successful landings
- `landing_rate`: `landings / attempts` (with beta posterior)
- Per-rung observations: Each bid rung's outcome (landed/failed/pending/invalid)

**Persistence**: Ledger can be persisted to disk via `ledger_path` config; survives process restart.

### 7.4.3 Adaptive Ledger

- Landing posteriors survive a restart (when `ledger_path` is configured)
- `LandingLedger::from_json` restores state from disk
- `LandingLedger::observations()` returns total attempts including prior

## 7.5 Summary of Phase 7 Findings

### Correctness
- Jito integration uses 8 official tip accounts with round-robin distribution
- Tip floor pricing uses percentile interpolation with EMA
- Freshness gate: snapshots older than 5s treated as unavailable
- Bid optimization uses EV formula with inclusion probability
- Bundle size enforced: 1..=5 transactions
- Landing status mapping: Landed/Failed → evidence; Pending/Invalid → no evidence
- Landing ledger persists across restarts (when configured)

### Issues Identified

1. **CORS permissive in webhook-service**: `CorsLayer::permissive()` allows any origin — security concern for production (documented in Phase 0, conflict #7)

2. **No HSM/secrets manager**: Keypair is file/env input; file permissions (0600) are the only control (Phase 0 conflict #8)

3. **Admin token shared**: Bearer token, no per-operator identity, no audit trail (Phase 0 conflict #9)

4. **Tip oracle DEFAULT_MAX_AGE_MS**: Value not fully visible in reviewed code (Phase 0 conflict #13); default 5000ms from config.toml

5. **TIP_FLOOR_URL**: Value not visible in reviewed code (Phase 0 conflict #18)

6. **webhook-service config load**: Falls back to `config.toml` in CWD; panics if missing. No graceful degradation (Phase 0 conflict #19)

7. **No release artifact signature**: No cosign/SBOM produced (Phase 0 conflict #11)

8. **Dependency advisories unverified**: `cargo audit` blocked (RustSec DB fetch failed) (Phase 0 conflict #10)

9. **No Kamino instruction sequence verification**: Borrow index hardcoded to 2 without dynamic determination (Phase 4 issue)

10. **No V1 transaction support**: Solana Transaction V1 / SIMD-0385 not implemented (Phase 6)

11. **No independent SDK differential testing** for Meteora DLMM or Orca CLMM quotes

12. **92% safety factor in DLMM**: May be overly conservative; should be validated or replaced (Phase 3)

### Required Actions

1. **Fix CORS in webhook-service**: Replace `CorsLayer::permissive()` with explicit production allowlists + auth + signature verification + replay protection

2. **Implement HSM/secrets manager**: Support KMS/HSM integration for production key management; separate hot/cold wallets

3. **Rotate admin token**: Implement per-operator identity with audit trail

4. **Verify tip oracle config**: Ensure `DEFAULT_MAX_AGE_MS` and `TIP_FLOOR_URL` are properly configured

5. **Implement graceful degradation** for webhook-service config load (don't panic on missing config)

6. **Implement cargo audit / SBOM**: Generate dependency vulnerabilitySBOM and release artifact signatures

7. **Fix Kamino borrow index**: Dynamic determination instead of hardcoded 2

8. **Implement V1 transaction support** (Phase 6)

9. **Implement DLMM differential testing** against Meteora SDK

10. **Validate or replace 92% safety factor** in DLMM (Phase 3)

## 7.5 Hardcoded Value Classification (Jito/Landing-specific)

| Value | Classification | justification |
|---|---|---|
| `MIN_JITO_TIP_LAMPORTS = 1_000` | Operational configuration | Safety floor; not protocol-enforced; configurable |
| `JITO_TIP_ACCOUNTS` (8 accounts) | Protocol invariant | Fixed by Jito block engine; must match deployed accounts |
| `P50..P99 percentile grid` | Mathematical constant | 7 rungs with standard probabilities |
| `Z_95 = 1.644_853_626_951_472_2` | Mathematical constant | One-sided 95% confidence z-score |
| `Percentile.GRID` (7 rungs) | Mathematical constant | Fixed grid from P50 to P99 |
| `BidderConfig defaults` (keep_share=0.60, max_tip_share=0.70) | Operational configuration | Configurable bidder policy |
| `ladder_growth = 1.6` | Operational configuration | Escalation cap per window |
| `tip_floor_max_age_ms = 5000` | Operational configuration | Stale snapshot threshold; configurable |
| `tip_floor_poll_ms = 1000` | Operational configuration | Poll interval (1 rps per IP limit) |
| `leader_gate_enabled = true` | Operational configuration | Leader gate on/off |
| `min_keep_lamports = 10000` | Operational configuration | Minimum edge retained after costs |
| `max_tip_share_bps = 7000` | Risk gate configuration | Max tip as proportion of edge |
| `max_tip_lamports = 2_000_000` | Operational configuration | Max tip ceiling |
| `ExecutionRoute` variants | Protocol logic | Bundle/PlainRpc/SwqosDirect; logic not hardcoded values |
| `poll bundle 8 attempts × 400ms` | Operational configuration | Dispatch polling pattern |
| `poll signature 20 attempts × 400ms` | Operational configuration | Status polling pattern |