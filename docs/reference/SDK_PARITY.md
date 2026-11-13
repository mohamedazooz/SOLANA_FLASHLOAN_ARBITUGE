# Official SDK parity references

This note records the upstream terminology and math references used to align
the Rust adapters. It does not claim that any adapter is accepted for live
execution. SDKs are references or test-only dependencies; upstream code is not
vendored into the runtime.

## Solana dependency selection

Do not add `solana-sdk = "1.18"`, `solana-client = "1.18"`, or
`anchor-lang = "0.29.0"` solely for quote parity. The quote math needs no RPC
client or Anchor runtime, and those versions predate this workspace's
`solana-pubkey = "2.2"` dependency. Adding them would introduce an unnecessary
second Solana API generation. Use a protocol's math-only official crate as a
dev dependency when one is independently consumable; otherwise pin and document
the upstream source used to generate differential fixtures.

## Supported adapter terminology

| Protocol | Upstream reference | Upstream terminology | Local mapping | Parity status |
|---|---|---|---|---|
| Orca Whirlpools | [`orca_whirlpools_core` 2.1.1](https://github.com/orca-so/whirlpools/tree/cc683ec92acb5e7606caa3d8e514d9383e9c7c11/rust-sdk/core) | `tick_index_to_sqrt_price`, `sqrt_price`, `token_a`, `token_b`, `a_to_b`, `trade_fee` | `tick_index_to_sqrt_price_x64`, `sqrt_price_x64`, `token_mint_a`, `token_mint_b`, `fee_paid` | Tick-to-sqrt-price is bounded against the official implementation in `arb-math` integration tests. Swap-quote parity remains open. |
| Raydium AMM v4 | [`raydium-amm`](https://github.com/raydium-io/raydium-amm/tree/91739d8d9f5cf347e0ce14ebe4d2bb7069fb31b6/program) | `amount_in`, `minimum_amount_out`, `swap_base_input` | `amount_in`, `quote`, `base_reserve`, `quote_reserve` | Source reference only; quote/account parity is not accepted. The upstream program crate pins `solana-program` 3.0 and is not added as a dependency. |
| Meteora DLMM | [`dlmm-sdk`](https://github.com/MeteoraAg/dlmm-sdk/tree/1532e540db41ac405dbf4c5bb2ddfceaa1f98ff1/commons) | `quote_exact_in`, `swap_for_y`, `amount_out`, `fee`, `protocol_fee` | `quote_dlmm_multi_bin`, `amount_out`, `fee_amount`, `protocol_fee` | Source reference only. The upstream quote crate is an unpublished workspace crate, so no standalone Cargo dependency is declared. |
| Pump.fun bonding curve | [`pump-public-docs`](https://github.com/pump-fun/pump-public-docs) and [`pump-rust-client`](https://crates.io/crates/pump-rust-client) | `virtual_token_reserves`, `virtual_sol_reserves`, `real_token_reserves`, `buy`, `sell`, fee basis points | `virtual_token_reserves`, `virtual_sol_reserves`, `real_token_reserves`, `quote_with_fee_bps` | Terminology and formula reference only; live transaction building remains disabled. |
| PumpSwap | [`pump-public-docs`](https://github.com/pump-fun/pump-public-docs) | pool vault reserves, `virtual_quote_reserves`, protocol/creator fee components | `effective_quote_reserve`, `PumpSwapFeeConfig` | Source reference only; live transaction building remains disabled. |

The official Orca core crate is a `dev-dependency` only. Its tick-price
comparison covers both tick bounds, adjacent boundary ticks, representative
negative, zero, and positive ticks, plus a fixed grid across the supported tick
range, allowing at most 16 Q64.64 units of rounding difference. The local
shared CLMM conversion uses the Uniswap-style Q128.128 reciprocal path; Orca's
official core uses direct Q64.64 constants for negative ticks and a separate
Q32.64 path for positive ticks, so the two implementations can differ slightly
at some ticks. This is a bounded numerical comparison, not bit-for-bit parity.
Any quote-level parity claim still requires coherent upstream pool/tick-array
inputs and exact fee/rounding comparisons.

## Out of current adapter scope

Jupiter and Phoenix do not have adapters in this workspace's current DEX quote
surface. Jito is an execution/landing component, not a quote-math reference.
They are therefore not installed or represented as supported protocols by this
parity pass. Adding them requires a separate protocol-scope decision and their
own differential, dependency, and execution acceptance evidence.
