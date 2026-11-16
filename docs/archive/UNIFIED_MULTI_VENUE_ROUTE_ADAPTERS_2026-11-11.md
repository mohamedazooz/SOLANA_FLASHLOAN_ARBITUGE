# Unified Multi-Venue Route Graph and Protocol Adapter Integration

**Date:** 2026-11-11  
**Workspace:** `work_22new_research_integrated`  
**Execution mode:** shadow / live disabled  
**Scope:** route discovery, per-protocol quote dispatch, mint continuity, and current protocol/version research.

## 1. Integration result

The routing layer now has two explicit entry paths:

1. `UnifiedRouteGraph`: route discovery over normalized `PoolState` snapshots, followed by the strict `protocol_quote_adapter()` registry.
2. `AdapterRouteGraph`: route discovery over concrete `arb_dex::AmmPool` objects. It calls each decoded pool adapter's own `quote()` method, rather than converting every venue to a generic CPMM/CLMM formula.

Both graph paths enumerate simple cycles from an anchor mint. A candidate may use each pool once, has bounded depth (2–8 hops), and is rejected if the route does not close back to the starting mint. `quote_route()` now enforces mint continuity at every hop and rejects pool reuse in a single route. This prevents the prior class of false routes where each hop independently matched a pool but the output mint of hop N did not equal the input mint of hop N+1.

## 2. Protocol adapter matrix

| Protocol | Dedicated quote implementation | Route status in current code | Important gate |
|---|---|---|---|
| Raydium AMM V4 | `RaydiumV4Pool::quote` / `RaydiumV4QuoteAdapter` | Quote supported | Uses native fee numerator/denominator with ceiling; requires active status and effective reserves |
| Raydium CPMM | `RaydiumCpmmPool::quote` / `RaydiumCpmmQuoteAdapter` | Quote supported | Reads current pool/config fee and swap-disable status; transfer-fee mints rejected by route graph until exact mint schedule is available |
| Raydium CLMM | `RaydiumClmmPool::quote` / `RaydiumClmmQuoteAdapter` | Quote supported within math domain | Tick-array coverage required; fee rates outside the current `u16` math domain are rejected, never clamped |
| Orca Whirlpool | `OrcaWhirlpool::quote` / `OrcaWhirlpoolQuoteAdapter` | Quote supported | Coherent tick arrays and nonzero fee required |
| Meteora DLMM | `MeteoraDlmmPool::quote` / `MeteoraDlmmQuoteAdapter` | Spot function / supported collect-fee modes only | On-chain status, function type, collect-fee mode, bin arrays, volatility reference and fee params must be present |
| Meteora DAMM v2 | `MeteoraDammV2Pool::quote` / quote-only registry entry for normalized `PoolState` | **Fail-closed until complete fee context** | Current account join does not yet prove dynamic fee config/current point; Compounding mode requires layout-version reserve balances and must not use concentrated-liquidity math |
| Raydium LaunchLab | `RaydiumLaunchLabPool::quote` / quote-only registry entry for normalized `PoolState` | **Fail-closed until PlatformConfig fee join** | Removed fabricated 100 bps default; no quote until authoritative fee config is verified |
| Pump.fun | `PumpFunBondingCurve::quote` / `PumpFunQuoteAdapter` | Quote supported for modeled curve state | Current normalized adapter is restricted to the supported WSOL-quoted model; non-WSOL quote-mint/depth-2 routes need the new Pump SDK account graph |
| PumpSwap | `PumpSwapPool::quote_exact_in` / quote-only registry entry for normalized `PoolState` | Quote supported through concrete `AmmPool` adapter | Requires coherent pool header + both vault balances + live Pump fee config; creator/protocol/LP fee buckets are part of quote state |

A quote-capable route is not automatically executable. `AdapterRouteQuote::all_hops_execution_ready` is true only if every pool exposes resolved execution accounts. The transaction builder must still validate instruction-specific metas, account ownership, writable/signer flags, and current dependency versions.

## 3. Venue-specific model requirements

### Meteora DLMM

