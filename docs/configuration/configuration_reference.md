# Configuration Reference

**Repository:** Solana Arbitrage Bot  
**Source of truth:** `crates/arb-config/` + `config/base.toml`  
**Schema:** `config/config.schema.json`

Every configuration key documents: name, type, unit, default, allowed range,
source, security implications, and restart/reload requirements.

---

## [bot]

| Key | Type | Unit | Default | Allowed range | Source | Security | Reload |
|-----|------|------|---------|---------------|--------|----------|--------|
| `mode` | enum | — | `shadow` | shadow/simulate/canary/live | TOML/CLI/ENV `ARB_MODE` | Live mode is opt-in | Restart |
| `live_enabled` | bool | — | `false` | true/false | TOML/ENV `ARB_LIVE` | **High** — gates live trading | Restart |

## [grpc]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `primary_url` | string | URL | `""` | http/https | TOML/ENV | Endpoint metadata | Restart |
| `primary_token_env` | string | env name | `"HELIUS_API_KEY"` | non-empty | TOML | Secret env name | Restart |
| `failover_url` | string\|null | URL | `null` | http/https | TOML | — | Restart |
| `failover_token_env` | string\|null | env name | `null` | non-empty | TOML | Secret env name | Restart |
| `commitment` | enum | — | `"processed"` | processed/confirmed/finalized | TOML | — | Restart |
| `ping_reply` | bool | — | `true` | — | TOML | — | Restart |

## [rpc]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `url` | string | URL | `""` | http/https | TOML | Endpoint metadata | Restart |
| `commitment` | enum | — | `"processed"` | processed/confirmed/finalized | TOML | — | Restart |

## [jito]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `regions` | array[string] | — | `["frankfurt","amsterdam","ny"]` | ≥1 | TOML | — | Restart |
| `tip_floor_percentile` | enum | p50/p75/p90/p99 | `"p50"` | — | TOML | — | Restart |
| `tip_share_bps` | Bps | bps | `5500` | 0..=10000, ≤ max | TOML | Economic | Restart |
| `max_tip_lamports` | Lamports | lamports | `2000000` | >0 | TOML | Economic cap | Restart |
| `max_tip_share_bps` | Bps | bps | `7000` | 0..=10000 | TOML | Economic cap | Restart |
| `min_keep_lamports` | Lamports | lamports | `10000` | >0 | TOML | Safety | Restart |
| `keep_share` | f64 | ratio | `0.60` | [0,1] | TOML | Economic | Restart |
| `ladder_growth` | f64 | multiplier | `1.6` | >1.0 | TOML | Economic | Restart |
| `tip_floor_max_age_ms` | Milliseconds | ms | `5000` | >0 | TOML | Freshness | Restart |
| `tip_floor_poll_ms` | Milliseconds | ms | `1000` | ≥1000 | TOML | Rate-limit compliant | Restart |
| `leader_gate_enabled` | bool | — | `true` | — | TOML | Execution safety | Restart |
| `kobe_url` | string\|null | URL | `null` | https | TOML | — | Restart |
| `ledger_path` | string\|null | path | `"data/adaptive_ledger.json"` | — | TOML | — | Restart |
| `leader_refresh_secs` | Seconds | s | `60` | >0 | TOML | — | Restart |

## [jupiter]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `base_url` | string | URL | `"https://api.jup.ag/swap/v2"` | https | TOML | Endpoint | Restart |
| `api_key_env` | string | env name | `"JUPITER_API_KEY"` | non-empty | TOML | Secret env name | Restart |
| `timeout_ms` | Milliseconds | ms | `400` | >0 | TOML | — | Restart |

## [wallet]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `keypair_path` | string | path | `"solana_keypair.json"` | non-empty | TOML | **High** — key material | Restart |
| `cold_wallet_pubkey` | string\|null | address | `null` | valid pubkey | TOML | — | Restart |
| `min_sol_balance_lamports` | Lamports | lamports | `0` | ≥0 | TOML | Safety floor | Restart |

## [strategy]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `base_tokens` | array[string] | mint addr | WSOL+USDC | ≥1 | TOML | — | Restart |
| `min_net_profit_abs` | Lamports | lamports | `50000` | >0 | TOML | Profit floor | Restart |
| `min_net_profit_bps` | Bps | bps | `5` | 0..=10000 | TOML | Profit floor | Restart |
| `max_price_impact_bps` | Bps | bps | `40` | 0..=10000 | TOML | Economic | Restart |
| `max_state_age_slots` | Slot | slots | `2` | >0 | TOML | Freshness/safety | Restart |
| `graph_max_age_slots` | Slot\|null | slots | `null` | ≥ state age | TOML | Freshness | Restart |
| `use_flash_loan` | bool | — | `true` | — | TOML | Execution mode | Restart |
| `max_pools` | usize\|null | count | `1000` | >0 | TOML | Resource cap | Restart |
| `discovery_refresh_secs` | Seconds\|null | s | `15` | >0 | TOML | — | Restart |
| `discovery_min_tvl_usd` | UsdMicros\|null | USD | `25000.0` | ≥0 | TOML | — | Restart |

