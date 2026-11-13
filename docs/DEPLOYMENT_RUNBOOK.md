# Deployment Runbook — VPS

Target: the existing VPS deployment architecture (no local Agave validator, no
full RPC node). All market data and transport go through configured providers
(Helius RPC/WSS primary; Solami/DRPC per provider policy).

## 1. Environment

Copy `.env.example` → `.env` and fill in. `.env` is git-ignored — never commit it
and never echo its contents in tickets or logs.

Required for operation:

| Key | Purpose |
|---|---|
| `HELIUS_API_KEY` / `HELIUS_RPC_URL` / `HELIUS_WSS_URL` / `HELIUS_BETA_URL` | RPC + WS market data + transaction transport |
| `WEB_ADMIN_PASSWORD` | dashboard login (**required**, placeholders rejected) |
| `WEB_BIND` | default `127.0.0.1:8080` — keep localhost |
| `WEB_SESSION_TTL_SECS` | 60..=86400 (default 3600) |
| `WEB_AUDIT_LOG` | JSONL audit path (default `data/web_audit.log`) |
| `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CONTROL_CHAT_IDS` | optional Telegram control; absent → disabled |
| `MIN_NET_PROFIT_USD` | net-profit floor override (e.g. `0.05`) — max'ed with TOML floor |
| `SOL_USD_PRICE` | fallback SOL price for the floor conversion |
| `ARB_LIVE=0`, `DRY_RUN=true` | **safety defaults — see §6** |

Wallet keys, if ever configured for a live transition, belong only in `.env` /
secret manager — never in `config.toml` or the repo.

## 2. Build

Docker (multi-stage; builds the React bundle in a `web-builder` stage and copies
`web/dist` into the runtime image):

```bash
docker build -t arb-bot .
docker run -d --name arb-bot \
  --env-file .env \
  -p 127.0.0.1:8080:8080 \
  -v arb-data:/app/data \
  arb-bot
```

Native:

```bash
cd web && npm ci && npm run build && cd ..
cargo build --release -p arb-bot
ARB_LIVE=0 DRY_RUN=true ./target/release/arb-bot
```

## 3. Reverse proxy + HTTPS (Caddy example)

The bot API binds localhost only. TLS terminates at the proxy:

```caddy
bot.example.com {
    encode zstd gzip
    reverse_proxy 127.0.0.1:8080
}
```

Caddy provisions/renews certificates automatically. Nginx equivalent: `proxy_pass
http://127.0.0.1:8080;` + `proxy_set_header X-Forwarded-For $remote_addr;` (the
API uses this header for rate-limit identity) + WebSocket upgrade headers for
`/api/ws`. Ensure the proxy sets `X-Forwarded-For` so rate limits key on real
client IPs.

## 4. Smoke test after deploy

```bash
curl -s http://127.0.0.1:8080/health
curl -s -X POST http://127.0.0.1:8080/api/login \
  -H 'content-type: application/json' -d '{"password":"<WEB_ADMIN_PASSWORD>"}'
# then from the browser: https://bot.example.com → login → Overview
```

Verify in the dashboard: mode shows `shadow`, providers show Helius connected,
slot is advancing, WS indicator is live.

## 5. Telegram control

1. Create the bot with @BotFather → token into `TELEGRAM_BOT_TOKEN`.
2. Send `/status` from your account; find your chat id from the audit log or
   bot logs; put it in `TELEGRAM_CONTROL_CHAT_IDS` (comma-separated allowlist).
3. Restart. Commands `/status /start /stop /shadow /paper /live /opportunities
   /pnl /providers /config /emergency-stop /clear-emergency-stop` hit the same
   `execute_command` as the web UI. `/live` still requires `ARB_LIVE=1` +
   explicit confirm.

## 6. Live-transition policy

`ARB_LIVE=0` / `DRY_RUN=true` are the shipping defaults. Live execution requires
**all** of: operator edits `.env` (`ARB_LIVE=1`), restarts the process, selects
`live` in the dashboard **and** types `CONFIRM-LIVE`, and the command is audited.
Anything less fails closed. No automatic live activation exists.

## 7. Operations

- Logs: `journalctl -u arb-bot` (systemd) or `docker logs -f arb-bot`; audit:
  `data/web_audit.log`; dashboard Logs panel.
- Emergency stop: dashboard Risk panel or `/emergency-stop` — halts dispatch
  immediately; clear with `/clear-emergency-stop` after review.
- Restart safety: the process is stateless across restarts except rings/audit
  log; blockhash staleness is handled per-dispatch. (Execution state persistence
  for ambiguous-timeout recovery is a **known gap** — see discovery report R12.)
- Update: `git pull && docker build && docker compose up -d` (or rebuild native
  binary); no data migration needed.

## 8. CI

`.github/workflows/rust-ci.yml`: fmt + clippy `-D warnings` + workspace tests +
`cargo audit`, plus a `web` job (Node 22, `npm ci`, `npm run build`).
