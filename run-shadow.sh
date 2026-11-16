#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
if ! command -v docker >/dev/null 2>&1; then
  echo "Docker is required. Install Docker Engine + Compose plugin first." >&2
  exit 1
fi
if [[ ! -f .env.shadow.local ]]; then
  cp .env.shadow.example .env.shadow.local
  chmod 600 .env.shadow.local
  echo "Created .env.shadow.local. Add read-only provider credentials, then run ./run-shadow.sh again."
  exit 2
fi
if grep -Eq '^(SOLANA_PRIVATE_KEY|OPERATOR_PRIVATE_KEY|JITO_AUTH|ARB_LIVE)=.*[^=[:space:]]' .env.shadow.local; then
  echo "Refusing unsafe env file: remove wallet/private-key/live execution variables." >&2
  exit 3
fi
mkdir -p data
chmod 700 data
docker compose -f docker-compose.shadow.yml build
docker compose -f docker-compose.shadow.yml up -d
docker compose -f docker-compose.shadow.yml ps
