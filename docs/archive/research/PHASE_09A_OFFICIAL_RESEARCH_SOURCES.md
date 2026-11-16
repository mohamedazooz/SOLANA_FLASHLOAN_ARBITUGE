# Phase 09A research sources

Research checked on 2026-11-07. Prefer deployed program source and official SDKs as the executable specification; docs are explanatory and may lag code.

## Raydium CLMM

- Raydium CLMM math (sqrt price, token deltas, single-tick and multi-tick swap): https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/math.mdx
- Raydium CLMM swap implementation: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/libraries/swap_math.rs
- Raydium CLMM swap account requirements and tick-array bitmap extension: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs
- Raydium CLMM V2 Token-2022 transfer-fee handling and remaining accounts: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap_v2.rs
- Raydium CLMM instruction docs: https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/instructions.mdx

## Meteora DLMM

- Meteora DLMM SDK reference: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/typescript-sdk/reference.mdx
- Meteora DLMM SDK repository: https://github.com/MeteoraAg/dlmm-sdk
- Meteora TypeScript SDK README: https://github.com/meteoraag/dlmm-sdk/blob/main/ts-client/README.md
- Python client bin-array and quote methods: https://github.com/MeteoraAg/dlmm-sdk/blob/main/python-client/dlmm/dlmm/dlmm.py

## Solana Token-2022

- Transfer-fee extension and withheld-fee accounting: https://solana.com/docs/tokens/extensions/transfer-fees
- Token program vs Token-2022 program and ATA derivation: https://solana.com/docs/payments/how-payments-work

## Engineering interpretation

- Raydium's on-chain swap code is the authority for fee direction, rounding, dynamic fees, tick traversal and required accounts.
- Meteora SDK's exact-in/exact-out quote result includes consumed input, output, fee, protocol fee, price impact and the BinArray public keys required for execution. Compare every field, not only output amount.
- Token-2022 transfer fees are distinct from pool swap fees; account-level withheld fee state must not be silently omitted from realized PnL.
- Never treat a UI/API price or an SDK quote from a different slot as an authoritative execution quote.