- Use bin traversal, not constant-product approximation.
- Preserve `LbPair.status`, `function_type`, `collect_fee_mode`, `index_reference`, volatility accumulator/reference, and last update timestamp.
- The normalized `DlmmState` now carries the decoded status byte; the quote path no longer fabricates `status = 0` or substitutes `active_bin_id` for `index_reference`.
- Unknown function types and unsupported fee modes fail closed.
- Current Token-2022 handling is deliberately rejected when transfer-fee/risky flags are present. A single BPS value is not an adequate substitute for the active epoch schedule, maximum fee, and transfer-hook account list.
- Bin-array decoding currently incorporates limit-order liquidity into effective bin balances. Keep differential tests against the current Meteora quote SDK because fee attribution and limit-order behavior changed in recent SDK releases.

### Meteora DAMM v2

- This is not one universal curve. Concentrated liquidity uses Q64.64 price/liquidity bounds; `CollectFeeMode::Compounding` is constant-product and must use layout-version reserve balances (`token_a_amount`, `token_b_amount`) rather than deriving reserves from `sqrt_price_x64`/`liquidity`.
- Current code blocks compounding mode and blocks quotes until the full fee context and current point are joined. This is intentional; a stale/partial fee schedule can make an apparently profitable route unprofitable on-chain.
- RateLimiter is deprecated for new configs/pools; existing pools still need their legacy behavior modeled accurately.

### Pump.fun / PumpSwap

- Current public docs list Pump SDK 4.1.0, PumpSwap SDK 2.1.0, and `pump-rust-client` 0.4.1.
- Bonding-curve trades v3 and PumpSwap trades v2 retain protocol/creator fee buckets in the curve/pool. Quote math must include these fees even though they are swept later.
- `multi_hop_swap` is a Pump-native instruction for chains of Pump bonding curves/PumpSwap pools. It is not a generic cross-DEX router: all hops must be Pump pools/curves, directions must be consistent, and the instruction applies one final slippage check. For a mixed Raydium/Meteora/Orca/Pump route, compose venue-specific instructions in one outer Solana transaction instead.
- The Pump-native route supports no intermediate user token account for the middle Pump coin. Generic cross-DEX routing still needs explicit token-account and balance-flow planning.

### Raydium

- CPMM pool status is a bitmask; bit 2 disables swaps. Do not treat any nonzero status as universally disabled.
- Current CPMM instructions/docs include creator-fee configuration; pool/config fee joins must be kept current.
- Raydium CLMM requires initialized tick arrays and exact fee-rate domain. The adapter no longer clamps an out-of-domain `u32` fee rate to `u16::MAX`, which would have changed the quoted fee silently.
- AMM V4 uses its native numerator/denominator and is now quoted with ceiling fee arithmetic instead of truncating the fee to integer BPS first.

### Orca Whirlpool

- Keep token ordering/direction tied to the pool's actual mint A/B fields.
- Tick-array coverage must span the simulated path. Missing arrays are a quote rejection, not a reason to synthesize liquidity or fall back to CPMM.
- Use the Orca Rust client/core math as the differential oracle for tick traversal, fee rounding, and final sqrt-price.

## 4. Unified route contract

For a route with pool sequence `P0..Pn` and anchor mint `M0`:

- `mint_in(P0) == M0`
- `mint_out(Pi) == mint_in(Pi+1)` for every adjacent hop
- `mint_out(Pn) == M0` for an arbitrage cycle
- Each pool appears at most once in the simple-cycle planner
- Every hop is quoted by the adapter for its exact `DexProtocol`
- A missing state field, fee schedule, tick/bin array, transfer-fee context, or account dependency rejects the route

The atomic execution shape remains:

`Kamino Flash Borrow → venue-specific swap instruction(s) → Kamino Flash Repay`

All borrow, swaps, and repay must be inside the same Solana transaction. Jito bundle atomicity is not a substitute for the lender's same-transaction borrow/repay introspection.

## 5. Solana transaction formats and transport policy

As of the research date, Solana mainnet supports Legacy, V0, and V1:

| Format | Packet limit | ALT support | Resource configuration |
|---|---:|---|---|
| Legacy | 1,232 bytes | No | Legacy defaults / Compute Budget instructions |
| V0 | 1,232 bytes | Yes | Legacy defaults / Compute Budget instructions |
| V1 | 4,096 bytes | No | Resource limits are encoded in the message |

