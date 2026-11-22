# Economic Invariants — Phase 4 (Mission §9)

Canonical list of economic invariants with enforcement point, test, and evidence. All arithmetic: integer `u64`/`u128`; f64 appears only in USD telemetry and BF pre-screen.

| # | Invariant | Enforcement | Test | Status |
|---|---|---|---|---|
| EI-01 | Net profit calculation is integer-only (`u128`); no f64 in decisions | `arb-math::economic` (single CostLedger) | `differential.rs`, `fuzzing_economic_decimals.rs` | ✅ |
| EI-02 | Flash fee = `max(amount·sf, 2^60)` U68F60, ties-away, RAW-bits pre-round guard | `arb-math::economic::klend` | `kamino_fee_boundary_tests` | ✅ |
| EI-03 | Net Profit > configured minimum enforced identically at scanner, builder, resolver, sim, risk | ladder documented in ARBITRAGE_ENGINE_AUDIT §3 | `three_leg_net_economics_tests`, `hop_slippage_once_tests` | ✅ |
| EI-04 | Old/pre-existing balances cannot conceal a loss | executor delta guard (`IntermediateInventorySubsidized`), `derive_swap_legs` final-hop floor, `split_return_min_out` | FL-13/14/15 matrix + tests | ✅ |
| EI-05 | Estimated PnL ≠ Realized PnL | provenance-filtered store + signed gauge (balance deltas only) | arb-store/arb-metrics tests | ✅ |
| EI-06 | Stale quotes cannot pass the execution gate (slot skew, write-version, dependency manifests) | `slot_integrity.rs`, `dependency_manifest.rs`, `max_slot_skew = 0` | ingest skew tests | ✅ |
| EI-07 | Borrow sizing ≤ live Kamino supply-vault liquidity (fail-closed) | sizing guards | `BorrowAboveKaminoLiquidity` tests | ✅ |
| EI-08 | Fee/reference rounding is conservative (ties-away, no fee under-estimation) | roundup points in `arb-math::economic` | boundary vectors | ✅ |
| EI-09 | Route identity preserves direction & hop order (dedup keeps best) | `route_dedup.rs` | dedup regression tests | ✅ |
| EI-10 | Every executed instruction references only allow-listed protocols/venues | executor CPI + allowlists | `PoolNotAllowlisted` tests | ✅ |
| EI-11 | Slippage applied exactly once per hop; final hop bound by the stricter of slippage and economic floor | `derive_swap_legs`, `split_return_min_out` | `hop_slippage_once_tests` (4+), `split_return_min_out_tests` (4) | ✅ |
| EI-12 | Realized PnL increments only from on-chain balance deltas after confirmed landing | `arb-metrics` gauge | metrics tests | ✅ |

Outstanding deliberate limitations: best-observed (not globally proven) sizing; dedicated DLMM optimizer active (must not be wired before FL-N2 correction); 0x1773 unresolved blocks live execution (`docs/active/OPEN_ISSUES.md`).
