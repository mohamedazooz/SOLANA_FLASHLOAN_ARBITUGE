> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Provider Operations & Multi-RPC Supervision

## 1. Yellowstone gRPC & Streaming Layer
- **Primary Yellowstone Endpoint**: Low-latency direct gRPC stream ingesting slot updates, block commits, and account state deltas.
- **Subscription Filtering**: Filtered exclusively for targeted protocol program IDs (Meteora DLMM, Raydium CPMM/CLMM, Orca, Kamino Reserves) to eliminate redundant network load.
- **Backpressure & Drop Protection**: High-throughput lock-free channels with bounded buffers prevent allocation leaks under cluster spam conditions.

## 2. Multi-RPC Supervised Pool
The `RpcPool` and `ProviderSupervisor` maintain continuous health checks across multiple independent RPC backends:
- **Primary Provider**: Dedicated Helius RPC / Yellowstone gRPC.
- **Secondary Provider**: Fallback RPC cluster (Triton / QuickNode / Alchemy).
- **Slot Lag Detection**: Measures slot drift every tick. If a provider lags by $> 3$ slots, it is quarantined and failover occurs immediately.
- **Bank Generation Coherence**: Ensures pool state updates across different legs belong to coherent validator banks, eliminating split-brain route evaluations.
