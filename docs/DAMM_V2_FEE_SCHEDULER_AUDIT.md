# Meteora DAMM v2 fee scheduler audit

## Scope and reference

Implementation is checked against the Meteora damm-v2 program source at upstream commit:

8ea1d74d92ea843f0348d6875654ee7fbe765dc5

Relevant source files:

- programs/cp-amm/src/state/fee.rs
- programs/cp-amm/src/base_fee/fee_time_scheduler.rs
- programs/cp-amm/src/base_fee/fee_market_cap_scheduler.rs
- programs/cp-amm/src/math/fee_math.rs
- programs/cp-amm/src/state/pool.rs
- programs/cp-amm/src/instructions/swap/ix_swap.rs

## Base fee modes

The BaseFeeMode byte is not a binary linear/exponential switch:

| Value | Mode | Period input |
|---:|---|---|
| 0 | Time scheduler, linear | (current_point - activation_point) / period_frequency |
| 1 | Time scheduler, exponential | Same time/slot period |
| 2 | Deprecated rate limiter | Flat cliff fee for existing pools |
| 3 | Market-cap scheduler, linear | Increase in sqrt price relative to initial sqrt price |
| 4 | Market-cap scheduler, exponential | Same sqrt-price step count |

Unknown values must fail closed.

The fee denominator is 1,000,000,000. Time and market-cap schedulers use the terminal scheduled fee before activation. Market-cap scheduling also returns the terminal fee after the scheduler expiry point. period_frequency == 0 is a valid flat-fee time-scheduler configuration only when the other time-scheduler fields are all zero.

## Exponential arithmetic

Meteora computes:

cliff_fee_numerator * (1 - reduction_factor / 10,000)^passed_period

using Q64.64 fixed-point exponentiation by squaring and floor shifts after each multiplication. A floating-point powf or decimal approximation is not a safe replacement. The local implementation uses checked integer arithmetic and returns None on invalid factors or overflow.

## Dynamic fee

The current pool's stored dynamic-fee state contributes:

ceil((volatility_accumulator * bin_step)^2 * variable_fee_control / 100,000,000,000)

The total trading-fee numerator is base fee plus dynamic fee, capped by pool fee version:

- fee version 0: 500,000,000 (50%)
- fee version 1: 990,000,000 (99%)

The dynamic-fee accumulator is already stored in the pool account. The quote path must use the same state snapshot as the pool price; it must not independently recompute or add the variable fee twice.

## Fee allocation and fee placement

The trading fee is charged once. The protocol, referral, compounding, and LP-claiming values are allocations of that fee, not additional fees to subtract from swap output.

- Protocol allocation: floor(trading_fee * protocol_fee_percent / 100)
- Referral allocation: floor(protocol_allocation * referral_fee_percent / 100), only when a referral is present
- Compounding allocation: floor(post_protocol_fee * compounding_fee_bps / 10,000)
- LP claiming allocation: remaining post-protocol amount

The collect-fee mode controls whether the fee is removed from input or output:

- BothToken: fee on output for either direction.
- OnlyB: fee on output for A-to-B; fee on input for B-to-A.
- Compounding: same directional placement as OnlyB.

The allocation breakdown must sum exactly to the one trading-fee amount.

## Pool account offsets used by the parser

Offsets below include the 8-byte Anchor discriminator:

- 8..40: BaseFeeInfo
- 48: protocol fee percent
- 50: referral fee percent
- 54..56: compounding fee bps
- 56..152: dynamic-fee state
- 64..68: max volatility accumulator
- 68..72: variable fee control
- 72..74: bin step
- 120..136: volatility accumulator
- 152..168: initial sqrt price
- 168..200: token A mint
- 200..232: token B mint
- 232..264: token A vault
- 264..296: token B vault
- 296..328: whitelisted vault (not a config account)
- 360..376: liquidity
- 424..440: min sqrt price
- 440..456: max sqrt price
- 456..472: current sqrt price
- 472..480: activation point
- 480: activation type (slot or timestamp)
- 481: pool status
- 484: collect-fee mode
- 486: fee version
- 696: layout version

The current point must be supplied from a clock source matching activation_type. Local wall-clock time is not a valid substitute for a timestamp-activated pool. A mismatched clock type invalidates the fee snapshot.

## Swap account graph correction

The official SwapCtx does not include a config account. The pool account itself contains the vault references. The whitelisted_vault at pool offset 296..328 is not a config and is not a swap account. The local PoolExecutionAccounts::MeteoraDammV2 now contains only the required authority, vaults, and event authority; hydration rejects undersized pool data.

## Chronos is separate

Chronos in arb-math and arb-risk is the route-level ranking and risk/EV layer: it includes negative-log edge weighting with lambda multiplied by measured latency, plus a fixed-point survival-probability gate. It is not Meteora DAMM v2 BaseFeeMode and does not calculate the pool trading fee. Keep Chronos; repair the DEX scheduler independently.

## Execution status / remaining gate

DAMM v2 remains non-executable in the quote engine until the full quote path is proven against upstream behavior, including:
- concentrated-liquidity versus compounding-pool math and layout-version reserve semantics;
- exact token A/B direction from the decoded pool mints (not inferred by sorting a hop's mint keys);
- input/output fee placement and fee allocation;
- Token-2022 transfer-fee epoch state and any transfer-hook requirements;
- min-out and integer rounding parity.

Passing fee-scheduler tests does not authorize live execution.
