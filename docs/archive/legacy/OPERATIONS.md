> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# OPERATIONS RUNBOOK & PRODUCTION DISPATCH SPECIFICATION

## 1. System Configuration & Safety Defaults

The system is configured with strict fail-closed safety defaults to prevent unintended mainnet fund loss:
- `ARB_LIVE=0`: The bot is in dry-run/simulation mode. No transactions are broadcast to the network.
- `DRY_RUN=true`: Transactions are assembled, signed, and simulated against local SVM/Bankrun or RPC `simulateTransaction`, but not landed.
- `SHADOW_MODE=true`: Dispatches simulated bundles to measure latency, quote accuracy, and slot progression without risk of capital loss.

### Environment Setup:
```bash
cp .env.example .env
# Required environment variables:
# HELIUS_RPC_URL="https://mainnet.helius-rpc.com/?api-key=YOUR_KEY"
# HELIUS_WSS_URL="wss://mainnet.helius-rpc.com/?api-key=YOUR_KEY"
# JITO_BLOCK_ENGINE_URL="https://ny.mainnet.block-engine.jito.wtf"
# KEYPAIR_PATH="/path/to/payer-keypair.json"
# WEB_ADMIN_PASSWORD="StrongSecurePasswordHere"
```

---

## 2. Telemetry, Health Checks & Observability

The bot provides real-time health checks, metrics, and administration interfaces:
- **Prometheus Metrics**: `http://localhost:8080/metrics`
  - `arb_ticks_total`: Total quote/search ticks executed.
  - `arb_opportunities_discovered_total`: Viable cross-pool opportunities found.
  - `arb_simulations_success_total`: Successful simulation count.
  - `arb_execution_latency_microseconds`: Microsecond latency from ingestion to assembly.
- **Telegram Alerting**: Real-time push notifications on quote execution, simulated profit, and risk triggers.
- **Fail-Closed Web API**: Requires `Bearer <WEB_ADMIN_PASSWORD>`.

---

## 3. Incident Response & Safe Shutdown

### Immediate Emergency Shutdown (Kill Switch):
```bash
# Set dry-run immediately or kill the bot process
curl -X POST http://localhost:8080/api/kill -H "Authorization: Bearer <WEB_ADMIN_PASSWORD>"
```
Or kill the process directly:
```bash
pkill -f "arb-bot"
```

### Ambiguous Dispatch Recovery:
If a transaction submission times out or RPC disconnects:
1. Do not re-sign or construct a new transaction with altered slippage.
2. The supervisor's `recovery.rs` queries signature status against `getSignatureStatuses`.
3. If confirmed on-chain, update store balances and record realized PnL.
4. If expired past `lastValidBlockHeight` (plus 160 slots safety margin), mark the dispatch intent as expired.
