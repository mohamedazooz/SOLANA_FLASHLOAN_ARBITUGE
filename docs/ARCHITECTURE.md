# Architecture

Status: describes the code as it exists at baseline `541cdec` plus the
remediation in the current working tree. It is a map, not an approval. Where the
runtime is unproven this document says so explicitly.

## 1. Runtime topology

```
                    ┌──────────────────────────────┐
  env / .env ──────▶│ arb-bot config loader        │
  config.toml       │ (validation, MAX_TRADE_SIZE) │
                    └──────────────┬───────────────┘
                                   │
      ┌────────────────────────────┼─────────────────────────────┐
      │                            │                             │
      ▼                            ▼                             ▼
┌──────────────┐          ┌──────────────────┐          ┌────────────────┐
│ arb-ingest   │          │ supervisor::run  │          │ observability  │
│ Yellowstone  │─────────▶│ route search +   │─────────▶│ Prometheus     │
│ gRPC / WS    │  pools   │ economics + risk │          │ Telegram       │
│ RPC reconcile│          └────────┬─────────┘          │ event log      │
└──────────────┘                   │                    └────────────────┘
                                   ▼
                         ┌──────────────────┐
                         │ arb-tx builder   │
                         │ Kamino borrow →  │
                         │ swaps → repay    │
                         └────────┬─────────┘
                                  ▼
                         ┌──────────────────┐          ┌────────────────┐
                         │ arb-sim (RPC /   │          │ arb-risk gate  │
                         │ optional litesvm)│          │ kill switch    │
                         └────────┬─────────┘          └────────────────┘
                                  ▼
                    ┌───────────────────────────┐
                    │ supervisor::dispatch      │
                    │ mode gate → Jito / SWQoS  │
                    └──────────────┬────────────┘
                                   ▼
                    ┌───────────────────────────┐
                    │ arb-land + arb-store      │
                    │ bundle status / outcomes  │
                    └───────────────────────────┘
```

The diagram is the intended single path. The code currently has **two quote
paths**: the supervisor's direct `quote_route` path (used) and the opt-in
`arb_engine::guarded` path (defined, not wired — see `GUARD-01`). No claim is
made that every binary uses the same path.

## 2. Workspace crates

| Crate | Responsibility | Runtime status |
|---|---|---|
| `arb-constants` | Program IDs, mints, seeds, Jito tip accounts, protocol discriminators. | Used. |
| `arb-types` | Newtypes (`Lamports`, `Slot`, `TokenAmount`, `Bps`), `PoolState`, `SwapHop`, `EconomicQuote`, errors. | Used. |
| `arb-math` | Checked CPMM/CLMM/DLMM math, Q64.64 helpers, cost ledger/net-profit arithmetic. | Used by engine/tx. |
| `arb-dex` | Pool decoders and quote adapters: Raydium v4/CPMM/CLMM, Orca Whirlpool, Meteora DLMM, Pump.fun, PumpSwap. | Decoders used; Pump live execution disabled. |
| `arb-engine` | Route graph, Bellman-Ford search, optimal sizing, guarded quote. | Graph/sizing used; guarded quote unused. |
| `arb-ingest` | Yellowstone gRPC, provider supervisor, slot integrity, snapshot reconcile, quote readiness, discovery providers. | Used; provider-specific parity not fully proven. |
| `arb-risk` | Limits, circuit breaker, persistent kill switch, Chronos EV gate. | Used before build/dispatch. |
| `arb-sim` | RPC `simulateTransaction`, optional LiteSVM, fail-closed defaults. | Used only in Live or when `ENABLE_PREFLIGHT_SIMULATION=true`. |
| `arb-tx` | Instruction/transaction builders, Kamino flash-loan borrow/repay, address lookup tables, Jupiter (disabled). | Used; Pump.fun and PumpSwap builders fail closed. |
| `arb-land` | Jito bundle submission, tip policy, leader schedule, landing ledger, status polling. | Used in Live only. |
| `arb-store` | SQLite (WAL) store, optional Postgres, attempts/fills/daily PnL, outcomes. | Used. |
| `arb-metrics` | Prometheus registry and counters/gauges. | Used; simulated vs realized profit are separate counters. |
| `arb-mcp` | Standalone MCP server/library. | Not part of the bot runtime. |
| `arb-bot` | Config, supervisor, dispatch, admin HTTP, Telegram, binaries. | Entry point. |

`webhook-service/` is intentionally excluded from the Cargo workspace and is not
built by the bot image.

## 3. Binaries

| Binary | Purpose | Execution risk |
|---|---|---|
| `arb-bot` | Main runtime. `--mode shadow|simulate|canary|live`. | Live requires `ARB_LIVE` truthy and explicit mode. |
| `alchemy-webhook` | Receives Alchemy webhook events. | Separate process; no trading authority. |
| `live-scanner` | Scanning utility. | Must not be treated as the audited execution path. |
| `validate` | Config/startup validation. | Read-only. |
| `discover` | Pool discovery. | Read-only. |
| `latency-bench` | Slot/RPC latency diagnostic. | Read-only; never builds swaps. |
| `replay` | Offline replay utility. | Offline. |

