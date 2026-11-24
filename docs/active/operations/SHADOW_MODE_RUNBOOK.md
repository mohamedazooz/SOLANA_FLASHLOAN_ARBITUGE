# SHADOW_MODE_RUNBOOK

**Definition:** shadow mode executes the ENTIRE pipeline — detection, analysis, sizing, compile, preflight simulation, dispatch decision, landing reconciliation, persistence — but never broadcasts. The only difference from live is the final byte never leaves the machine.

## Enforcing shadow

- Compile-time guard: every shadow deployment is built with `cargo build --release --locked -p arb-bot --bin arb-bot --features shadow-only`. That feature forces `simulate`, removes wallet/live-arming variables before runtime initialization, and makes wallet-signer loading return `None`.
- `.env`: provisioning forces `ARB_MODE=simulate`, `ARB_LIVE=0`, `LIVE=false`, `LIVE_TRADING=false`, `EXECUTE_LIVE=false`, and `DRY_RUN=true`; it removes `ARB_CANARY` because its presence is an arming signal.
- Service boundary: `scripts/deploy_vps.sh` and the GitHub deploy workflow install `50-shadow-safety.conf` with systemd `UnsetEnvironment` for arming flags, wallet key, and V1 transport toggles. The nohup path unsets the same values. The Docker image is also built with `shadow-only`.
- Web control confirm-live gate: `arb-bot/src/supervisor/control_service.rs` (`CONFIRM-LIVE` double-gate, ~755) must be typed explicitly; it cannot be toggled by the dashboard alone.
- Kill switch file `data/kill_switch` (default; `KILL_SWITCH_FILE`) pauses all dispatch decisions even in shadow.

## Starting a shadow soak (the 24 h gate)

1. Confirm `.env` contains `ARB_MODE=simulate`, `ARB_LIVE=0`, `LIVE=false`, `LIVE_TRADING=false`, `EXECUTE_LIVE=false`, and `DRY_RUN=true`; `ARB_CANARY` must be absent (`.env` permissions: 600).
2. Confirm `sudo systemctl cat arb-bot` includes `/etc/systemd/system/arb-bot.service.d/50-shadow-safety.conf` with `UnsetEnvironment` for the arming variables and wallet key.
3. Restart: `sudo systemctl restart arb-bot && systemctl is-active arb-bot`; then verify the startup log contains `shadow-only build: live/canary arming is disabled; forcing simulate`.
4. Watch: `journalctl -u arb-bot -f`, Prometheus `curl -s 127.0.0.1:9100/metrics`.
5. Restore the ledger every morning: `sqlite3 data/<db> "SELECT count(*), status FROM dispatch_intents GROUP BY status, date(started_at);"` and `reconciliation_state`.
6. Record hourly, verbatim: opportunities detected / rejected (with `QuoteGateRejection` reason), gates passed, dry-run verdicts, CU consumption estimates, state age, quote drift (mid vs bid-ask), queue depth, and memory RSS.

## What shadow PROVES vs does NOT

| Proven by shadow | NOT proven |
|---|---|
| decode/quote/size math on live state | actual landing probability |
| compile → preflight simulation success | Jito bundle acceptance economics |
| dispatch decision correctness + risk gates | any realized profit |
| reconciliation + store round-trip (incl. bundle-id NULLs) | op-getBlock landing evidence |

A successful dry-run is NOT a landing. `[5b]` fail-closed verdicts (e.g. final 9,932,589 < repay 10,000,100) are the CORRECT output for economically negative routes.

## Exit criteria (to request live authorization)

- 24 h continuous run, zero crashes, zero unexplained restarts.
- PnL ledger: `estimated vs realized` columns distinguishable in every `attempts` row; `dispatch_intents.bundle_id` NULL-by-design exceptions all explained.
- All 10 reliability suites green on the exact running commit.
- Operator review →mission §22 checklist in `PRODUCTION_READINESS.md` → explicit live authorization (never automatic).
