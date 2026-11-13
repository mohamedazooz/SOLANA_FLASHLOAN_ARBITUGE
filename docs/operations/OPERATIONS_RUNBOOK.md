# Operations Runbook

This runbook is for running the bot in `shadow`/`simulate`/`canary`. **Live is
disabled by default and must not be authorized until `RELEASE_READINESS.md`
gates pass.** Commands are written for a Linux host; Windows is not a supported
build target for this workspace (see `BUILD-01`).

## 1. Prerequisites

- Linux x86_64, Docker 24+ / Compose v2, or a Rust 1.99.1 toolchain.
- Rust toolchain pinned by `rust-toolchain.toml`; do not use a moving `stable`.
- `.env` created from `.env.example` with real values, `chmod 600`.
- `config/config.toml` created from `config/config.example.toml`; never commit it.
- Keypair file mounted read-only if the strategy needs a signer; in shadow mode
  no signer is required.
- Persistent `data/` volume; `KILL_SWITCH_FILE` must point inside it.
- Outbound HTTPS to the configured gRPC/RPC/Jito/Telegram providers.

## 2. First start (shadow)

```bash
cp .env.example .env
# fill values, then:
chmod 600 .env
mkdir -p config data
cp config/config.example.toml config/config.toml
# edit config/config.toml (endpoints, mode=shadow)

docker compose config -q
docker compose up -d postgres redis
docker compose up -d arb-bot
docker compose logs -f arb-bot
```

Expected startup log: configuration loaded, mode `shadow`, no admin listener
warning if a valid `SENTINEL_ADMIN_KEY` is set. If the admin key is missing or a
placeholder, the log explicitly says the admin API is disabled.

## 3. Health and status

```bash
curl -fsS http://127.0.0.1:9100/metrics | head
curl -fsS http://127.0.0.1:9101/health
curl -fsS http://127.0.0.1:9101/status
```

`/status` returns `RUNNING` or `PAUSED`. Metrics include `fills_total`,
`net_profit_total`, `simulated_net_profit_total`, `state_age_slots`,
breaker state, tip share, and Pump spread/graduation counters. Shadow and
canary edges appear **only** in the simulated series.

## 4. Pause and resume (authenticated)

```bash
# pause
curl -fsS -X POST http://127.0.0.1:9101/pause \
  -H "Authorization: Bearer $SENTINEL_ADMIN_KEY"

# resume
curl -fsS -X POST http://127.0.0.1:9101/resume \
  -H "Authorization: Bearer $SENTINEL_ADMIN_KEY"
```

`X-Admin-Token: <key>` is also accepted. A missing/wrong token returns `401`.
Pause writes `data/kill_switch`; resume removes it. If the file exists at
startup, the bot starts `PAUSED` even if the admin API is unreachable.

Pause is not a substitute for stopping the process in an emergency; both are
acceptable. Never delete the marker manually while the bot is running — use
`/resume`.

## 5. Kill switch

| Action | Effect |
|---|---|
| `POST /pause` | Marker written; `RiskGate` rejects every route; survives restart. |
| `touch $KILL_SWITCH_FILE` | Same as pause after the next `is_blocked()` check. |
| `POST /resume` | Marker removed; trading allowed again (still subject to all gates). |
| Stop the process | No new orders; an already-sent bundle may still land. |

`KILL_SWITCH_FILE` defaults to `data/kill_switch`. Set it explicitly in
production and put it on persistent storage.

## 6. Provider outage / degraded data

Ingestion is WS + HTTP-RPC only (gRPC transport was removed from the tree; see
`../PROJECT_STATE.md`).

1. Watch `state_age_slots` and the provider health logs.
2. If staleness exceeds `max_state_age_slots`, quotes are rejected by design; no
   action is needed beyond restoring the feed.
3. If the primary provider is down, configure the failover endpoints in `.env`
   (`SOLANA_RPC_URL` / `SOLANA_RPC_PRIMARY` / `SOLANA_MAINNET_RPC_FALLBACK`,
   `RPC_MAX_CONCURRENT_REQUESTS`, `RPC_MIN_INTERVAL_MS`; WS subscriptions via
   `SOLANA_WS_1..3_URL`, `WS_COMMITMENT`, `WS_ENCODING`); the provider supervisor
   tracks health. Do not disable freshness checks to "keep trading".
