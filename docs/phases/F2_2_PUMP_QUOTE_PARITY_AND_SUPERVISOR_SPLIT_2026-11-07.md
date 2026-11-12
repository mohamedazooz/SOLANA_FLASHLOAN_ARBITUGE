# F2.2 — PumpSwap quote parity and Supervisor decomposition

## Implemented in source

### PumpSwap adapter

- Added canonical Pool PDA + bump verification from `["pool", index, creator, base_mint, quote_mint]` before any pool can be joined.
- Verified both vault addresses against their canonical ATAs using the actual Token or Token-2022 program ID before promotion.
- Added GlobalConfig disable flags to `PumpSwapPool` state.
- Enforced buy/sell disable bits independently in `quote_exact_in` (bit 3 = buy, bit 4 = sell).
- Changed joined-state `trading_enabled()` to retain a pool when at least one direction is enabled; added `trading_enabled_for(is_buy)` for explicit checks.
- Preserved Mayhem and Holder Rewards pools in the quote path when fee state is verified. Cashback remains subject to instruction-version restrictions at the future transaction-builder boundary.
- Limited per-pool creator-fee override to custom quote pairs; SOL/USDC pairs retain their configured tier schedules.
- Rejected stale FeeConfig and GlobalConfig snapshots outside `max_state_age_slots` before promotion.
- Added tests for directional disable flags and Mayhem/Holder Reward quote availability.
- Moved Pump state-join tests to `pump_state_tests.rs` so the production state-join module remains below 1,000 lines.

### Supervisor structure

The 2,114-line `supervisor.rs` was split into focused source modules without `include!` fragments or duplicated call paths:

- `supervisor.rs` — Supervisor state, construction, and market snapshot reporting.
- `supervisor/run.rs` — lifecycle, slot loop, market snapshot, opportunity discovery and risk/economic prechecks.
- `supervisor/execution.rs` — instruction assembly, route resolution, blockhash and verified input helpers.
- `supervisor/dispatch.rs` — mode dispatch, preflight simulation, Jito/SWQoS send and settlement tracking.

All four files are below 1,000 lines. The `run.rs` method remains a substantial orchestration method but is now below the 1,000-line limit; further extraction should be based on typed cycle context and compile-tested, not line slicing.

## Not implemented in F2.2

- Dedicated PumpSwap quote is not yet wired into `arb_engine::quote_hop`; graph insertion is candidate-only and the exact route gate must continue to reject PumpSwap until that integration is complete.
- PumpSwap V1/V2 instruction builder and account-meta validation.
- Pump.fun V3 and PumpSwap V2 atomic route builders.
- Synthetic migration quote and post-completion pool leg.
- Pump multi-hop swap builder and its asymmetric fee allocation.
- PumpSwap Token-2022 transfer-fee quote support.
- Yellowstone fork rollback, replay gap recovery and account-state generation tracking.
- Differential tests against pump-rust-client and local-validator transaction simulation.

## Acceptance gates

1. `cargo fmt --all --check`
2. `cargo check --workspace --all-targets`
3. `cargo test --workspace`
4. `cargo clippy --workspace --all-targets -- -D warnings`
5. Differential tests for PumpSwap buy/sell exact-in against `pump_rust_client::math::amm` using the same account fixtures.
6. Replay tests with Pool, vault, FeeConfig and GlobalConfig updates across slots and write versions.
7. Local-validator tests for buy/sell disabled flags, Mayhem, Holder Rewards, cashback instruction restrictions and v2 fee buckets.
8. Shadow replay on a recorded mainnet account stream; no live signing until every gate passes.
