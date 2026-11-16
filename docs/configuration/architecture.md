# Configuration Architecture

**Repository:** Solana Arbitrage Bot  
**Audience:** Operators, security reviewers, platform engineers  
**Status:** Phase 2 (implemented) — `crates/arb-config/`

---

## 1. Design Goals

The configuration system implements the mission's config architecture:

1. **One authoritative loading architecture** — compiled protocol invariants →
   `base.toml` → environment-specific TOML → environment variables → CLI
   overrides.
2. **Operator configuration cannot override protocol invariants** or mandatory
   safety limits.
3. **Every field has an explicit typed unit, validation, documented meaning,
   safe operational range, and optional environment override.**
4. **Invalid values produce actionable errors** identifying the config key
   without revealing secrets. NaN, Infinity, negative/overflowing values are
   rejected; nothing is silently clamped.
5. **Secrets are never stored in plaintext** in config files; only secret env
   var *names* are declared. Values are resolved by a secret provider.

---

## 2. Precedence (lowest → highest)

| Priority | Source | Notes |
|----------|--------|-------|
| 1 | Compiled protocol invariants (`arb-constants`) | Immutable; cannot be overridden |
| 2 | `config/base.toml` | Deployment-independent operational defaults |
| 3 | Environment TOML (`development` / `staging` / `production.toml`) | Selected via `ARB_MODE` / `--mode` context |
| 4 | Environment variables | Declared overrides (see `config/environment_variables.md`) |
| 5 | CLI overrides | Applied by the caller after load |

Protocol invariants (program IDs, mints, discriminators, PDA seeds, Q64 scales,
bin layout, MTU, CU caps) live in `crates/arb-constants/` and are **not**
operator-configurable.

---

## 3. Directory Layout

```
config/
  base.toml              # Complete, deployment-independent defaults
  development.toml       # Dev overrides (shadow mode, permissive CORS)
  staging.toml           # Staging (simulate, confirmed commitment)
  production.toml        # Production (shadow default, strict safety)
  config.example.toml    # Example (copy to config.toml; no real secrets)
  config.schema.json     # JSON Schema for validation/tooling
.env.example             # Example env vars (never commit .env with secrets)
```

Each `*.toml` is a **complete, independently loadable** `AppConfig` (all
sections present). This keeps `serde` deserialization simple and avoids fragile
partial-merge logic.

---

## 4. Top-Level Structures

| Structure | Responsible crate | Key fields |
|-----------|-------------------|------------|
| `AppConfig` | `arb-config` | Aggregates all sub-configs |
| `ModeConfig` | `arb-config` | `mode` (shadow/simulate/canary/live), `live_enabled` |
| `ProviderConfig` | `arb-config` | RPC/Geyser URLs, token env names, commitment |
| `JitoConfig` | `arb-config` | Tip floor percentile, share bps, caps, leader gate |
| `JupiterConfig` | `arb-config` | Base URL, API key env, timeout |
| `WalletConfig` | `arb-config` | Keypair path, cold wallet, min SOL reserve |
| `StrategyConfig` | `arb-config` | Base tokens, profit floors, price impact, state age |
| `RiskConfig` | `arb-config` | Trade size, loss limits, velocity, inflight bundles |
| `ChronosConfig` | `arb-config` | EV gate: lambda, max age, min EV, costs |
| `StoreConfig` | `arb-config` | Persistence URL |
| `ObservabilityConfig` | `arb-config` | Metrics listen address |
| `AdminConfig` | `arb-config` | Admin listen, token env name |
| `SecurityConfig` | `arb-config` | CORS allowlist, max body, signature required |

---

## 5. Newtypes (Units)

Reused from `arb-types` so units are never ambiguous:

| Newtype | Underlying | Unit | Used for |
|---------|-----------|------|----------|
| `Lamports` | `u64` | lamports | SOL balances, tips, fees |
| `TokenAmount` | `u64` | atomic token units | swap amounts |
| `Bps` | `u16` | basis points (max 10,000) | fees, price impact, tip share |
| `Slot` | `u64` | Solana slot | state age, cooldown |
| `ComputeUnits` | `u32` | compute units | transaction CU |
| `UsdMicros` | `i64` | micro-USD | loss limits, trade size, EV |
| `Milliseconds` | `u64` | milliseconds | timeouts, freshness |
| `Seconds` | `u64` | seconds | refresh intervals |

`Bps::MAX = 10,000` (100%). `ComputeUnits::MAX = 1,400,000` (Solana runtime cap).

---

## 6. Validation Rules (startup)

Rejected with actionable errors (key identified, no secrets revealed):

- `min_profit` must be positive.
- `max_trade_size_usd` must be finite and positive.
- `max_hourly_loss_usd` must be positive and ≤ `max_daily_loss_usd`.
- `tip_share_bps` must be within `0..=10_000` and ≤ `max_tip_share_bps`.
- `max_tip_share_bps` ≤ `Bps::MAX` (10,000).
- `price_impact` within `0..=10_000` bps.
- `state_age` positive.
- `graph_max_age_slots` ≥ `max_state_age_slots`.
- CU limits must not exceed runtime cap.
- URLs must use approved schemes (`http`/`https` for provider; `https` for Jupiter).
- Production CORS must have explicit allowed origins (never `permissive`).
- Live mode requires `live_enabled=true`, `ARB_LIVE=1`, non-empty CORS allowlist,
  webhook signature required, admin token env set, real RPC URL set.
- NaN, Infinity, negative, overflowing, empty required lists, placeholder
  secret env names are rejected.
- Unknown TOML keys are rejected (strict parsing).

Nothing is silently clamped.

---

## 7. Secret Handling

- Config files declare only secret **env var names** (e.g. `token_env =
  "SENTINEL_ADMIN_KEY"`), never values.
- Real secret values are provided via `.env` (local dev) or a production secret
  manager.
- `.env` must never be committed; `.env.example` is shipped instead.
- No secret values in `Debug` output, tracing, error messages, or generated
  reports.
- Explicit failure if required credentials are missing.
- Key file permission checks (keypair file 0600).

---

## 8. Adding a New Config Key

1. Add the field to the relevant struct in `crates/arb-config/src/lib.rs`
   with an explicit newtype unit and `#[serde(...)]`.
2. Add a default in `AppConfig::default_safe()`.
3. Add validation in `crates/arb-config/src/validation.rs`.
4. Add environment override in `crates/arb-config/src/loader.rs` (only for
   well-known, auditable keys).
5. Add a test in `validation.rs` `mod tests`.
6. Update `config/config.schema.json` and `docs/configuration/configuration_reference.md`.
7. Update `config/config.example.toml` and `.env.example`.

Every key documents: name, type, unit, default, allowed range, source,
security implications, and restart/reload requirements.
