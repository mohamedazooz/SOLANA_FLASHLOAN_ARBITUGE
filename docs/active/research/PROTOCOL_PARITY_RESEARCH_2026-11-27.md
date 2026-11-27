# Protocol Parity Research — 2026-11-27

## Scope and method

Reviewed official program repositories, SDK/client code, and protocol documentation for Raydium CLMM, Solana Token-2022 TransferHook, Meteora DLMM, Orca Whirlpools, Kamino KLend, and Jito transaction submission. The implementation rule is **source first, fail closed on missing account state, and never infer execution support from a successful quote or builder test**.

## Raydium CLMM

Authoritative sources:

- Program state layouts: https://github.com/raydium-io/raydium-clmm/tree/master/programs/amm/src/states
- AmmConfig: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/config.rs
- PoolState: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/pool.rs
- TickArrayState: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/tick_array.rs
- ObservationState: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/oracle.rs
- Dynamic fee model: https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/math.mdx
- CPI/client account model: https://github.com/raydium-io/raydium-cpi

Verified invariants applied in code:

- PoolState's canonical PDA is derived from the pool seed, AmmConfig, sorted mint pair, optional little-endian seed index and stored bump; the pool address must match those seeds.
- AmmConfig.trade_fee_rate is a native millionths rate, not discovery fee_bps; its tick_spacing must equal PoolState.tick_spacing.
- PoolState is 1,544 bytes in the current official layout. The DynamicFeeInfo structure ends at byte 1,176, but the account continues through padding1 and padding2; a 1,176-byte prefix is not a full account-layout validation. PoolState contains the swap status, fee_on, seed index, tick bitmap and an 80-byte DynamicFeeInfo tail. A short/truncated tail cannot prove dynamic fees are disabled.
- TickArrayState carries its own pool link and signed start_tick_index; the account discriminator, full account length, pool link, expected start, initialized_tick_count, parsed initialized ticks and program owner must all match. The selected start indices must also be set in the PoolState default bitmap.
- Current official TickState uses bytes 116..168 as reserved padding, not limit-order counters. A parser must not reinterpret those bytes as order amounts; initialized state is based on liquidity_gross for this layout.
- Vault PDAs use the pool_vault seed with pool state and mint; each token account must be initialized, use the expected mint, and have the pool state as authority. ObservationState uses the observation seed plus pool state.
- ObservationState has an initialized byte, recent epoch, observation index, then the pool key. Its pool key is not at byte offset 8.
- The default pool bitmap is 1,024 bits with offset 512 (array indices -512…511); out-of-range traversal requires the separate bitmap extension.
- Dynamic fees are updated from volatility state during swap steps. The current quote path deliberately uses a maximum-volatility upper bound, which is conservative but is not exact runtime-fee parity.
- fee_on modes 1/2 are direction-dependent. The direction-free graph state is restricted to FromInput (mode 0).

Implementation status: coherent quote hydration now batches the full 1,544-byte PoolState, AmmConfig, both SPL mints, both vaults, ObservationState, optional bitmap extension and bitmap-selected tick arrays in one RPC context. It validates PoolState/AmmConfig/vault/observation PDAs and canonical bumps, mint decimals and owners, vault mint/authority/initialized state, account owners/discriminators/links, initialized tick counts, bitmap membership and config tick spacing. The selector supports the default bitmap and 14-page positive/negative extension; it bounds the quote to three selected arrays per direction. CLMM remains quote-only: Token-2022, fee-on-output, exact adaptive-fee transition parity, simulation and dispatch remain gated.

## Raydium AMM v4

Authoritative sources:

- AmmInfo and AmmStatus: https://github.com/raydium-io/raydium-amm/blob/master/program/src/state.rs
- Swap processor and vault checks: https://github.com/raydium-io/raydium-amm/blob/master/program/src/processor.rs

Verified invariants applied in code:

- AmmInfo is the fixed 752-byte repr(C, packed) account. Its nonce is a u64 at offset 8 and must equal the canonical AMM authority PDA bump (254 on Mainnet); the fee ratio is at 176/184, accrued PnL at 192/200, open time at 224, coin/pc vaults at 336/368, and coin/pc mints at 400/432.
- Curve reserves are the vault token amounts minus need_take_pnl_coin and need_take_pnl_pc; underflow is a hard rejection. The exact fee numerator/denominator are retained instead of reconstructing quote math from rounded basis points.
- AmmStatus::swap_permission permits Initialized (1), SwapOnly (6), and WaitingTrade (7); WaitingTrade additionally requires Clock.unix_timestamp >= pool_open_time. Disabled, WithdrawOnly, LiquidityOnly, and OrderBookOnly are not quoteable.
- Pool, both vaults and Clock are refreshed together. Vault accounts must be legacy SPL accounts with the expected mint, canonical AMM authority, initialized state, no delegate, no native reserve, zero delegated amount, and no close authority.

