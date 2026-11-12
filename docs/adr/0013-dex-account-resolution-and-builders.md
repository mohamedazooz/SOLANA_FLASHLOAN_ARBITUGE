# ADR-0013: Dynamic On-Chain Account Resolution & DEX Instruction Builders

**Date**: 2026-11-09  
**Status**: accepted  
**Deciders**: Principal Solana Architect, Protocol Engineering Lead  

## 1. Context & Architectural Reality

An exhaustive reverse-engineering audit revealed why testing with live execution (`--mode live`) fails across 5 out of 6 supported protocols:

1. **Raydium V4**: `arb-tx/src/lib.rs:1489-1495` falls back to `RaydiumV4Accounts::mainnet_sol_usdc()`. If `hop.pool_id` is any other pool, it swaps against the SOL-USDC OpenBook market and vaults, triggering an immediate on-chain revert (`0x1` / custom error).
2. **Orca Whirlpool**: 
   - When accounts are missing, derives vaults via `[b"token_vault_a", pool]`, which does not exist on Solana (vaults are SPL token accounts stored in the Whirlpool struct).
   - Even when `PoolExecutionAccounts::OrcaWhirlpool` is supplied, `OrcaWhirlpoolAccounts::for_whirlpool` discards `tick_array_pdas` and hardcodes `tick_spacing = 64, tick_current = 0`, causing swaps on non-zero tick pools to revert.
   - `arb-ingest` derived tick arrays using `start.to_be_bytes()` instead of Anchor's `start.to_string().as_bytes()`.
3. **Meteora DLMM**:
   - Fallback derives `[b"reserve_x", pool]` (non-existent PDA).
   - Discards `bin_array_pdas` from `PoolExecutionAccounts::MeteoraDlmm` and hardcodes `METEORA_SOL_USDC_BIN_STEP = 10`. Any pool with bin step != 10 (e.g. 20, 25, 80) builds wrong bin arrays and reverts.
4. **Raydium CPMM**: Fallback hardcodes single `amm_config` (`D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ`).
5. **Raydium CLMM**: Fallback hardcodes the *CPMM* config (wrong program owner!) and hardcodes `tick_spacing = 60, tick_current = 0`.
6. **PumpSwap**: `arb-tx:1449` returns `TxError::UnsupportedProtocol`, resulting in 0% execution.
7. **Pipeline Propagation**: `supervisor/run.rs:949` stripped `execution_accounts` during split-routing re-quoting by creating `SwapHop { execution_accounts: None, .. }`.

## 2. Core Decisions

### Decision 1: Fail-Closed Account Resolution (No Dummy Fallbacks)
Remove all fallback derivations that substitute canonical SOL-USDC accounts or invalid PDA seeds for non-canonical pools. If a pool lacks verified `PoolExecutionAccounts`, fail-closed with `TxError::MissingExecutionAccounts` during transaction compilation. This prevents burning priority fees and Jito tips on doomed transactions.

### Decision 2: Direct Utilization of Precomputed Execution Accounts
Update `build_swap_hop_instruction`:
- **Raydium V4**: Use `amm_authority`, `open_orders`, `target_orders`, `base_vault`, `quote_vault`, `serum_program`, `serum_market`, `serum_bids`, `serum_asks`, `serum_event_queue`, `serum_coin_vault`, `serum_pc_vault`, `serum_vault_signer` directly from `PoolExecutionAccounts::RaydiumV4`.
- **Orca Whirlpool**: Consume `vault_a`, `vault_b`, `oracle`, and `tick_array_pdas` directly from `PoolExecutionAccounts::OrcaWhirlpool`. Fix `arb-ingest` tick array seed derivation to `[b"tick_array", whirlpool.as_ref(), start_tick.to_string().as_bytes()]`.
- **Meteora DLMM**: Consume `reserve_x`, `reserve_y`, `oracle`, and `bin_array_pdas` directly from `PoolExecutionAccounts::MeteoraDlmm`. If dynamic bin arrays are needed, read `bin_step` and `active_bin_id` from the hop or pool state.
- **Raydium CPMM**: Consume `amm_config`, `authority`, `vault_0`, `vault_1`, and `observation_state` from `PoolExecutionAccounts::RaydiumCpmm`.
- **Raydium CLMM**: Consume verified CLMM `amm_config`, `vault_0`, `vault_1`, `observation_state`, and `tick_array_pdas` from `PoolExecutionAccounts::RaydiumClmm`.
- **PumpSwap**: Implement native `build_pump_swap_instruction` for `buy` and `sell` using Anchor discriminators and the 12-account matrix.

### Decision 3: Pipeline Preservation
Preserve `execution_accounts` throughout the entire opportunity pipeline:
`PoolState` -> `Bellman-Ford / Engine Edge` -> `SwapHop` -> `ExecutionOpportunity` -> `supervisor/run.rs` -> `arb-tx`.

---

## 3. Master TODO & Priority Matrix

| Phase | Priority | Component | Scope / File | Status |
|:---|:---:|:---|:---|:---:|
| **P0** | **Highest** | **Fix Account Resolution in `build_swap_hop_instruction`** | `crates/arb-tx/src/lib.rs:1436-1730` | 🔄 In Planning |
| **P0** | **Highest** | **Fix Ingest Account Parsing & Seed Derivations** | `crates/arb-ingest/src/registry.rs:1865-2124` | 🔄 In Planning |
| **P0** | **Highest** | **Implement PumpSwap Native Builder** | `crates/arb-tx/src/lib.rs` + `crates/arb-dex/src/pump_swap.rs` | 🔄 In Planning |
| **P1** | High | Yellowstone `apply_raw_account` Decoding | `crates/arb-ingest/src/yellowstone.rs` + `lib.rs:666` | ⏳ Pending |
| **P1** | High | PreWarmedTxTemplate Swap Amount Patching | `crates/arb-tx/src/template.rs:166-200` | ⏳ Pending |
| **P1** | High | PumpFunRaydiumArbitrageBuilder v0 + ALT Migration | `crates/arb-tx/src/pump_fun_raydium.rs:347-352` | ⏳ Pending |
| **P1** | High | Raydium CPMM & CLMM Pool Discovery Hubs | `crates/arb-ingest/src/discovery.rs` | ⏳ Pending |
| **P2** | Medium | Wire Guarded Quote as Sole Dispatch Path | `crates/arb-engine/` + `crates/arb-bot/src/supervisor/run.rs` | ⏳ Pending |
| **P2** | Medium | Durable Execution State Machine & Reconciliation | `crates/arb-store/` + `crates/arb-land/` | ⏳ Pending |
| **P2** | Medium | Multi-hop Fork Simulation Engine | `crates/arb-sim/` + `crates/arb-tx/` | ⏳ Pending |
| **P3** | Low | Wire Disconnected Ingest Streams | `arb-ingest/solami_stream.rs`, `pumpdev_stream.rs`, `bitquery.rs`, `dexscreener.rs` | ⏳ Pending |

---

## 4. Consequences & Verification

### Positive
- Unlocks genuine on-chain execution for non-canonical pools across Raydium V4, Orca, Meteora DLMM, Raydium CPMM, Raydium CLMM, and PumpSwap.
- Eliminates silent capital loss on reverts due to mismatched accounts.
- Fail-closed guarantees transactions are only simulated and sent when all required accounts are verified.

### Negative / Complexity
- Pools without fully tracked dependent accounts (e.g., untracked OpenBook market or tick arrays) cannot be traded until accounts are ingested.
- Registry memory footprint increases slightly to store execution account vectors.
