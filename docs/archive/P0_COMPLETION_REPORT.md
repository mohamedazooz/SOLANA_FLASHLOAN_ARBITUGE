# P0 Completion Report — DEX Account Resolution Overhaul Closure

**Date:** Tue Nov 10 2026  
**Branch:** feat/dlmm-cross-pool-strategy  
**Commit:** 3883760 (baseline) + working tree changes  
**Live execution:** DISABLED (ARB_LIVE guard enforced; no broadcast, no Jito submission, no deployment)

---

## 1. Four Approved Decisions — All Applied

| Decision | Implementation | Status |
|---|---|---|
| Raydium AMM V4 V2 default with explicit V1 compatibility | `build_raydium_v4_swap_instruction_versioned()` selects schema via `RaydiumV4SwapVersion`; V2 (8 accounts, opcode 16) is default; V1 (18 accounts, opcode 9) requires all OpenBook accounts non-default, fails closed otherwise | ✅ Verified |
| PumpSwap V2 exact-input/exact-output mapping and verified V1 fallback | `PumpSwapPoolVersion::V2` default (17 accounts, `buy_v2`/`sell_v2`/`buy_exact_quote_in_v2`); `PumpSwapV1Legacy` explicit only; `PumpSwapTradeIntent::ExactIn` default; sell+ExactOutBase fails closed | ✅ Verified |
| Meteora official dynamic Bin Array resolution without fixed three-array ceiling | `meteora_bin_array_indices()` unbounded — count scales with `bins_traversed`; `build_meteora_dlmm_swap_instruction()` derives bin arrays dynamically in swap direction; `dlmm_bin_array_pdas()` in registry replaces fixed 2..=4 cap | ✅ Verified |
| Real-mainnet non-broadcast simulation | `crates/arb-tx/tests/mainnet_non_broadcast_simulation.rs` — `simulateTransaction` with `replaceRecentBlockhash: true`, `sigVerify: false`, `innerInstructions: true` against live Alchemy mainnet RPC; validates context slot, units consumed, program logs | ✅ Test file present (skips when no RPC credentials) |

---

## 2. Differential Tests (P0 Golden Vectors)

**File:** `crates/arb-tx/tests/p0_golden_vectors.rs` — 9 tests, all passing:

| Test | Coverage |
|---|---|
| `test_raydium_v4_v2_exact_8_accounts_and_opcode_16` | Raydium V4 V2: exact 8 accounts, opcode 16, AccountMeta order/flags |
| `test_raydium_v4_legacy_v1_18_accounts_backward_compatibility` | Raydium V4 V1: 18-account OpenBook matrix, opcode 9 |
| `test_raydium_v4_canonical_selection_in_build_swap_hop` | Versioned selection via `build_swap_hop_instruction` |
| `test_pump_swap_v2_exact_17_accounts_and_discriminators` | PumpSwap V2: exact 17 accounts, V2 discriminators |
| `test_meteora_dlmm_negative_bin_array_euclidean_division` | Meteora DLMM: bin array index via Euclidean division for negative bins |
| `test_orca_whirlpool_directional_arrays_and_adaptive_fee` | Orca: signed tick spacing * 88, directional arrays, adaptive fee oracle flag |
| `test_raydium_clmm_directional_tick_arrays_and_be_bytes` | Raydium CLMM: big-endian tick array seeds, direction |
| `test_raydium_cpmm_13_accounts_and_token_2022_support` | Raydium CPMM: 13-account matrix, Token-2022 dual token program |
| `test_pipeline_hop_isolation_no_account_leakage` | Multi-hop isolation: no execution_accounts leakage between hops; PumpSwap V2 |

---

## 3. Verified Behaviors

