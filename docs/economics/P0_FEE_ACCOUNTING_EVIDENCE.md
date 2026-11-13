# P0 — Fee and Profit Accounting Evidence

**Status:** VERIFIED by unit tests (offline, deterministic)
**Scope:** repair of the reported cross-currency (USDC vs lamports) profit-accounting defect
**Live execution:** DISABLED

This document is the authoritative worked example for how the engine accounts for
fees and profit. Any report that contradicts the numbers below is wrong and must be
corrected.

---

## 1. The defect that was repaired

The previous accounting path summed two quantities that do not share a unit:

| Quantity | Unit | Value |
|---|---|---|
| Round-trip quote-token delta | raw USDC base units (6 decimals) | −86 |
| Network cost | lamports (SOL base units, 9 decimals) | 9,000 |

The old code produced `−86 + (−9,000) = −9,086` and then divided by the 100,000-raw
input to report `−908 bps`. Both numbers are unit-incoherent: `−9,086` is labelled
"lamports" while 86 of those units are USDC base units, and the bps figure converts a
lamport sum against a USDC denominator without applying an exchange rate.

Two independent defects were involved:

1. **Cross-currency arithmetic.** Adding a 6-decimal token amount to a 9-decimal
   native amount without a conversion.
2. **Double counting.** DEX swap fees that are already embedded in the quote output
   were being subtracted a second time as an execution cost.

## 2. Correct accounting, same observation

Inputs (per the reported observation):

| Item | Raw amount | Decimals | Currency |
|---|---|---|---|
| Initial quote balance | 100,000 | 6 | USDC |
| Final quote balance | 99,914 | 6 | USDC |
| Hop 1 output (intermediate) | 1,160,000 | 9 | SOL |
| Base transaction fee | 5,000 | 9 | SOL |
| Priority fee | 4,000 | 9 | SOL |

Step 1 — gross profit, strictly in quote-mint raw units:

```text
gross = final - initial = 99,914 - 100,000 = -86 raw USDC
```

Step 2 — convert the SOL-denominated network cost with an explicit,
timestamped, conservative rate. Rate used: `1 SOL = 180.00 USDC`
(configured floor, recorded with slot and timestamp).

```text
network_sol = 5,000 + 4,000 = 9,000 lamports
network_quote = 9,000 * 180 * 10^6 / 10^9 = 1,620 raw USDC
```

Step 3 — net profit in the same unit:

```text
net = gross - network_quote = -86 - 1,620 = -1,706 raw USDC
```

Step 4 — basis points, same denominator as the input:

```text
bps = floor(-1,706 * 10,000 / 100,000) = -170 bps
```

### Before / after

| Report field | Broken value | Correct value |
|---|---|---|
| Gross profit | −9,086 (unit mislabelled) | −86 raw USDC |
| Net profit | −9,086 lamports | −1,706 raw USDC |
| Net profit bps | −908 | −170 |
| DEX fee treatment | subtracted again | embedded, not re-subtracted |

The correct answer is **not profitable**. The route loses 1,706 raw USDC (−0.001706
USDC), dominated by network cost, not by swap fees.

## 3. Why DEX fees are not deducted from net profit

`quote_dlmm_multi_bin` (and the DLMM exact-out path) already returns the **net**
amount out after the pool fee has been taken inside the bin traversal. Subtracting a
"swap fee" line item from that output again would double count.

Therefore `FeeItem` records DEX fees with `embedded: true`. They contribute to
`total_embedded_dex_fees_raw` for reporting and for fee-attribution analysis, but
they are excluded from `total_execution_cost_quote_raw`.

The reported example carries 400 raw units of embedded DEX fee per hop (800 total).
Those 800 units are already inside the 99,914 final balance. Net profit stays
−1,706, not −2,506.

## 4. Cost separation

Every cost carries an explicit class, and the classes are never collapsed:

| Class | Example | Deducted from net profit? |
|---|---|---|
| Network base fee | 5,000 lamports | yes (after conversion) |
| Priority fee | 4,000 lamports | yes (after conversion) |
| Jito tip | 0 in paper mode | yes, if present |
| DEX swap fee | 400 + 400 raw | no — embedded in quote |
| Token-2022 transfer fee | per mint config | no — embedded in quote/amount |
| Account-creation rent | 2,039,280 lamports | no — refundable, tracked separately |
| Irrecoverable rent | rent on closed accounts | yes |

`total_refundable_rent_lamports` is surfaced but never subtracted, because the rent is
recoverable when the temporary account is closed.

## 5. Enforcement mechanism

The accounting is not a convention, it is a type system:

- `TypedTokenAmount` carries `(raw_amount, Currency, decimals, ConversionSource, slot, timestamp)`.
  `checked_add`/`checked_sub`/`checked_signed_diff` return
  `DimensionalArithmeticError::CurrencyMismatch` for USDC-vs-SOL, and
  `DecimalMismatch` for same-currency different-decimals.
- `CompileTypedAmount<SolTag>` / `CompileTypedAmount<UsdcTag>` are distinct types, so
  `sol.checked_add(usdc)` does not compile.
- `TimestampedRate` will not convert from a rate whose slot is older than the
  configured lag (`StaleConversionRate`).
- `DimensionalProfitabilityReport::compute` returns net profit only in
  `report.quote_mint` raw units, with `net_profit_bps` computed against the same
  input amount.

## 6. Tests that prove it

| Test | File | Asserts |
|---|---|---|
| `test_exact_repair_of_reported_p0_profit_accounting_bug` | `crates/arb-types/tests/dimensional_currency_tests.rs` | gross `-86`, network `9,000` lamports, converted `1,620`, net `-1,706`, bps `-170`, embedded DEX `800`, `is_profitable == false`, and explicitly `assert_ne!` against `-9,086` / `-908` |
| `test_negative_invalid_cross_currency_addition_rejected` | same | USDC + SOL and USDC − SOL both error with `CurrencyMismatch` |
| `test_negative_decimals_mismatch_rejected` | same | same currency, different decimals errors |
| `test_compile_time_typed_amount_safety` | same | `SolTag`/`UsdcTag` are not interchangeable |
| `test_profitable_route_with_dimensional_safety` | same | `+5,000` gross, `3,380` net, `+338` bps, refundable rent excluded |
| `test_property_addition_commutativity_and_identity` | same | identity and commutativity hold |

Run with:

```powershell
cargo test -p arb-types --test dimensional_currency_tests
```

## 7. Residual limitations

1. The SOL→USDC rate is a configured floor, not a live oracle quote. It is
   timestamped and conservative, but a real deployment must source it from a fresh
   pool quote or Pyth feed and reject stale values.
2. The reported case is a single captured observation, not a live simulation run.
   It proves the arithmetic, not the market.
3. Limit-order liquidity folded into `amount_x`/`amount_y` at decode time is quoted
   as curve liquidity; its protocol-fee attribution is an approximation (see
   `docs/adr/ADR-0014_METEORA_DLMM_0_12_COMPATIBILITY.md`).
4. `net_profit_bps` floors toward negative infinity. A −1.706 % result reports as
   −170 bps, not −171. This is deliberate and used consistently for both signs.

## 8. Claim labels

- Arithmetic repair: **VERIFIED**
- Unit/property/negative tests: **TESTED**
- Runtime path in the paper pipeline: **TESTED (unit-level)**
- Realized market profit: **NOT VERIFIED** (none claimed)
- Live readiness: **NOT AUTHORIZED**
