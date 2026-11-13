# Mathematical Audit — Independent Verification (PHASE B)

## Method

Two independent implementations are maintained:

1. **Production engine**: `crates/arb-math/src/economic.rs` (`calculate_net_profit_raw`, U256 checked arithmetic, fail-on-underflow) and `crates/arb-math/src/dlmm.rs` (bin swap math, Q64.64).
2. **Independent oracle**: `audit/math_oracle/oracle.py` — Python arbitrary-precision integers, derived from protocol specifications (Raydium constant-product, Meteora DLMM bin semantics), NOT copied from the Rust code.

Vectors: `audit/test_vectors/cpmm_vectors.json` (13), `audit/test_vectors/dlmm_single_bin_vectors.json` (6).
Differential harness: `crates/arb-math/tests/python_oracle_differential.rs` (50 tests).
Oracle self-verify: `python3 audit/math_oracle/oracle.py --verify` → PASS (regenerated this audit).

## Authoritative net profit formula

```
Net Profit = Final_Received
           − Borrow_Principal
           − Flash_Loan_Fee
           − DEX_Fees
           − Token_Transfer_Fees
           − Account_Creation_Costs
           − Other_Verified_Costs
           − Network_Costs (base fee + priority fee + tip, converted at verified rate)
```

* All amounts in raw integer token units; lamport costs converted via the verified mint rate.
* Principal is deducted exactly once (the repay instruction transfers principal + fee; the profit identity subtracts principal once).
* Borrow principal is NOT counted as profit: `gross = final − principal`, `net = gross − all costs`.
* Expected vs conservative: conservative uses each hop's enforced `min_amount_out`; the dispatch gate uses the conservative value.

## ROI

```
ROI = Net_Profit / Borrow_Principal × 100
```

Verified in `ECONOMIC_CALCULATION_COMPARISON.csv` (NET-PROFIT-* rows): ROI is computed
against the borrow principal, never against gross output or fee totals.
Historical defect (fixed in the earlier phase): ROI was divided by the wrong denominator,
producing inflated values; the current engine and all tests pin the principal denominator.

## Break-even analysis (verified)

* Minimum final output to repay: `principal + flash_fee` (exact integers).
* Minimum final output to be net-profitable: `principal + flash_fee + Σ costs + min_profit_floor`.
* Maximum affordable priority fee: `gross_profit_after_flash_fee − min_profit_floor` (lamports, via verified rate).
* Maximum affordable tip: same envelope; `TipStrategy::pick_dynamic_tip` is capped by the risk config.
* Break-even borrow size: the size optimizer (`meteora_size_optimizer`) searches the feasible ladder and rejects sizes whose conservative net is below the floor.

## Kamino flash fee (exact, verified)

On-chain formula reproduced from klend `ReserveFees::calculate_fees(amount, flash_loan_fee_sf, FeeCalculation::Exclusive, .., has_referrer=false)`:

```
fee_bits = (amount << 60) * flash_loan_fee_sf >> 60   == amount * sf (exact, U128)
fee_bits = max(fee_bits, 1 << 60)                     // minimum_fee = 1 atomic unit
fee      = round(fee_bits / 2^60)                     // ties away from zero
```

* `flash_loan_fee_sf` is a U68F60 bit pattern (60 fractional bits), read LIVE from the reserve config — never a bps guess.
* Main-market WSOL reserve: `sf = 11_529_215_046_068` = 1e-5 = 0.1 bps → 10,000 lamports per 1 SOL borrow (verified vector in the CSV).
* Fee is collected at repay, on top of principal; the repay instruction passes `principal` (not principal+fee) — the handler transfers the fee separately. Passing principal+fee as the repay amount would double-charge and revert; the builder is pinned by `bundle_tests.rs` and `challenger_m2_tests.rs`.

## Verification results

| Model | Vectors | Rust vs Oracle | Verdict |
|---|---|---|---|
| CPMM swap (upfront-fee ceil / gamma) | 13 | 0 difference | MATCH |
| DLMM single-bin (Q64.64, both directions) | 6 | 0 difference | MATCH |
| Kamino flash fee (U68F60 exclusive) | 4 | 0 difference | MATCH |
| Net-profit identity + ROI | 3 | 0 difference | MATCH |

Full row-level data: `ECONOMIC_CALCULATION_COMPARISON.csv` (26 rows, 0 mismatches).
Property/fuzz: `differential.rs` (12), `fuzzing_suite.rs`, `adversarial_tests.rs` — all pass.