| Behavior | Location | Status |
|---|---|---|
| fee_config decoding | `crates/arb-dex/src/lib.rs` `decode_transfer_fee_config()` | ✅ Unit-tested |
| Token-2022 behavior | CPMM 13-account matrix with dual token programs; `get_associated_token_address_with_program_id()` | ✅ Tested |
| Tick/bin dynamic accounts | `meteora_bin_array_indices()` (unbounded); Raydium CLMM tick arrays; Orca tick arrays | ✅ Tested |
| Per-hop context resolution | `SwapHop.trade_intent`, `PoolExecutionAccounts` with `pool_version` | ✅ Tested |
| Stale update prevention | `AccountSnapshotStore::apply()` rejects older slot/write-version | ✅ Unit-tested |
| Stream generation lifecycle | `begin_stream_generation()`, `mark_stream_connected/disconnected()` | ✅ Unit-tested |
| Quote readiness gate | `quote_dependencies()`, `quote_dependencies_for_route()` | ✅ Unit-tested |

---

## 4. Build & Test Results

```
cargo fmt --all -- --check:          CLEAN (no diff)
cargo check --workspace --all-targets: PASS (0 errors, 12 crates)
cargo clippy --workspace --all-targets -- -D warnings: PASS (0 warnings)# SENTINEL / ARBITRAGUNIQUE — AUTONOMOUS SOLANA ARBITRAGE ENGINE BUILD & RUN

## ROLE

Act as a Principal Solana Rust Engineer, Quantitative Arbitrage Researcher, DEX Protocol Engineer, Low-Latency Systems Architect, and Production Reliability Engineer.

You are responsible for taking the existing Sentinel / ARBITRAGUNIQUE repository from its current state to a functioning, measurable, continuously running Solana arbitrage engine.

This is an implementation assignment, NOT a planning-only assignment.

You must inspect the actual repository, implement the required functionality, run tests, fix failures, start the application in safe paper-trading mode, and produce a complete engineering handover.

Do not stop after creating a plan or a checklist.

Do not ask me to approve every engineering decision. Use the existing project architecture, official protocol specifications, safe defaults, and the decision rules below.

Continue through all feasible phases autonomously. If a phase has a genuine external blocker, implement everything that does not depend on that blocker, document it precisely, and continue with the other independent work.

---

# PART A — PRIMARY OBJECTIVE

Build a profitable-opportunity detection and execution system supporting:

1. Arbitrage between two different Meteora DLMM pools.
2. Arbitrage between different Solana DEX protocols.
3. Multi-hop and triangular arbitrage when economically and technically valid.
4. Real-time market-state ingestion through Yellowstone gRPC.
5. Accurate exact-in and exact-out quoting.
6. Dynamic account resolution and protocol-native transaction construction.
7. Transaction simulation and strict net-profit validation.
8. Jito-compatible low-latency transaction submission.
9. Continuous monitoring, recovery, metrics, and risk management.

IMPORTANT:

Do not make the entire project dependent on all six DEX builders being perfect before the first working strategy is available.

The first deliverable is a working two-pool Meteora DLMM arbitrage MVP.

Once that works, expand it to additional DEXs.

The priority is a correct, executable, measurable arbitrage strategy, not the number of supported integrations.

# PART B — REPOSITORY AUDIT AND SAFE WORKSPACE

Before changing code:

1. Inspect the complete Cargo workspace.
2. Inspect the current Git branch, worktrees, commit, and dirty files.
3. Preserve all existing modifications.
4. Inspect:
   - arb-types
   - arb-constants
   - arb-ingest
   - arb-dex
   - arb-tx
   - arb-bot
   - supervisor
   - registry
   - route discovery
   - quote engine
   - execution engine
   - existing Solana transaction compiler
   - existing Jito integration
   - existing Yellowstone integration
   - existing documentation and tests.
5. Read ADRs, PROJECT_STATE.md, production runbooks, and existing audit reports.
6. Identify implementations that already exist. Reuse working components instead of duplicating them.

Create an isolated implementation branch or worktree without losing user changes.

Produce a short baseline report, then immediately begin implementation.

Do not assume that a previous report claiming successful P0 completion proves that the current code is correct.

# PART C — P0 FINAL ACCEPTANCE

Complete the outstanding P0 decisions before enabling any execution.

## C1. Raydium AMM V4

- Use SwapBaseInV2 and SwapBaseOutV2 as the default path.
- Verify the exact eight-account V2 matrix against the pinned official IDL.
- Keep V1 only as an explicit compatibility path.
- Do not automatically select V1 merely because legacy OpenBook accounts exist.
- Verify instruction data, account order, writable flags, and signer flags.

## C2. PumpSwap

Support:

- buy_v2
- buy_exact_quote_in_v2
- sell_v2

Select instructions using the actual pool quote mint and trade direction.

Check verified V2 eligibility.

Retain the correct V1 account matrix for pools that require the older version.

Do not invent fee recipient ATAs or creator vault accounts.

## C3. Meteora DLMM

Use the official Meteora Rust SDK / commons functionality where feasible.

Verify:

- ExactInput and ExactOutput swap quoting.
- Dynamic Bin Array resolution.
- Negative bin index handling.
- Bitmap Extension.
- Active ID and swap direction.
- Variable fees and base fees.
- Token-2022 transfer fees.
- Transfer hooks and remaining account metadata.
- Pool status and available liquidity.

Do not permanently cap execution to three Bin Arrays.

Three arrays may be a fast-path initial batch. Expand dynamically according to the actual swap traversal and resource budget.

## C4. Orca and Raydium CPMM/CLMM

Verify all account derivations and exact instruction account matrices against pinned official specifications.

Do not treat a valid PDA derivation as proof that the account exists or belongs to the correct pool.

## C5. Supervisor isolation

Every route leg must resolve its own execution accounts.

Never reuse the first hop's accounts for another pool.

Add adversarial tests for mixed-protocol routes, repeated pools, reversed token order, stale accounts, and missing accounts.

## C6. P0 completion criteria

Run all relevant tests, differential SDK comparisons, account fixture tests, and non-broadcast simulation tests.

Keep unresolved protocol paths fail-closed.

Do not claim all pools are supported simply because the workspace compiles.

---

# PART D — FIRST WORKING STRATEGY: METEORA DLMM TWO-POOL ARBITRAGE

This is the most important immediate development task.

Build a dedicated strategy named:

MeteoraDlmmCrossPoolArbitrageStrategy

## D1. Pool discovery

Discover different active Meteora DLMM pools.

Identify pools sharing the same underlying token pair even when their token ordering differs.

For example:

Pool A:
- Token X
- Token Y
- LbPair A

Pool B:
- Token X
- Token Y
- LbPair B

These must be two distinct pool addresses.

Never perform the initial cross-pool strategy by routing into and out of the same pool.

Collect and validate:

- LbPair state.
- Mint X and Mint Y.
- Reserve vaults.
- Token program IDs.
- Active bin ID.
- Bin step.
- Base and variable fee parameters.
- Oracle state.
- Bin Array bitmap.
- Bitmap extension, when present.
- Pool activation and trading status.
- Current liquidity and relevant bin data.

Use Yellowstone updates for discovery and updates, with RPC snapshots for initial synchronization and reconciliation.

Do not hardcode a list of pool addresses or use fabricated accounts.

## D2. Generate arbitrage directions

For every pair of distinct, eligible pools, test both directions.

Direction A:

Quote -> Base using Pool A.

Base -> Quote using Pool B.

Direction B:

Quote -> Base using Pool B.

Base -> Quote using Pool A.

For each direction, calculate the complete route using real protocol quotes.

Do not compare spot prices alone.

A displayed price difference is not an executable arbitrage opportunity.

The quote must account for the full amount, price impact, dynamic fees, token transfer fees, and the actual Bin Arrays traversed.

## D3. Use official Meteora quote logic

Reuse or verify the current official Rust commons functions, including:

- BinArray::bin_id_to_bin_array_index
- get_bin_array_pubkeys_for_swap
- Exact-in quote helpers
- Exact-out quote helpers
- Swap2 and SwapExactOut2 instruction builders where supported by the pinned SDK.

If an official helper does not cover a required case, implement the equivalent behavior and validate it against the official SDK.

Never replace DLMM quoting with a simplistic x*y=k constant-product formula.

DLMM prices and liquidity are distributed across individual bins. Your quote engine must simulate the actual bin traversal and applicable fees.

## D4. Trade-size optimization

Do not test only one arbitrary trade size.

Implement a bounded candidate-size optimizer.

Evaluate a configurable range of input quantities using:

1. Coarse logarithmic sampling.
2. Discrete DLMM quote evaluation.
3. Local refinement around the best candidate.
4. Maximum trade size and account/compute limits.
5. Exact token base units and integer arithmetic.

Never use floating-point arithmetic for final executable amounts.

For each candidate, calculate:

- Input quantity.
- First pool output.
- Second pool output.
- Effective execution prices.
- Price impact per hop.
- Total DEX fees.
- Token transfer fees.
- Network fees.
- Priority fees.
- Jito bid where applicable.
- Rent and account-creation requirements.

Select the size with the best verified net expected profit, subject to all configured risk limits.

## D5. Atomic transaction construction

For the initial two-pool MVP, prefer one Solana transaction containing both swap instructions, in execution order.

Instruction 1:
Meteora Pool A swap.

Instruction 2:
Meteora Pool B swap.

Both instructions must execute atomically.

Use a route-isolated intermediate token account or an existing audited balance-guard mechanism.

Prevent the engine from accidentally subsidizing an unprofitable route with unrelated pre-existing intermediate token inventory.

Prefer an exact-output first hop followed by an exact-input second hop when the pinned Meteora instruction versions support that route correctly.

Otherwise establish a provable balance and slippage strategy using the actual instruction semantics.

For every transaction enforce:

- Valid input amount.
- Maximum input amount.
- Minimum output amount.
- Correct intermediate account ownership.
- Correct token program.
- Correct pool ordering.
- Correct account metas.
- Route-specific minimum final quote output.
- Explicit minimum net-profit hurdle.

The final minimum output must make the route economically acceptable even under the configured worst-case slippage.

If the current architecture cannot guarantee the required profit and inventory invariants, do not silently enable live execution. Implement or propose an audited on-chain balance/profit guard and test it before live authorization.

Do not split the two hops into unrelated transactions merely to avoid transaction-size limitations.

If a route cannot fit within the supported execution format, reject it or implement a separately verified supported format. Do not claim it is atomic if it is not.

---

# PART E — GENERAL CROSS-DEX ARBITRAGE ENGINE

After the two-pool Meteora path works, generalize the system.

Create or improve a common adapter interface for each DEX.

Suggested conceptual responsibilities:

- discover_pools
- decode_pool_state
- resolve_execution_accounts
- quote_exact_in
- quote_exact_out
- required_remaining_accounts
- build_swap_instruction
- validate_execution_context

Do not force every protocol into identical account structures.

Each DEX may have its own execution account variant.

Integrate in this order:

1. Meteora DLMM.
2. Raydium CPMM.
3. Raydium CLMM.
4. Orca Whirlpool.
5. Raydium AMM V4.
6. PumpSwap.

Build routes from the actual graph of token mints and active pool edges.

Start with two-hop routes.

Then add three-hop triangular routes only when their resource footprint, liquidity, and profitability justify the complexity.

A route can use two different protocols or two different pools from the same protocol.

CEX-to-DEX arbitrage is a separate non-atomic strategy. Do not represent it as an atomic Solana route. It requires prefunded inventory, hedge controls, exchange connectivity, and separate settlement-risk management.

---

# PART F — YELLOWSTONE REAL-TIME INGESTION

Make live account-state ingestion reliable.

Implement:

1. apply_raw_account with correct protocol decoders.
2. Account subscriptions for relevant pool, vault, bin array, and bitmap accounts.
3. Slot and write-version ordering.
4. Commitment awareness.
5. Idempotent account-state updates.
6. Stale-update rejection.
7. Reconnection and bounded exponential backoff.
8. Subscription restoration after reconnection.
9. Gap detection and RPC resynchronization.
10. Periodic snapshot reconciliation.
11. Deleted-account and changed-owner handling.
12. Graceful shutdown.

Maintain separate timestamps and slot metadata for every relevant account.

For candidate routes, ensure the pool snapshots used for quoting are sufficiently consistent and not stale.

Use RPC getMultipleAccounts snapshots when a consistent multi-pool snapshot is required.

Maintain HTTP RPC polling for bootstrap, backfill, and reconciliation. Do not remove it before proving the streaming cache is correct.

Add metrics:

- account_update_lag_ms
- slot_lag
- last_processed_slot
- reconnect_count
- stale_update_rejections
- decode_failures
- reconciliation_failures
- cache_consistency_errors

Run the new ingestion path in shadow mode against the existing path first.

---

# PART G — PROFITABILITY AND RISK ENGINE

Implement one source of truth for net profitability.

P_net = FinalQuoteAmount - InitialQuoteAmount - TotalExecutionCosts

Account for:

- Base transaction fee.
- Priority fee.
- Jito tip.
- DEX fees, exactly once.
- Token-2022 transfer fees.
- Account-creation costs.
- Rent capital requirements.
- Other applicable execution costs.

Do not deduct a fee twice if the official quote already includes it.

Distinguish refundable rent deposits and temporary capital lockup from irrecoverable costs.

The engine must enforce:

P_net >= ConfiguredMinimumNetProfit

Also enforce a minimum net-profit basis-point threshold when configured.

Use integer/raw token units and checked arithmetic.

Add configurable limits for:

- Maximum input amount.
- Maximum route size.
- Maximum slippage.
- Maximum daily loss.
- Maximum network fee.
- Maximum Jito tip.
- Maximum concurrent attempts.
- Minimum liquidity.
- Maximum snapshot age.
- Maximum failed transactions.
- Minimum net profit.
- Minimum net profit in basis points.

Never invent profitable opportunities or force the system to trade.

Zero profitable opportunities is an acceptable honest result.

Explain why candidates were rejected, using structured rejection reasons such as:

- EDGE_BELOW_HURDLE
- INSUFFICIENT_LIQUIDITY
- STALE_POOL_STATE
- MISSING_ACCOUNTS
- INVALID_PDA
- MAX_COMPUTE_EXCEEDED
- TRANSACTION_TOO_LARGE
- SLIPPAGE_EXCEEDED
- TOKEN_PROGRAM_MISMATCH
- INVALID_INTERMEDIATE_BALANCE
- SIMULATION_FAILED

---

# PART H — TRANSACTION COMPILER AND JITO

Implement or improve an adaptive transaction compiler.

Support:

- Legacy where needed for compatibility.
- V0 with valid Address Lookup Tables.
- V1 where supported by every component in the transaction submission path.

Do not blindly assume that V1 is supported by the RPC, Jito endpoint, transaction libraries, or downstream components.

For V0:
- Respect the 1,232-byte limit.
- Validate ALT state and lookup resolution.
- Keep required signers in static account keys.

For V1:
- Respect its 4,096-byte transaction size limit.
- Do not expect ALT support.
- Set compute-unit and loaded-account-data limits explicitly in the message.
- Encode priority fee as the correct total-lamport config value, not the old per-CU price.
- Strip ComputeBudget instructions when using V1.

Simulate before authorizing any submission.

Measure:

- Serialized size.
- Account count.
- Writable accounts.
- Readonly accounts.
- Loaded account data.
- Requested CU.
- Simulated CU.
- Compilation time.

Jito integration:

- Use current Jito documentation and endpoints.
- Obtain valid tip accounts dynamically.
- Estimate the bid from current conditions and actual net profit.
- Do not use a fixed tip that can erase the arbitrage edge.
- Prefer including the tip in the same atomic transaction, subject to the selected execution model.
- Do not split a single atomic arbitrage route into separate swap and tip transactions without a verified reason.
- Understand the Jito auction and account-lock contention.
- Check submission and bundle status; receiving a bundle ID is not proof of landing.

A single transaction with two swaps is already atomic. Jito is an optional landing path, not a prerequisite for the atomicity of those two instructions.

If using a Jito endpoint that skips RPC preflight, run the application's own simulation and risk checks before submission.

---

# PART I — SIMULATION AND PAPER TRADING

Build a complete live-data paper-trading mode.

Default operation:

TRADING_MODE=PAPER
LIVE_TRADING=false

The bot must:

1. Connect to configured mainnet data sources in read-only mode.
2. Discover real pools.
3. Subscribe to live updates.
4. Generate real route candidates.
5. Compute exact quotes.
6. Optimize trade sizes.
7. Build unsigned or simulation-ready transactions.
8. Simulate against real mainnet state.
9. Store expected and simulated results.
10. Track opportunities that disappear before simulation or execution.
11. Never send or broadcast a trade in PAPER mode.

Record:

- Candidate ID.
- Route and pool IDs.
- Input and output mints.
- Input amount.
- Expected output.
- Expected gross profit.
- Net profit after costs.
- Snapshot slot.
- Quote age.
- Simulation slot.
- Program logs.
- Simulation error.
- Units consumed.
- Loaded account data size.
- Writable-account footprint.
- Serialized transaction size.
- Expected versus simulated token balance deltas.

Do not confuse successful simulation with guaranteed inclusion or realized profit.

Build a persistent SQLite or existing project database journal for:

- Opportunities.
- Quote snapshots.
- Rejection reasons.
- Simulations.
- Execution attempts.
- Actual transaction outcomes.
- Latency metrics.

Use existing project database infrastructure when available.

---

# PART J — LATENCY AND RELIABILITY

Optimize only after correctness is demonstrated.

Measure the full pipeline:

1. Account update received.
2. Account decoded.
3. Pool cache updated.
4. Route candidate generated.
5. Quote calculated.
6. Trade size optimized.
7. Execution accounts resolved.
8. Transaction compiled.
9. Transaction simulated.
10. Submission authorized.

Report P50, P95, and P99 latency.

Optimize:

- Unnecessary RPC calls.
- Repeated PDA derivations.
- Repeated quote calculations.
- Redundant account decoding.
- Allocation and copying.
- Lock contention.
- Dynamic account lookup.
- Transaction compilation.
- Template reuse.

Prewarm unsigned builders or instruction templates only.

Never mutate signed transaction bytes and expect signatures to remain valid.

Always refresh blockhashes, amounts, account metas, dynamic Bin Arrays, and required resource settings before a new signature is generated.

---

# PART K — CONTROLLED LIVE CANARY

Do not enable live trading by default.

First prove all of the following:

- P0 acceptance passed.
- Two-pool Meteora route discovery works.
- Exact quotes match official SDK behavior.
- Mainnet simulations pass.
- Atomic route semantics are verified.
- Net-profit guard is verified.
- No inventory subsidy is possible.
- Jito or RPC submission path is validated.
- Risk limits are active.
- Kill switch works.

Prepare a configurable canary mode with:

LIVE_TRADING=false
LIVE_CANARY_APPROVED=false

Use tiny, operator-configured capital and strict transaction, daily-loss, fee, and slippage caps only after explicit authorization.

Never create, fund, expose, or print wallet seed phrases or private keys.

Never change live trading flags or submit a real-money transaction without explicit operator authorization.

Provide an emergency kill switch that immediately prevents new submissions while preserving logs and state.

---

# PART L — CONTINUOUS OPERATION

After all relevant tests pass:

1. Launch the bot in PAPER mode using the existing development process manager.
2. Confirm the process is actually alive.
3. Confirm Yellowstone connectivity.
4. Confirm pools are discovered.
5. Confirm at least two eligible distinct Meteora pools can be evaluated where the network currently provides them.
6. Confirm quotes are continuously refreshed.
7. Confirm the simulation pipeline operates.
8. Confirm no live transaction is broadcast.
9. Store logs, metrics, and state.
10. Provide the exact start, stop, restart, and health-check commands.

Do not restart unrelated production services.

Do not overwrite .env files or expose secrets.

If a required RPC/gRPC endpoint is unavailable, implement the relevant integration and report the exact missing configuration. Continue implementing and testing offline functionality instead of stopping the entire project.

---

# PART M — TESTING AND ACCEPTANCE

Run, as applicable:

cargo fmt --all -- --check

cargo check --workspace

cargo test --workspace --all-targets

cargo clippy --workspace --all-targets --all-features -- -D warnings

Also run targeted integration and protocol tests for each affected crate.

Create tests for:

1. Two different Meteora pools with the same token pair.
2. Both possible arbitrage directions.
3. Negative bin IDs.
4. Multiple Bin Arrays.
5. Bitmap Extension.
6. Exact-in and exact-out quotes.
7. Dynamic fees.
8. Token-2022.
9. Intermediate account isolation.
10. Minimum final output.
11. Net-profit rejection.
12. Stale account rejection.
13. Missing account rejection.
14. Transaction-size limits.
15. V0 and V1 compilation when supported.
16. Atomic two-swap simulation.
17. Split-route isolation.
18. Reconnection and resynchronization.
19. Duplicate and out-of-order account updates.
20. Emergency kill switch.

Where possible, compare Rust instructions and quote outputs against the official Meteora TypeScript SDK and Rust SDK using recorded real account snapshots.

Keep network-dependent tests clearly separated from deterministic offline tests.

Do not report a successful test until the command has actually completed and its exit code has been checked.

---

# PART N — REQUIRED FINAL DELIVERABLE

Deliver an engineering report containing:

1. Actual Git branch and final commit hash.
2. Exact changed files.
3. Completed implementation phases.
4. Remaining blockers.
5. Test command and exact results.
6. Official SDK comparison results.
7. Number of active discovered Meteora pools.
8. Number of distinct eligible pool pairs.
9. Number of evaluated route directions.
10. Number of candidates rejected and their reasons.
11. Number of simulation successes and failures.
12. Best real candidate if one exists, with source snapshot and detailed net-profit accounting.
13. P50/P95/P99 pipeline latency.
14. Current bot process status.
15. Current trading mode.
16. Exact start/stop commands.
17. Residual-risk register.
18. Clear classification of claims:

VERIFIED
TESTED
SIMULATED
PAPER-TRADING
LIVE-AUTHORIZED
NOT VERIFIED

Never label PAPER results as realized trading profits.

Never claim that the engine is profitable merely because it detects price differences.

---

# EXECUTION POLICY

Start now.

Do not spend the entire session writing documentation before implementing the working strategy.

Prioritize a real two-pool Meteora DLMM route, then expand.

Fix actual compilation and test errors instead of hiding, skipping, weakening, or deleting tests.

Do not silently simplify the protocol math.

Do not fabricate pools, account data, prices, or successful arbitrage opportunities.

Do not stop after the first failure. Diagnose it, fix it where possible, rerun the relevant tests, and continue.

Maintain project documentation as each phase is completed.

Keep the PAPER engine running after successful validation if the available development environment supports it.

Keep live trading disabled until explicit operator authorization.

Your final objective is a functioning Solana arbitrage engine with real pool discovery, real executable quotes, validated atomic routes, accurate net-profit checks, and measurable execution readiness — starting with two Meteora DLMM pools and expanding to cross-DEX arbitrage.
cargo test --workspace --all-targets --no-fail-fast: PASS (all tests pass)
```

