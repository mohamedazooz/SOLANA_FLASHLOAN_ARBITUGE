# 02 — Mathematical Audit

All formulas verified by reading source (not by trusting test names), and spot-checked
independently (fee rounding, bin-index division, transfer-fee semantics vs official SPL/Meteora
source — see `14_EXTERNAL_RESEARCH_REFERENCES.md`). Full inventory with line numbers:
`audit_work/02_MATH_ACCOUNTING.md`.

## 1. Spot price / execution price / price impact

| Quantity | Implementation | Expression | Notes |
|---|---|---|---|
| CPMM out | `cpmm_out` arb-math/cpmm.rs:67 | floor(`in × (reserve_out − fee) / (reserve_in + in×(1−f))`)-style constant product with upfront fee ceil | integer u128; fee ceil matches on-chain |
| CPMM fee | `cpmm_fee_paid` :173 | `ceil(amount × fee_num / fee_denom)` | conservative |
| Price impact | `cpmm_price_impact_bps` :139 | bps, u16 clamp to 10_000 | integer only |
| CLMM | `quote_clmm` clmm.rs:995 | per-tick Q64.64 step logic | SDK-parity tested (`orca_sdk_parity.rs`, adversarial) |
| DLMM | `quote_dlmm_view` dlmm.rs:904 | P_i=(1+step/10_000)^i; outputs floor, inputs/fees ceil, protocol fee floor | **SDK-byte-equal on captured mainnet pool (tolerance 0)** |
| Variable fee | `compute_variable_fee` :395 | ceil(vfc·(va·bs)² / 1e11) | matches SDK doc formula |
| Total fee | `compute_total_fee_rate` :425 | min(base + variable, 100_000_000) (10% cap) | |
| kamino flash fee | `calculate_flash_fee_raw` economic.rs:460 | ceil(borrow × sf / 2^64) — U68F60 fixed point | live sf ≈ 11_529_215_046_068 verified on-chain 2026-11-04 (kamino.rs:185-187) |
| base→lamports | `base_raw_to_lamports` economic.rs:244; `dimensional.rs:310` | rational rate, ceil (costs) vs floor (conversions) | **two converters disagree on rounding (economic floor vs dimensional ceil)** |

No floats on amount/fee paths (f64 confined to sizing brackets, Chronos scoring and display).

## 2. The critical dimensional defects

### 2.1 Token-2022 cross-pool quote double-subtraction — CONFIRMED CRITICAL
`apply_transfer_fee_ceil(amount, bps, max)` returns `amount − ceil(amount·bps/10_000)`
(arb-math/src/lib.rs:69-77) despite the name suggesting it returns the fee. Callers:

```
meteora_quote_engine.rs:118-123  fee = apply_transfer_fee_ceil(amount_in, …); amount_in −= fee
meteora_quote_engine.rs:134-139  (same, outbound)
```

`effective_amount_in` becomes `ceil(fee(amount_in))` (e.g. 1% of intended at 1% bps). Same on the
outbound leg: `hop2_final_amount_out` becomes the *fee*, so `spread_bps`/`gross_profit` are
meaningless and the route is silently dropped. The correct usage exists in the same repo
(`arb-dex/src/meteora_dlmm.rs:426,475`), proving intent. **All fixtures use
`transfer_fee_bps = 0` — CI is green while the engine is wrong for exactly the Token-2022 pool
class it exists to price.** Impact magnitude: at 1% bps, quoted profit is off by 100×; sign flips
at 50%. This is fail-closed at the VM (tx would revert) rather than fund-loss, but it makes the
cross-pool DLMM strategy quote nothing but noise.

### 2.2 `status` / `collect_fee_mode` hardcoded in the quote engine — CONFIRMED HIGH
`meteora_quote_engine.rs:61` sets `status: 0` (pool "enabled") and `:72` sets
`collect_fee_mode: 0` (InputOnly), ignoring `pool.is_active` and the decoded
`dlmm.params.collect_fee_mode` (arb-types/src/lib.rs:786). OnlyY pools are mispriced
(fee charged on input instead of output); disabled pools quote as enabled.

### 2.3 Mixed-unit subtraction (lamports vs base-mint raw) — CONFIRMED HIGH
- `run.rs:676-679`: `net_before_tip = gross(lamports) − flash_fee_raw(base raw) − 5_000 − priority` — base==WSOL makes it coincidentally correct.
- `derive_swap_legs` (arb-tx/lib.rs:3020-3024): floor = `borrow + flash_fee_raw + tip_raw + min_profit_raw` — tip (lamports) added to base-raw.
- `run.rs:1456-1457`: `deductions_base = (gross_profit − real_net)/base_scale` — subtracts lamports from base raw before scaling.
- `run.rs:432`: `EconomicConfig.min_profit_raw = TokenAmount(min_net_profit_abs)` — lamport constant into base-raw field.
- `run.rs:1048/1059`, `dispatch.rs:110`, `arb-risk/src/lib.rs:592`: `Lamports(real_net_base_raw)` — base raw typed as lamports.
- `run.rs:1700`, `dispatch.rs:110`: same lamport constant compared against base raw.