Only `arb-bot` is covered by the mode gating described in
[`EXECUTION_LIFECYCLE.md`](EXECUTION_LIFECYCLE.md). Auxiliary binaries need a
separate guard review before they are used in production.

## 4. Data flow

1. **Ingest** — Yellowstone gRPC streams account updates and slot status. The
   client tracks a stream generation, current slot, and per-account
   `AccountSnapshot` (owner, slot, write version, provider, bank, generation).
   RPC reconciliation can bootstrap missing accounts; `quote_dependencies_for_route`
   returns a typed `QuoteReadinessError` when anything is stale, missing, or from
   a dead/mixed bank.
2. **State construction** — the registry builds `PoolState` values from decoded
   account bytes. Pools must be fresh (`max_state_age_slots`) to be quoteable; a
   stale pool is never used as a fallback (`L-05`).
3. **Route search** — the supervisor builds a graph from fresh pools only, with
   `strategy.graph_max_age_slots` as an explicit horizon (defaults to the quote
   freshness policy). Bellman-Ford/negative-cycle search produces candidate
   cycles; optimal sizing is bounded by pool liquidity and
   `risk.max_trade_size_usd`.
4. **Economics** — each hop is quoted with checked integer math. The single-route
   builder computes a full profit floor including flash fee, DEX fees, network
   fee, priority fee, tip, and `min_net_profit_abs`; the result is compared with
   the conservative min-out, not the expected output.
5. **Risk** — `RiskGate` checks kill switch, circuit breaker, velocity, daily and
   hourly loss, per-trade USD notional, state age, price impact, tip share,
   tip cap, minimum profit, and (flash path) inflight bundles.
6. **Build** — `TxBuilder` creates borrow → swap(s) → repay instructions with
   the Kamino fee read from the reserve state. Pump.fun/PumpSwap and Jupiter
   routes fail closed in this build.
7. **Simulate** — in Live (or with the env flag), RPC simulation is mandatory;
   any error, timeout, or `success=false` rejects. LiteSVM is optional and fails
   closed when the feature is off.
8. **Dispatch** — mode-gated. Shadow/Simulate/Canary never submit. Live submits
   only when `ARB_LIVE` is truthy, after balance and final stream-readiness
   checks.
9. **Observe** — outcomes are recorded in the store; Telegram alerts are
   labelled; Prometheus exposes separate realized/simulated series.
10. **Reconcile** — bundle IDs are tracked via Jito status polling. A bundle ID
    is never treated as a transaction signature or as landing proof. Durable
    idempotent reconciliation is **not** accepted (`14.5` open).

## 5. Trust boundaries

| Boundary | Trust assumption | Failure mode |
|---|---|---|
| Market data | Yellowstone/RPC provider is honest and low-latency. | Stale/dead/mixed-bank data must be rejected by readiness checks; provider outage must degrade to no trades. |
| RPC simulation | Simulation reflects the same bank and state as the bundle. | Simulation error/timeout rejects. A successful simulation is not landing proof. |
| Jito | Bundle acceptance means "accepted", not "landed". | Status must be polled; bundle ID must never be logged as a signature. |
| Operator | Admin token holder is authorized. | Missing/placeholder token disables the admin listener; `/pause` and `/resume` require a bearer token. |
| Secrets | `.env`/keypair are provided out of band. | No secret is committed; the image excludes `.env` and keypairs. Rotation is a manual operator procedure. |
| Dependencies | crates.io/GitHub content is trustworthy. | `cargo-deny` policy + lockfile; `cargo-audit` currently unverified due to network. |

## 6. Deployment

- **Image**: multi-stage Debian, non-root `arb`, `curl` healthcheck against
  `127.0.0.1:9100/metrics`, config mounted read-only, data on a volume.
- **Compose**: bot + `alchemy-webhook` + Postgres + Redis. Postgres/Redis have no
  host ports; bot ports bind to loopback. Default command is `--mode shadow`.
- **Config**: `config/config.example.toml` is the shipped reference. Real
  deployments must provide `config/config.toml` and `.env` out of band. The
  image copies the example as a safe default so a missing mount fails with a
  clear configuration error, not a missing-file error.
- **State**: `data/` holds SQLite, adaptive landing ledger, and the kill-switch
  marker. Lose it and the bot restarts with an empty ledger; an existing marker
  keeps trading blocked.

## 7. Known architectural gaps

1. `arb_engine::guarded` is not the only quote path (`GUARD-01`).
2. Split routing and Jupiter execution are intentionally disabled in the
   supervisor; the code for split parsing remains unreachable.
3. No dashboard is connected (`Gate F` open). Prometheus + Telegram are the only
   operator surfaces.
4. Replay/fault-injection acceptance and a real shadow soak are not established.
5. The auxiliary binaries are not governed by the main mode gate.
