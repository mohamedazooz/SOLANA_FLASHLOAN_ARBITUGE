> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Environment Variables

**Repository:** Solana Arbitrage Bot  
**File:** `.env.example` (copy to `.env` for local dev; never commit real secrets)

Configuration files declare only secret **env var names**; the actual values are
resolved at runtime by the secret provider (env for local development).

---

## Live Execution Gate

| Variable | Type | Example | Purpose |
|----------|------|---------|---------|
| `ARB_LIVE` | truthy flag | `1` / `true` / `yes` / `on` | Opt-in for live execution. Absent/falsy ⇒ never live. |
| `ARB_MODE` | enum | `shadow` / `simulate` / `canary` / `live` | Bot mode (overrides TOML `bot.mode`). |
| `ARB_CONFIG` | path | `config.toml` | Config file path. |
| `ARB_EVENT_LOG` | path | `logs/arb_events.log` | Event log file path. |

## RPC / Provider Credentials

| Variable | Type | Example | Declared in |
|----------|------|---------|-------------|
| `HELIUS_API_KEY` | secret | `<key>` | `[grpc] primary_token_env` |
| `SHYFT_API_KEY` | secret | `<key>` | `[grpc] failover_token_env` |
| `JUPITER_API_KEY` | secret | `<key>` | `[jupiter] api_key_env` |

## Admin Authentication

| Variable | Type | Example | Declared in |
|----------|------|---------|-------------|
| `SENTINEL_ADMIN_KEY` | secret | `<strong random token>` | `[admin] token_env` |

## Operational Overrides (well-known, auditable)

| Variable | Type | Example | Overrides |
|----------|------|---------|-----------|
| `MAX_TRADE_SIZE_USD` | f64 USD | `10000.0` | `[risk] max_trade_size_usd` |
| `CHRONOS_DECAY_LAMBDA_PER_SEC` | f64 | `3.0` | `[chronos] decay_lambda_per_sec` |
| `CHRONOS_MAX_AGE_MS` | u64 ms | `400` | `[chronos] max_age_ms` |
| `CHRONOS_MIN_EV_USD` | f64 USD | `0.05` | `[chronos] min_ev_usd` |

## Execution Safety

| Variable | Type | Example | Purpose |
|----------|------|---------|---------|
| `KILL_SWITCH_PATH` | path | `KILL_SWITCH` | Kill switch marker file path. |
| `COLD_WALLET_ADDRESS` | address | `<pubkey>` | Profit sweep destination (public key). |

---

## Rules

- **Never commit `.env` with real values.** Commit only `.env.example`.
- Secret env names are declared in config TOML; values come from env/secret
  manager.
- Placeholder secret env names (`changeme`, `placeholder`, `token`, empty) are
  rejected at startup.
- Required missing credentials cause explicit startup failure (fail closed).
