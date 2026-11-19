# Economics and Sizing

Status: source-level description of the cost model and sizing logic. No measured
profit, landing probability, or fee calibration is claimed. The numbers below are
model behavior, not a trading forecast.

## 1. Unit discipline

- Token amounts are raw integer units (`TokenAmount`), with `base_decimals`
  carried separately. No f64 is used for on-chain amounts in critical paths.
- Lamport costs (network fee, priority fee, Jito tip) are converted to base-mint
  raw units through an explicit rational rate
  (`lamports_to_base_mint_rate / lamports_to_base_rate_denom`) with checked
  arithmetic. A missing rate rejects the quote; it is never defaulted.
- `EconomicQuote` carries signed expected and conservative net PnL; negative
  values represent losses and are preserved.

## 2. Cost ledger

`arb-math::economic::CostLedger` is the canonical cost model. Every entry carries
an ID, scope, type, unit, raw amount, and a source string.

| Cost | Source | Notes |
|---|---|---|
| Flash-loan fee | On-chain Kamino `ReserveConfig.fees.flash_loan_fee_sf` (Q64.64) | `ceil(borrow * sf / 2^64)`. No bps default. |
| AMM swap fee | Pool state (fee numerator/denominator or protocol fee fields) | `Option<u16>`; zero denominator or missing fee rejects, no 25-bps default. |
| Token-2022 transfer fee | Mint transfer-fee config at the active epoch | Applied when the mint has a config. |
| Account creation | Rent for ATAs/accounts created by the route | Included as a verified cost. |
| Other verified costs | Explicit entries only | No placeholder bucket. |
| Network fee | Base signature fee | Lamports. |
| Priority fee | Compute-unit price × CU limit | Lamports. |
| Jito tip | Tip strategy output | Lamports. |

The net formula is:

```
net = final_received
      - principal
      - flash_fee
      - dex_fees
      - transfer_fees
      - account_creation_cost
      - other_verified_costs
      - network_fee
      - priority_fee
      - jito_tip
```

`minimum_net_profit_signed_raw` uses `minimum_final_amount_raw` (the conservative
min-out), not `expected_final_amount_raw`. `EconomicQuote::is_profitable()`
requires both `meets_min_profit` and a strictly positive conservative net. This
closes the historical `ECON-01` failure mode where expected output was treated as
guaranteed.

## 3. Minimum profit gate

`RiskGate::check_min_profit` requires:

```
net_lamports >= min_net_profit_abs
net_lamports >= amount_in * min_net_profit_bps / 10_000
```

Defaults: `min_net_profit_abs = 10_000` lamports, `min_net_profit_bps = 8`.
These are policy defaults, not profitability evidence.

## 4. Sizing

- **Two-pool cycles**: `arb_engine::optimal_two_pool` uses the closed-form
  constant-product optimum; `golden_section_size` is available for cycles where
  the closed form does not apply.
- **Three-pool cycles**: `optimal_three_pool` / golden-section search over the
  input range.
- **Bounds**: input is capped by `risk.max_trade_size_usd` converted to base
  units, by the active bin's available liquidity on DLMM, and by pool reserves.
- **Failure**: if the optimizer returns no positive size, the route is skipped.
  There is no minimum-size fallback and no "trade anyway" branch.

Sizing is advisory to the risk gate: the gate re-checks the USD notional and all
costs after sizing, so an optimizer bug cannot bypass the cap by itself.

## 5. Tip policy

`arb_land::TipStrategy` starts from a base share (`tip_share_bps`, default 5500
in `config.toml`) and scales toward `max_share_bps` for larger edges, bounded by:

- a floor (`tip_floor_percentile`, e.g. p50),
- a hard cap (`max_tip_lamports`),
- a minimum retained amount (`min_keep_lamports`),
- the risk gate's `max_tip_share_bps` and `max_tip_lamports`.

A tip is only emitted when the edge survives it; `pick_tip` returns `None`
otherwise. `DEFAULT_JITO_TIP_FLOOR_LAMPORTS = 10_000` is a floor, not a target.

## 6. Worked example (illustrative, not measured)

Borrow 1 SOL of WSOL, one Raydium v4 hop, 25 bps pool fee:

```
principal          = 1_000_000_000 lamports-equivalent
pool fee (25 bps)  ≈ 2_500_000
flash fee (sf)     = ceil(1e9 * 11_529_215_046_068 / 2^64) = 625
network fee        = 5_000
priority fee       = CU price * CU limit (route-dependent)
tip                = share of remaining edge, capped
```

If the route returns less than `principal + all costs + min_profit`, the
conservative net is non-positive and the quote is not profitable. The real
`sf` value must be read from the reserve account; the example is only arithmetic
that demonstrates ceiling rounding.

## 7. What is not proven

1. No realized PnL, fill rate, or landing-rate measurement exists in this
   repository.
2. No fee calibration against live DEX state; pool fees are decoded but a
   differential comparison with official SDK quotes is missing.
3. The Jupiter path is disabled, so its fee/route assumptions are not part of the
   accepted model.
4. Split routing is disabled; the aggregate min-out equivalence for split routes
   is not implemented.
5. The `min_net_profit_bps` is applied to `amount_in` in base units; for a
   non-SOL base mint the USD conversion happens separately in the supervisor.
   This dual conversion needs a single-source audit before release.
