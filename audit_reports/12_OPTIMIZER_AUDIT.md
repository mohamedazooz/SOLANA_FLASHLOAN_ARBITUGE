# 12 — Optimizer and Route-Economics Audit

Evidence: optimizer re-audit (this session) + `audit_work/02_MATH_ACCOUNTING.md` §5 +
`trade_size_optimization_tests.rs` (read). There is **no single optimizer** — four parallel
implementations with divergent accounting exist; only the cross-pool DLMM one and the CPMM closed
forms sit on live paths.

## 1. Which optimizer runs at runtime

| Optimizer | Code | Runtime? |
|---|---|---|
| `golden_section_size` (f64, 48 iters, `[1_000, u64::MAX]`) | arb-engine/src/lib.rs:245-268 | **Yes — but only the DFS+Bellman-Ford fallback** (run.rs:600); multi-basin DLMM curves are unimodality-violating |
| `optimal_two_pool` / `optimal_three_pool` (closed form) | arb-math/src/cpmm.rs:244/307 | Yes — main CPMM 2/3-hop path (lib.rs:604,742) |
| `cross_pool_dlmm::optimize_size` (integer coarse→fine, multi-basin) | arb-engine/src/cross_pool_dlmm.rs:235-345 | **Yes — DLMM cross-pool (run.rs:535)** |
| `MeteoraDlmmTradeSizeOptimizer` | meteora_size_optimizer.rs | **No** — replay bin + sim/tests only |
| `arb-execution::sizing` | arb-execution/src/sizing.rs | **No** — crate unreferenced |

## 2. Search domains and phases

**golden_section_size**: bracket `[lo, hi]`; caller `find_multi_hop_cycles` passes `[1_000,
max_trade_size]` with `max_trade_size = u64::MAX` under flash loans (run.rs:477-481); 48 iterations
no-tolerance; f64; residual uncertainty ≈ 1.6e9 raw units; `((a+b)/2) as u64` returns an **unobserved
midpoint** (lib.rs:267) which is re-quoted later without comparing alternatives; objective =
`final_out − d` (gross, no flash/priority/tip); 96 `cycle_profit_at` evaluations, no memoization;
quote failure → 0 (indistinguishable from unprofitable).

**cross_pool_dlmm::optimize_size**: `lo = cfg.min_input.max(1).max(min_liquidity_bin_raw)`;
`hi = min(max_trade_size, cfg.max_input, cap1)`; cap2 inside `two_hop_quote` with `MAX_RESIZE_PASSES = 4`.
Coarse: 1.5× ladder (≤128) + hi + linear fill of `coarse_samples` (2..=64). Basin detection: local
maxima with `i64::MIN`-style edges; **every detected peak gets a fine sweep** (default
`refine_samples ≥ 4` equal-spaced, step = span/steps, no dedup). Selection:
`max_by_key(gross)` over evaluated points — **best observed**, honestly labeled; spread hurdle
`spread_bps < cfg.min_spread_bps` rejects (:424-427). Overflow: `conv_u64` saturates to `u64::MAX`
instead of rejecting (:431-433) — a u64::MAX min-out would brick an instruction instead of erroring.

**MeteoraDlmmTradeSizeOptimizer**: defaults `10_000..10_000_000` raw; coarse 1.5× ladder ≤128 + hi +
24-point linear fill; basin detection identical; **`candidate_basins.truncate(max_candidate_basins=4)`
keeps the leftmost 4 in grid order — not the 4 most profitable** (:345) — right-side peaks silently
dropped; empty-peak fallback to top-4-by-profit. Fine: 10 equal-spaced steps per basin with a
`quote_map` dedup. Selection: `max_by_key(net_profit_quote_raw)`; explicitly labeled
`"best_observed_candidate"` (:408) and never claims global optimum; `bounded_early =
coarse_points.len() >= 128`. Objective: `net = gross − network_cost` with **hardcoded SOL price
$180** and **hardcoded 9_000 lamports network cost** ("5k base + 4k priority") and **no flash fee,
no tip** (:114-115, :143-158, :190); CU cap 400_000 > MAX_COMPUTE_UNIT_LIMIT (350_000).
`net_profit_bps = net × 10_000 / size` unchecked i64 (:192).

