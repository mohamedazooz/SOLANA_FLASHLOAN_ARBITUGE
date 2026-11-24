#!/usr/bin/env bash
# Shadow-mode deployment for a fresh Ubuntu/Debian VPS.
# Run ON the VPS as the deploy user:  bash scripts/deploy_vps.sh
#
# What it does:
#   1. Installs system build deps (sudo required).
#   2. Installs rustup; the repo's rust-toolchain.toml pins the exact toolchain.
#   3. Clones the repository (or uses an existing checkout).
#   4. Creates/updates the gitignored .env (secrets are prompted, never embedded here).
#   5. Builds the release binary.
#   6. Installs a systemd unit (sudo) or falls back to nohup, then starts shadow mode.
#   7. Verifies: RPC health, Telegram getMe, service status.
#
# Safety invariants enforced here:
#   - ARB_MODE is forced to simulate; live/canary arming flags are removed or set false.
#   - The binary is built with `shadow-only`, which cannot load wallet material.
#   - The systemd and nohup launch paths remove inherited arming flags and wallet keys.
#   - Secrets stay in the VPS .env (git-ignored); nothing is echoed back.
set -euo pipefail

REPO_URL="https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git"
DEST="${HOME}/SOLANA_FLASHLOAN_ARBITUGE"
BRANCH="${1:-main}"

echo "==> [1/7] System packages"
if command -v sudo >/dev/null 2>&1; then
    sudo apt-get update -qq
    sudo apt-get install -y -qq curl git build-essential pkg-config libssl-dev jq >/dev/null
else
    echo "    sudo not available — assuming packages already installed"
fi

echo "==> [2/7] Rust toolchain (pinned by rust-toolchain.toml)"
if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    . "$HOME/.cargo/env"
fi
cargo --version

echo "==> [3/7] Repository (${BRANCH})"
if [ -d "$DEST/.git" ]; then
    git -C "$DEST" fetch origin "$BRANCH"
    git -C "$DEST" checkout "$BRANCH"
    git -C "$DEST" pull --ff-only origin "$BRANCH"
else
    git clone --depth 1 -b "$BRANCH" "$REPO_URL" "$DEST"
fi
cd "$DEST"

echo "==> [4/7] .env (secrets prompted; values never echoed)"
if [ ! -f .env ]; then
    cp .env.example .env
fi
# Strip placeholder lines we are about to set, keep everything else the operator configured.
sed -i '/^HELIUS_API_KEY=/d; /^HELIUS_RPC_URL=/d; /^HELIUS_WSS_URL=/d; /^TELEGRAM_BOT_TOKEN=/d; /^TELEGRAM_CHAT_ID=/d' .env
prompt_secret() {
    local var="$1" desc="$2" val=""
    while [ -z "$val" ]; do
        read -r -s -p "Paste ${desc} (input hidden): " val; echo
    done
    printf '%s=%s\n' "$var" "$val" >> .env
}
grep -q '^HELIUS_API_KEY=' .env || prompt_secret "HELIUS_API_KEY" "Helius API key (add this VPS IP to the allowlist first)"
grep -q '^TELEGRAM_BOT_TOKEN=' .env || prompt_secret "TELEGRAM_BOT_TOKEN" "Telegram bot token (from @BotFather)"
grep -q '^TELEGRAM_CHAT_ID=' .env || prompt_secret "TELEGRAM_CHAT_ID" "Telegram chat id"
# Helius URLs derive from the key if absent.
grep -q '^HELIUS_RPC_URL=' .env || printf 'HELIUS_RPC_URL=https://mainnet.helius-rpc.com/?api-key=%s\n' "$(grep '^HELIUS_API_KEY=' .env | cut -d= -f2-)" >> .env
grep -q '^HELIUS_WSS_URL=' .env || printf 'HELIUS_WSS_URL=wss://mainnet.helius-rpc.com/?api-key=%s\n' "$(grep '^HELIUS_API_KEY=' .env | cut -d= -f2-)" >> .env
# Fail-closed shadow invariants (never enable live from this script).
# The binary is also compiled with shadow-only, which ignores arming values and
# refuses to load wallet material. Keep the on-disk operator config explicit too.
set_env_value() {
    local key="$1" value="$2"
    if grep -q "^${key}=" .env; then
        sed -i "s|^${key}=.*|${key}=${value}|" .env
    else
        printf '%s=%s\n' "$key" "$value" >> .env
    fi
}
set_env_value ARB_MODE simulate
set_env_value ARB_LIVE 0
set_env_value LIVE false
set_env_value LIVE_TRADING false
set_env_value EXECUTE_LIVE false
set_env_value DRY_RUN true
# ARB_CANARY is presence-armed; remove it entirely rather than writing a falsy value.
sed -i '/^ARB_CANARY=/d' .env
chmod 600 .env

echo "==> [5/7] Shadow-only release build (first build takes several minutes)"
cargo build --release --locked -p arb-bot --bin arb-bot --features shadow-only

