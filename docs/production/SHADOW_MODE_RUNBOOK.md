# Shadow Mode Runbook

Shadow mode = full pipeline (ingest → discovery → sizing → compilation →
RPC simulation → recovery bookkeeping) with **zero network dispatch**.
No wallet secret is required; the shadow path builds an unsigned V1
envelope and simulates it with `sigVerify=false`. Nothing is ever sent.

## 1. Preconditions

- Linux + Rust 1.99.1 (`rust-toolchain.toml`); `.env` present (git-ignored)
  with Helius RPC/WSS credentials.
- Confirm: `ARB_LIVE=0`, `DRY_RUN=true` in `.env`. The binary refuses live
  dispatch without both `ARB_LIVE=1` AND `SOLANA_TX_V1_DIRECT_ENABLED`, and
  shadow mode never dispatches regardless.

## 2. Start

```bash
set -a; . ./.env; set +a
cargo run --release -p arb-bot --bin arb-bot
```

- Web dashboard: `http://127.0.0.1:<port>` (login with `WEB_ADMIN_PASSWORD`).
- Telegram listener starts only if `TELEGRAM_CONTROL_CHAT_IDS` is set.

## 3. What to watch (dashboard sections)

| Section | Expectation in shadow |
|---|---|
| A · Overview | mode=shadow, slot advancing, pools tracked > 0 |
| B · Live Opportunities | opportunities with net profit ≥ `MIN_NET_PROFIT_USD`; rejections visible |
| C · Pools | state freshness < `max_state_age_slots`; per-protocol coverage in J |
| D · Execution Monitor | attempts recorded with stage outcomes; **never** "sent" in shadow |
| G · Risk | kill switch state; emergency-stop test (below) |

## 4. Daily verification checklist

1. Slot freshness: no `stale state` rejections trending up (J · Route Monitor).
2. Simulation success rate: recorded in attempt outcomes; investigate >5% failure.
3. Predicted-vs-observed: for a sample of simulated opportunities, compare the
   simulated `amountOut` against the pool's next observed state change.
4. Resource usage: RSS/CPU stable (no monotonic growth).
5. Logs: no unexplained errors; `dispatch_aborted_*` must be absent in shadow.
6. Reconnect behavior: kill the WSS connection once per day; verify backoff +
   jitter reconnect and **no stale-data execution** during the gap.

## 5. Control-plane drills (run each, verify from BOTH web + Telegram)

- `stop` → new opportunities stop being dispatched (attempts stop advancing).
- `start` → resumes.
- `emergency-stop` → everything blocked; `start` refuses until
  `clear-emergency-stop`; after clearing, execution stays paused until an
  explicit `start` (latch semantics — verified by unit test).
- Config change (e.g. `MIN_NET_PROFIT_USD=0.05`) → both interfaces show the
  same value; invalid values are rejected.

## 6. Soak exit criteria (Gate 2 of LIVE_READINESS_CHECKLIST.md)

- ≥24 h continuous run.
- Written report: opportunity count, rejected count, simulation success
  rate, stale-rejection rate, predicted-vs-observed discrepancies,
  resource usage.
- Zero unexplained transaction states, zero dispatch attempts in shadow.

## 7. Incident handling

- Any anomaly: `emergency-stop` from either interface, then capture logs +
  the SQLite/Postgres attempt/intent tables before restarting.
- Dispatch intents left `pending` by a crash are reconciled at next startup
  (evidence-based; never resubmitted) — verify the startup
  `reconciling dispatch intents` log line and resolved statuses.