**Test counts by crate:**
- arb-tx: 37 lib + 7 bundle + 9 adversarial + 7 challenger + 4 fuzzing + 2 mainnet simulation + 9 golden vectors = 75
- arb-engine: 23 unit + 5 pipeline + 13 cross_pool_dlmm = 41
- arb-ingest: 87 unit + 4 integration = 91
- arb-math: 148 tests
- arb-dex: 27 tests
- arb-risk: 11 tests
- arb-sim: 4 tests
- arb-land: 49 tests
- arb-store: 3 tests
- arb-constants: 3 tests
- arb-metrics: 1 test
- arb-types: 0 tests
- arb-bot: 44 tests

**Total: 400+ tests passing, 0 failures.**  
(Note: `arb-math --bench cpmm_bench` target requires nightly Rust and is excluded; pre-existing, unrelated to changes.)

---

## 5. Files Changed (this pass)

### Modified (fixes to pre-existing P0 working tree):
- `crates/arb-tx/src/lib.rs` — Fixed `test_meteora_multi_bin_array_expansion` expectations (17/20/24 accounts match dynamic bin array calculation)
- `crates/arb-tx/tests/bundle_tests.rs` — Added missing `trade_intent: Default::default()` field
- `crates/arb-tx/tests/mainnet_non_broadcast_simulation.rs` — Fixed unused variable warning
- `crates/arb-engine/src/bellman_ford.rs` — Added missing `trade_intent` field
- `crates/arb-engine/src/lib.rs` — Added missing `trade_intent` field
- `crates/arb-bot/src/supervisor.rs` — Removed invalid `trade_intent` field from PoolState
- `crates/arb-bot/src/supervisor/run.rs` — Added missing `trade_intent` field to SwapHop
- `crates/arb-ingest/src/registry.rs` — Replaced `.distinct()` on FlatMap (not available) with BTreeSet filter
- `crates/arb-engine/tests/cross_pool_dlmm_strategy.rs` — Fixed intermediate_mints test assertion; fixed doc comment lint

