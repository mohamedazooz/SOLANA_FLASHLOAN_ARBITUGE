# Continuous Deployment to VPS (GitHub Actions)

`.github/workflows/deploy-vps.yml` deploys `main` to the operator's VPS on every
push to `main` (i.e., every merged PR) and on manual `workflow_dispatch`.

## What the job does

1. SSHes into the VPS (key preferred, password fallback via `sshpass`).
2. `git reset --hard origin/main` in `~/SOLANA_FLASHLOAN_ARBITUGE`.
3. `cargo build --release -p arb-bot --bin arb-bot`.
4. `sudo systemctl restart arb-bot` (or nohup restart when systemd is not installed).
5. Verifies the service is active and prints the last journal lines.

The VPS `.env` (all provider/Telegram keys) is never touched by CI — it stays
on the server only. If the restart fails, the bot's Telegram failure alerts
(`ExecStopPost` + watchdog) notify the operator.

## One-time setup

### 1. Deploy SSH key (recommended over password)

On the VPS:

```bash
ssh-keygen -t ed25519 -f ~/.ssh/deploy_ed25519 -N "" -C "github-deploy"
cat ~/.ssh/deploy_ed25519.pub >> ~/.ssh/authorized_keys
```

Copy the private key contents (`~/.ssh/deploy_ed25519`) into the
`VPS_SSH_KEY` secret. It is used only from GitHub Actions.

### 2. Passwordless sudo limited to the restart command

```bash
echo "$(whoami) ALL=(ALL) NOPASSWD: /usr/bin/systemctl restart arb-bot, /bin/systemctl restart arb-bot" \
  | sudo tee /etc/sudoers.d/arb-bot-deploy
sudo chmod 440 /etc/sudoers.d/arb-bot-deploy
```

### 3. Repository secrets

Settings > Secrets and variables > Actions > New repository secret:

| Secret            | Value                                        |
|-------------------|----------------------------------------------|
| `VPS_HOST`        | VPS IP (e.g. `102.206.132.212`)              |
| `VPS_USER`        | SSH username (e.g. `machsol`)                |
| `VPS_SSH_KEY`     | contents of `~/.ssh/deploy_ed25519` (recommended) |
| `VPS_SSH_PASSWORD`| VPS password (only if no key is configured)  |
| `VPS_PORT`        | optional, defaults to `22`                   |

## Safety notes

- Only `push` to `main` deploys; feature branches never touch the VPS.
- `concurrency: deploy-vps` serializes deployments (no overlapping builds).
- Secrets are never echoed; the remote script receives no secret values.
- If the SSH key is ever leaked, remove its line from `authorized_keys` on
  the VPS and generate a replacement.
