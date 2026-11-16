#!/usr/bin/env bash
# Periodic health probe for the arb-bot shadow soak. Alerts Telegram on:
#   - service not active
#   - event log stale (no writes for > WATCHDOG_STALE_MINS minutes, default 10)
#   - Helius RPC unhealthy
#   - disk usage >= 90%
# Run via systemd timer (arb-bot-watchdog.timer) every 5 minutes.
set -uo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] || exit 0
TOKEN="$(grep '^TELEGRAM_BOT_TOKEN=' .env | cut -d= -f2-)"
CHAT="$(grep '^TELEGRAM_CHAT_ID=' .env | cut -d= -f2-)"
[ -n "${TOKEN}" ] && [ -n "${CHAT}" ] || exit 0

HTTP="$curl"

STATE_DIR="logs"
STATE_FILE="${STATE_DIR}/watchdog_state"
mkdir -p "$STATE_DIR"
HOSTNAME="$(hostname)"
TS="$(date '+%Y-%m-%d %H:%M:%S %Z')"

send_alert() {
    "$HTTP" -s -m 10 -X POST "https://api.telegram.org/bot${TOKEN}/sendMessage" \
        -d "chat_id=${CHAT}" --data-urlencode "text=$1" -d "parse_mode=HTML" >/dev/null 2>&1 || true
}

failures=""

# 1) Service active?
if command -v systemctl >/dev/null 2>&1; then
    if ! systemctl is-active --quiet arb-bot; then
        failures="${failures}\u2022 service arb-bot is NOT active\n"
    fi
fi

# 2) Event log freshness (bot writes continuously in shadow mode)
LOG_FILE="$(grep '^ARB_EVENT_LOG=' .env | cut -d= -f2- || true)"
LOG_FILE="${LOG_FILE:-logs/arb_events.log}"
if [ -f "$LOG_FILE" ]; then
    STALE_MINS="${WATCHDOG_STALE_MINS:-10}"
    if [ "$(find "$LOG_FILE" -mmin -"${STALE_MINS}" | wc -l)" -eq 0 ]; then
        failures="${failures}\u2022 event log stale (> ${STALE_MINS} min without writes): ${LOG_FILE}\n"
    fi
else
    failures="${failures}\u2022 event log missing: ${LOG_FILE}\n"
fi

# 3) Helius RPC health
HELIUS_KEY="$(grep '^HELIUS_API_KEY=' .env | cut -d= -f2-)"
if [ -n "$HELIUS_KEY" ]; then
    SLOT="$("$HTTP" -s -m 15 -X POST "https://mainnet.helius-rpc.com/?api-key=${HELIUS_KEY}" \
        -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}' \
        | jq -r '.result // empty' 2>/dev/null || true)"
    if [ -z "$SLOT" ]; then
        failures="${failures}\u2022 Helius RPC unhealthy (no slot)\n"
    fi
fi

# 4) Disk usage
DISK_PCT="$(df -P . | awk 'NR==2 {gsub("%",""); print $5}')"
if [ "${DISK_PCT:-0}" -ge 90 ]; then
    failures="${failures}\u2022 disk usage ${DISK_PCT}% (>= 90%)\n"
fi

if [ -n "$failures" ]; then
    if [ "$(cat "$STATE_FILE" 2>/dev/null || echo ok)" = "ok" ]; then
        printf '\U0001F6A8 <b>arb-bot watchdog</b>\nHost: <code>%s</code>\nTime: %s\n\n%b\nCheck: journalctl -u arb-bot -n 50' \
            "$HOSTNAME" "$TS" "$failures" > /tmp/wd_msg.txt
        "$HTTP" -s -m 10 -X POST "https://api.telegram.org/bot${TOKEN}/sendMessage" \
            -d "chat_id=${CHAT}" --data-urlencode "text=$(cat /tmp/wd_msg.txt)" -d "parse_mode=HTML" >/dev/null 2>&1 || true
        echo "failing" > "$STATE_FILE"
    fi
    exit 1
fi

if [ "$(cat "$STATE_FILE" 2>/dev/null || echo ok)" != "ok" ]; then
    printf '\u2705 <b>arb-bot watchdog: recovered</b>\nHost: <code>%s</code>\nTime: %s\nAll checks passed.' \
        "$HOSTNAME" "$TS" > /tmp/wd_msg.txt
    "$HTTP" -s -m 10 -X POST "https://api.telegram.org/bot${TOKEN}/sendMessage" \
        -d "chat_id=${CHAT}" --data-urlencode "text=$(cat /tmp/wd_msg.txt)" -d "parse_mode=HTML" >/dev/null 2>&1 || true
fi
echo "ok" > "$STATE_FILE"
exit 0