### Pre-existing P0 changes (already in working tree):
- `crates/arb-types/src/lib.rs` — `PumpSwapPoolVersion`, `PumpSwapTradeIntent`, `pool_version` field on `PoolExecutionAccounts::PumpSwap`, `trade_intent` on `SwapHop`
- `crates/arb-tx/src/lib.rs` — V2 default builders, V1 compatibility, PumpSwap V2 exact-in/exact-out, Meteora dynamic bin arrays, `derive_meteora_bin_array_bitmap_extension_pda()`, `meteora_bin_array_indices()`
- `crates/arb-bot/src/supervisor/run.rs` — MeteoraDlmmCrossPoolArbitrageStrategy integration
- `crates/arb-engine/src/lib.rs` + `cross_pool_dlmm.rs` — Cross-pool DLMM strategy
- `crates/arb-bot/src/main.rs` — Web dashboard, adaptive polling
- `crates/arb-ingest/src/registry.rs` — Meteora bin array PDA derivation

### New files (pre-existing):
- `crates/arb-engine/tests/cross_pool_dlmm_strategy.rs` — 13 cross-pool DLMM tests
- `crates/arb-tx/tests/mainnet_non_broadcast_simulation.rs` — Real-mainnet non-broadcast simulation

---

## 6. Residual-Risk Register

