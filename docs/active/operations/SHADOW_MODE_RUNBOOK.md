# SHADOW_MODE_RUNBOOK

**Definition:** shadow mode executes the ENTIRE pipeline — detection, analysis, sizing, compile, preflight simulation, dispatch decision, landing reconciliation, persistence — but never broadcasts. The only difference from live is the final byte never leaves the machine.

## Enforcing shadow

- `.env`: `ARB_LIVE=0`, `DRY_RUN=true` (provisioning script `scripts/deploy_vps.sh` hard-forces both; containers default to `--mode shadow`).
- Web control confirm-live gate: `arb-bot/src/supervisor/control_service.rs` (`CONFIRM-LIVE` double-gate, ~755) must be typed explicitly; it cannot be toggled by the dashboard alone.
- Kill switch file `data/kill_switch` (default; `KILL_SWITCH_FILE`) pauses all dispatch decisions even in shadow.

## Starting a shadow soak (the 24 h gate)

1. On the VPS: confirm `ARB_LIVE=0` / `DRY_RUN=true` (`.env`, 600 perms).
2. `sudo systemctl restart arb-bot && systemctl is-active arb-bot`.
3. Watch: `journalctl -u arb-bot -f`, Prometheus `curl -s 127.0.0.1:9100/metrics`.
4. Restore the ledger every morning: `sqlite3 data/<db> "SELECT count(*), status FROM dispatch_intents GROUP BY status, date(started_at);"` and `reconciliation_state`.
5. Record hourly, verbatim: opportunities detected / rejected (with `QuoteGateRejection` reason), gates passed, dry-run verdicts, CU consumption estimates, state age, quote drift (mid vs bid-ask), queue depth, and memory RSS.

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
