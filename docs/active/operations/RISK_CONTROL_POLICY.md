# RISK CONTROL POLICY — Phase 9 (Mission §14)

**Status:** ACTIVE — source-verified 2026-11-25. Binding policy: any change to a
threshold below requires a documented economic justification (mission §14) and lands
via a reviewed PR.

---

## 1. Risk limits (schema-specific startup validation + runtime RiskGate)

The legacy bot config is validated in `crates/arb-bot/src/config.rs::load` and
runtime limits are enforced by `crates/arb-risk::RiskGate`. The alternate AppConfig
schema additionally uses `crates/arb-config/src/validation.rs`; do not assume that
validator is applied to the legacy `[bot]/[rpc]` config.

| Limit | Key | Enforcement |
|---|---|---|
| Max trade size | `risk.max_trade_size_usd > 0` | legacy config load + RiskGate pre-trade |
| Max daily loss | `risk.max_daily_loss_usd > 0` | legacy config load + RiskGate |
| Max hourly loss | `≤ max_daily_loss_usd` | legacy config load (cross-field) |
| Max consecutive failures | `> 0` | legacy config load + RiskGate breaker |
| Max slippage | per-hop floor + `HOP_SLIPPAGE_MAX_BPS` builder clamp | opportunity.rs stamp (Phase 4) |
| Min net profit | `strategy.min_net_profit_abs` + bps | integer CostLedger gate (EI-enforced) |
| Max borrow amount | ≤ verified Kamino vault liquidity | `kamino_available_flash_liquidity` gate, opportunity.rs:140–168 |
| Max priority fee | `cap_cu_price_by_profit` share ceiling | opportunity.rs:236–252 |
| Max Jito tip | `jito.max_tip_lamports` / `max_tip_share_bps` / refuse-above-ceiling bidder | legacy config load + bidder refuse-above-ceiling |
| Probe TVL floor | `discovery_min_tvl_usd` | arb-risk defaults (lib.rs:455) |
| Min SOL reserve | wallet validation | arb-risk limits |

Fail-closed rule: schema-specific startup validation rejects missing/invalid critical configuration before execution; runtime RiskGate rechecks economic limits before dispatch.
before any execution; nothing clamped).

## 2. Mode gates

Runtime mode is selected by `main.rs` from environment variables, not from the
legacy `[bot]` TOML section:

1. Explicit `ARB_MODE` takes precedence.
2. Otherwise `ARB_CANARY` selects canary mode when present.
3. Otherwise a truthy `ARB_LIVE` / `LIVE_TRADING` selects live mode.
4. Otherwise the default is `observe` (non-live).

Live dispatch requires `BotMode::Live` AND a truthy `ARB_LIVE` (or the documented
`LIVE_TRADING` alias); `main.rs` refuses live mode without the environment gate.
The legacy `[bot]/[rpc]` schema does not read `[bot].mode` or `[bot].live_enabled`.
The alternate AppConfig schema additionally requires `mode.live_enabled=true`.
No base/profile TOML files are merged automatically; the selected file is logged
at startup and its SHA-256 provenance is appended to `data/config_load_audit.jsonl`.

## 3. Breakers & switches (all verified in source)

| Control | Location | Trigger / behavior |
|---|---|---|
| Global kill switch (memory + persisted) | `arb-risk::KillSwitch` (lib.rs:115–170) | engage blocks immediately; persistence failure surfaces `persistence_error` (never silent) |
| Fire-time re-check | `dispatch.rs:649–665` | abort before send, outcome recorded, Telegram notified |
| Emergency stop | `control().is_emergency_stopped()` (Telegram/Admin) | same fire-time gate |
| Consecutive-failure breaker | `RiskGate` + `max_consecutive_failures` | opportunity rejection stage |
| RPC breaker | `RpcPool` per-endpoint backoff + 429 classification + token buckets | rotation, never silent failure |
| Provider breaker | `ProviderSupervisor` open-circuit + half-open probe | requires independent cluster slot to restore |
| Jito breaker | all-engine submit failure = hard error | no silent public-RPC bundling fallback |
| Durable dispatch intent | `arb_store::DispatchIntentRecord` persisted pre-send | send refused if persistence fails (`dispatch.rs:747`) |
| Proven-expiry-only retry | `TransactionLifecycle` (arb-land/lifecycle.rs) | `Unknown` never resends; expiry needs block-height proof |

## 4. High-risk action authorization matrix

| Action | Authorization required |
|---|---|
| Read-only endpoints (status, balances, logs) | Admin token (constant-time compare) |
| Engage/release kill switch via Telegram/Admin | Admin token + event log |
| Config threshold change (`min_net_profit`, tip/lamport caps, loss limits) | Documented economic justification + reviewed PR (§1 header) |
| Live mode enable | `ARB_LIVE=1` + operator approval; alternate AppConfig also requires `mode.live_enabled=true` |
| On-chain program deploy/upgrade | Explicit operator approval — never automatic |
| Fund movement / wallet ops | Operator only; the bot never exposes key material through any admin API |

## 5. Failure economics (non-negotiable invariants)

1. Estimated PnL ≠ realized PnL; the reconciliation path uses on-chain deltas
   (Phase 4 `ECONOMIC_INVARIANTS.md`, EI-gates).
2. Failed-landing cost (priority fee + base fee write-off) is recorded through
   `record_outcome` before any retry decision (R12 doctrine, `recovery.rs`).
3. A tip may never be repriced above the refuse ceiling — the opportunity is refused
   (`bidder.rs` ceiling rule).
4. Stale market data never gates a trade (tip floor 5 s freshness; quote readiness
   fail-closed slot gates).
5. Any loss-limit breach stops trading; restart requires operator action, not automatic
   resume (kill switch persists across restarts until released).

## 6. Config-change audit trail

Current: every successful config load appends one JSONL provenance row to
`data/config_load_audit.jsonl` with Unix timestamp, selected config path, SHA-256,
and runtime mode. No config values or secret material are written. This provides
append-only file provenance; it does not claim a semantic field-by-field diff.

## 7. Operator checklist (per deployment)

- [ ] Correct profile file passed explicitly (`--config config/production.toml`).
- [ ] `ARB_LIVE` unset unless live is explicitly authorized.
- [ ] Admin token set from a secret provider (never in TOML).
- [ ] Listen addresses remain loopback unless a reverse proxy with auth fronts them.
- [ ] Kill-switch persistence path configured (not memory-only).
- [ ] Loss limits reviewed against current capital and the economic justification log.
