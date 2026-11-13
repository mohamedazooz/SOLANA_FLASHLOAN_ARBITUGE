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
#   - ARB_LIVE=0 and DRY_RUN=true are written unless ALREADY set to those values
#     in an existing .env; the script never enables live mode.
#   - Secrets live only in the VPS .env (git-ignored); nothing is echoed back.
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
grep -q '^ARB_LIVE=' .env && sed -i 's/^ARB_LIVE=.*/ARB_LIVE=0/' .env || echo 'ARB_LIVE=0' >> .env
grep -q '^DRY_RUN=' .env && sed -i 's/^DRY_RUN=.*/DRY_RUN=true/' .env || echo 'DRY_RUN=true' >> .env
chmod 600 .env

echo "==> [5/7] Release build (first build takes several minutes)"
cargo build --release -p arb-bot --bin arb-bot

echo "==> [6/7] Service"
if command -v systemctl >/dev/null 2>&1 && systemctl --user >/dev/null 2>&1 || [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
    sed -e "s|__HOME__|${HOME}|g" -e "s|__USER__|${USER}|g" scripts/arb-bot.service > /tmp/arb-bot.service
    sudo cp /tmp/arb-bot.service /etc/systemd/system/arb-bot.service
    sudo systemctl daemon-reload
    sudo systemctl enable --now arb-bot
    sleep 5
    sudo systemctl status arb-bot --no-pager | head -12 || true
else
    echo "    systemd unavailable — starting under nohup"
    set -a; . ./.env; set +a
    nohup ./target/release/arb-bot >> shadow_soak.log 2>&1 &
    echo "    PID $! — logs: $(pwd)/shadow_soak.log"
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
echo "Soak checklist: docs/production/SHADOW_MODE_RUNBOOK.md (>=24h + daily reports)"