## [risk]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `max_trade_size_usd` | UsdMicros | USD | `10000.0` | >0, finite | TOML/ENV `MAX_TRADE_SIZE_USD` | **High** — loss cap | Restart |
| `max_daily_loss_usd` | UsdMicros | USD | `5.0` | >0 | TOML | **High** — loss cap | Restart |
| `max_hourly_loss_usd` | UsdMicros | USD | `2.0` | >0, ≤ daily | TOML | **High** — loss cap | Restart |
| `max_consecutive_failures` | u64 | count | `8` | >0 | TOML | Circuit breaker | Restart |
| `min_landing_rate` | f64 | ratio | `0.02` | [0,1] | TOML | Landing quality | Restart |
| `max_attempts_per_slot` | u32 | count | `3` | >0 | TOML | Execution | Restart |
| `max_inflight_bundles` | usize | count | `2` | >0 | TOML | Execution | Restart |
| `max_tip_spend_ratio` | f64 | ratio | `0.7` | [0,1] | TOML | Economic | Restart |
| `max_orders_per_hour` | usize | count/hr | `120` | >0 | TOML | Velocity | Restart |
| `cooldown_slots` | Slot | slots | `75` | >0 | TOML | Circuit breaker | Restart |

## [chronos]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `enabled` | bool | — | `true` | — | TOML | EV gate | Restart |
| `decay_lambda_per_sec` | f64 | /s | `3.0` | ≥0, finite | TOML/ENV `CHRONOS_DECAY_LAMBDA_PER_SEC` | Economic | Restart |
| `max_age_ms` | Milliseconds | ms | `400` | 100..=5000 | TOML/ENV `CHRONOS_MAX_AGE_MS` | Freshness/safety | Restart |
| `min_ev_usd` | UsdMicros | USD | `0.05` | ≥0 | TOML/ENV `CHRONOS_MIN_EV_USD` | EV gate | Restart |
| `failed_attempt_cost_lamports` | Lamports | lamports | `10000` | ≥0 | TOML | Economic | Restart |
| `beta_risk_factor` | f64 | factor | `0.0005` | ≥0 | TOML | Economic | Restart |

## [store]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `url` | string | URL | `"sqlite://arb_sentinel.db"` | — | TOML | — | Restart |

## [metrics]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `listen` | SocketAddr | host:port | `"127.0.0.1:9100"` | — | TOML | Localhost only | Restart |

## [admin]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `listen` | SocketAddr | host:port | `"127.0.0.1:9101"` | — | TOML | Localhost only | Restart |
| `token_env` | string | env name | `"SENTINEL_ADMIN_KEY"` | non-empty | TOML | **High** — admin auth | Restart |

## [security]

| Key | Type | Unit | Default | Range | Source | Security | Reload |
|-----|------|------|---------|-------|--------|----------|--------|
| `cors_allow_origins` | array[string] | origin | `[]` | — | TOML | **High** — production allowlist | Restart |
| `max_request_body_bytes` | usize | bytes | `1048576` | >0 | TOML | DoS mitigation | Restart |
| `webhook_signature_required` | bool | — | `true` | — | TOML | **High** — webhook auth | Restart |

---

## Cross-Field Constraints (enforced at startup)

- `risk.max_hourly_loss_usd` ≤ `risk.max_daily_loss_usd`
- `jito.tip_share_bps` ≤ `jito.max_tip_share_bps` ≤ `Bps::MAX` (10,000)
- `strategy.graph_max_age_slots` ≥ `strategy.max_state_age_slots` (when set)
- Live mode ⇒ `mode.live_enabled == true` AND `ARB_LIVE=1` AND non-empty
  `security.cors_allow_origins` AND `security.webhook_signature_required == true`
  AND non-empty `provider.primary_url`

## Security Notes

- No secret values appear in this file; only env var names.
- `cors_allow_origins` must be an explicit allowlist in production (never
  permissive).
- Placeholder secret env names (`changeme`, `placeholder`, `token`, empty) are
  rejected at startup.
