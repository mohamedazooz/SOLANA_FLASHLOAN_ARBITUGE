# F2.2 — PumpSwap quote parity research (2026-11-07)

## Primary references checked

1. Pump official public documentation — PumpSwap state, fee tiers, creator/holder flags, effective quote reserve, and instruction behavior:
   - https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_SWAP_README.md
   - https://pump.fun/docs/fees
2. Pump official public documentation — Fee program, canonical pool definition and market-cap formula:
   - https://github.com/pump-fun/pump-public-docs/blob/main/docs/FEE_PROGRAM_README.md
3. Pump Rust client reference, `pump_rust_client 0.5.0`, `math::amm`:
   - https://docs.rs/pump-rust-client/latest/pump_rust_client/math/amm/index.html
   - https://docs.rs/pump-rust-client/latest/src/pump_rust_client/math/amm.rs.html
4. PumpSwap v2 trade semantics and retained fee buckets:
   - https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/PUMP_SWAP_TRADE_V2.md
   - https://github.com/pump-fun/pump-public-docs/blob/main/docs/VIRTUAL_QUOTE_RESERVES_FEE_ADJUSTMENT.md
5. Pump synthetic migration rules:
   - https://github.com/pump-fun/pump-public-docs/blob/main/docs/SYNTHETIC_MIGRATION.md
6. Yellowstone stream subscription/replay behavior:
   - https://docs.corvus-labs.io/solana-rpc/grpc/usage
   - upstream protocol reference: https://github.com/rpcpool/yellowstone-grpc

## Verified protocol rules applied in F2.2

- Both Pool vaults are checked against the canonical associated-token addresses derived from the pool authority, mint, and actual token program.
- A PumpSwap Pool account is PDA-derived from `["pool", index, creator, baseMint, quoteMint]`; the adapter verifies both the key and stored bump.
- Canonical PumpSwap pool detection uses `pool.creator == PDA(pump, ["pool-authority", base_mint])`; `coin_creator` is a separate fee-recipient identity.
- Pool price uses `quote_vault_amount + signed virtual_quote_reserves`.
- Sell payout liquidity uses `quote_vault_amount - protocol_fees - creator_fees`.
- `disable_flags` bit 3 disables buys; bit 4 disables sells. These are directional controls, not a single all-or-nothing active flag.
- PumpSwap exact-in buy computes the net amount and fee components using integer arithmetic, then prices `net - 1` against the effective reserves.
- PumpSwap sell computes the fee-free constant-product quote, charges LP/protocol/creator fees on that gross quote, and requires real quote reserves to cover `gross - LP fee`.
- Mayhem pools can trade through PumpSwap v2. Holder-reward pools retain ordinary trading math; the creator fee component is routed to holders. Cashback pools are not supported by PumpSwap v2 instructions, but this is an instruction-builder compatibility issue, not a different constant-product curve. The current project has no PumpSwap transaction builder yet, so no such pool is eligible for live execution.
- Dynamic fee selection is based on the FeeConfig ladders and canonical-pool classification. The Pump fee schedule is mutable; static fee tables are not an acceptable source of truth.
- Creator-fee override is gated by GlobalConfig and applies only to eligible custom quote pairs; SOL/USDC use their schedule tiers.

## Important boundaries

- The Rust client is a useful reference implementation for quote math; it is not a substitute for checking the deployed program's IDL and live account state.
- The current local adapter rejects Token-2022 mints with unsupported extensions, including transfer-fee behavior. It does not claim to support arbitrary Token-2022 fee extensions.
- Fee and GlobalConfig snapshots are required to be within the configured slot freshness window before promotion.
- The dedicated `PumpSwapPool::quote_exact_in` adapter is not yet connected to the generic `arb_engine::quote_hop` route evaluator. Generic route evaluation must reject PumpSwap rather than silently apply generic CPMM math.
- The quote adapter does not create PumpSwap V1/V2 instructions, select fee recipients, build creator-vault/volume-accumulator accounts, or prove transaction account ordering.
- Synthetic migration is not implemented in this increment. A curve-crossing buy remains non-executable until the dedicated v3 curve + post-completion pool quote and builder are completed.
- Yellowstone processed updates may be forked out. Full fork-aware rollback/replay and transaction-level state reconciliation remain a separate acceptance gate.

## Why this increment remains Shadow-only

No Rust compiler, Cargo, rustfmt, Docker, local validator, or live RPC credentials were available in the execution environment. The changes below were statically inspected and packaged, but compilation, differential replay against pump-rust-client, local-validator execution, and mainnet simulation were not performed.
