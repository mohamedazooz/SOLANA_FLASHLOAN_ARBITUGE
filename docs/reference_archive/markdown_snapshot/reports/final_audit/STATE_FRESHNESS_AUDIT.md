# Data Freshness & Execution Safety (PHASE K)

## Verified mechanisms

* **Slot freshness**: every candidate carries `snapshot_slot`; quote/sim lag ≤ 2 enforced
  (measured ≤ 1 on live runs). Stale state fails closed — no stale-cache fallback exists
  after RPC failure (grep-audited: the account store serves only freshness-tagged reads;
  provider rotation happens BEFORE any execution decision, and failure to establish fresh
  state stops the candidate).
* **Provider rotation**: `RpcPool` rotates configured providers with health telemetry and
  429 cooldowns; failover never substitutes an older snapshot for a failed fresh fetch.
* **Duplicate/out-of-order updates**: WS/Geyser ingestion stamps slot+write-version;
  out-of-order and duplicate account updates are dropped by the store's freshness keys.
* **Fork invalidation**: processed-status observations are never treated as confirmed;
  dispatch decisions use explicit state transitions (submission ≠ execution).
* **Duplicate submission prevention**: pending-dispatch reconciliation at startup
  (`reconcile_pending_dispatches`, R12) + per-slot attempt limits (`max_attempts_per_slot`)
  + inflight bundle caps (`max_inflight_bundles`).

## Safety controls (verified present)

| Control | Where |
|---|---|
| Live disabled by default | `ARB_MODE` env-only, default `simulate`; live requires ARB_MODE=live AND ARB_LIVE truthy AND CONFIRM-LIVE for runtime switches |
| Max exposure / borrow / cost | `RiskConfig` (max_trade_size_usd, max_daily_loss_usd, max_hourly_loss_usd, max_tip_spend_ratio) |
| Min conservative profit | config floor + `MIN_NET_PROFIT_USD` env raise |
| Provider health gates | RpcPool telemetry |
| State freshness gates | snapshot_slot + lag enforcement |
| Kill switch | control service (Telegram/web), mode re-resolution from env |
| Simulated vs realized PnL | separate ledgers; no simulated result is ever reported as executed |
| Secret hygiene | `.env` gitignored; no key material in logs (audited) |

## Known gaps

* Kamino supply-vault liquidity intermittently unverifiable at slot N (data-pipeline gap)
  → fails closed, skips flash-funded capture (documented, no unsafe fallback).
* 24-hour soak not yet run in this audit window (previous phases ran 3–4-minute live
  evidence windows with zero broadcast).