echo "==> [6/7] Service"
if command -v systemctl >/dev/null 2>&1 && systemctl --user >/dev/null 2>&1 || [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
    sed -e "s|__HOME__|${HOME}|g" -e "s|__USER__|${USER}|g" scripts/arb-bot.service > /tmp/arb-bot.service
    sudo cp /tmp/arb-bot.service /etc/systemd/system/arb-bot.service
    # EnvironmentFile is loaded before UnsetEnvironment; this drop-in removes
    # runtime arming flags and the wallet key from the service environment.
    sudo install -d -m 0755 /etc/systemd/system/arb-bot.service.d
    printf '%s\n' '[Service]' 'UnsetEnvironment=ARB_MODE ARB_CANARY ARB_LIVE LIVE_TRADING LIVE EXECUTE_LIVE SOLANA_PRIVATE_KEY SOLANA_TX_V1_DIRECT_ENABLED SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED SOLANA_TX_V1_JITO_ENABLED SOLANA_TX_V1_JITO_INGRESS_VERIFIED' | sudo tee /etc/systemd/system/arb-bot.service.d/50-shadow-safety.conf >/dev/null
    sed -e "s|__HOME__|${HOME}|g" -e "s|__USER__|${USER}|g" scripts/arb-bot-watchdog.service > /tmp/arb-bot-watchdog.service
    sudo cp /tmp/arb-bot-watchdog.service /etc/systemd/system/arb-bot-watchdog.service
    sudo cp scripts/arb-bot-watchdog.timer /etc/systemd/system/arb-bot-watchdog.timer
    sudo chmod +x "${HOME}/SOLANA_FLASHLOAN_ARBITUGE/scripts/arb_bot_watchdog.sh" \
                   "${HOME}/SOLANA_FLASHLOAN_ARBITUGE/scripts/arb_bot_failure_alert.sh"
    sudo cp scripts/arb-bot.logrotate /etc/logrotate.d/arb-bot
    DEPLOY_STARTED_UTC="$(date --utc +%Y-%m-%dT%H:%M:%SZ)"
    sudo systemctl daemon-reload
    sudo systemctl enable arb-bot
    sudo systemctl restart arb-bot
    sudo systemctl enable --now arb-bot-watchdog.timer
    sleep 5
    sudo systemctl status arb-bot --no-pager | head -12 || true
    if ! sudo journalctl -u arb-bot --since "$DEPLOY_STARTED_UTC" --no-pager 2>/dev/null | grep -Fq "shadow-only build: live/canary arming is disabled; forcing simulate"; then
        echo "ERROR: arb-bot did not confirm shadow-only mode in its startup log"
        sudo journalctl -u arb-bot -n 40 --no-pager || true
        exit 1
    fi
    sudo systemctl list-timers arb-bot-watchdog.timer --no-pager | head -5 || true
else
    echo "    systemd unavailable — starting under nohup"
    pkill -f 'target/release/arb-bot' 2>/dev/null || true
    sleep 2
    set -a; . ./.env; set +a
    unset ARB_MODE ARB_CANARY ARB_LIVE LIVE_TRADING LIVE EXECUTE_LIVE SOLANA_PRIVATE_KEY SOLANA_TX_V1_DIRECT_ENABLED SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED SOLANA_TX_V1_JITO_ENABLED SOLANA_TX_V1_JITO_INGRESS_VERIFIED
    nohup ./target/release/arb-bot >> shadow_soak.log 2>&1 &
    BOT_PID=$!
    sleep 5
    if ! kill -0 "$BOT_PID" 2>/dev/null || ! grep -Fq "shadow-only build: live/canary arming is disabled; forcing simulate" shadow_soak.log; then
        tail -40 shadow_soak.log || true
        echo "ERROR: shadow-only process failed startup verification"
        exit 1
    fi
    echo "    PID $BOT_PID — logs: $(pwd)/shadow_soak.log"
fi

echo "==> [7/7] Verification"
HELIUS_KEY="$(grep '^HELIUS_API_KEY=' .env | cut -d= -f2-)"
HEALTH="$(curl -s -m 15 -X POST "https://mainnet.helius-rpc.com/?api-key=${HELIUS_KEY}" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getHealth","params":[]}')"
echo "Helius RPC: ${HEALTH}"
TG_TOKEN="$(grep '^TELEGRAM_BOT_TOKEN=' .env | cut -d= -f2-)"
TG_OK="$(curl -s -m 15 "https://api.telegram.org/bot${TG_TOKEN}/getMe" | jq -r '.ok' 2>/dev/null || echo false)"
echo "Telegram getMe ok: ${TG_OK}"
echo
echo "Done. Dashboard: http://127.0.0.1:8000 (login: WEB_ADMIN_PASSWORD from .env)"
echo "Soak checklist: docs/active/operations/SHADOW_MODE_RUNBOOK.md (>=24h + daily reports)"
