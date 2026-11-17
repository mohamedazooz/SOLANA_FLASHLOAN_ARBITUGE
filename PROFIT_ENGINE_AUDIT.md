# PROFIT_ENGINE_AUDIT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Net profit model

`calculate_net_profit_raw(expected_out, size_in, flash_fee, dex_fees=0, transfer_fees=0, account_creation=0, other=0, base_fee, priority_fee, tip, lamports_per_base_raw, lamports_per_base_denom)` → base-mint raw units.

Costs accounted: flash loan fee, base tx fee, priority fee, Jito tip. DEX fees already inside `expected_out`. Transfer fees excluded by discovery (fail-closed tripwire re-verifies at execution).

## Gates in opportunity.rs (verified structurally)

1. `net_before_tip = gross - flash_fee - base_fee - priority_fee` (checked_sub; underflow → reject).
2. `t_max = net_before_tip - min_net_floor - contingency_buffer` (saturating); must be ≥ MIN_JITO_TIP else skip.
3. Tip clamped to t_max; if clamped EV ≤ 0 → skip.
4. `real_net_base_raw` must be > 0 and ≥ `min_net_base_raw` (converted from lamports floor).
5. Implausibility gate: ROI bps ≤ `ARB_MAX_IMPLAUSIBLE_ROI_BPS` (default 100000) — rejects phantom edges.
6. Chronos EV gate (USD-denominated, requires fresh SOL price) — fail closed if no price.
7. Risk gate: notional/impact/tip-share checks; flash-loan path uses `evaluate_flash_loan`.

## Conversion safety

- `lamports_to_base_raw_checked` / `base_raw_to_lamports_checked` (dimensional module) used; on Err → fail closed.
- min profit floor converted from lamports to base-raw; Err → reject.
- SOL price required for USD notional; missing → reject.

## Verification gaps

- No live execution evidence this session; "profit" claims in prior reports are simulation/estimation.
- f64 used in EV gate and telemetry; authoritative profit decisions use integer/raw units (acceptable but should be audited for rounding).
- `min_net_profit_abs` configurable via config.toml + MIN_NET_PROFIT_USD override.

## Status

Mathematically sound structure; realized-profit verification pending live run.
