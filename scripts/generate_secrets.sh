#!/usr/bin/env bash
# Generates strong operator secrets into .env without touching existing values.
# Usage: ./scripts/generate_secrets.sh [--force]
#
# Fills (only if absent or REPLACE_ME placeholder, unless --force):
#   SENTINEL_ADMIN_KEY   - bearer token for the admin HTTP API (/pause, /resume)
#   WEB_ADMIN_PASSWORD   - password for the web dashboard login
#
# .env is gitignored; never commit it. Rotation: rerun with --force and
# restart the bot (keys are read at startup only).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${REPO_ROOT}/.env"
FORCE=0
[[ "${1:-}" == "--force" ]] && FORCE=1

if ! command -v openssl >/dev/null 2>&1; then
    echo "error: openssl is required" >&2
    exit 1
fi

touch "${ENV_FILE}"
chmod 600 "${ENV_FILE}"

set_secret() {
    local key="$1"
    local value="$2"
    if grep -qE "^${key}=" "${ENV_FILE}"; then
        local current
        current="$(grep -E "^${key}=" "${ENV_FILE}" | head -1 | cut -d= -f2-)"
        if [[ "${FORCE}" -eq 1 || -z "${current}" || "${current}" == REPLACE_ME* ]]; then
            sed -i.bak "s|^${key}=.*|${key}=${value}|" "${ENV_FILE}"
            rm -f "${ENV_FILE}.bak"
            echo "rotated: ${key}"
        else
            echo "kept existing: ${key} (use --force to rotate)"
        fi
    else
        printf '%s=%s\n' "${key}" "${value}" >> "${ENV_FILE}"
        echo "created: ${key}"
    fi
}

set_secret SENTINEL_ADMIN_KEY "$(openssl rand -hex 32)"
set_secret WEB_ADMIN_PASSWORD "$(openssl rand -base64 24 | tr -d '/+=' | head -c 32)"

echo
echo "secrets written to ${ENV_FILE} (permissions 600, gitignored)"
echo "restart the bot to activate; admin API and web dashboard will enable automatically"
