# CLMM / DLMM code-level audit — 2026-11-07

## Confirmed issues addressed in this package

| Area | Previous behavior | Current behavior |
|---|---|---|
| CLMM `compute_swap_step` | Swallowed arithmetic error and returned a fabricated price/output tuple | Returns `Result` and propagates typed math errors |
| CLMM multi-step traversal | Saturating amount/fee totals and silent stop after 256 iterations | Checked arithmetic; explicit `NoProgress`, `IterationLimitExceeded`, `IncompleteQuote` |
| CLMM Orca adapter | Fixed 8-tick heuristic; fallback to ±1% price when tick conversion failed | Requires decoded tick arrays; calls multi-tick `quote_clmm`; native fee-rate integer retained |
| DLMM `pow_q64` | Underflow/overflow replaced by zero/one/`u128::MAX` | Unrepresentable result rejects with `MathOverflow` |
| Engine amount conversion | Clamped `u128` input/bin amounts into `u64` | Checked conversion; out-of-domain quote rejects |
| CLMM ceil helper | `saturating_add(1)` | `checked_add(1)` and typed overflow |

## Confirmed remaining risks (not marked fixed)

- Raydium CLMM is not represented by an independent native `arb-dex` adapter. The existing shared CLMM math does not prove its account layout, bitmap traversal, dynamic fee behavior or Token-2022 V2 behavior.
- Current `parse_tick_array_account` is Whirlpool-shaped (88 ticks, 9,988-byte account). Do not use it for Raydium CLMM TickArrayState.
- Ingest currently fetches only three neighboring Orca tick arrays and three Meteora bin arrays. Large exact-in swaps may need additional arrays.
- `find_next_tick_boundary` and initialized-tick crossing semantics need golden tests against the deployed Orca/Raydium programs. Do not change inclusive/exclusive boundary semantics from intuition alone.
- DLMM dynamic-fee math, reference decay, volatility accumulator update order, protocol fee split and rounding still require differential verification against current Meteora SDK/program source and fixtures.
- DLMM quote view copies a route-local bin window. This is not a global cache clone. The update path must remain per-account/per-pool and avoid cloning the entire market cache.
- `quote_pool` still returns `0` for unsupported/failed CLMM/DLMM quotes because its public API is scalar. Phase 09B must replace this with a typed `Result<Quote, QuoteError>` so a failed quote cannot be confused with a legitimate zero output.
- The current CU/account-count estimates in `arb-engine::adapters` are ranking heuristics, not measured instruction costs.

## Required tests before acceptance

1. Golden account fixtures for Orca TickArray, Raydium TickArray and TickArrayBitmapExtension, Meteora LbPair and BinArray.
2. Differential exact-in and exact-out quotes across direction, tick/bin crossings, fee boundaries and Token-2022 fees.
3. Invariants: no reserve creation, no negative reserve, no overflow, no unconsumed input hidden as success, no zero-progress loop, and exact input/fee accounting.
4. Replay from the same account snapshot must be deterministic and produce identical quote outputs.
