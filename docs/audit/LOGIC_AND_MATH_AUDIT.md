# Logic and Math Audit

Status: source review of the arithmetic and economic decision logic. Tests were
run in the Linux verification environment; see `TEST_EVIDENCE.md`. No runtime
market comparison is claimed.

## 1. Numeric model

| Layer | Representation | Failure behavior |
|---|---|---|
| Raw token amounts | `u64` / `u128` (`TokenAmount`) | Checked add/sub/mul; overflow returns `ArbError::Overflow`. |
| Prices / CLMM | Q64.64 fixed point | `pow_q64` returns `Result`; unrepresentable values reject. |
| Fees | Basis points + protocol fixed point | Missing/zero denominator rejects; no default bps. |
| Net PnL | `i128` signed raw | Negative losses preserved, not clamped to zero. |
| USD notional | `f64` only at the risk-gate boundary | Non-finite/zero rejects. |

The critical-path rule is: **no `saturating_*` and no default fee** where the
result authorizes a trade. Saturating arithmetic is acceptable only in
non-decision telemetry or where an explicit clamp is the documented intent.

## 2. AMM math

### CPMM (Raydium v4, Raydium CPMM, Pump.fun/PumpSwap quote)

- Constant product with fee denominator 10,000:
  `amount_out = amount_in_after_fee * reserve_out / (reserve_in + amount_in_after_fee)`.
- Fee is read from pool state; `PoolState::fee_bps()` returns `Option<u16>` and
  `None` on a zero denominator (`FIN-05` fixed; no 25-bps fallback).
- Output is computed in wide integers and checked down to `u64`.
- Zero reserves, zero input, or zero output produce a typed error, not a quote.

### CLMM (Raydium CLMM, Orca Whirlpool)

- Q64.64 sqrt-price/tick math. Tick arrays must cover the swap range; missing
  coverage fails closed (`CLMM-01` fixed).
- `pow_q64` rejects unrepresentable results instead of returning a phantom price.
- Fee growth and liquidity-net calculations are checked; an extreme tick or
  liquidity value must not wrap.

### DLMM (Meteora)

- Discrete bin ladder. Active-bin liquidity caps the input
  (`amount_x` for A→B, `amount_y` for B→A).
- Bin-id bounds are validated; out-of-range returns `BinIdOutOfRange`.
- Volatility-scaled fee is decoded; a missing/zero fee rejects.
- `pow_q64` overflow returns `MathOverflow` (`DLMM-01` fixed).

## 3. Economics logic

- `build_economic_quote` sums flash fee, DEX fees, transfer fees, account cost,
  other verified costs, network fee, priority fee, and tip with checked
  arithmetic.
- Expected and conservative (min-out) PnL are separate signed fields.
- `is_profitable()` uses the conservative value and `meets_min_profit`.
- The single-route min-out includes the full profit floor, not slippage alone.
- `ECON-01` fixed: fixtures no longer assume expected output is guaranteed.

## 4. Fixed findings in this area

| ID | Fix | Evidence |
|---|---|---|
| FIN-01 | Kamino fee decoded from `flash_loan_fee_sf`, ceiling-rounded; no hardcoded bps. | `arb-tx/src/kamino.rs` (`calculate_kamino_flash_fee`, reserve decoder). |
| FIN-05 | `fee_bps()` returns `Option`; no default. | `arb-types/src/lib.rs`. |
| M-07 | `pow_q64` returns `Result`; unrepresentable Q64.64 rejects. | `arb-math/src/dlmm.rs` + tests. |
| ECON-01 | Conservative min-out drives profitability; signed losses preserved. | `arb-math/src/economic.rs` + tests. |
| CLMM-01 | Missing tick-array coverage fails closed. | `arb-math/src/clmm.rs` + tests. |
| DLMM-01 | Extreme-bin overflow returns a typed math error. | `arb-math/src/dlmm.rs` + tests. |

## 5. Open logic concerns

1. **Dual USD conversion.** `RiskGate::check_min_profit` compares net lamports to
   `amount_in * bps`, while the supervisor separately converts the base mint to
   USD for the notional cap. For a non-SOL/non-USDC base mint the two conversions
   are not proven to use the same rate. This needs a single economic quote
   carrying one verified rate.
2. **Split routing.** Disabled and unreachable; aggregate min-out equivalence is
   not implemented. Do not re-enable without re-quoting every leg and the return
   hop.
3. **Jupiter.** Disabled; its fee/route assumptions are outside the accepted
   model.
4. **Transfer-fee mints.** The active-epoch fee is applied, but the full
   Token-2022 extension matrix (transfer hooks, confidential transfer) is not
   covered.
5. **Overflow policy coverage.** Checked arithmetic exists in the reviewed paths,
   but a workspace-wide grep for `saturating_`/`unwrap` in decision code is not a
   proof that every path is safe. A targeted lint/audit is recommended.
6. **No property-based test for the full cost ledger** across arbitrary
   fee/size/decimals combinations; current tests are example-based.

## 6. Review checklist for future math changes

- [ ] Every new division checks the denominator.
- [ ] Every fee has an explicit source; no `unwrap_or(25)`.
- [ ] Every amount conversion carries decimals and a verified rate.
- [ ] Negative PnL remains representable and tested.
- [ ] A rejected quote cannot mutate reusable pool state (the guarded path uses
      a private copy).
- [ ] Tests cover zero, one, max-u64, and unrepresentable Q64.64 inputs.
