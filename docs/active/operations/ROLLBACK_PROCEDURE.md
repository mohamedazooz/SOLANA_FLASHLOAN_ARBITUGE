# Rollback Procedure

**Precondition for trust:** every rollback path below returns the system to a binary **and, when applicable, configuration** that were previously healthy. Rollback never invents state.

## Which rollback applies?

| Deployment path used | Rollback mechanism |
|---|---|
| `deploy-vps.yml` (GitHub Actions) | Automatic rollback already ran if health failed. Manual: `cmd: git checkout --detach $(cat ~/SOLANA_FLASHLOAN_ARBITUGE/.deploy_previous_commit)` |
| `deploy/production/deploy.sh` (release dirs + `current` symlink) | `bash deploy/production/rollback.sh` |
| Docker compose | `git checkout <previous_sha> -- Dockerfile docker-compose.yml && docker compose up -d --build` (shadow-only container, no data held) |
| Config-only regression | Restore the backed-up `.env` from `update_server_env.sh` (`cp .env.bak.<ts> .env`) or the prior `config/config.toml`, then `sudo systemctl restart arb-bot` |

## Procedure (checkout path — the pinned-commit workflow)

1. **Stop tracing before acting.** Confirm the bad commit: `git rev-parse HEAD` on the box vs the workflow summary's Status line; the automatic rollback has already run for any build/restart/health failure — verify the box is actually on the previous commit (`git log --oneline -1`).
2. **If still on the bad commit** (health failed without triggering rollback, or symptoms appeared later):
   ```bash
   cd ~/SOLANA_FLASHLOAN_ARBITUGE
   sudo systemctl stop arb-bot                       # or pkill -f 'target/release/arb-bot'
   git checkout --detach "$(cat .deploy_previous_commit)"
   cargo build --release --locked -p arb-bot --bin arb-bot
   sudo systemctl start arb-bot
   sleep 5 && systemctl is-active arb-bot
   ```
3. **Verify economic invariants after rollback**, not just liveness:
   - `data/` SQLite reconciles against `pending_dispatch_intents` (no orphan intents): see `DATABASE_RECOVERY.md` §crash-restart checklist.
   - No unexplained balance movement: check the landing ledger + Telegram alerts.
   - Kill switch: engage the marker in `data/` while investigating if losses are possible/suspected (`RISK_CONTROL_POLICY.md`).
4. **Record**: append the incident (commit, symptom, rollback time, verification evidence) to `docs/active/status/KNOWN_ISSUES.md` (or a new diagnosis ID there) before re-attempting any deploy.

## Procedure (release-directory path)

```bash
bash deploy/production/rollback.sh    # flips `current` symlink to second-newest release dir
systemctl is-active arb-bot
```

Caveats (real, documented):
- `rollback.sh` picks by mtime (`ls -dt releases/* | sed -n '2p'`) and does not mark the bad release; if you must roll back twice in a row, relabel directories or move the bad release out of `releases/` first.
- It restores the binary only. If the bad release also changed `config/`, restore that config from the release dir it came from.

## Never do

- Never `git push --force` to "fix" history before a rollback.
- Never blind-retry a possibly-landed transaction during an incident (durable-store reconciliation first — `DATABASE_RECOVERY.md` invariant).
- Never roll back the SQLite WAL files forward/backward manually; recovery is by opening the store (migration chain is monotonic).

## Test evidence

- Workflow rollback logic: exercised by review of the shell branch conditions; live must be validated on the VPS by the operator (deliberately dispatch-only, not CI-tested — deploying is operator-authorized action by mission rule §4.5).
- `config/config.example.toml` + Docker HEALTHCHECK cover container liveness; documented in `DEPLOYMENT_ARCHITECTURE.md`.
