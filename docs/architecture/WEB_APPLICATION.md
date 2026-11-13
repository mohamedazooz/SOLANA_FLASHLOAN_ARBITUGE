# Web Application — Architecture & API

Directive-2 deliverable (R6/R7/R8): one unified control service, one authenticated
Axum REST+WS API, one React dashboard. Telegram and the web app call the **same**
`execute_command()` — there is no second control plane.

## Components

| Layer | File(s) | Role |
|---|---|---|
| Control service | `crates/arb-bot/src/control_service.rs` | Single command authority: mode switching, kill switch, config overrides, bounded event rings (opportunities 256 / executions 256 / logs 1000), audit JSONL |
| REST+WS API | `crates/arb-bot/src/web_api.rs` | Axum server, session auth, rate limiting, WS fan-out, serves built React bundle |
| Telegram listener | `crates/arb-bot/src/telegram_control.rs` | Long-poll `getUpdates`; maps commands to the same `execute_command` |
| Frontend | `web/` (React 18 + TypeScript + Vite 6) | 9 panels A–I, 5 s REST poll + live WS updates with reconnect |

## Startup wiring (`crates/arb-bot/src/main.rs`)

1. `ControlService` created (audit log path from `WEB_AUDIT_LOG`,
   default `data/web_audit.log`).
2. `supervisor.attach_control(control)` — supervisor publishes slot, pool count,
   stream/RPC health into the shared snapshot each heartbeat tick
   (`publish_control_state`, `supervisor/run.rs`).
3. Supervisor mode becomes a shared `Arc<AtomicU8>`; web/Telegram mutate the same
   atomic the run loop reads (`Supervisor::mode()`).
4. `web_api::serve` spawned on `WEB_BIND` (default `127.0.0.1:8080` — **localhost
   only**; expose via reverse proxy, never 0.0.0.0).
5. `telegram_control::run_listener` spawned if `TELEGRAM_BOT_TOKEN` +
   `TELEGRAM_CONTROL_CHAT_IDS` are set; otherwise Telegram is disabled (alert-only).

## Security model

- **Fail-closed start**: the server refuses to start without a real
  `WEB_ADMIN_PASSWORD` (placeholder values rejected).
- **Sessions**: 32 random bytes → stored as SHA-256 hash in-memory; raw token is
  returned once at login. TTL `WEB_SESSION_TTL_SECS` (60..=86400, default 3600).
  Max 64 concurrent sessions; oldest evicted.
- **WS auth**: browsers cannot set headers on WebSocket, so
  `GET /api/ws?token=<session>` is accepted in addition to `Authorization: Bearer`.
- **Rate limits**: login 10/min/IP, mutating endpoints 60/min/IP
  (`X-Forwarded-For` honored so the reverse proxy's client IP is used).
- **Dangerous actions**: `live` mode requires `ARB_LIVE=1` **and**
  `confirm="CONFIRM-LIVE"`; emergency-stop engage/clear are audited.
- **Audit log**: every control command appended as JSONL
  (`ts, source, actor, command, ok`).
- **No secrets in the browser**: the frontend holds only the session token
  (`sessionStorage`); keys stay in server-side env.

## REST API

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/login` | — | `{password}` → `{token, expires_in}` |
| POST | `/api/logout` | Bearer | invalidate session |
| GET | `/api/status` | Bearer | mode, slot, pools, kill switch, providers, counters |
| GET | `/api/opportunities` | Bearer | ring buffer of captured opportunities |
| GET | `/api/executions` | Bearer | ring buffer of executions/drops/rejections |
| GET | `/api/logs` | Bearer | bounded log ring |
| GET | `/api/pnl` | Bearer | realized counters (quote-time economics only) |
| GET | `/api/providers` | Bearer | provider health snapshot |
| GET | `/api/config` | Bearer | active strategy config + overrides |
| POST | `/api/config` | Bearer | validated `ConfigOverrides` merge |
| POST | `/api/control/{action}` | Bearer | `status/start/stop/shadow/paper/live/opportunities/pnl/providers/config/emergency-stop/clear-emergency-stop` |
| GET | `/api/ws` | Bearer or `?token=` | live event fan-out |
| GET | `/health` | — | liveness (unauthenticated) |
| GET | `/` | — | serves `web/dist/index.html` (falls back to legacy `dashboard/`) |

`ConfigOverrides` validation: `min_net_profit_usd` ≥ 0 and finite;
`min_borrow_lamports ≤ max_borrow_lamports`; `slippage_bps` in 1..=10000.
Overrides are applied at the capture boundary (`capture_three_leg_dlmm`) —
min-net-profit is converted to lamports with the last known live SOL price and
max'ed with the TOML floor.

## Dashboard panels (A–I)

A **Overview** — 12 stat cards (mode, slot, pools, providers, opportunities,
executions, success rate, PnL, risk state).
B **Live Opportunities** — route, borrow, expected out, net profit, eligibility;
filters: protocol / eligible-only / min-net.
C **Pools & Liquidity** — tracked pools, freshness, provider source.
D **Execution Monitor** — signature, stage, provider, status, failure reason.
E **Strategy Config** — validated overrides form (same validation as API).
F **Providers** — health/latency/errors per provider.
G **Risk & Emergency** — start/stop/shadow/paper; live requires typing
`CONFIRM-LIVE`; emergency stop engage/clear.
H **Logs & Analytics** — live logs + counters.
I **Telegram** — connectivity, recent commands/alerts.

Frontend polls REST every 5 s and applies WS pushes live; WS reconnects with
backoff and re-authenticates via `?token=`.

## Local development

```bash
cd web && npm ci && npm run build     # outputs web/dist/ (git-ignored)
cargo run -p arb-bot                  # serves dist/ + API on 127.0.0.1:8080
npm run dev                           # Vite dev server, /api proxied to 127.0.0.1:8080
```

## Tests

- `control_service` — 12 unit tests (command matrix, override validation,
  mode gating, audit append).
- `web_api` — auth/control matrix (sequential env-safe) +
  `web_server_end_to_end_with_ws_upgrade`: real TCP listener on an ephemeral
  port, real login, authenticated REST call, raw WS handshake asserting
  `HTTP/1.1 101`.