Implementation status: Raydium v4 support is preserved. The ingestion path publishes quote state only from a coherent same-slot snapshot with the exact fee ratio and PnL-adjusted reserves. Broader Mainnet SDK differential coverage across both directions and multiple input sizes remains outstanding; execution remains subject to the existing economic and simulation gates.

## Token-2022 TransferHook

Authoritative sources:

- Token-2022 program: https://github.com/solana-program/token-2022
- Client integration guide: https://github.com/solana-foundation/solana-com/blob/main/apps/docs/content/docs/en/tokens/extensions/transfer-hook-integration.mdx
- SPL Token JS helpers: https://github.com/solana-program/token-2022/tree/main/clients/js

Verified invariants:

- A mint with a configured transfer hook requires the hook's extra accounts on every applicable transfer; omitting them causes the transfer to fail.
- ExtraAccountMetaList entries may be fixed public keys, PDAs, PDAs under another program, or PDAs whose seeds read data from prior accounts.
- Data-seeded resolution requires the relevant account data and ordered prior-address context; parsing a static list is not complete hook support.
- The extra-meta flags include signer and writable semantics and must be preserved exactly.

Implementation status: the static-meta parser now preserves signer/writable bits and rejects non-boolean flag bytes. Dynamic seed/PDA resolution, live account fetching, and swap-instruction parity are not implemented. Token-2022 execution remains disabled; epoch-aware TransferFeeConfig is quote-only.

## Meteora DLMM

Authoritative sources:

- Official SDK: https://github.com/MeteoraAg/dlmm-sdk
- Account model: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/program/accounts.mdx

Verified invariants:

- BinArray contains 70 bins; array selection is signed and must use floor semantics for negative IDs.
- The default bitmap covers bin-array indices -512…511; extension pages are required only when the active path leaves that range.
- Dynamic fees depend on the live volatility/reference-bin state and the traversal path; a single static fee is not a substitute.
- The active quote path must preserve empty-bin slots and validate each selected array's pool link, index and owner.

Implementation status: generic graph routing and swap_hop support DLMM. The separate two-pool flash evaluator remains disconnected; do not wire a parallel path until its Jito-tip liability and shared runtime cost inputs are complete.

## Orca Whirlpools

Authoritative sources:

- Program and SDK: https://github.com/orca-so/whirlpools
- Core quote math: https://github.com/orca-so/whirlpools/tree/main/programs/whirlpool-core

Classic Whirlpool pools have frozen Mainnet differential coverage. Adaptive-fee pools need their oracle/volatility inputs and their own differential vectors; a classic static-fee fixture does not prove adaptive-fee parity. Keep adaptive-fee paths gated unless the fee configuration and oracle are in the same coherent snapshot.

## Kamino KLend and Jito

Authoritative sources:

- Kamino lending program: https://github.com/Kamino-Finance/klend
- Jito low-latency transaction and bundle docs: https://docs.jito.wtf/lowlatencytxnsend/

Operational invariants:

- Flash-loan borrow/repay instructions must satisfy the lending program's instruction ordering and amount/mint/account checks; transaction assembly success alone is not proof of repay validity.
- Solana priority fees derive from compute-unit limit and micro-lamports per CU. A Jito tip is a separate transfer/liability and must not be folded into the priority-fee field or omitted from the net-profit floor.
- The active economic gate must use the same live flash-loan fee, base/priority fee and tip inputs that dispatch will use.

## Venues intentionally left gated

Meteora DAMM v2, DBC and Raydium LaunchLab remain disabled until protocol-native state hydration, fee/curve semantics, complete account graphs, exact-in/out support where applicable, differential fixtures and non-broadcast simulation are complete. PumpSwap creator/cashback fee paths remain restricted to modes whose live fee accounts and account semantics are verified.

## Release rule

A protocol is not production-ready merely because it decodes, quotes, builds, or passes unit tests. Enablement requires coherent live account provenance, protocol-native quote parity, sizing-boundary tests, exact instruction/account parity, simulation and a sustained read-only soak. No live transaction submission was performed as part of this research or remediation.
