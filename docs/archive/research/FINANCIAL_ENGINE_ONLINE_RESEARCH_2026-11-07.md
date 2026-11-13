# Financial Engine Research Register — 2026-11-07

The following official / primary sources were reviewed before defining the next financial-core phase:

1. Pump.fun bonding curve: constant-product virtual-reserve curve, graduation to PumpSwap, 1.25% bonding curve fee overview. https://pump.fun/docs/bonding-curve
2. Pump.fun fee schedule (last updated 17 Oct 2026): tiered PumpSwap canonical fees, non-canonical fees, quote-asset fee collection, creator/protocol/LP fee handling, custom-pair and holder-rewards distinctions. https://pump.fun/docs/fees
3. Pump public docs — bonding-curve V3 trade instructions and synthetic migration semantics. https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/TRADE_V3.md
4. Pump public docs — PumpSwap V2 trade instructions and fee retention semantics. https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/PUMP_SWAP_TRADE_V2.md
5. Pump public docs — fee sweep behavior for V2/V3 trade instructions. https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/SWEEP_FEES.md
6. Solana transaction fees: base fee and priority fee; priority fee uses ceil(CU price × CU limit / 1,000,000). https://solana.com/docs/core/fees
7. Solana confirmation: processed / confirmed / finalized are distinct and processed can be forked out. https://solana.com/developers/cookbook/transactions/confirmation
8. Jito bundle semantics: up to five sequential transactions, same-slot all-or-nothing; sendBundle acknowledgement returns a bundle ID but does not prove landing; query getBundleStatuses. https://docs.jito.wtf/lowlatencytxnsend/
9. Jito protocol JSON-RPC reference: sendBundle and getBundleStatuses. https://github.com/jito-labs/mev-protos/blob/master/json_rpc/http.md
10. Meteora DLMM SDK reference: exact-in/out quote, consumed input, output, fee, protocol fee, end price, BinArray accounts and insufficient-liquidity errors. https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/typescript-sdk/reference.mdx
11. Raydium CLMM math: stepwise swap across initialized ticks and liquidity changes. https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/math.mdx
12. Raydium CLMM instruction accounts: dynamic tick-array coverage and optional bitmap extension requirements. https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/instructions.mdx
13. Marginfi flash-loan guide: borrow/repay bookend instructions, same-transaction atomicity and constraints. This is a protocol-specific reference, not a claim that Kamino has identical instruction semantics. https://docs.marginfi.com/guides/flashloans

## Research conclusions

- Pump's current 1.25% bonding-curve fee makes the code's fixed 1.00% stale for the current published SOL/USDC schedule. PumpSwap cannot share the same flat fee constant: canonical pools are market-cap tiered, and non-canonical pools have a separate schedule.
- Fees are denominated in the pool quote asset. The adapter must preserve this denomination until the central economic engine converts it.
- Pump V3 / PumpSwap V2 can retain protocol and creator fees in curve/pool accounting instead of paying every component out on each trade. Accounting must model charged fees separately from immediate transfers.
- Jito bundle acceptance is not landing. Bundle ID and transaction signature are different identifiers and need separate state machines.
- Solana priority fee estimate uses requested CU limit, whereas actual consumed CU is a separate transaction metadata measurement.
- A lender's flash-loan lifecycle is protocol-specific. The model must enforce the actual lender's borrow/repay constraints, and not assume Jito bundle atomicity lets a loan span multiple transactions.
- Exact CLMM/DLMM/Pump quotes require protocol-specific state traversal. Bellman-Ford or marginal-cycle detection is candidate discovery only.