**CPMM closed forms**: optimal swap amounts analytically; integer floor on each step (safe);
non-CPMM hops seed 100_000; DLMM bottleneck expansion can *increase* size beyond the analytical
optimum (relies on the final economic gate, not optimizer optimality).

## 3. Profit objective vs the authoritative accounting engine

`build_economic_quote` (economic.rs:274-388) is checked U256, ceiling division, signed i128
projections, `Err(Overflow)` on any overflow — the strongest module in the repo. Its flaws are at
the call site, not inside: `make_economic_opportunity` passes `minimum_final == expected_final`
(zero slippage haircut, lib.rs:369-374), zeroes DEX/transfer/other/network/priority fees and passes
only flash fee + configured tip ceiling (lib.rs:372-378). The supervisor later re-derives with the
actual bid and adds base+priority (run.rs:805-818). **Two divergent net-profit definitions coexist;
neither is fed the real slippage-constrained min output; the genuinely conservative min leg
(hop_inputs-derived, lib.rs:1186-1192) exists and is never wired in.**

Flash fee is double-deducted across the two gates (opportunity + dispatch) — conservative, but any
$ figure printed between them is inconsistent.

`CostLedger` (the "no double count" ledger, economic.rs:89-123) is **implemented and never used**.
`calculate_net_profit_raw` cannot express a loss (`Err(Overflow)`); `is_profitable()` therefore means
"not losing by more than the float representation allows", not "makes min profit".

## 4. CU and transaction size in the decision

- Decision CU = `estimate_route_cu` (arb-tx/src/lib.rs:145) with per-hop protocol table; override
  by measured CU when cached (run.rs:652-653); limit = ×1.15 capped at 350_000; priority price =
  `adaptive_cu_price` (p75×1.25) then profit-capped at 20% of gross (`cap_cu_price_by_profit`,
  2_000 bps) — cap uses **gross**, looser than needed.
- **No transaction-size feedback exists in the live decision** — an oversized V0 would still be
  built and submitted-then-dropped. `arb-execution::sizing` does exact V0 MTU accounting but the crate
  is unreferenced.
- Meteora-optimizer heuristics (400k CU bin-array model) feed nothing at runtime.

## 5. "Best observed" vs "proven global optimum"

Nothing in this system claims a global optimum except silence. The Meteora optimizer is honestly
labeled; the cross-pool path likewise selects the max evaluated point without basin-completeness
proof (coarse-grid peaks narrower than the 1.5× ladder are invisible); golden_section has no
multi-basin notion at all and returns an unobserved midpoint. **Any claim in project docs that the
engine "finds the optimal size" is false.** Report: best observed candidate only — flash fee, tip,
priority fee and base fee now vary per build so the true global optimum is not computable in-band.

## 6. Suspicious ROI / mislabeling reproduction

- `spread_bps` in cross-pool results is `gross/principal` (capital ROI), not a two-pool spread —
  mislabeled, unverified unit tests on it.
- `net_profit_bps` (meteora) can exceed 10_000 unchecked; no saturation.
- USD "profit" assumes WSOL base and the same SOL price twice (self-consistent at 9 decimals).
- Strategy `$0.20 net floor` comment vs enforced `min_net_profit_abs = 50_000` lamports (~$0.009 at $180) — 20× gap between documentation and enforcement.

## 7. Tests

`trade_size_optimization_tests.rs` — two tests; both assert `Some(...)` and non-empty evaluation
lists; no assertion on optimality vs brute force, no two-basin adversarial fixture, no boundary
cases (`lo==hi`,`=0`,`>hi`), no golden-section coverage at all, no fee-inclusion assertion, and one
test writes `reports/phase15_2/optimizer_search_statistics.json` into the repo (side effect).
`adversarial_*` cover math units, not the search.

## 8. Required fixes (input to remediation, not applied)

1. Pick one optimizer per path and delete the rest to `replay`/`sim` only.
2. Feed real slippage-haircut min-out into `build_economic_quote`; delete the min==expected leg.
3. Decide and code-review a single net definition; delete `CostLedger` or use it.
4. Clamp `i64 × 10_000` with checked_mul.
5. Make the Phase-15.2 stats test write to a temp dir.
6. Test a two-separated-peaks profit curve and assert both basins are explored.
