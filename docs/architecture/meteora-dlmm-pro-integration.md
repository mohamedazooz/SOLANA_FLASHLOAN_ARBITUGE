# Archived research: Meteora DLMM Pro (not in active runtime)

## Status

**Retired from the active codebase.** The `DexProtocol::MeteoraDlmmPro` enum variant, quote adapter, decoder routing, account hydration hooks, CU estimates and transaction dispatch references have been removed. This document is retained only as research context; it does not describe a supported or planned runtime path. Do not add the protocol back without a separate, explicit implementation decision and complete official-SDK parity. The previous local `arb-math::dlmm_pro` approximation has been removed because it used fabricated 1:1 limit-order fills, a synthetic tick walk, saturating liquidity changes, fixed six-decimal assumptions and `active_liquidity` as both token reserves. None of those values is a valid substitute for a protocol snapshot.

## Authoritative implementation

Use the official Meteora repository:

- Repository: https://github.com/MeteoraAg/dlmm-pro-sdk
- Rust crate: `dlmm-pro-commons`
- Current documented commons release: `0.1.1` (not published on crates.io)
- Pin: `6c8ab97c3451f6801bce623a887e9add80b78d4e`
- Program ID: `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n`

The official changelog says to pin the Rust commons to that commit. Do not use a floating `main` dependency. The SDK repository is MIT licensed.

## Required account snapshot

The quote must be built from one coherent slot/context and include:

1. `DynamicPool::deserialize` for the Pool header and extension-gated dynamic tail.
2. `DynamicTickArray::deserialize` for each tick array, using the owning pool to choose the correct tick layout.
3. `TickArrayBitmapExtension` when the pool reports that the extension is initialized.
4. `DynamicOracle::deserialize` and the clock/slot data required by the quote.
5. Both raw mint accounts, including Token-2022 extensions and transfer-hook metadata.
6. Tick arrays in the exact traversal order returned by `tick_arrays_for_swap`.
7. Any limit-order state/accounts and referral context required by the swap.

Do not use generated Anchor `AccountDeserialize` on dynamic-tail accounts. The official docs explicitly direct integrators to the `Dynamic*` parsers.

## Quote contract

Use official `quote_exact_in(QuoteExactInParams { ... })`. It models dynamic fees, MM liquidity, limit-order matching, collect-fee mode, protocol/host/partner fees and Token-2022 transfer fees. Reject the route if `amount_left != 0`; the on-chain swap requires the full input to be consumed. Use `transfer_fee_excluded_out_amount` as the user's minimum-output basis and preserve the separate input/output transfer-fee values in the per-hop ledger.

The official Rust commons currently depends on Anchor 1.0.2 / Anchor SPL 1.0.2 and newer Solana/SPL crates than the core workspace. Before adding it to `arb-dex`, verify the dependency graph and define explicit Pubkey/Clock/account-data bridges. Do not silently coerce SDK types or duplicate protocol math to avoid this compatibility work.

## Acceptance gates before enabling quotes

- Pin the exact official commit and record it in `Cargo.lock`.
- Decode all required accounts with the official dynamic parsers and verify owner, discriminator, PDA, mint and slot consistency.
- Fetch mint account data and preserve token program, decimals, transfer-fee and transfer-hook semantics.
- Run differential quote tests against the official TypeScript SDK and official Rust commons fixtures for both directions, all collect-fee modes, dynamic-fee states, partial liquidity, limit orders and Token-2022.
- Compare exact output, each fee component, remaining input and next tick/liquidity state; do not use tolerances that hide atom-level rounding differences.
- Build the actual transaction and verify account order, remaining accounts, compute budget and serialized packet size.
- Keep `AmmPool::quote` and execution-account discovery fail-closed until every gate passes.
