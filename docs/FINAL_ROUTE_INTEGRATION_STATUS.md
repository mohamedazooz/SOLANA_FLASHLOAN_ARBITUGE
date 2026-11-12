
## Follow-on route unification pass — 2026-11-11

See [`UNIFIED_MULTI_VENUE_ROUTE_ADAPTERS_2026-11-11.md`](UNIFIED_MULTI_VENUE_ROUTE_ADAPTERS_2026-11-11.md).

This pass adds a concrete `AdapterRouteGraph` over the nine `arb_dex::AmmPool` protocol adapters, in addition to the normalized `UnifiedRouteGraph`. It enforces mint continuity, cycle closure, no repeated pool in a route, bounded route depth, and per-pool adapter quoting. The normalized quote registry now has explicit adapters for every protocol variant and fails closed for venues whose complete state is not represented there.

Additional safety corrections: DLMM on-chain status and `index_reference` are preserved rather than synthesized; DLMM quote paths reject unsupported function/fee modes and approximate Token-2022 transfer-fee paths; Raydium V4 quotes use native fee numerator/denominator with ceiling and no fabricated default fee/status; Raydium CLMM fee rates outside the math domain are rejected rather than clamped; Meteora DAMM v2 compounding and unjoined fee contexts are rejected; LaunchLab no longer receives a fabricated 100 bps fee.

**Important integration boundary:** the new concrete adapter graph is available as an engine API, but the supervisor still consumes normalized `PoolState`/`ExecutionOpportunity` objects. Full supervisor-to-concrete-adapter object wiring and per-cross-venue transaction-builder acceptance remain open. No live route is enabled by this pass.

Rust verification remains unavailable in this environment (`cargo`, `rustc`, and `rustfmt` are not installed). The new source and regression tests are packaged, but not represented as compiled or passing.