V1 is active on mainnet. It is not correct to keep a universal 1,232-byte rejection for every transport. It is equally incorrect to assume that every wallet, RPC, block engine ingress, relayer, and validator path accepts V1 just because the protocol format is active.

Compiler policy:

1. Prefer V0 + ALT where it is supported and reduces account-key bytes.
2. Consider V1 only when the actual serialized signed transaction exceeds 1,232 bytes, is at most 4,096 bytes, has supported inline account count, has explicit CU and loaded-account-data limits, and the complete send/receive path has V1 capability enabled.
3. V1 does not use ALTs. Do not try to attach a V0 lookup table to a V1 message.
4. Keep transport capability separate: direct RPC/SWQoS, Jito `/transactions`, and Jito bundle ingress each need their own proven capability flag and integration test.
5. Jito bundle ingress remains conservatively capped at 1,232 bytes in this workspace until an actual signed V1 payload is accepted and landed through that ingress. Do not infer V1 support from validator release numbers alone.
6. All transaction readers/subscribers must opt in to `maxSupportedTransactionVersion: 1` before consuming V1 transactions.

## 6. New source research used

- Solana Versioned Transactions: https://solana.com/docs/core/transactions/versioned-transactions
- Solana Larger Transaction Sizes / V1 activation: https://solana.com/upgrades/larger-transaction-sizes
- Meteora DLMM SDK changelog: https://github.com/MeteoraAg/dlmm-sdk/blob/main/CHANGELOG.md
- Meteora DAMM v2 changelog: https://github.com/MeteoraAg/damm-v2/blob/main/CHANGELOG.md
- Meteora DAMM v2 SDK changelog: https://github.com/MeteoraAg/docs/blob/main/developer-guides/damm-v2/changelog.mdx
- Raydium CPMM instructions: https://github.com/raydium-io/raydium-docs-v1/blob/main/products/cpmm/instructions.mdx
- Raydium CLMM instructions: https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/instructions.mdx
- Raydium SDK integration quick-start: https://github.com/raydium-io/raydium-docs-v1/blob/main/quick-start/index.mdx
- Pump public docs / SDK versions: https://github.com/pump-fun/pump-public-docs
- PumpSwap multi-hop specification: https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/MULTI_HOP_SWAP.md
- PumpSwap v2 trade specification: https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/PUMP_SWAP_TRADE_V2.md
- Orca Whirlpools releases: https://github.com/orca-so/whirlpools/releases

## 7. Verification record and remaining blockers

Implemented in this pass:

- Explicit per-protocol adapter registry for all nine `DexProtocol` variants in the normalized route engine.
- Concrete `AdapterRouteGraph` that routes over `arb_dex::AmmPool` implementations and calls each protocol's own quote implementation.
- Strict mint continuity, cycle closure, no pool reuse, and bounded cycle enumeration.
- `DlmmState.status` persistence and removal of fabricated DLMM status/index-reference values in the route quote path.
- Raydium V4 native fee numerator/denominator ceiling quote.
- Raydium CLMM out-of-domain fee rate rejection instead of silent clamp.
- Meteora DAMM v2 compounding and incomplete fee-context rejection.
- Raydium LaunchLab fabricated default fee removal; it stays disabled until the PlatformConfig fee join is implemented.
- Token-2022 transfer-fee routes rejected unless complete epoch/max-fee/hook context is available.
- Regression tests added for mint discontinuity, adapter registration, and two-pool concrete-adapter route quoting.

Not verified in this environment:

- `cargo fmt`, `cargo check`, `cargo clippy`, unit/integration tests, fuzzing, LiteSVM, bankrun, or mainnet differential tests. The current environment does not have `cargo`, `rustc`, or `rustfmt` installed.
- Full supervisor integration of `AdapterRouteGraph` into every live/shadow opportunity path.
- Atomic transaction builders for every cross-venue pair. Route discovery/quote support is not equivalent to builder acceptance.
- Complete dynamic-fee/current-point account joins for DAMM v2 and PlatformConfig fee join for LaunchLab.
- V1 landed transaction proof for each transport, including Jito bundle ingress.

**Safety status:** Shadow only. No live trading was enabled.
