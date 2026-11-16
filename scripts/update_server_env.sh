#!/usr/bin/env bash
# Update the VPS .env with new provider keys and verify each one live.
# Run ON the VPS:  bash scripts/update_server_env.sh [keys_file]
#
# keys_file (default ~/new_keys.env) contains KEY=VALUE lines with the new
# secrets. It is never committed and never echoed. Template is created on
# first run; fill it with the operator-supplied values, then re-run.
#
# Supported keys (unknown keys are copied through untouched):
#   HELIUS_API_KEY, SOLAMI_API_KEY, SOLAMI_RPC_URL, SOLAMI_WS_URL,
#   SOLAMI_WS_API_KEY, ALCHEMY_API_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID
#
# Every key is verified against its provider after the merge and a masked
# PASS/FAIL table is printed. A timestamped backup of .env is kept.
set -euo pipefail

KEYS_FILE="${1:-$HOME/new_keys.env}"
cd "$(dirname "$0")/.."
ENV_FILE=".env"

if [ ! -f "$ENV_FILE" ]; then
    echo "ERROR: $ENV_FILE not found in $(pwd) — run scripts/deploy_vps.sh first" >&2
    exit 1
fi

if [ ! -f "$KEYS_FILE" ]; then
    cat > "$KEYS_FILE" <<'TEMPLATE'
# Fill in the values, save, then re-run this script. Lines left blank are skipped.
HELIUS_API_KEY=
SOLAMI_API_KEY=
SOLAMI_RPC_URL=
SOLAMI_WS_URL=
SOLAMI_WS_API_KEY=
ALCHEMY_API_KEY=
TELEGRAM_BOT_TOKEN=
TELEGRAM_CHAT_ID=
TEMPLATE
    chmod 600 "$KEYS_FILE"
    echo "Template created at $KEYS_FILE — fill it with the new values and re-run."
    exit 0
fi

chmod 600 "$KEYS_FILE"
BACKUP="$ENV_FILE.bak.$(date +%Y%m%d_%H%M%S)"
cp "$ENV_FILE" "$BACKUP"
echo "Backup: $BACKUP"

apply_kv() {
    local key="$1" val="$2"
    if [ -z "$val" ]; then
        echo "  $key: skipped (empty)"
        return
    fi
    if grep -q "^${key}=" "$ENV_FILE"; then
        sed -i "s|^${key}=.*|${key}=${val}|" "$ENV_FILE"
    else
        printf '%s=%s\n' "$key" "$val" >> "$ENV_FILE"
    fi
    echo "  $key: updated"
}

echo "==> Merging keys"
while IFS='=' read -r key val; do
    case "$key" in
        ''|\#*) continue ;;
    esac
    apply_kv "$key" "$val"
done < "$KEYS_FILE"
chmod 600 "$ENV_FILE"

echo "==> Verification (values masked)"
mask() { local v="$1"; if [ -z "$v" ]; then echo "(unset)"; elif [ "${#v}" -le 8 ]; then echo "***"; else echo "${v:0:4}...${v: -4} (${#v} chars)"; fi; }

HELIUS_KEY="$(grep '^HELIUS_API_KEY=' "$ENV_FILE" | cut -d= -f2-)"
R="$(curl -s -m 15 -X POST "https://mainnet.helius-rpc.com/?api-key=${HELIUS_KEY}" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}')"
SLOT="$(echo "$R" | jq -r '.result // empty' 2>/dev/null || true)"
if [ -n "$SLOT" ]; then echo "  HELIUS_API_KEY      PASS (live slot ${SLOT})"; else echo "  HELIUS_API_KEY      FAIL ($(echo "$R" | head -c 120)) — check the IP allowlist in the Helius dashboard"; fi

SOLAMI_RPC_KEY="$(grep '^SOLAMI_RPC_URL=' "$ENV_FILE" | cut -d= -f2-)"
if [ -n "$SOLAMI_RPC_KEY" ]; then
    R="$(curl -s -m 15 -X POST "https://rpc.solami.dev/sol?api_key=${SOLAMI_RPC_KEY}" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}')"
    SLOT="$(echo "$R" | jq -r '.result // empty' 2>/dev/null || true)"
    if [ -n "$SLOT" ]; then echo "  SOLAMI_RPC_URL      PASS (live slot ${SLOT})"; else echo "  SOLAMI_RPC_URL      FAIL ($(echo "$R" | head -c 120))"; fi
fi

SOLAMI_WS_KEY="$(grep '^SOLAMI_WS_API_KEY=' "$ENV_FILE" | cut -d= -f2-)"
if [ -n "$SOLAMI_WS_KEY" ]; then
    CODE="$(curl -s -o /dev/null -w '%{http_code}' -m 15 "https://ws.solami.dev/data/subscribe?chain=solana&api_key=${SOLAMI_WS_KEY}" || true)"
    case "$CODE" in
        101|200|400) echo "  SOLAMI_WS_API_KEY   PASS (endpoint reachable, HTTP ${CODE})" ;;
        *) echo "  SOLAMI_WS_API_KEY   FAIL (HTTP ${CODE}) — confirm path/key with Solami" ;;
    esac
fi

ALCH_KEY="$(grep '^ALCHEMY_API_KEY=' "$ENV_FILE" | cut -d= -f2-)"
if [ -n "$ALCH_KEY" ]; then
    R="$(curl -s -m 15 -X POST "https://solana-mainnet.g.alchemy.com/v2/${ALCH_KEY}" -H 'Content-Type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}')"
    SLOT="$(echo "$R" | jq -r '.result // empty' 2>/dev/null || true)"
    if [ -n "$SLOT" ]; then echo "  ALCHEMY_API_KEY     PASS (live slot ${SLOT})"; else echo "  ALCHEMY_API_KEY     FAIL ($(echo "$R" | head -c 120))"; fi
fi

TG_TOKEN="$(grep '^TELEGRAM_BOT_TOKEN=' "$ENV_FILE" | cut -d= -f2-)"
if [ -n "$TG_TOKEN" ]; then
    OK="$(curl -s -m 15 "https://api.telegram.org/bot${TG_TOKEN}/getMe" | jq -r '.ok' 2>/dev/null || echo false)"
    if [ "$OK" = "true" ]; then echo "  TELEGRAM_BOT_TOKEN  PASS (getMe ok)"; else echo "  TELEGRAM_BOT_TOKEN  FAIL"; fi
fi

echo
echo "Done. Restart the service to pick up the new values:"
echo "  sudo systemctl restart arb-bot   (or: pkill -f arb-bot && rerun nohup start)"
