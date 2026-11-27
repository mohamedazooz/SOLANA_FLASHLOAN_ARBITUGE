# Raydium V4 / CPMM Quote Validation Fix — 2026-11-27

## Scope

This change hardens the existing Raydium V4 hydration and the validation-only SOL/USDC round-trip diagnostics. It does not change the CPMM quote formula, because the current CPMM adapter already follows the official fee arithmetic and is retained as the reference implementation for differential tests.

## Raydium AMM V4 facts verified against the on-chain source

Official sources:

- AMM V4 processor: https://github.com/raydium-io/raydium-amm/blob/master/program/src/processor.rs
- AMM V4 account layout and fee fields: https://github.com/raydium-io/raydium-amm/blob/master/program/src/state.rs
- AMM V4 reserve math: https://github.com/raydium-io/raydium-amm/blob/master/program/src/math.rs

The current 8-account `SwapBaseInV2` path:

1. Checks `AmmStatus::swap_permission()`.
2. For `WaitingTrade`, reads Clock and rejects until `unix_timestamp >= pool_open_time`; once open, the on-chain program promotes the status to `SwapOnly`.
3. Computes curve balances with `calc_total_without_take_pnl_no_orderbook`, i.e. vault amount less `need_take_pnl` on each side.
4. Computes the input swap fee as `ceil(amount_in * swap_fee_numerator / swap_fee_denominator)`, then applies the constant-product curve to the fee-adjusted input.

The V4 `AmmInfo` fields used by the decoder are:

| Field | Offset |
|---|---:|
| status | 0 |
| coin_decimals / pc_decimals | 32 / 40 |
| swap_fee_numerator / denominator | 176 / 184 |
| need_take_pnl_coin / pc | 192 / 200 |
| pool_open_time | 224 |
| coin_vault / pc_vault | 336 / 368 |
| coin_mint / pc_mint | 400 / 432 |

The quote snapshot is now built from one `getMultipleAccounts` context containing AmmInfo, both vaults, and Clock. Vault owners, SPL token-account mint/authority/state, mint owners/decimals, pool owner, and all snapshot slots are checked before publishing a quoteable `PoolState`. V4 reserves are derived from the on-chain coin/pc orientation and subtract accrued PnL with checked arithmetic. The exact fee numerator/denominator is preserved rather than reconstructed from rounded BPS.

The default V2 adapter dependency set now contains only the pool's two vaults; OpenBook market/orderbook accounts remain relevant only to the explicit legacy V1 instruction path. A fresh AmmInfo update invalidates previously observed vault balances, and the standalone adapter validates vault mint, authority, initialization state, delegate, and close-authority fields before using the amount.

## CPMM reference parity

Official sources:

- Fee arithmetic: https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/curve/fees.rs
- Exact-in curve math: https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/curve/calculator.rs
- Swap instruction accounting: https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/instructions/swap_base_input.rs
- Creator-fee direction: https://github.com/raydium-io/raydium-cp-swap/blob/master/programs/cp-swap/src/states/pool.rs

CPMM uses a 1,000,000 fee denominator. Trade and creator input fees use ceiling arithmetic; protocol and fund shares use floor arithmetic. When the creator fee is charged on output, the curve output is calculated first and the creator fee is then rounded up and deducted. The existing local CPMM implementation follows these distinctions, so this patch does not replace it with an approximate BPS formula.

## Validation diagnostic corrections

- Negative gross PnL is now reported as a gross route loss; only positive gross followed by non-positive net is described as execution costs exceeding the gross edge.
- Flash-fee and signed-PnL calculation errors invalidate a probe instead of becoming fabricated maximum costs or `i128::MIN`.
- The analytical two-pool CPMM formula is explicitly a heuristic seed only. The reported size is the best candidate found from protocol-native exact quotes, using a coarse ladder plus four bounded local-refinement scales.
- Price impact is labelled curve impact to distinguish the fee-excluded curve component from transaction costs.

## Tests added

- Full V4 account layout and exact fee-ratio decode.
- Invalid status, decimals, fee denominator, and truncated account rejection.
- WaitingTrade timestamp gating.
- Checked reserve subtraction for `need_take_pnl`.
- Legacy SPL vault mint, authority, initialized state, delegate, and close-authority validation.
- Clock owner and embedded-slot validation.
- Distinct verdicts for gross route loss, costs exceeding gross edge, below-hurdle net, and unavailable Jito tip floor.

## Operational safety

The `validate` binary remains read-only and ends without submitting transactions. V4 states fail closed when any required dependency is absent, stale, malformed, incorrectly owned, incorrectly oriented, or not tradeable.
