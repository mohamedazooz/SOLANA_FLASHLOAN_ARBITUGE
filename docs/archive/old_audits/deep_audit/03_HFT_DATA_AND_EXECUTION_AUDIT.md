> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# HFT Data Stream & Execution Engine Verification Report
**Module Focus:** `crates/arb-ingest`, `crates/arb-store`, `crates/arb-tx`, `crates/arb-land`  
**Auditor:** HFT & Data Stream Lead / Supervisor  

---

## 1. Data Ingestion Architecture & Yellowstone gRPC

### Ingestion Pipeline (`crates/arb-ingest/src/yellowstone_grpc.rs`)
- **Direct Geyser Ingestion:** Account notifications arrive over bidirectional HTTP/2 gRPC streams directly from validator memory (Triton, Helius LaserStream, or private Geyser nodes).
- **Subscription Scope:** Granular filtering on targeted DEX Program IDs:
  - Raydium CPMM / AMM v4 / CLMM
  - Meteora DLMM (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`)
  - Orca Whirlpool
  - Pump.fun bonding curves
- **Slot Invalidation (`crates/arb-ingest/src/slot_integrity.rs`):**
  - Tracks slot progression and fork switches.
  - If a slot is marked dead (orphaned fork), all pool updates stamped with that slot are immediately discarded.

---

## 2. Hot-Path Isolation Audit

### Code Verification against `LOW_LATENCY_BOTS.md`
The hot path is strictly isolated to sub-millisecond execution:
```text
Event Receive -> Zero-Copy Decode -> StateGuard Update -> Opportunity Filter -> Tx Compile -> Jito Submit
```

**Verified Invariants on the Critical Path:**
1. **NO RPC Calls:** Neither `getLatestBlockhash`, `getBalance`, nor `getAccountInfo` are called synchronously during an opportunity tick.
2. **Cached Blockhashes:** `blockhash_cache` maintains a recent blockhash in memory with TTL $\approx 2-3\text{ seconds}$, completely removing 150-300ms network roundtrips.
3. **Pre-Warmed Templates (`ram_templates`):** In `crates/arb-bot/src/supervisor.rs`, pre-warmed transaction byte arrays are stored in memory; only dynamic fields (amounts, tips, blockhash) are patched in-place at fire time.

---

## 3. In-Memory State & Lock Contention Audit (`crates/arb-store`)

- **StateGuard Implementation:** Replaces coarse global `RwLock` structures with fine-grained per-pool or copy-on-write atomic pointers (`Arc<AtomicPtr>` / `parking_lot::RwLock`).
- **Tombstones:** Accounts that drop to `lamports == 0` (closed accounts) are explicitly recorded as tombstones, preventing the bot from quoting on ghost liquidity.

---

## 4. Transaction Assembly & ALTs (`crates/arb-tx`)

- **Address Lookup Tables (ALTs):** Because 3-leg triangle arbitrage involves at least 3 distinct pools, multiple token accounts, bin arrays, and protocol program IDs ($>35$ accounts), standard legacy transactions ($1232\text{ bytes}$) would exceed MTU limits without ALTs.
- **ALTs Compression:** Compresses 32-byte public keys into 1-byte lookup indices, bringing the compiled transaction size well within Solana's wire limit.
- **V1 Transport:** Supports V1 packet framing (up to 4096 bytes) when running via direct Jito Block Engine connections.

---

## 5. Jito Bundle Submission & Dynamic Tip Calibration (`crates/arb-land`)

### Code Verification in `crates/arb-land/src/tip_oracle.rs` & `jito.rs`
1. **No Public Mempool:** Transactions are bundled and transmitted exclusively via Jito's private Block Engine gRPC endpoint.
2. **Leader Schedule Gating:** `jito_service.leader_for(current_slot)` verifies whether the upcoming slot leader is a Jito-enabled validator. If not, the transaction is held back, saving priority fees.
3. **Dynamic Tip Calibration:**
   $$\text{Tip} = \text{clamp}\left(\text{Net Profit} \times \text{Tip Fraction}, \text{MIN\_TIP}, \text{MAX\_TIP}\right)$$
   Where tip fraction scales with competition density (typically 50% to 75% of expected gross profit).
4. **Randomized Tip Accounts:** Randomly chooses across Jito's 8 official tip accounts to eliminate state write contention at the tip receiver account.
