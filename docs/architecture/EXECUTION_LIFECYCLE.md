# Execution Lifecycle

Status: describes the implemented lifecycle and its guards. Durable, idempotent
bundle/signature/finality reconciliation is **not** accepted (`14.5` open).

## 1. Modes

| Mode | Builds tx | Simulates | Submits | Accounting |
|---|---|---|---|---|
| `shadow` (default) | Yes | No (unless `ENABLE_PREFLIGHT_SIMULATION=true`) | **No** | `simulated_net_profit_total`, outcome `simulated` |
| `simulate` | Yes | Same as above | **No** | Same as shadow |
| `canary` | Yes | Same as above | **No** | `simulated_net_profit_total`, outcome `canary`, label `Canary-Simulated-…` |
| `live` | Yes | **Mandatory** | Only if `ARB_LIVE` is truthy, after balance + stream checks | Realized counters only after landing evidence |

A mode is not a substitute for the `ARB_LIVE` gate: `--mode live` with
`ARB_LIVE=0` exits at startup (`main.rs`) and would not submit even if reached
(`dispatch.rs` checks the same truthiness helper).

## 2. Lifecycle

```
1. INGEST      account/slot updates → snapshots + stream generation
2. READY       quote_dependencies_for_route: owner/slot/write_version/bank/generation
3. QUOTE       checked per-hop quotes → EconomicQuote (expected + conservative)
4. SEARCH      graph from fresh pools only → candidate cycles
5. SIZE        optimal input under risk cap and pool liquidity
6. RISK        kill switch → breaker → velocity → losses → notional → age
               → impact → tip share/cap → min profit → inflight bundles
7. BUILD       Kamino borrow → swap(s) → repay, full-floor min-out
8. SIMULATE    Live only (or env flag): LiteSVM optional, RPC mandatory
9. FIRE        wait for leader window → final stream/generation guard
10. SEND       Jito bundle (multi-region) or SWQoS raw transaction
11. ACCEPT     bundle ID / signature accepted (NOT landing proof)
12. LAND       poll Jito statuses; reconcile signature/finality
13. RECORD     store outcome; metrics; Telegram; ledger
```

### Step 6 — risk

`RiskGate::evaluate_full` / `evaluate_flash_loan` reject on any of: kill switch,
open breaker, velocity ≥ `max_orders_per_hour`, daily/hourly loss, notional >
`max_trade_size_usd`, state age > `max_state_age_slots`, impact >
`max_price_impact_bps`, tip share/lamports over caps, net below the absolute/bps
floor, and (flash path) inflight bundles per slot. All checks are fail-closed.

### Step 7 — build

- The borrow amount is the sized input; the flash fee is read from the reserve
  state and ceiling-rounded.
- The final return hop uses the full profit floor: principal + flash fee + DEX
  fees + network + priority + tip + `min_net_profit_abs`. Slippage alone is not
  sufficient.
- Pump.fun/PumpSwap remain fail-closed. Split and Jupiter paths are disabled.
- Transaction size is checked against 1232 bytes before simulation (Live path)
  and rejected with an operator alert.

### Step 8 — simulate

- `BotMode::Live` always simulates.
- Shadow/Simulate/Canary simulate only when `ENABLE_PREFLIGHT_SIMULATION=true`.
- LiteSVM is optional (`arb-sim/litesvm`, off by default); when unavailable it
  reports an explicit "feature disabled" error and the RPC simulation still runs.
- RPC error, timeout, missing result, or `success=false` → no submission.
- A successful simulation is **not** landing proof and does not increment fills.

### Steps 9–11 — dispatch

- Leader-aware fire-time wait is only used for Jito routes with a known Jito
  leader; the wait is skipped when the target slot has no Jito leader.
- Final guard re-checks stream connection, generation, current slot, and
  `max_state_age_slots` immediately before submission.
- Submission result is an acceptance ID. It is recorded as `sent`; it is not a
  fill, not a signature, and not proof of landing.

### Steps 12–13 — reconciliation

- `arb-land` exposes `get_bundle_statuses`, `poll_bundle_until_terminal`,
  `BundleTracker`, and inflight status parsing.
- The store records attempt/outcome rows (`sent`, `simulated`, `canary`, etc.).
- **Gap**: there is no durable state row that atomically transitions
  accepted → landed/failed with an idempotency key, and no tested recovery for an
  ambiguous send timeout. A crash after send can leave an accepted bundle with no
  terminal record. This is why `14.5` stays OPEN.

## 3. Failure handling

| Failure | Behavior |
|---|---|
| Ingest disconnect | Stream readiness fails; no submission. Reconnect metrics increment. |
| Stale/missing/dead-bank account | `QuoteReadinessError`; route rejected. No stale fallback. |
| Quote overflow/underflow | Typed math error; route rejected. |
| Risk rejection | No build/send; rejection alert (above threshold). |
| Simulation failure/timeout | No send in Live. |
| Bundle send error | Alert; attempt outcome recorded as not sent. |
| Bundle accepted but not landed | Status polling; no realized PnL. Landing reconciliation is incomplete. |
| Kill switch active | Risk gate rejects every route; persistent marker survives restart. |
| Admin token missing/placeholder | Admin listener disabled; `/pause`/`/resume` unavailable. |
| Process restart | Kill-switch marker re-blocks; landing ledger reloads; in-memory inflight/breaker state resets. |

## 4. Idempotency and crash safety

- The kill-switch marker is durable and fail-safe.
- The adaptive landing ledger is persisted to `data/adaptive_ledger.json`.
- SQLite uses WAL.
- There is **no** exactly-once execution guarantee. A crash between signing and
  recording can produce an untracked transaction. Until a durable execution
  state machine with idempotent reconciliation is implemented and tested, live
  execution remains disabled by default and must not be authorized.
