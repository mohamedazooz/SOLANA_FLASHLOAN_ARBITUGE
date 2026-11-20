> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Jito MEV Landing Subsystem & Dynamic Tip Oracle

## 1. Jito Block Engine Integration Architecture
Arbitrage bundles are dispatched exclusively through Jito Block Engine's private bundle submission endpoints, bypassing public mempools to guarantee frontrunning protection and atomic execution.

### Multi-Region Endpoint Topology
1. **Amsterdam (EU)**: `https://amsterdam.mainnet.block-engine.jito.wtf`
2. **Frankfurt (EU)**: `https://frankfurt.mainnet.block-engine.jito.wtf`
3. **New York (US East)**: `https://ny.mainnet.block-engine.jito.wtf`
4. **Salt Lake City (US West)**: `https://slc.mainnet.block-engine.jito.wtf`
5. **Tokyo (Asia)**: `https://tokyo.mainnet.block-engine.jito.wtf`

## 2. Dynamic Tip Oracle & Bidding Strategy
- **Live Feed Ingestion**: Ingests real-time Jito Tip Floor distribution via REST (`https://bundles.jito.wtf/api/v1/bundles/tip_floor`) and WebSocket push (`wss://bundles.jito.wtf/api/v1/bundles/tip_stream`).
- **Freshness Gate**: Max allowed snapshot age is 5,000 ms (`DEFAULT_MAX_AGE_MS`). If the snapshot is stale, the engine refuses to price tips blind (fail-closed).
- **Tip Budget & Hurdles**:
  - Minimum Tip: 1,000 lamports (`MIN_JITO_TIP_LAMPORTS`).
  - Maximum Tip Share: Strictly capped at 30% of projected net profit (`max_tip_share = 0.30`).
  - Canary Absolute Hard Cap: 0.01 SOL (10,000,000 lamports / ~$2.00).
  - Hurdle: If the market tip floor exceeds 30% of projected net profit, the trade is rejected (`RefuseReason::CeilingTooLowForFloor`).

## 3. Tip Account Selection by Slot
Jito maintains 8 rotating tip recipient accounts. The engine deterministically selects the active recipient for the current slot:
```rust
pub fn jito_tip_account_for_slot(slot: u64) -> Pubkey
```
Ensures 100% agreement between validator tip verification and transaction instruction accounts.

## 4. Inflight Bundle Lifecycle & Recovery
- **Submission ≠ Landing**: Submission returning a bundle UUID only confirms ingestion by the Block Engine.
- **Ambiguous Timeout Protocol**: If confirmation times out, the engine transitions the intent to `PendingRecovery` and monitors on-chain signature status.
- **Expiration Invariant**: A transaction is marked expired only when the current slot strictly exceeds `last_valid_block_height + 160`. Re-signing or resubmitting an inflight transaction is strictly prohibited to prevent double-spending.
