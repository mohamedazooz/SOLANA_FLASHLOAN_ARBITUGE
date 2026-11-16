#!/usr/bin/env bash
set -euo pipefail

echo "=== Deploying Solana Flashloan Arbitrage Engine ==="

INSTALL_DIR="/opt/solana-arb-bot"
RELEASE_TAG=$(date +%Y%m%d_%H%M%S)
RELEASE_DIR="${INSTALL_DIR}/releases/${RELEASE_TAG}"

mkdir -p "${RELEASE_DIR}/bin" "${RELEASE_DIR}/config"

echo "Copying release binaries..."
cp target/release/arb-bot "${RELEASE_DIR}/bin/"
cp target/release/validate "${RELEASE_DIR}/bin/"

echo "Copying configuration files..."
cp -r config/* "${RELEASE_DIR}/config/"

echo "Validating configuration schema..."
"${RELEASE_DIR}/bin/validate" --config "${RELEASE_DIR}/config/production.toml"

echo "Updating current symlink..."
ln -sfn "${RELEASE_DIR}" "${INSTALL_DIR}/current"

echo "Restarting service..."
systemctl restart arb-bot.service

echo "Deployment completed successfully for release ${RELEASE_TAG}."
