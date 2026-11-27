# Reverse Engineering — Solana Arbitrage Bot Execution Path

**Review date:** 2026-11-27
**Source baseline:** `main` at merge commit `34364ad6d36eea58643f3c7d15a1e8534dc4ae82` (PR #204)
**Review type:** Static source review; no runtime configuration or credential values inspected.

## Scope and safety

This document describes the current source-level execution path and the gates that must pass before a transaction can be broadcast. It is not a claim that the deployed bot is currently armed, healthy, profitable, or able to land transactions. Runtime environment values, wallet state, RPC credentials, `.env` contents, and provider entitlements were not inspected for this revision. No credential values are included. Do not infer live readiness from code-level capability alone.

The previous 2026-11-26 draft contained environment-specific assertions and stale findings. This revision removes those claims and corrects the source findings that changed: `base_tokens` is now consumed to build graph anchors; V1 compile and dispatch use the same `v1_transport_flags` helper; graph freshness is capped by `max_state_age_slots`; derived-account hydration is event-driven with a 50 ms coalescing delay and a 5 s fallback sweep; and the old “only 9 pools” statement was not a valid interpretation of the registry.

## 1. Startup and mode selection

- `crates/arb-bot/src/main.rs::parse_args` resolves configuration in this order: `ARB_CONFIG`, `--config` / `-c`, then the canonical `config/config.toml` path.
- If the canonical file is absent and neither an environment path nor CLI path is supplied, startup fails before the Tokio runtime. The error directs the operator to create `config/config.toml`, set `ARB_CONFIG`, or pass `--config`.
- Mode is selected from `ARB_MODE` when supplied; otherwise canary/live arming flags are considered and the safe fallback is `observe`. A `shadow-only` build forces `simulate`.
- `live_execution_enabled()` is a separate opt-in check. In a non-shadow build it accepts a truthy `ARB_LIVE`, `LIVE_TRADING`, `LIVE`, or `EXECUTE_LIVE`. Selecting `ARB_MODE=live` alone is not sufficient to authorize live dispatch.

## 2. Market-data ingestion and discovery

- `crates/arb-ingest/src/helius_ws.rs` defines an 8-second WebSocket handshake timeout.
- `WS_MAX_ACCOUNT_SUBS` controls the account-subscription budget. Invalid or zero values fall back to 48; the configured value is capped at 1,024.
- Subscription cost is venue-dependent: Raydium V4/CPMM pools consume two vault subscriptions; Orca/DLMM pools consume one pool-account subscription. Actual coverage therefore depends on discovered pools, account layout, provider limits, and the configured budget—not on a fixed number of pools.
- The stream seeds required accounts, subscribes to slot/account updates, and applies live updates through the ingestion client. Older/out-of-order state is guarded by slot/write-version checks.
- Derived-account hydration is event-driven: notifications are coalesced for 50 ms before draining pending work, with a 5-second fallback sweep. This replaces the older draft's blanket 500 ms polling claim.
- HTTP catalog discovery is conditional on `strategy.http_discovery_enabled`. When disabled, candidates must arrive through the WebSocket account-update stream or other explicitly wired discovery paths. Production pool counts must be measured from a live registry snapshot; the registry's venue adapters are not a pool count.

## 3. Graph construction and candidate generation

- `crates/arb-bot/src/supervisor.rs` resolves configured `strategy.base_tokens` through the verified anchor-mint resolver and passes the resulting anchors to `MarketGraph::new`. WSOL is used as a fallback only when no configured anchor resolves. The old claim that `base_tokens` is ignored is obsolete.
- `crates/arb-bot/src/supervisor/strategy.rs` filters pools by slot freshness before graph construction.
- The effective graph age is `min(configured graph_max_age_slots or max_state_age_slots, max_state_age_slots)`, then bounded to at least one slot. A graph-age setting cannot make a pool older than the quote/state freshness policy eligible.
- Fresh pools feed graph construction and cycle discovery. The resulting opportunities are ranked before economic and execution gates. A pool being registered or hydrated does not imply that it forms a valid cycle or is executable.

## 4. Economic and risk gates

`crates/arb-bot/src/supervisor/opportunity.rs` applies fail-closed checks before assembly. The active path includes route/account dependency validation, flash-liquidity and repayment sizing, flash-fee and network-cost accounting, minimum-profit and contingency floors, price/ROI plausibility checks, leader/transport suitability, Token-2022 transfer-fee verification, state freshness, Chronos expected-value checks, and risk-gate evaluation.

A quote-level positive edge is not realized profit. It remains an estimate until the transaction lands and pre/post balance evidence is available. A missing or unverifiable required input should reject the opportunity rather than silently assume zero cost.

## 5. V1 capability gates and transaction assembly

- `Supervisor::v1_transport_flags()` in `crates/arb-bot/src/supervisor/dispatch.rs` is the shared source for both opportunity compilation and dispatch. The earlier reported drift between `opportunity.rs` and `dispatch.rs` is superseded.
- Direct V1 requires `SOLANA_TX_V1_DIRECT_ENABLED`, `SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED`, and an RPC allowlist accepted by `RpcPool::has_v1_rpc_allowlist()`.
- Jito V1 requires `SOLANA_TX_V1_JITO_ENABLED` and `SOLANA_TX_V1_JITO_INGRESS_VERIFIED`.
- Simulate mode has an explicit capability bypass for compilation/testing; it does not authorize live broadcast.
- Assembly is performed through the supervisor execution path and the canonical transaction builder. The builder validates route closure, mint chaining, hop inputs, repayment/profit floor, compute-unit bounds, and atomic borrow/swap/repay integrity before compilation.
- The V1-only compiler is used; the implementation must reject transactions that exceed configured V1 limits rather than silently downgrade or remove instructions. Exact account, instruction, and wire-size limits must be checked against the constants in the current compiler before changing routing policy.

## 6. Dispatch, broadcast, and landing

- Live dispatch independently checks `live_execution_enabled()` and that the HTTP transaction RPC adapter is configured.
- The dispatch path applies transport capability checks and a mandatory preflight simulation (`sim_enabled` is currently set to `true` in the source path reviewed).
- Before sending, it rechecks kill-switch and stream/state readiness conditions and persists dispatch intent. Persistence failures are fail-closed.
- The selected transport sends through the configured SWQoS/direct RPC path or Jito bundle path. A successful send/accepted bundle is not proof of landing.
- Landing tracking and reconciliation resolve ambiguous sends using signature/block-height evidence. PnL provenance must distinguish actual balance-derived results from estimates.

## 7. Operator verification checklist

Before enabling any live mode, verify all of the following against the deployment itself:

1. The exact config file selected by `ARB_CONFIG` / `--config` exists and passes config validation.
2. Mode and live-arming flags are intentional; shadow-only builds remain non-broadcasting.
3. WebSocket endpoint capability, account-subscription limits, reconnect/failover behavior, and actual pool-update coverage are measured.
4. Discovery is intentionally enabled or the WebSocket-only pool universe is sufficient and observable.
5. Base-token anchors resolve as intended; graph and quote freshness windows agree.
6. Flash-loan liquidity, fee calculations, Token-2022 transfer fees, network fees, priority fees, and Jito tips are independently verified.
7. V1 direct/Jito verification flags and the RPC allowlist match the actual provider endpoints and have passed a controlled canary.
8. Preflight simulation, durable dispatch-intent persistence, kill-switch behavior, landing reconciliation, and actual PnL provenance are tested.
9. No credentials are copied into reports, logs, command lines, or CI artifacts. Rotate credentials only if a real exposure is confirmed by the operator/security review.

## 8. Verification and limitations

This document is a static code-path review. It does not claim a live RPC simulation, Jito ingress canary, mainnet landing, successful production boot, or realized PnL. No environment values or secret material were read for this revision. CI for the code changes in PR #204 passed before that PR was merged; this documentation-only PR must receive its own CI result.

## 9. Source files reviewed

- `crates/arb-bot/src/main.rs`, `crates/arb-bot/src/lib.rs`
- `crates/arb-bot/src/supervisor.rs`
- `crates/arb-bot/src/supervisor/{run,strategy,opportunity,dispatch,execution,recovery}.rs`
- `crates/arb-ingest/src/{helius_ws,registry,rpc_pool,lib}.rs`
- `crates/arb-engine/src/lib.rs`
- `crates/arb-tx/src/{primitives/builder,transport/adaptive,transport/selection}.rs`
- `config/{base,production,config.example}.toml`

---

*End of report. This is a source-level architecture description, not a live-readiness certification.*
