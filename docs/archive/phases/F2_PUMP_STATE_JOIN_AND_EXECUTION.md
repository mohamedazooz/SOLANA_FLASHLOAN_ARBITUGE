# F2 — Pump Protocol State Joining and Execution Adapters

**Objective:** Complete Pump.fun + PumpSwap without creating a second pricing, financial, risk, execution, or settlement path. All candidate routes must pass through the same exact-quote engine, signed financial evaluation, risk gate, transaction compiler, Jito/SWQoS transport, and settlement reconciliation.

## F2.1 — Yellowstone account ingestion and PumpSwap shadow promotion

**Status: partially delivered; PumpSwap-only promotion and Kamino input migration are in source. Rust build/test still required.**

- [x] Subscribe to Pump.fun, PumpSwap, Pump Fees, and Kamino program-owned accounts.
- [x] Discover PumpSwap Pool accounts from Yellowstone owner-filtered account updates.
- [x] Dynamically subscribe to each PumpSwap Pool, both vaults, both mints, PumpSwap GlobalConfig, Pump Fees FeeConfig, Kamino WSOL reserve, and WSOL mint.
- [x] Enforce owner/discriminator/PDA checks and Pool/vault same-slot coherence.
- [x] Validate token account mint/authority/state and mint owner/decimals/supply.
- [x] Reject Token-2022 mints with extensions until transfer-fee/hook semantics are implemented.
- [x] Decode PumpSwap FeeConfig BPS tiers, standard/stable ladders, exotic flat fees, and bump.
- [x] Decode PumpSwap GlobalConfig disable flags and `creator_fee_configurable` gate.
- [x] Use `pool.creator` for canonical pool classification; do not substitute `coin_creator`.
- [x] Promote coherent PumpSwap state into the shared cache for candidate discovery only.
- [x] Read Kamino flash-loan fee and WSOL mint decimals from Yellowstone snapshots; no HTTP JSON-RPC fallback in the Supervisor's verified-input path.
- [ ] Pump.fun curve discovery-to-account mapping and promotion.
- [ ] Fork-aware rollback/reconciliation after processed-slot reorgs and stream reconnect gaps.

## F2.2 — Pump.fun state and fee engine

**Status: adapter made dynamic-fee-only; live state join not implemented.**

- [ ] Treat PumpDev/WebSocket feeds as discovery triggers only; derive bonding-curve PDAs and fetch authoritative state from Yellowstone.
- [x] Decode the current 83-byte Pump.fun `BondingCurve` account including creator, mayhem and cashback flags; reject truncated/legacy state without mode flags.
- [ ] Decode Pump.fun mint, Pump `Global`, and Pump Fees `FeeConfig` with versioned account layouts.
- [ ] Derive the Pump.fun FeeConfig PDA with seeds `[b"fee_config", pump_fun_program_id.as_ref()]` under the Pump Fees program. The IDL's `FEE_CONFIG_EXTRA_SEED` bytes are the Pump program ID bytes; do not reuse the PumpSwap program ID seed.
- [ ] Resolve protocol, creator, cashback, and mode-dependent fees from the curve + Global + FeeConfig state at a coherent slot.
- [ ] Compute the correct curve market-cap tier using the current protocol definition and integer arithmetic; never use a fixed 125 BPS fallback.
- [x] Remove static Pump.fun 125 BPS and 6-decimal assumptions from the quote adapter.
- [x] Make Pump.fun quotes require explicit fee BPS, mint decimals, and bonding-curve state; registry discovery skips Pump.fun until those values are joined.
- [ ] Differential-test Pump.fun buy/sell fee rounding and reserve mutations against the current official SDK and simulated transactions.
- [ ] Add explicit support or fail-closed rejection for cashback, mayhem, buyback, custom quote mint, and other curve modes.

## F2.3 — PumpSwap exact quote and route integration

- [x] Decode Pool virtual quote reserve, fee buckets, creator, vaults, and mode flags.
- [x] Use `quote_vault + signed virtual_quote_reserves` for the pricing reserve and `quote_vault - protocol_fees - creator_fees` for real payout liquidity.
- [x] Implement exact-in integer fee rounding from the current Pump Rust-client math for supported untaxed quote mints.
- [x] Reject disabled buy/sell states and unsupported mayhem/cashback/holder-reward pools.
- [ ] Differential-test every quote against official Pump SDK/Rust-client outputs and transaction simulation.
- [ ] Integrate the PumpSwap adapter into the shared route exact-quote stage, not just candidate graph promotion.
- [ ] Add Token-2022 transfer-fee/hook handling or maintain explicit rejection.
- [ ] Add multi-hop and pump-coin quote routing only after all dependency accounts and per-route fee semantics are decoded.

## F2.4 — Migration and transaction builders

- [ ] Implement Pump V2/V3 buy/sell instruction builders from the current official IDL, including fee program, fee recipient, creator vault, event authority, volume accumulator, and version-specific accounts.
- [ ] Implement PumpSwap buy/sell V2 instruction builders and verify account order, signer/writable flags, discriminators, and argument encoding against official fixtures.
- [ ] Implement synthetic migration straddle quote for the final Pump.fun buy that crosses completion; never clamp the amount as an ordinary curve fill.
- [ ] Decode fee charge and sweep/distribution events; never count a sweep as new trading revenue.
- [ ] Build flash borrow + all swaps + repay + on-chain minimum-profit guard atomically in one transaction where lender semantics require it.
- [ ] Keep Jito bundles as transport/inclusion only. Bundle acceptance is not landing confirmation; reconcile bundle status and every signature.
- [ ] Reconcile actual transaction fee, compute units, tips, token deltas, flash-loan repayment, and realized PnL.

## F2.5 — Supervisor and codebase decomposition

- [ ] Split `arb-bot/src/supervisor.rs` into orchestration, market-state, opportunity, risk, execution, and settlement modules. Preserve one public Supervisor API and one call path.
- [ ] Split every source file over 1,000 lines; do not use `include!` as a superficial split. Move cohesive functions and types into modules and update imports/call sites.
- [ ] Consolidate provider health/failover and tracked-account subscriptions into one registry with source ownership and deterministic cleanup.
- [ ] Remove dead code only after call-graph and Cargo-target verification.
- [ ] Add module-level docs, architecture diagram, env reference, state machine, and operational runbook.

## F2.6 — Verification and release gates

- [ ] `cargo fmt --all --check`
- [ ] `cargo check --workspace --all-targets`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] Property/adversarial tests for fee-tier boundaries, integer rounding, reserve underflow/overflow, stale/mixed-slot state, forks, reconnect gaps, disabled pools, and unsupported token extensions.
- [ ] Recorded mainnet account fixtures and quote-vs-SDK differential replay.
- [ ] Shadow replay with quote-vs-simulation error distribution and realized net PnL reconciliation.
- [ ] Live signing and canary remain disabled until every acceptance gate passes and the operator explicitly authorizes them.

## Non-negotiable execution boundary

No fallback to fixed fees, static reserves, stale cache, guessed decimals, or HTTP JSON-RPC market-state reads. Missing account state or unsupported protocol mode means **skip the candidate**. A PumpSwap graph edge or Pump.fun discovery event is not an executable opportunity until exact quote, conservative min-out PnL, risk, transaction simulation, and settlement requirements all pass.
