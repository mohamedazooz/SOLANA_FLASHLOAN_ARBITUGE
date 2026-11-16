# Rollback Plan

How to revert this system to a known-good state at each layer, in the order an
incident would demand. Read with
[DEPLOYMENT_RUNBOOK.md](DEPLOYMENT_RUNBOOK.md) and
[OPERATIONS_RUNBOOK.md](OPERATIONS_RUNBOOK.md).

## 0. Stop trading first, diagnose second

1. **Kill switch (fastest, no redeploy):** create the marker file —
   `touch ./data/kill_switch` (path from `KILL_SWITCH_FILE`). Startup and the
   run loop treat its existence as PAUSED (fail safe).
2. **Control plane:** `POST /pause` (web API, authenticated) or the Telegram
   pause command — both go through the single `ControlService`.
3. **Process stop:** `sudo systemctl stop arb-bot` (systemd deployment) or
   `./stop-shadow.sh` (shadow deployment).
4. Only after the above: revoke network paths if compromise is suspected
   (rotate `SOLANA_PRIVATE_KEY`, `WEB_ADMIN_PASSWORD`, Telegram token — all
   live only in the git-ignored `.env`).

## 1. Code rollback (git)

- `main` is linear-protected: never force-push; roll back **forward** with a
  revert commit.
- Identify the last known-good merge: `git log --oneline --merges -5`.
- Revert the offending PR's merge commit:
  `git revert -m 1 <merge-sha>` → push → let CI validate → redeploy.
- Single-commit regressions: `git revert <sha>`.
- If the bad change is on an unmerged branch: close/ignore the branch; `main`
  is untouched by definition.

## 2. Binary/deployment rollback

- Docker: re-deploy the previous image tag
  (`docker compose -f docker-compose.yml up -d` after re-pointing the tag);
  the multi-stage Dockerfile rebuilds reproducibly from the pinned 1.99.1
  toolchain if a rebuild is required.
- systemd/VPS: previous release directory + `sudo systemctl restart arb-bot`;
  the watchdog (`scripts/arb-bot-watchdog.timer`, PR #12) must be re-checked
  after any manual process surgery so it does not fight the operator.

## 3. State/data rollback

- **Dispatch intents / reconciliation:** on startup the supervisor reconciles
  durable dispatch intents (`supervisor/recovery.rs`). After a rollback, start
  the binary once in shadow mode and let reconciliation settle before resuming
  live modes; ambiguous sends are resolved by signature-status lookup, never by
  blind resubmission.
- **Store:** SQLite (`DATABASE_URL=sqlite:///...`) — stop the bot, copy the
  `.db` + `-wal` + `-shm` files aside, then restart; WAL replay is automatic.
  Postgres — restore from the last dump; the store is an audit/settlement
  record, never a trading-input authority, so a stale store degrades
  observability, not trading correctness.
- **Kill-switch / heartbeat / audit logs** (`data/`, `logs/`): append-only
  evidence; never truncate during an incident — copy, then investigate.

## 4. Configuration rollback

- `.env`: restore the previous `.env` backup (file lives only on the host).
  After the 2026-11-13 prune, `.env.example` contains only consumed keys; an
  old `.env` with extra keys is harmless (they are simply unread).
- `config.toml`: keep the previous version in deploy history; the binary
  validates on startup and fails closed on a bad config.
- **Mode rollback:** any live/experimental mode goes back to
  `ARB_LIVE=0`, `ARB_MODE=shadow` — the load-bearing safe defaults.

## 5. Verification after any rollback

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```
plus one shadow-mode smoke run (positive: durable store created; negative:
impossible store path refuses startup with exit 101 — the R-I fail-closed
evidence pair in [../quality/TEST_EVIDENCE.md](../quality/TEST_EVIDENCE.md)).
A rollback that has not passed this pair is not "rolled back"; it is
"unverified".