### 2.4 "Conservative" min leg is not conservative — CONFIRMED MEDIUM
`make_economic_opportunity` (arb-engine/src/lib.rs:369-374) passes `minimum_final = final_out`
(expected value), and zeroes DEX/transfer/account/other/network/priority fees (`lib.rs:372-378`).
The `meets_min_profit` gate therefore under-charges by exactly the network+priority costs (they are
re-added at dispatch, run.rs:805-818 — the two formulas disagree).
`cycle_profit_at` zeroes decimals (arb-engine/src/lib.rs:209-210).

### 2.5 Optimizer arithmetic risks — CONFIRMED LOW/MEDIUM (see 12_OPTIMIZER_AUDIT.md)
- `golden_section_size` (arb-engine/src/lib.rs:245-268): f64 bracket over `[1_000, u64::MAX]`
  (multi-hop caller, :1090; supervisor passes `u64::MAX` when flash loans on, run.rs:477-481);
  48 iters → residual uncertainty ≈ 1.6e9 raw units; returned size is an *unobserved midpoint*
  (:267); assumes unimodality on a step-function DLMM curve.
- `net_profit_bps = net × 10_000 / size` (meteora_size_optimizer.rs:191-195): unchecked i64 mul.
- `calculate_net_profit_raw` cannot represent losses (`Err(Overflow)` on `gross < borrow`).

### 2.6 Ceiling-rounding divergence between converters — CONFIRMED LOW
`economic.rs:258` (floor) vs `dimensional.rs:323` (ceil) on lamports→base-raw. Cost paths use
ceiling (favourable to protocol), value paths floor; tolerable but inconsistent and undocumented.

### 2.7 Overflow / truncation sites
`golden_section_size` f64→u64 casts; `conv_u64` saturates to `u64::MAX` instead of rejecting
(cross_pool_dlmm.rs:431-433); `u16` fee-bps flooring in dex `fee_bps()` (meteora_dlmm.rs:229-238);
unchecked `fee_numerator as u16` (pair_discovery.rs:217-218); `pos > 255` silently inlines
(arb-tx/lib.rs:3553-3565); div-by-zero panic sites enumerated at arb-engine/src/lib.rs:431,630,832,856,1154;
currency.rs:393.

## 3. Profit reconstruction (independent ledger)

For a WSOL flash route expecting out `E` (raw), borrow `B`, sf `s`, priority fee `P`, tip `T`,
base fee 5_000 lamports:

```
flash_fee_raw = ceil(B × s / 2^64)
net_dispatch  = (E − B − flash_fee_raw − 5_000 − P − T)   [calculate_net_profit_raw, economic.rs:399]
net_opportunity (build_economic_quote at lib.rs:345) =
    (final_out − B) − (flash_fee + tip_as_raw + 0 network + 0 priority + 0 dex + …)
```

Per-instruction fund flow and per-leg fee attribution: see `09_WALLET_AND_PROFIT_FLOW.md`.

**Double counting check:** no fee is subtracted twice inside a single formula. But the same cost
(flash fee) is deducted in both the opportunity gate and the dispatch gate (two alternatives, not
additive — conservative but divergent), and the tip is *also* added into the flash min-out floor
(lib.rs:3020-3024) — that floor is an instruction-space bound, not a subtraction, so no arithmetic
double count; still denomination-mixed.

**Fee-in-quote check:** DLMM quote outputs already net of DEX fees; Token-2022 transfer fees are
*not* embedded in CPMM/CLMM outputs and are wired to nothing on the hot path (every live path
prices 2022 mints fee-free — arb-dex decodes the extension but no caller consumes it).

## 4. Test evidence

| Test | Covers | Result |
|---|---|---|
| `dlmm_bin_array_boundary_tests.rs` | bin index floor division, crossings, 4-5 array traversal, exact-out | PASS (unverified: uses locally re-implemented bitmap closure, not production fn) |
| `meteora_sdk_differential_tests.rs` | PDA byte-equality, ±1 raw unit vs official SDK | PASS |
| `meteora_mainnet_sdk_differential.rs` | tolerance-0 on a captured mainnet pool (16 quote cases) | PASS (but simulates CollectFeeMode 1 over mode-0 bins — mode 1 is never reached by the engine) |
| `adversarial_tests.rs`, `adversarial_challenge_tests.rs` | math edge cases | PASS |
| `trade_size_optimization_tests.rs` | optimizer | 2 tests, both weak (no optimality assertion; one writes files into `reports/`) |
| Token-2022 inside quote (`quote_single_hop` bps>0) | — | **MISSING** (this is where 2.1 hides) |
| Flash fee > 0 fixtures | — | **MISSING** (all sf = 0) |
| Split-unit subtraction regression | — | **MISSING** |

## 5. Independent reference calculations
- Transfer fee, 1% bps on 1_000_000 raw: fee = ceil(1_000_000×100/10_000) = 10_000; net = 990_000.
  Buggy path yields effective_amount_in = 10_000 (fee only) — 99× understated. (SPL source:
  `transfer_fee/mod.rs calculate_fee` — ceil + maximum_fee cap.)
- Bin index: bin −1 → −1, bin −70 → −1, bin −71 → −2 (floor semantics), vs C trunc division
  (−1 → 0) which the code correctly avoids (`div_euclid`).
- Kamino fee: `ceil(borrow × sf / 2^64)` with sf ≈ 11_529_215_046_068 (≈ 6.25e-7 fraction, ~0.006 bps) →
  on a 1 SOL borrow the fee is ≈ 625 lamports; Kamino enforces a 1-lamport minimum (official source).
