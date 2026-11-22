# RISK CONTROL POLICY — Phase 9 (Mission §14)

**Status:** ACTIVE — source-verified 2026-11-22. Binding policy: any change to a
threshold below requires a documented economic justification (mission §14) and lands
via a reviewed PR.

---

## 1. Risk limits (validated, fail-closed — `crates/arb-config/src/validation.rs`)

| Limit | Key | Enforcement |
|---|---|---|
| Max trade size | `risk.max_trade_size_usd > 0` | validation.rs:267; RiskGate pre-trade |
| Max daily loss | `risk.max_daily_loss_usd > 0` | validation.rs:272; RiskGate |
| Max hourly loss | `≤ max_daily_loss_usd` | validation.rs:283 (cross-field) |
| Max consecutive failures | `> 0` | validation.rs:288; RiskGate breaker |
| Max slippage | per-hop floor + `HOP_SLIPPAGE_MAX_BPS` builder clamp | opportunity.rs stamp (Phase 4) |
| Min net profit | `strategy.min_net_profit_abs` + bps | integer CostLedger gate (EI-enforced) |
| Max borrow amount | ≤ verified Kamino vault liquidity | `kamino_available_flash_liquidity` gate, opportunity.rs:140–168 |
| Max priority fee | `cap_cu_price_by_profit` share ceiling | opportunity.rs:236–252 |
| Max Jito tip | `jito.max_tip_lamports` / `max_tip_share_bps` / refuse-above-ceiling bidder | bidder.rs + validation.rs:80–110 |
| Probe TVL floor | `discovery_min_tvl_usd` | arb-risk defaults (lib.rs:455) |
| Min SOL reserve | wallet validation | arb-risk limits |

Fail-closed rule: missing/invalid critical configuration aborts startup (validation runs
before any execution; nothing clamped).

## 2. Mode gates

```
shadow (default, base.toml) → simulate → canary → live
live ⇒ ARB_LIVE=1 AND live_enabled=true (double gate)
```

Mode is a loaded config value; the supervisor prints the mode at boot
(`main.rs:386`). Observe mode skips opportunity processing entirely
(`opportunity.rs:128–138`). No config combination silently selects live: live requires
both the profile flag and the env gate.

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
| Live mode enable | `ARB_LIVE=1` + `live_enabled=true` + operator approval (separate from code merge) |
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

Current: Telegram notification + outcome/event logging for rejections and aborts.
**Gap (registered):** no append-only file/store for config-threshold changes —
CFN-F-2026-11-22-N3 (P3). Proposed hook: emit an audit record row per config load with
the diff of threshold values vs the previously persisted profile.

## 7. Operator checklist (per deployment)

- [ ] Correct profile file passed explicitly (`--config config/production.toml`).
- [ ] `ARB_LIVE` unset unless live is explicitly authorized.
- [ ] Admin token set from a secret provider (never in TOML).
- [ ] Listen addresses remain loopback unless a reverse proxy with auth fronts them.
- [ ] Kill-switch persistence path configured (not memory-only).
- [ ] Loss limits reviewed against current capital and the economic justification log.
