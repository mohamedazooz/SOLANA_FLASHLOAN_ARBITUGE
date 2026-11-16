#!/usr/bin/env bash
# Sends a Telegram alert when the arb-bot service stops abnormally.
# Wired as systemd ExecStopPost (SERVICE_RESULT is set by systemd).
# Reads TELEGRAM_BOT_TOKEN / TELEGRAM_CHAT_ID from the repo .env.
set -uo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] || exit 0
TOKEN="$(grep '^TELEGRAM_BOT_TOKEN=' .env | cut -d= -f2-)"
CHAT="$(grep '^TELEGRAM_CHAT_ID=' .env | cut -d= -f2-)"
[ -n "${TOKEN}" ] && [ -n "${CHAT}" ] || exit 0

RESULT="${SERVICE_RESULT:-unknown}"
EXIT_CODE="${EXIT_CODE:-?}"
if [ "${RESULT}" = "success" ]; then
    exit 0
fi
HOSTNAME="$(hostname)"
TS="$(date '+%Y-%m-%d %H:%M:%S %Z')"
LAST_ERR="$(journalctl -u arb-bot -n 5 --no-pager 2>/dev/null | tail -5 | sed 's/[<>&]//g' | head -c 600 || true)"
TEXT="🛑 <b>arb-bot service failure</b>
🖥️ Host: <code>${HOSTNAME}</code>
⏰ Time: ${TS}
❗ Result: <code>${RESULT}</code> (exit ${EXIT_CODE})
📜 Last log lines:
<code>${LAST_ERR:-unavailable}</code>

Restart with: sudo systemctl restart arb-bot"
curl -s -m 10 -X POST "https://api.telegram.org/bot${TOKEN}/sendMessage" \
    -d "chat_id=${CHAT}" --data-urlencode "text=${TEXT}" -d "parse_mode=HTML" >/dev/null 2>&1 || true
exit 0
