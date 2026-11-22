# INACTIVE Features — Implemented, Tested, Not Wired Into Production

Mission rule (ACTIVE / INACTIVE / ARCHIVE): an inactive feature may have working,
tested source code; inactive means it is not reachable from the production
execution path today. This file records why, where the source lives, what
reactivation requires, and the risks of reactivation.

---

## 1. On-chain `arb-executor` CPI path (`execute_three_leg`)

- **Status:** INACTIVE (implemented + unit-tested, not reachable from production)
- **Source location:**
  - On-chain program: `programs/arb-executor/src/` (dispatcher `lib.rs`, canonical
    three-leg body `instructions/execute_three_leg.rs`, tip variant
    `instructions/execute_three_leg_with_tip.rs`, CPI layer `cpi/meteora_dlmm.rs`)
  - Client builder: `crates/arb-tx/src/flash/arb_executor.rs`
    (`build_arb_executor_three_leg_instruction`, assembler at `arb_executor.rs:471/:581`)
- **Why inactive (evidence):**
  - Production execution path: `crates/arb-bot/src/supervisor/execution.rs::assemble_native_ixs`
    routes ONLY to `cross_dex_flash_arb::assemble_cross_dex_opportunity`,
    `TxBuilder::assemble_flash_arbitrage` (generic N-hop flash), or non-flash
    `TxBuilder::assemble_arbitrage`. Nothing in `arb-bot`/`arb-engine` calls the
    executor builders (repo-wide caller trace).
  - The program is **not deployed**: `ArbExec1111111111111111111111111111111111111`
    (`crates/arb-constants/src/lib.rs:658`) returns `null` from mainnet
    `getAccountInfo` (verified via RPC read, 2026-11-22). Even if wired, every
    CPI would fail with `ProgramAccountNotFound`.
- **What it buys if activated:** single top-level instruction per 3-hop route;
  balance-delta validation and intermediate-inventory isolation enforced ON-chain
  (`verify_sequential_balance_transitions`), immune to client assembly drift;
  smaller message layout (33 + bin-arrays accounts) with `execute_three_leg_with_tip`
  handling the Jito tip post-validation.
- **Why the direct flash path was chosen instead (operator rationale, recorded):**
  a Solana transaction is already atomic; the direct flash assembly
  (borrow -> N swaps -> repay -> close -> tip) needs no program deployment,
  no upgrade-authority custody, no deployed-binary/source correspondence
  maintenance, and its solvency contract is enforced by the shared fail-closed
  validator (`validate_flash_funded_three_pool_params`) plus per-leg min-out
  chaining. Fewer moving parts for the same atomicity.
- **Reactivation requirements:**
  1. Generate a REAL program keypair (the current vanity placeholder is not
     deployed and must not ship), fund deploy rent (~0.5-1.5 SOL), and deploy the
     verified SBF build — **operator approval required**.
  2. Prove deployed-binary/source correspondence (`docs/active/security/PROGRAM_DEPLOYMENT_VERIFICATION.md`).
  3. Wire a config-gated branch in `assemble_native_ixs` selecting the executor
     path for uniform-DLMM 3-hop routes (feature/config flag, default OFF).
  4. Add LiteSVM integration test for the full 33+c account layout + tip variant.
  5. Shadow-soak with the executor path before any live enablement.
- **Known risks on reactivation:** CPI stack depth currently 4 of Agave budget 5;
  compute-budget accounting of the inner program; account layout drift between
  client builder (`arb_executor.rs`) and on-chain parser must stay byte-exact
  (discriminators + Borsh field order).

## 2. `compile_3leg_meme_dlmm_arb` three-leg direct compiler

- **Status:** INACTIVE as a production executor (ACTIVE as discovery/sizing engine)
- **Source:** `crates/arb-tx/src/flash/compiler.rs:363`
- **Why:** `arb-engine/three_leg_meme_pipeline.rs` uses this compiler context for
  exact quoting, size search, and economics (discovery); the resulting
  `ThreeLegMemeArbitrageContext` is never compiled at runtime in production —
  final execution goes through `TxBuilder::assemble_flash_arbitrage` (generic
  flash path). Kept fully tested (`three_leg_meme_compiler_tests.rs`) as the
  byte-accurate V0/V1 reference and as the reactivation target if flash-less
  direct execution is ever chosen.
- **Reactivation:** call it from the dispatch path for flash-less modes, with
  `meme_*_ata_exists` prefilled from an actual RPC probe (defaults are
  fail-safe CreateIdempotent but emit unnecessary instructions).

## 3. `build_flash_funded_three_pool_transaction` (Kamino flash-funded three-pool assembler)

- **Status:** INACTIVE (implemented + tested, unwired)
- **Source:** `crates/arb-tx/src/flash/arb_executor.rs` + shared validator
  `validate_flash_funded_three_pool_params` (Phase 1C parity work, commit `1a5d526`)
- **Why:** production 3-hop execution uses the generic flash builder instead;
  this assembler embodies the Phase 1C-validated solvency contract and pairs with
  the executor path above (it emits `arb_executor::execute_three_leg`).
- **Reactivation:** same as item 1 (it consumes the executor instruction) or
  wire it directly for the flash-funded direct variant.

## 4. Removed this phase

- `crates/arb-tx/src/flash/pump_raydium_builder.rs` (648 lines) + the
  `pump-v1-legacy-builder` feature — merged unbuildable since PR #73 (circular
  arb-engine dependency, glob-re-export ambiguity), never compiles in any CI
  matrix or default build, zero callers. Historical references remain in
  `docs/archive/legacy/BUG_NOTES.md` (test/defect history preserved verbatim).
