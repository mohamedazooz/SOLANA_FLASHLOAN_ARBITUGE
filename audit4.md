Comprehensive Technical Audit: arb-tx, arb-sim, arb-land, and arb-risk

This audit covers the architecture, mechanics, concrete implementations, and critical bugs/gaps across crates/arb-tx/src/, crates/arb-sim/src/, crates/arb-land/src/, crates/arb-risk/src/, and crates/arb-bot/src/jito_service.rs.

1. Swap Instructions & Atomic Arbitrage Assembly (crates/arb-tx/src/)
A. Assembly Architecture (lib.rs, template.rs, pump_fun_raydium.rs)

In crates/arb-tx/src/lib.rs, atomic arbitrage transactions are assembled via TxBuilder:

TxBuilder::assemble_atomic_bundle (lib.rs:1810-1888): Builds a strictly ordered 7-instruction (2-hop) or 8-instruction (3-hop) bundle:
[0] SetComputeUnitLimit (COMPUTE_BUDGET_LIMIT_2HOP = 260_000 / 320_000 for 3-hop).
[1] SetComputeUnitPrice (dynamic micro-lamports priority fee).
[2] Kamino FlashBorrow (amount: $d^*$).
[3..] Swap Legs derived via derive_swap_legs (lib.rs:2073-2128), ensuring $amount_in$ equals previous hop's expected output and final hop $min_out$ enforces the solvent profit floor ($borrow + flash_fee + tip + min_profit$).
[5] (or [6]) Kamino FlashRepay ($repay = borrow + fee$, referencing borrow_instruction_index = 2).
[6] (or [7]) Jito Tip Transfer with MEV sandwich protection (jitodontfront marker).
TxBuilder::assemble_flash_arbitrage (lib.rs:2132-2294): Full flash route with ephemeral Associated Token Accounts (ATAs). Flow: ComputeBudget -> Idempotent ATAs -> WSOL unwrapping (if Hop 1 is Pump.fun) -> FlashBorrow -> Swap Hops -> WSOL re-wrapping (if final hop is Pump.fun) -> FlashRepay -> Close ATAs (reclaiming 100% rent 0.00203928 SOL to payer) -> Jito Tip.
TxBuilder::assemble_jupiter_flash_arbitrage (lib.rs:2299-2350): Delegates swap instruction resolution to Jupiter v6 Swap API while sandwiching Kamino Borrow at the start and Kamino Repay + Tip at the end.
B. Protocol-by-Protocol Breakdown: Actual On-Chain Builders vs. Jupiter vs. Missing/Placeholder
Protocol	Implementation Status	Native Instruction Builder	Accounts Matrix	Account Resolution Reality
PumpSwap	Missing / Placeholder	❌ None	None	Explicitly returns TxError::UnsupportedProtocol("PumpSwap transaction adapter is not enabled; shadow-only") (lib.rs:1441).
Raydium V4	Broken for non-canonical pools	✅ build_raydium_v4_swap_instruction_18 (lib.rs:577-616), opcode 9 (SwapBaseIn)	18 accounts (RaydiumV4Accounts, lib.rs:491-574)	Critical Flaw: In build_swap_hop_instruction (lib.rs:1442-1455), it takes RaydiumV4Accounts::mainnet_sol_usdc() and only overwrites accounts.amm_pool = hop.pool_id.0. All other 17 accounts (Serum market, Open Orders, Bids, Asks, Coin/PC vaults) remain hardcoded to SOL-USDC! Will revert on-chain for any other pool.
Orca Whirlpool	Broken for non-canonical pools	✅ build_orca_whirlpool_swap_instruction_11 (lib.rs:692-749), Anchor discriminator [248, 198, 158, 145, 225, 117, 135, 200]	11 accounts (OrcaWhirlpoolAccounts, lib.rs:619-689)	Critical Flaw: In lib.rs:1464-1484, for arbitrary pools it attempts to derive token_vault_a/b with seeds [b"token_vault_a", pool], which do NOT exist in Whirlpool. Furthermore, hardcodes tick_spacing = 64 and tick_current = 0, deriving invalid tick arrays.
Meteora DLMM	Broken for non-canonical pools (requires Jupiter)	✅ build_meteora_dlmm_swap_instruction (lib.rs:874-996), METEORA_SWAP2_DISCRIMINATOR	16 base accounts + 2..=4 dynamic Bin Arrays (lib.rs:807-858)	Critical Flaw: In lib.rs:1510-1533, it attempts to derive reserves via PDA seeds [b"reserve_x", pool], which is invalid (reserves are stored in the LbPair account, not derived). Hardcodes bin_step = METEORA_SOL_USDC_BIN_STEP (lib.rs:1547). Only SOL-USDC canonical pool is valid.
Raydium CPMM	Partial (derived vaults, hardcoded config)	✅ build_raydium_cpmm_swap_instruction (lib.rs:1143-1175), RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR	13 accounts (RaydiumCpmmAccounts, lib.rs:999-1112)	Derives vaults via [b"pool_vault", pool, mint], observation via [b"observation", pool], authority via [b"vault_and_lp_mint_auth_seed"]. Flaw: Hardcodes amm_config to "D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ" (lib.rs:1562).
Raydium CLMM	Broken / Invalid Config	✅ build_raydium_clmm_swap_instruction (lib.rs:1289-1324), RAYDIUM_CLMM_SWAP_DISCRIMINATOR	12 accounts (RaydiumClmmAccounts, lib.rs:1178-1257)	Critical Flaw: amm_config is set to "D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ" (lib.rs:1584), which is a CPMM config, NOT a CLMM config! tick_spacing is hardcoded to 60, and tick_current defaults to 0 (lib.rs:1587-1588).
Pump.fun	Fully functional on-chain builder	✅ build_pump_fun_swap_instruction (lib.rs:1396-1431), Buy/Sell discriminators	12 accounts (PumpFunAccounts, lib.rs:1327-1375)	Derives global ([b"global"]), bonding_curve ([b"bonding-curve", mint]), event_authority ([b"__event_authority"]), handles WSOL wrap/unwrap and rent reclaim.
Jupiter v6 API	Complete aggregator client	JupiterClient (crates/arb-tx/src/jupiter.rs)	Dynamic via /swap-instructions and /swap	Queries Jupiter v6 REST API with multi-key rotation on 429 (jupiter.rs:67-284). Resolves 100% of accounts for arbitrary routes.
2. Flash Loans, ALTs, Compute Budget, and Jito Tips
A. Kamino Flash Loans (crates/arb-tx/src/kamino.rs)
Program ID: KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD.
Account Matrix (KaminoFlashAccounts): Exactly 12 accounts matching the Anchor IDL: user_authority (signer), lending_market_authority (PDA derived with [b"lma", lending_market]), lending_market, reserve, reserve_liquidity_mint, reserve_supply_vault, user_liquidity_ata, reserve_fee_vault, optional referrer accounts (defaulting to program ID), sysvar_instructions, and token_program.
Discriminators & Serialization:
flash_borrow_reserve_liquidity: 16 bytes = [174, 185, 149, 129, 218, 77, 249, 151] (8 bytes) + amount (8 bytes LE).
flash_repay_reserve_liquidity: 17 bytes = [194, 219, 120, 241, 107, 184, 199, 137] (8 bytes) + repay_amount (8 bytes LE) + borrow_instruction_index (1 byte u8).
Fee Calculation: calculate_kamino_flash_fee (kamino.rs:170-179) uses exact Q64.64 fixed-point arithmetic: $$\text{fee} = \left\lceil \frac{\text{borrow_amount} \times \text{fee_sf}}{2^{64}} \right\rceil$$
On-Chain Fee Ingestion: decode_reserve_flash_fee_sf (kamino.rs:221-250) reads live Reserve state, verifying discriminator [0x2b, 0xf2, 0xcc, 0xca, 0x1a, 0xf7, 0x3b, 0x7f], liquidity mint at byte offset 128 (8 + 120), active status at offset 4856 (8 + 4848), and flash_loan_fee_sf at offset 4904 (8 + 4896).
B. Address Lookup Tables (ALTs) (crates/arb-tx/src/alt.rs & lib.rs)
create_lookup_table (alt.rs:44-70): Derives ALT PDA [authority, recent_slot.to_le_bytes()] under AddressLookupTab1e1111111111111111111111111, issuing opcode 0 with recent_slot and bump.
extend_lookup_table (alt.rs:84-115): Emits opcode 2, chunking addresses into batches of MAX_ALT_EXTEND_ADDRESSES = 30 to guarantee transactions fit within the 1232-byte MTU limit.
compile_v0_message (lib.rs:2559-2724): Serializes Solana v0 versioned message format: [0x80 prefix | Header (3 bytes) | Static Keys | Blockhash (32 bytes) | Compiled Instructions | Address Table Lookups]. Maps accounts into static signers, static non-signers, and ALT table lookups (writable_indices, readonly_indices).
create_default_mainnet_alt (lib.rs:2750-2779): Pre-warms 24 static addresses (Token, ATA, System, Sysvar, ComputeBudget, Kamino, Raydium v4/SOL-USDC, Orca Whirlpool/SOL-USDC, Meteora DLMM, Serum, WSOL, USDC).
C. Compute Budget & Jito Tip Assembly
Compute Budget Instructions:
build_compute_unit_limit(units) (lib.rs:183-192): Program ComputeBudget111111111111111111111111111111, opcode 2 + units (LE u32).
build_compute_unit_price(micro_lamports) (lib.rs:194-204): Opcode 3 + micro_lamports (LE u64).
Always injected at positions [0] and [1].
calculate_fee_deduplicated_cu_price: when bundle_only = true, sets CU price to 0 to concentrate 100% of the priority budget into Jito tip and avoid double-paying.
Jito Tips & Sandwich Protection:
build_jito_tip_instruction (lib.rs:212-229): System Program Transfer (index 2) to one of the 8 verified tip accounts (ADaUMid..., DfXygSm..., etc.).
attach_jitodontfront (lib.rs:242-254) & build_marked_jito_tip_instruction: Appends jitodontfront1111111111111111111111111111 as a read-only account. The Jito block engine rejects bundles containing this marker unless the transaction sits at bundle index 0, guaranteeing anti-sandwich MEV protection.
3. Simulation, Risk, and Landing Subsystems
A. Simulation Engine (crates/arb-sim/)
RPC Simulation (lib.rs:42-139): RpcSimulator::simulate_async posts JSON-RPC simulateTransaction with replaceRecentBlockhash: true, commitment: "processed", and timeout (DEFAULT_SIM_TIMEOUT_MS = 1500ms). Extracts unitsConsumed, logs, and execution error.
In-Memory LiteSVM Simulation (litesvm_sim.rs:21-71):
Intended for sub-millisecond local pre-flight checks.
Critical Flaw: It calls let svm = LiteSVM::new(); svm.simulate_transaction(tx). It does not load any on-chain program ELF binaries (Raydium, Orca, Meteora, Kamino, Pump.fun) or account states into the VM! Any real transaction simulation fails instantly. Furthermore, litesvm is behind an optional Cargo feature disabled by default (Cargo.toml:20).
B. Risk Management (crates/arb-risk/src/lib.rs)
CircuitBreaker (lib.rs:16-107): Trips from Closed to Open after max_consecutive_failures (default 8). Remains Open for cooldown_slots (default 10 slots), then moves to HalfOpen. Requires success_threshold (default 3) consecutive successful executions to reset back to Closed.
KillSwitch (lib.rs:115-202): Atomic in-memory flag backed by a persistent file on disk (with_file). If the file exists, the system starts in a blocked state, ensuring safety persists across process crashes and restarts.
RiskGate (lib.rs:379-740): Enforces multi-layer invariants on every trade:
Kill switch & circuit breaker state.
Max trade size USD ($25.0 default).
Max order velocity (MAX_VELOCITY_ORDERS_PER_HOUR = 10, max_orders_per_hour = 120).
Max tip share (70%–85% dynamic cap) and max tip lamports (2,000,000 lamports).
Max price impact (40 bps) and max quote age (2 slots).
Profit floor (min 30,000 lamports and 8 bps).
Loss limits (hourly $2.0, daily $5.0).
In-flight bundles per slot limit (max 2 bundles).
Tip spend ratio ($tip / (tip + net) \le 0.7$).
Minimum landing rate floor (2% minimum after 20 attempts).
Chronos Spatiotemporal Expected Value (EV) Gate (lib.rs:250-377): Models edge decay: $$P_{\text{survival}} = e^{-\lambda \tau}$$ Calculates rigorous Expected Value: $$\text{EV} = P_{\text{survival}} \times \Pi_{\text{success}} + (1 - P_{\text{survival}}) \times \Pi_{\text{failure}}$$ Where $\Pi_{\text{failure}} = -(base_fee + priority_fee + failed_attempt_cost)$. Drops quotes if age $> 400\text{ ms}$ or $\text{EV}_{\text{USD}} < $0.05$.
C. Landing Engine (crates/arb-land/ & crates/arb-bot/src/jito_service.rs)
Tip Oracle (tip_oracle.rs): REST poller (TIP_FLOOR_URL = https://bundles.jito.wtf/api/v1/bundles/tip_floor) and WebSocket stream (TIP_WEBSOCKET_URL = wss://bundles.jito.wtf/api/v1/bundles/tip_stream). Enforces strict freshness (DEFAULT_MAX_AGE_MS = 5000ms); returns None if stale to prevent blind bidding.
Leader Schedule & Jito Validator Registry (leader.rs): Inverts getLeaderSchedule RPC response and joins with Kobe validator registry (https://kobe.mainnet.jito.network/api/v1/validators). Determines whether the target slot leader runs Jito-Solana (Landability::Jito vs NonJito vs Unknown).
Adaptive Bidder (bidder.rs): Computes hurdle and ceiling ($t_{\max}$), walks percentile rungs ($P_{25} \dots P_{99}$), evaluates expected value: $$\text{EV} = P(\text{land}) \times (\text{edge} - \text{tip})$$ Where $P(\text{land})$ is the Wilson score lower bound from LandingLedger. Enforces escalation cap: retry tip $\le \text{previous_tip} \times 1.6$.
Jito Service (crates/arb-bot/src/jito_service.rs): Wires the entire pipeline:
If leader is non-Jito: downgrades to plain RPC or SWQoS direct route, or refuses bundle tip.
Multi-region concurrent fanout across Amsterdam, Frankfurt, NY engines via shared_dispatch_client().
Verifies landings dual-track: Jito getBundleStatuses + Solana RPC getSignatureStatuses.
4. Critical Gaps, Bugs, and Missing Implementations
Critical Findings Table
#	Severity	File & Lines	Subsystem / Function	Defect / Vulnerability Description
1	P0 - Blocker	crates/arb-tx/src/lib.rs:1442-1455	build_swap_hop_instruction (DexProtocol::RaydiumV4)	Raydium V4 accounts not resolved: Copies RaydiumV4Accounts::mainnet_sol_usdc() and only overrides amm_pool. All 17 other accounts (vaults, Serum market, open orders) remain hardcoded to SOL-USDC. Reverts on-chain for any other pool.
2	P0 - Blocker	crates/arb-tx/src/lib.rs:1464-1484	build_swap_hop_instruction (DexProtocol::OrcaWhirlpool)	Orca Whirlpool account derivation invalid: Derives vaults with non-existent PDA seeds [b"token_vault_a", pool], hardcodes tick_spacing = 64 and tick_current = 0. Reverts on-chain for non-canonical pools.
3	P0 - Blocker	crates/arb-tx/src/lib.rs:1510-1533, 1547	build_swap_hop_instruction (DexProtocol::MeteoraDlmm)	Meteora DLMM reserve derivation invalid: Derives reserves with non-existent PDA seeds [b"reserve_x", pool] (reserves are stored in LbPair account data), and hardcodes bin_step to METEORA_SOL_USDC_BIN_STEP. Reverts on-chain.
4	P0 - Blocker	crates/arb-tx/src/lib.rs:1584-1599	build_swap_hop_instruction (DexProtocol::RaydiumClmm)	Raydium CLMM AmmConfig & Tick spacing mismatch: Sets amm_config to CPMM config (D4A2sqNAqq...), hardcodes tick_spacing = 60 and tick_current = 0. Reverts on-chain.
5	P0 - Blocker	crates/arb-tx/src/template.rs:166-200	PreWarmedTxTemplate::patch_and_sign	Swap amounts never patched in template: Patches borrow amount, repay amount, tip, and blockhash, but never updates swap instruction data (amount_in, min_amount_out). If borrow amount is changed on the hot path, swap legs execute with stale amounts, causing slippage or insolvency failure.
6	P1 - Critical	crates/arb-tx/src/pump_fun_raydium.rs:347-352, 530-553	PumpFunRaydiumArbitrageBuilder::build_signed	Legacy transaction MTU overflow: Uses legacy transaction format (TxBuilder::build_signed_transaction). 18 Raydium accounts + 12 Pump accounts + ComputeBudget + Tip produces 1293 bytes, exceeding the 1232 MTU limit and always failing validate_jito_bundle.
7	P1 - Critical	crates/arb-tx/src/lib.rs:1903	TxBuilder::build_kamino_bundle	Flash loan fee hardcoded to 0: In build_kamino_bundle, calls assemble_atomic_bundle with flash_loan_fee_sf = 0. If callers don't adjust, repay amount will omit protocol fees, causing Kamino on-chain repay failure (InsufficientRepayAmount).
8	P1 - Critical	crates/arb-sim/src/litesvm_sim.rs:45-59	simulate_locally	LiteSVM uninitialized sandbox: Creates fresh LiteSVM::new() without loading DEX BPF programs or account state. Any DEX transaction simulation fails immediately with ProgramAccountNotFound.
9	P1 - Major	crates/arb-tx/src/lib.rs:1441	build_swap_hop_instruction (DexProtocol::PumpSwap)	PumpSwap completely unsupported: Returns TxError::UnsupportedProtocol. No transaction builder exists.
10	P2 - Moderate	crates/arb-tx/src/pump_fun_raydium.rs:364-371	PumpFunRaydiumArbitrageBuilder::build_and_submit	Stub implementation: build_and_submit validates bundle locally and returns Ok(self.payer) without ever dispatching the transaction to Jito or RPC.
Summary Conclusion
DEX On-Chain Builders: Only Pump.fun and canonical SOL-USDC pools have functional native on-chain builders. For all arbitrary pairs across Raydium V4, Orca Whirlpool, Meteora DLMM, and Raydium CLMM, native account resolution is broken or uses hardcoded values, making Jupiter v6 Swap API the only functional path for non-canonical pools. PumpSwap has no builder at all.
Kamino & ALTs: Kamino 12-account matrix and Q64.64 math are mathematically sound. ALT creation, extension chunking (30 addrs/chunk), and v0 message compilation are implemented to Solana spec.
Simulation, Risk & Landing: Risk management (RiskGate, CircuitBreaker, KillSwitch, Chronos EV gate) and landing (TipOracle, AdaptiveBidder, JitoLander, JitoService) are comprehensively engineered with multi-region failover. However, litesvm_sim is non-functional due to uninitialized VM state, requiring live RPC simulateTransaction.
Immediate Action Required: Fix PreWarmedTxTemplate to patch hop swap amounts, convert PumpFunRaydiumArbitrageBuilder to v0 versioned transactions with ALTs, and ingest account data from arb-ingest into build_swap_hop_instruction instead of relying on SOL-USDC constants.