# Mathematical & Protocol Correctness Audit — 2026-11-08

Auditor role: senior HFT / quantitative trading engineer.
Scope: all crates, with emphasis on `arb-math` (CPMM/CLMM/DLMM/optimal sizing),
`arb-dex` (Pump.fun, PumpSwap, Raydium V4), and `arb-math::economic`.
Cross-checked against official Pump.fun/PumpSwap documentation (pump-public-docs)
and Raydium V4 on-chain math conventions.

## 1. Executive summary

- CPMM quote formula (`cpmm_out_fee`) matches the Raydium V4 on-chain convention
  (gamma = D - fee_bps, fee deducted from input). Verified numerically.
- Closed-form optimal two-pool sizing (`optimal_two_pool`) was verified against a
  brute-force optimizer over 2M candidate sizes: the closed form lands exactly on
  the profit peak. Formula is correct.
- Marginal degradation ceiling derivative is algebraically consistent with the
  peak (verified).
- Pump.fun bonding-curve quote matches the documented virtual-reserve CPMM with
  fee taken from the quote (SOL) side. Real-reserve bounds are enforced.
- PumpSwap adapter contains **two real correctness bugs** in `quote_exact_in`
  (buy fee construction and sell liquidity check). Details in §4.
- Raydium V4 fee-bps conversion assumes a 10,000 denominator; acceptable for v4
  (on-chain denominator is 10,000) but the helper `cpmm_fee_paid` uses ceiling
  rounding while Raydium uses floor — only affects cost accounting, not quotes.
- Token-2022 transfer-fee deduction (`apply_transfer_fee`) is `#[cfg(test)]`-only
  and not wired into any production quote path — a coverage gap, not a math bug.
- No floating point is used in executable amounts; all checked integer. Good.

## 2. CPMM core (arb-math/src/cpmm.rs)

### 2.1 `cpmm_out_fee` — CORRECT
```
out = gamma * dx * y / (D * x + gamma * dx)
```
Matches Raydium V4 on-chain `swap_base_in` (fee deducted from input, constant
product on virtual reserves). Verified by reference: Raydium computes
`amount_in_with_fee = dx*(D-fee)`, then `num = amount_in_with_fee*y`,
`den = x*D + amount_in_with_fee`, `out = num/den`. Identical.

### 2.2 `cpmm_price_impact_bps` — CORRECT (by design)
Uses fee-free output (gamma=10_000) so fees are not double-counted as impact.
Documented in code (B16 fix). Acceptable.

### 2.3 `optimal_two_pool` — CORRECT (numerically verified)
Verified closed form against brute-force over 2M sizes: peak profit identical.
Existence invariant N > D0 correctly rejects sub-fee spreads.

### 2.4 `optimal_three_pool` / `optimal_four_pool` — NOT INDEPENDENTLY VERIFIED
No brute-force verification performed (combinatorial). Recommend adding
property tests comparing closed form to a numerical optimizer for 3-hop.

### 2.5 `cpmm_fee_paid` / `cpmm_fee_paid_with_denom` — MINOR
Uses ceiling division (`div_ceil`). Raydium V4 uses floor (`amount*fee/D`).
This helper is used for cost accounting only, not for quote output, so the
overstatement is conservative. If used for exact fee reconciliation, switch to
floor to match on-chain. Recommend documenting the convention.

## 3. Pump.fun bonding curve (arb-dex/src/pump_fun.rs)

### 3.1 Formula — CORRECT
Buy: `fee = floor(amount_in * fee_bps / 10000)` (input SOL side),
`net = amount_in - fee`, `out = cpmm_out(net, virtual_sol, virtual_token, 10000)`.
Sell: `gross = cpmm_out(amount_in, virtual_token, virtual_sol, 10000)`,
`fee = floor(gross * fee_bps / 10000)`, `net = gross - fee`.
Matches Pump.fun docs (Uniswap-V2-style virtual reserves, fee on quote side).

### 3.2 Real-reserve bounds — CORRECT (conservative)
Buy checks `out <= real_token_reserves`; sell checks `gross <= real_sol_reserves`.
On-chain the extractable side is bounded by real reserves, so this is correct
and slightly conservative (rejects only impossible extractions).

### 3.3 Missing fee schedule — FAIL-CLOSED (correct)
`quote_with_fee` requires `fee_bps = Some(...)` and `token_decimals = Some(...)`;
no static 1.25% fallback. Matches audit requirement.

### 3.4 Mode flags — correct rejection of mayhem/cashback curves by this adapter.

## 4. PumpSwap (arb-dex/src/pump_swap.rs) — BUGS FOUND

### 4.1 Buy path (`quote_exact_in`, `base_to_quote == false`) — INCORRECT FEE MATH
Current code:
```rust
let total_bps = sum(fee_bps);
let mut net = input * 10000 / (10000 + total_bps);
let [fee_lp, fee_protocol, fee_creator] = fee_bps.map(|rate| fee_amount(net, rate));
let fees = [...];
let cost = net + sum(fees);
if cost > input { net -= cost - input; }
let swap_input = net - 1;
let out = cpmm_out(swap_input, quote_reserve, base_reserve, 10000);
```
This is an ad-hoc inverse approximation. The on-chain PumpSwap buy takes fees
from the input quote amount per component with ceiling rounding, then swaps the
remainder. The correct (and exact) computation is:

