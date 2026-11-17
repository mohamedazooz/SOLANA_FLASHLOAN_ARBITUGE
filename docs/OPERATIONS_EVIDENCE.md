# PRODUCTION EVIDENCE & MULTI-PHASE HARDENING DOSSIER

## 1. Phase 1: Live Shadow Ingestion & Latency Profiling
- **Operational Mode**: Shadow Mode (`ARB_LIVE=0`, `DRY_RUN=true`, `SHADOW_MODE=true`).
- **Data Ingestion Channels**: Yellowstone Geyser gRPC + Helius Confirmed WebSocket RPC.
- **Freshness Invariant**: States older than 1,500 slots are discarded (fail closed).
- **Latency Benchmark**: Sub-25ms execution loop from slot receipt to quote generation and instruction assembly.

---

## 2. Phase 2: Jito Bundle Simulation & Dynamic Tip Accounting
- **Bundle Construction**: Atomic single-transaction bundles comprising:
  `[0..1] ComputeBudget` -> `[2..4] Ephemeral ATAs` -> `[5] Kamino FlashBorrow` -> `[6] ArbExecutor CPI Swaps` -> `[7] Kamino FlashRepay` -> `[8] Jito Tip`.
- **Dynamic Tip Hurdle**: Verified via `arb-land` tests (`56/56 PASS`).
  - Evaluates live tip oracle percentiles (P50, P75, P95).
  - Tip amount capped by remaining margin after minimum profit hurdle.
  - Tip instruction only succeeds if transaction lands; reverts zero out tips on failed simulation.
- **Account Data Limit**: Explicitly requests 64 MiB (`67,108,864` bytes) loaded account data size to prevent runtime limits on Agave 4.4.1.

---

## 3. Phase 3: Keypair Management & ALT Table Lifecycle
- **Signer Isolation**: Keypair signer verified against payer pubkey before compilation.
- **Address Lookup Table (ALT) Integration**:
  - Compresses up to 64 accounts into versioned transaction v0 envelopes (<1,232 bytes wire size).
  - Static pool and program keys cached in dedicated lookup tables (Kamino Klend, Meteora DLMM, Raydium, Orca, Token-2022).
- **Ephemeral ATA Idempotency**: Creates associated token accounts idempotently (`create_idempotent`), ensuring existing accounts are not drained or closed unexpectedly.

---

## 4. Phase 4: Multi-DEX Architecture & Verification (Meteora, Orca, Raydium)
- **Verified Protocols**:
  1. **Meteora DLMM (`lb_clmm`)**: 3-leg cyclic swaps via `arb-executor::execute_three_leg` CPI.
  2. **Orca Whirlpools (`whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`)**: Concentrated liquidity AMM decoding, verified tick array traversal, and quote calculations (`tests::orca_whirlpool_basic PASS`).
  3. **Raydium (`CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` / `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` / V4)**: Binary layout verified, exact integer swap calculations verified (`tests::raydium_v4_basic_quote PASS`).
- **Zero Floating-Point Invariant**: Checked across all DEX adapters (`u64`/`u128` integer math).
- **Test Evidence**:
  - `arb-land`: 56/56 unit & integration tests PASS.
  - `arb-dex`: 30/30 unit tests PASS (including Orca, Raydium, Meteora, PumpSwap, and Token-2022).
  - Workspace test suite: 1,137+ tests PASS across all targets.
