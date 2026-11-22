# API REFERENCE — Phase 10 (Mission §15)

**Status:** ACTIVE — generated from `crates/arb-bot/src/web_api.rs:777–806` +
`crates/arb-bot/src/control_service.rs` (2026-11-22). Auth contract:
`Authorization: Bearer <token>`; token from `POST /api/login`; mutating endpoints are
per-IP rate limited; body limit 64 KiB; loopback bind by default.

---

## Sessions

| Endpoint | Body | Response |
|---|---|---|
| `POST /api/login` | `{"password": "…"}` | `{"token", "expires_at_unix"}` — 429 on window breach, 401 on bad password |
| `POST /api/logout` | — | `{"ok": true}` |

## Read endpoints (token required)

| Endpoint | Returns |
|---|---|
| `GET /api/status` | mode, paused, emergency_stopped, kill-switch persistence state (control_service.rs:398–422) |
| `GET /api/telemetry` | counters/histograms projection |
| `GET /api/ticker` | Sol price |
| `GET /api/opportunities` | rolling digest |
| `GET /api/executions` | execution records |
| `GET /api/rejections` | rejection log (stage/reason) |
| `GET /api/logs` | event tail |
| `GET /api/pnl` | PnL summary (mode-aware: live vs paper) |
| `GET /api/providers` | RPC/Jito provider health |
| `GET /api/routes` | route monitor |
| `GET /api/config` | current threshold overrides |

## Mutating endpoints (token + mutation rate limit)

| Endpoint | Body / Param | Semantics |
|---|---|---|
| `POST /api/control/status` | — | read-only pass-through |
| `POST /api/control/start` / `resume` | — | resume trading loop (mode-bound) |
| `POST /api/control/stop` / `pause` | — | pause execution |
| `POST /api/control/simulate` (or `paper`) | — | simulate/paper mode |
| `POST /api/control/live` | `{"confirm": "CONFIRM-LIVE"}` | **double gate**: requires `ARB_LIVE=1` env + confirmation token, else 400 fail-closed |
| `POST /api/control/emergency-stop` | — | engage emergency stop (logged) |
| `POST /api/control/clear-emergency-stop` | — | release (logged) |
| `POST /api/config` | ConfigOverrides (min_net_profit_usd, borrow bounds, slippage, hop_slippage…) | threshold changes go through the ControlService ledger |
| `POST /api/ws-ticket` | — | `{"ticket", "expires_…"}` — single-use |

## Realtime

- `GET /api/ws?ticket=<single-use>` — event stream (status/opportunity/execution);
  ticket is consumed on connect; server rejects unknown tickets (`web_api.rs:443`, test
  at :1242).

## Unauthenticated

| Endpoint | Purpose |
|---|---|
| `GET /health` | liveness ("ok") — no data |
| `GET /`, `/dashboard`, `/assets/{*path}` | static SPA shell (assets restricted path traversal guard) |

## Telegram control (parallel surface)

Commands dispatch through the SAME `execute_command` gate as `/api/control/*`
(control_service.rs:755+), including the CONFIRM-LIVE requirement and emergency-action
logging. Admin token checked via constant-time compare (`main.rs:370–372`).

## Error model

All handlers return `{"ok": false, "action": …, "error": …}` with 400/401/429 codes;
errors are logged server-side; no stack traces or secrets in responses (audited).