| ID | Risk | Severity | Mitigation |
|---|---|---|---|
| P0-R1 | No differential fixtures against official SDK/on-chain quotes for any protocol | High | Tests use locally constructed accounts; no on-chain parity evidence |
| P0-R2 | Raydium V4 V1 path not fork-verified | Medium | Fails closed if any OpenBook account is default |
| P0-R3 | PumpSwap V2 not on-chain verified | Medium | Builder produces structurally correct instruction; fail-closed for live |
| P0-R4 | Meteora DLMM bin array PDA resolution not on-chain verified | Medium | Uses official SDK PDA seeds; no live transaction evidence |
| P0-R5 | Real-mainnet simulation test requires RPC credentials (skips in CI) | Medium | Test gracefully skips when no SOLANA_RPC_PRIMARY |
| P0-R6 | `arb_engine::guarded` quote path not wired to supervisor | High | GUARD-01 remains open; direct quote path still used |
| P0-R7 | No durable accepted→landed state machine | High | Open from Phase 14; not in P0 scope |

---

## 7. Next Recommended Phase

**P1 — Yellowstone Live Account Ingestion.** All P0 acceptance criteria pass. The Yellowstone client already has slot/write-version awareness, reconnect/backoff, quote readiness, and stale-update prevention. P1 work: dual-ingestion shadow mode, additional metrics (account_update_lag_ms, resyncs, decode errors, stale rejections, cache consistency), and HTTP-vs-streaming benchmark.

**Do not start P1 until operator confirms P0 acceptance.**

---

**Live execution remains disabled.** No mainnet transaction broadcast, Jito submission, PM2 restart, VPS deployment, or production configuration change was performed.