4. If RPC simulation fails repeatedly, Live dispatch stops (fail closed). The
   preflight simulation before dispatch is mandatory (`ENABLE_PREFLIGHT_SIMULATION`
   controls only the earlier optional gate; the dispatch-time simulation cannot
   be turned off by environment). Shadow continues to evaluate but does not send.
5. If Jito is degraded, landing rate falls; do not raise the tip cap to
   compensate without an explicit budget decision.

## 6.1 Transaction-format transport gates (V0 / V1)

- Legacy/V0 remains capped at the 1,232-byte MTU; V1 (4,096-byte MTU, no ALT)
  is mainnet-active but **opt-in per transport**:
  - `SOLANA_TX_V1_DIRECT_ENABLED` — direct RPC/SWQoS `sendTransaction` path.
  - `SOLANA_TX_V1_JITO_ENABLED` — Jito bundle ingress.
- Both default to `false` (fail-closed). Enable only after every configured
  endpoint/validator path is verified V1-capable.
- `BYPASS_RISK_GATE` (diagnostics) is honored **only** in non-live modes and
  logs a `warn!` on every activation; in live mode it is ignored regardless of
  its value.

## 7. Restart recovery

1. `docker compose restart arb-bot` (or `systemctl restart`).
2. Confirm the mode, config path, and kill-switch state in the logs.
3. If the bot was paused, it stays paused; call `/resume` only after the cause is
   understood.
4. Check `data/` for the SQLite DB, adaptive ledger, and marker.
5. In-memory breaker/inflight state resets on restart. The landing ledger and
   marker persist. Durable execution reconciliation is **not** implemented; if a
   bundle was in flight during the restart, check the chain/Jito manually.

## 8. Logs and retention

- Container logs: `docker compose logs -f arb-bot`.
- Event log: set `ARB_EVENT_LOG` to a file; rotate it with the host logrotate.
- Never log secrets. Redact provider URLs/tokens before sharing logs.
- Keep at least 14 days of shadow logs for incident review.

## 9. Incident response

1. **Pause first** (`/pause` or stop the container).
2. Preserve evidence: logs, `data/`, the exact commit SHA, config (redacted).
3. If funds are at risk, move them to a safe address.
4. Rotate any credential that may have leaked; see [`SECURITY.md`](SECURITY.md).
5. Identify whether the incident was data, logic, execution, or credential
   related; do not resume until the root cause is fixed and tested.
6. Record timeline, impact, and remediation. No secret values in the report.

## 10. Pre-live checklist (do not skip)

- [ ] All Gate A–F items in `RELEASE_READINESS.md` are PASS with evidence.
- [ ] A shadow soak of at least 24h (preferably 72h) with zero unexplained
      rejects and truthful metrics.
- [ ] Fork/devnet simulation of the exact borrow→swap→repay route succeeds.
- [ ] Kill switch, admin auth, restart recovery, and provider-outage drills pass.
- [ ] Credentials rotated; no secret in git history/artifacts.
- [ ] Operator approval recorded out of band; `ARB_LIVE` set only for the run.
- [ ] Tip/risk budgets explicitly approved for the session.
- [ ] Rollback plan: stop + resume shadow, no live transaction left ambiguous.

## 11. Useful commands

```bash
# verify the workspace on Linux (Docker reference environment)
docker run --rm --network host -v "$PWD:/work" -w /work \
  -v arb-cargo-target:/target -e CARGO_TARGET_DIR=/target \
  rust:1.99.1-slim-bookworm bash scripts/verify-linux.sh

# format check / tests
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --lib --bins --tests --no-fail-fast --locked

# compose validation
cp .env.example .env && docker compose config -q
```

Do not run `cargo fmt --all` on a dirty tree without reviewing the diff; it
rewrites formatting only, but a large diff obscures the audit trail.

## 12. Support matrix

| Mode | Supported here | Notes |
|---|---|---|
| shadow | Yes | Default; no submission. |
| simulate | Yes | Same as shadow in this build. |
| canary | Yes | No submission; simulated metrics. |
| live | **No** | Gated by `ARB_LIVE` and incomplete acceptance. Do not enable. |
| Windows native | **No** | Use Linux/Docker (see `BUILD-01`). |
