# VPS Runbook

**Audience:** the operator running `arb-bot` on the VPS.
**Mode of truth:** commands below are verified against the tracked scripts (`scripts/deploy_vps.sh`, `scripts/update_server_env.sh`, `scripts/arb_bot_failure_alert.sh`, `scripts/arb_bot_watchdog.sh`, `deploy/production/*.sh`, `.github/workflows/deploy-vps.yml`).

## 1. Daily health

```bash
systemctl status arb-bot --no-pager | head -12     # service state since when
journalctl -u arb-bot -n 50 --no-pager             # last 50 log lines
curl -s http://127.0.0.1:9100/metrics | head -30   # Prometheus metrics (if exposed)
ls -la ~/SOLANA_FLASHLOAN_ARBITUGE/data/warm/      # warm state freshness
```

Watchdog: `arb-bot-watchdog.timer` (installed by `scripts/deploy_vps.sh`) runs `scripts/arb_bot_watchdog.sh` on its schedule; it restarts the service when dead and calls `scripts/arb_bot_failure_alert.sh` to page Telegram.

## 2. Deploy a reviewed commit

1. Merge the PR (never deploy an unmerged SHA unless deliberately reviewing on the box).
2. Go to **Actions → Deploy to VPS → Run workflow** on `main`, or check out the release-directory path below.
3. The workflow pins `github.sha`, verifies ancestry on main, builds `--locked`, restarts, health-checks, and **auto-rolls-back on failure**.
4. Operator prerequisite (one-time): create GitHub Environment `production` and add yourself as required reviewer — then every dispatch stops for your approval first.

Manual equivalent on the VPS (works without GitHub Actions access):

```bash
cd ~/SOLANA_FLASHLOAN_ARBITUGE
git fetch origin main
git rev-parse --short origin/main            # confirm the SHA you intend
git reset --hard origin/main                 # deploy box: discard local detritus only
cargo build --release --locked -p arb-bot --bin arb-bot
sudo systemctl restart arb-bot && sleep 5 && systemctl is-active arb-bot
```

## 3. Rotate provider keys

```bash
cd ~/SOLANA_FLASHLOAN_ARBITUGE
bash scripts/update_server_env.sh            # creates/edit ~/new_keys.env, then re-runs
```

The script backs up `.env` with a timestamp, merges keys, chmods 600, and live-verifies every provider with a masked PASS/FAIL table. It never echoes secret values.

## 4. Rotation isn't enough — restart behavior

- `Edit`: systemd unit is `scripts/arb-bot.service` installed as `/etc/systemd/system/arb-bot.service`; `Restart=` policy + watchdog timer are the restart layers.
- Kill switch: `data/` contains the kill-switch marker used by the bot; creating it stops new dispatches (see `RISK_CONTROL_POLICY.md`).
- Graceful stop: `sudo systemctl stop arb-bot` (SIGTERM honored by the bot; `stop_grace_period` in compose path is 30s).

## 5. First-time provisioning

```bash
ssh operator@vps
git clone https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git
cd SOLANA_FLASHLOAN_ARBITUGE
bash scripts/deploy_vps.sh main
```

The script **hard-forces `ARB_LIVE=0` and `DRY_RUN=true`** and will refuse to enable live mode. Live is enabled only by an explicit operator edit of `.env` on the box, after `docs/active/operations/RISK_CONTROL_POLICY.md` gates are satisfied (see also `SHADOW_MODE_RUNBOOK.md` and the production-readiness checklist in `docs/active/status/PRODUCTION_READINESS.md`).

## 6. Failure modes → actions

| Symptom | First check | Action |
|---|---|---|
| Service dead, watchdog looping | `journalctl -u arb-bot -n 100` | Fix root cause; kill-switch marker if RPC/key issues suspected |
| RPC failures in log | `curl $HELIUS_RPC_URL -d '{"jsonrpc":"2.0","id":1,"method":"getHealth"}'` | Rotate key via `update_server_env.sh`; check Helius allowlist for the VPS IP |
| Deploy job failed with rollback message | `.deploy_previous_commit` + git log on the box | Box is on previous commit already; read the job log before re-dispatching |
| Disk pressure | `df -h /` | Clean old artifacts (`find target/release/deps -name '*.d' -mtime +7 -delete`), prune old release dirs (keep ≥2 for rollback) |
| Log growth | `ls -la logs/` | logrotate policy `scripts/arb-bot.logrotate` is installed; check it is active (`sudo logrotate -d /etc/logrotate.d/arb-bot`) |

## 7. Secrets hygiene

- `.env` is 600, git-ignored, never echoed; only masked PASS/FAIL output is allowed.
- SSH access: dedicated deploy key stored as GitHub secret `VPS_SSH_KEY`; host key accepted only on first connect (`accept-new`) — treat a changed host key as an incident.
- Never paste keys into chat, logs, or tickets; if a key leaked, rotate immediately (Helius dashboard / @BotFather) and re-run `update_server_env.sh`.
