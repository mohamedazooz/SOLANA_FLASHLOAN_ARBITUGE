> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# OPERATIONS — WEBSOCKET STREAMING & MARKET-DATA INGESTION

**Author:** Senior Solana HFT Engineer  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. WebSocket Ingestion Architecture

In accordance with non-negotiable architectural constraints, market data ingestion is 100% streaming-driven over WebSockets:
- **Zero Paid Yellowstone gRPC:** No GeyserGrpcClient, no local validator node.
- **Provider Multi-Tenancy:** Ingests simultaneously from Solami Mirage, Solami Blur, and Helius WebSocket.
- **Hot-Path Zero Polling:** On-chain pool updates are pushed by the cluster validators directly over WebSocket streams.

---

## 2. Ingestion Endpoints & Transport Capabilities

| Provider Stream | Endpoint Format | Message Protocol | Primary Responsibility |
|---|---|---|---|
| **Solami Mirage** | `wss://ws.solami.dev/mirage/stream` | Binary SubscribeUpdate | Real-time multi-DEX account updates with write versions |
| **Solami Blur** | `wss://ws.solami.dev/data/subscribe` | JSON Event Stream | Real-time token creation, bonding curve completion, migrations |
| **Helius WS** | `wss://mainnet.helius-rpc.com/?api-key=...` | Standard JSON-RPC | `accountSubscribe` and `slotSubscribe` updates |
| **Alchemy / DRPC** | `wss://solana-mainnet.g.alchemy.com/v2/...` | Standard JSON-RPC | Secondary failover account streaming |

---

## 3. Subscription Budgets & Coverage Sizing

- **Maximum Account Subscriptions:** Controlled by `WS_MAX_ACCOUNT_SUBS` (default: 512, hard ceiling: 1024).
- **Per-Pool Account Demand:**
  - Raydium CPMM / V4: 2 subscriptions (base vault + quote vault).
  - Orca Whirlpool: 1 subscription (whirlpool state) + active tick arrays.
  - Meteora DLMM: 1 subscription (lb_pair state) + 2 to 4 active bin arrays.
  - Pump.fun: 1 subscription (bonding curve account).
- **Fail-Closed Budget Enforcement:** If total registered pool demand exceeds `WS_MAX_ACCOUNT_SUBS`, non-priority pools receive no WebSocket feed and are excluded from the arbitrage graph.

---

## 4. Heartbeat, Reconnection & Resynchronization

1. **Keep-Alive Protocol:** Ping frames are sent every 15 seconds. If no pong is received within 5 seconds, the socket is flagged unresponsive.
2. **Exponential Backoff:** Initial retry at 100ms, backing off to 250ms, 500ms, 1000ms, up to a maximum 5000ms ceiling.
3. **Stream Generation Invalidation:** Every reconnection increments `stream_generation`. In-flight quotes tied to prior generations are invalidated to eliminate stale state execution.
4. **Resynchronization:** Upon reconnection, initial account states are confirmed against RPC or high-water slot before quoting resumes.