```rust
let fee_lp       = ceil(input * lp_bps       / 10000);
let fee_protocol = ceil(input * protocol_bps / 10000);
let fee_creator  = ceil(input * creator_bps  / 10000);
let total_fee    = fee_lp + fee_protocol + fee_creator;   // checked
let net          = input - total_fee;                    // checked, > 0
let out          = cpmm_out(net, quote_reserve, base_reserve, 10000);
```

The current `-1` adjustment and inverse division introduce off-by-one (and
sometimes off-by-more) discrepancies versus on-chain quotes, which can cause
false-positive arbitrage edges or rejected valid trades. **Fix required.**

Note: per-component ceiling (not a single combined ceiling) is what the
on-chain program does (each fee recipient accrual is rounded up independently).

### 4.2 Sell path (`base_to_quote == true`) — OVER-STRICT LIQUIDITY CHECK
Current code checks `real_quote_reserve < without_lp` (where `without_lp =
gross - lp_fee`) before subtracting protocol/creator fees. The actual payout is
`out = gross - lp - protocol - creator`, which is strictly smaller than
`without_lp`. The correct check is `real_quote_reserve < out` (or `out >=
real_quote_reserve` rejected). The current check rejects valid sells whenever
the unswept fee buckets fall between `out` and `without_lp`. **Fix required.**

### 4.3 Effective quote reserve — CORRECT
`effective_quote_reserve = raw_quote_vault + virtual_quote_reserves (i128)`,
signed, with overflow/negative guards. Matches PumpSwap docs exactly.
Un-swept fee buckets are excluded from `real_quote_reserve`. Correct.

### 4.4 Tier selection — CORRECT
Strictly-increasing threshold validation, selects highest tier with threshold
<= market_cap. Matches documented behavior.

## 5. Raydium V4 (arb-dex/src/raydium_v4.rs)

### 5.1 Quote — CORRECT
Uses `cpmm_out` with gamma derived from `fee_bps()`. Matches on-chain.

### 5.2 `fee_bps()` conversion — ACCEPTABLE (with caveat)
`(fee_numerator * 10000) / fee_denominator` assumes denominator ≈ 10000.
Raydium V4 on-chain denominator is 10000, so exact. If a pool ever carried a
non-10000 denominator this would lose precision; recommend asserting
`fee_denominator == 10_000` or computing gamma directly from
`fee_numerator/fee_denominator`.

### 5.3 Reserve updates — external; no on-chain decoding of reserves in this
adapter (reserves come from vault balances). Correct by design.

## 6. Economic model (arb-math/src/economic.rs)

### 6.1 Net profit — CORRECT
All amounts integer (U256), checked arithmetic, signed PnL preserved, no
saturation. Network/priority/tip converted from lamports to base units with
ceiling rounding (conservative for liabilities). Good.

### 6.2 Kamino flash fee (Q64.64) — CORRECT
`ceil(borrow * sf / 2^64)`. Verified against documented mainnet value
(sf=11_529_215_046_068 → 625 lamports per 1 SOL). Correct.

### 6.3 `meets_min_profit` — CORRECT
Compares signed minimum net profit against `min_profit_raw`. Correct.

### 6.4 Cost ledger double-count prevention — CORRECT (structure)
`total_included_not_deducted` filters by `included_in_quote && !deducted_already`.
Correct as implemented.

## 7. CLMM / DLMM

Not fully audited (out of scope for this pass). Known open items per
`docs/math/CLMM_DLMM_MATH_AUDIT_2026-11-07.md`:
- Raydium CLMM has no native adapter.
- Orca tick-array traversal limited to 3 arrays.
- DLMM dynamic fee needs differential verification.

Recommend a separate focused audit with on-chain SDK differential tests.

## 8. Cross-cutting gaps

1. Token-2022 transfer fees: `apply_transfer_fee` is test-only; production
   quote paths do not deduct transfer fees. Either wire it in or document that
   Token-2022 transfer-fee mints are rejected (currently partially rejected in
   PumpSwap joiner via `token2022_has_risky_extension`).
2. `cpmm_fee_paid` ceiling vs on-chain floor (cost accounting only).
3. Optimal 3/4-pool closed forms lack brute-force verification.
4. No oracle/price-feed sanity checks in the math crate (out of scope).

## 9. Recommended fixes (priority order)

1. **P0** — Fix PumpSwap buy fee math (§4.1).
2. **P0** — Fix PumpSwap sell liquidity check (§4.2).
3. **P1** — Wire Token-2022 transfer fees into quote paths or hard-reject.
4. **P1** — Add brute-force verification tests for 3/4-pool optimal sizing.
5. **P2** — Reconcile `cpmm_fee_paid` rounding convention with on-chain.
6. **P2** — Add CLMM/DLMM differential tests against SDK.

## 10. Verification evidence

- `optimal_two_pool` closed form vs brute force (2M sizes, 5% spread case):
  closed-form dstar = 11108, brute-force optimum = 11108, profit identical.
- PumpSwap effective-quote-reserve identity verified against docs example
  (100 SOL + -0.03 virtual = 99.97 effective; matches).
- Kamino flash fee verified: sf=11_529_215_046_068 → 625 lamports/SOL.

---
End of audit.
