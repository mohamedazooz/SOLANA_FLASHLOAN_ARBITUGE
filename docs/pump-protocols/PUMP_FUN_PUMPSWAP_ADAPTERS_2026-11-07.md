# Pump.fun + PumpSwap integration — engineering status (2026-11-07)

## Scope

This change introduces a separate `DexProtocol::PumpSwap` identity and a dedicated PumpSwap adapter. Pump.fun bonding-curve pricing is updated from the obsolete 100 bps assumption to the current 125 bps standard schedule. Both protocols remain **shadow/quote-only** in the generic transaction builder until current program instruction account matrices and live fee state are joined and verified.

## Pump.fun

- Standard SOL/USDC bonding-curve total fee is 1.25% (125 bps).
- The curve is a virtual-reserve constant-product market; buy output is clamped by physical `real_token_reserves`; sell output is clamped by physical SOL reserves.
- `complete=true` is terminal for the bonding curve: the pool must not be quoted as active after graduation. Quotes that cross the remaining real token reserve are rejected until the V3 synthetic-migration quote path is implemented; they are not clamped into a fake fill.
- Fees are denominated in the quote asset: buys deduct the fee from quote input; sells deduct it from gross quote output. The 125 bps constant represents the currently published standard curve schedule only, not a universal replacement for the fee program. Custom-pair holder-reward coins can use a creator/holder rate that differs from the standard schedule. Such assets must not be enabled for live execution until their live fee configuration is decoded and included in the quote.
- Legacy Pump.fun instruction construction is not routed through the generic live hop builder. Current V2/V3 instructions have distinct account matrices and fee-retention behavior; no instruction is sent from this adapter.

## PumpSwap

- Added `DexProtocol::PumpSwap`, `PumpSwapPoolHeader`, `PumpSwapFeeConfig`, and `PumpSwapPool`.
- Pool header decoder verifies the Anchor discriminator and parses the stable account prefix plus optional appended fields.
- The two SPL vault balances are deliberately external inputs. A pool account alone is not enough to price a pool.
- Effective quote reserve is `raw quote vault amount + virtual_quote_reserves`, with checked signed arithmetic and rejection of negative/overflow values.
- Fee model uses parts per million (PPM), not integer bps, because official fee schedules can include fractional bps. Runtime fee components must come from decoded on-chain GlobalConfig/FeeConfig; this adapter intentionally does not hardcode a tier schedule.
- Added a strict selector for decoded on-chain FeeConfig tiers. No hardcoded canonical/non-canonical tier table is used for execution.
- Fees are applied in the quote asset: for base→quote sells, deduct from gross quote output; for quote→base buys, deduct from quote input.
- The pool quote adapter rejects zero liquidity, invalid fee totals, and arithmetic failures.
- `update_from_account` intentionally rejects a single-account update. A coherent state update must join the Pool account and both vault accounts at the same slot.

## Critical remaining integration work

1. Yellowstone subscription planner must subscribe to PumpSwap Pool accounts, base/quote vaults, Pump GlobalConfig, Pump fee-config PDAs, mint accounts and relevant Token-2022 extension data.
2. Join pool + vault + fee-config updates by slot/write-version. Reject mixed-slot state and incomplete updates. The fee tier array and GlobalConfig components are the source of truth; public web fee tables are documentation, not runtime constants.
3. Determine canonical vs non-canonical pool using the official PDA rule, and determine SOL/USDC/custom-pair/holder-reward mode from decoded state.
4. Calculate market cap using integer fixed-point arithmetic from live pool price × protocol-defined supply; no `f64` in the execution path. Recompute fee tier on every relevant state update.
5. Add PumpSwap pool discovery by program account scan/log subscription, not DexScreener metadata alone.
6. Implement PumpSwap `buy`/`sell` instruction account builder from the current official IDL; verify account order, signer/writable flags, token program variants, fee recipient selection, slippage and event decoding.
7. Implement current Pump `buy_v2`/`sell_v2` or `buy_v3`/`sell_v3` builders and all required fee/creator/fee-program accounts. Support cashback/holder-reward/custom pair branches only after explicit tests.
8. Add fork-aware state invalidation, replay fixtures from mainnet transactions, quote-vs-simulation differential tests, and transaction metadata PnL reconciliation.
9. Only then enable Pump adapters in `execution_capability`; until then, live hop construction fails closed.

## Tests added in source

- PumpSwap pool account discriminator/layout decoding.
- Effective quote reserve includes virtual quote adjustment.
- Both swap directions quote using quote-asset fee deduction.
- PPM tier schedule preserves fractional bps.
- Invalid fee totals and zero liquidity reject.
- Pump.fun standard curve fee expectation updated to 125 bps.

## Non-claims

This phase does not claim PumpSwap discovery, PumpSwap transaction execution, current Pump instruction execution, mainnet readiness, or compiler/test success. No live transaction path is enabled by this change.
