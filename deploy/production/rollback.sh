#!/usr/bin/env bash
set -euo pipefail

echo "=== Rolling Back Solana Flashloan Arbitrage Engine ==="

INSTALL_DIR="/opt/solana-arb-bot"

# Find previous release directory
PREV_RELEASE=$(ls -dt "${INSTALL_DIR}/releases"/* | sed -n '2p')

if [ -z "${PREV_RELEASE}" ]; then
    echo "ERROR: No previous release found to roll back to!"
    exit 1
fi

echo "Rolling back to: ${PREV_RELEASE}"
ln -sfn "${PREV_RELEASE}" "${INSTALL_DIR}/current"

echo "Restarting service..."
systemctl restart arb-bot.service

echo "Rollback completed successfully."
