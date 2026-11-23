# INCIDENT_RESPONSE

**Triage order: money-safety > data-safety > availability.** When in doubt, engage the kill switch first; diagnosis of a stopped system is safe, diagnosis of a still-running erroneous system is not.

## Immediate decision ladder

1. **Suspect bad economics/loop error (unprofitable trades passing, wrong sizes):**
   ```bash
   touch data/kill_switch          # durable pause even across restarts
   sudo systemctl stop arb-bot     # belt & suspenders
   ```
2. **Suspect leaked secrets/compromised key:** rotate Helius key (dashboard) + Telegram token (@BotFather) FIRST, then rotate the on-disk wallet keypair, then `bash scripts/update_server_env.sh`. If wallet signing could have occurred from another host, treat funds as exposed: move funds to a fresh wallet.
3. **Suspect RPC/WS deception (false prices):** kill switch, then verify with a second provider (`validate` binary cross-checks our math against Jupiter venue-restricted quotes on live reserves).
4. **Bot crash-loop:** `journalctl -u arb-bot -n 200`, check `data/` SQLite WAL integrity (`PRAGMA integrity_check;`), review `DATABASE_RECOVERY.md` checklist. Watchdog only restarts on death; it must be disabled (`sudo systemctl stop arb-bot-watchdog.timer`) while diagnosing a poisoned state.

## Possibly-landed transaction during the incident (CRITICAL RULE)

**Never blind-retry.** If any dispatch may have landed (bundle accepted, signature submitted, timeout but no negative proof):
1. Reconcile by signature + bundle id: `arb-store` `dispatch_intents` (`status`, `bundle_id`) and `attempts` rows; Jito bundle status API; `getSignatureStatuses`.
2. Only a *negative* landing proof (no signature status + bundle not landed) allows a new attempt.
3. Exception rows with `bundle_id = NULL` after a send must be investigated, not assumed unlanded.

## Evidence collection (before touching anything)

```bash
journalctl -u arb-bot --since "24 hours ago" > /tmp/incident_journal.log
cp data/*.db* /tmp/incident_db_backup/
sqlite3 data/<db> ".dump attempts" > /tmp/incident_attempts.sql
curl -s 127.0.0.1:9100/metrics > /tmp/incident_metrics.prom
```

## After the incident

- Append to `docs/active/status/KNOWN_ISSUES.md`: symptom, root cause, evidence, fix or gate.
- If a defect exists in code: new register finding ID → branch → PR (mission workflow).
- Only then: remove `data/kill_switch`, disable watchdog pause, restart, verify health per `VPS_RUNBOOK.md` §1.
- Post-mortem rule: "we fixed the symptom" is not closure; the regression test that fails on the old behavior is closure.
