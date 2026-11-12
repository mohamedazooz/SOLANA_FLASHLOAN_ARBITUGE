# Financial Engine F1 — implementation record

Date: 2026-11-07  
Status: **source changes applied; compile/test not verified in this environment**  
Calculation model: `ECONOMIC_CALCULATION_VERSION = 2`

## Changes applied

1. `EconomicQuote` now preserves signed expected PnL and signed conservative minimum-output PnL (`i128`). A losing quote is a valid economic result, not an arithmetic overflow.
2. `EconomicQuote::is_profitable()` now requires the conservative minimum-output net PnL to exceed zero and the configured minimum-profit floor.
3. `min_profit_raw` is treated as base-mint raw units, consistent with the public type, strategy config, supervisor comments, and existing call sites. It is no longer incorrectly converted as if it were lamports.
4. Lamport liabilities converted into base-mint units use checked ceiling division. A zero numerator or denominator rejects the quote; no `.max(1)` denominator repair remains in this conversion path.
5. The cost ledger totals are unit-scoped (`Lamports` or `TokenRaw`) and use checked addition. Mixing unlike raw units is no longer possible through this aggregate API.
6. Input validation rejects unsupported decimals and a minimum output larger than expected output.
7. Calculation version incremented to 2 because execution eligibility semantics changed.
8. `live-single-exec` was removed from Cargo's executable targets and its source labelled quarantined. The old diagnostic still contains HTTP JSON-RPC reads, hardcoded fallback sizing/tip, and a risk-gate result that was ignored before transaction assembly. It must not be run or represented as a production executor.

## Important compatibility note

`net_profit_raw` and `gross_profit_raw` remain non-negative compatibility projections for existing reporting consumers. They are **not** the authoritative decision values. New code must use:

- `expected_net_profit_signed_raw`
- `minimum_net_profit_signed_raw`
- `gross_profit_signed_raw`
- `is_profitable()`

A zero compatibility projection can mean a break-even or losing trade; inspect the signed fields to distinguish them.

## Tests added

- Negative expected and min-out PnL is retained as signed loss and rejected.
- Minimum-profit floor is not converted from lamports.
- Zero conversion denominator fails closed.
- Liability conversion rounds upward.

Existing unit tests were updated for the unit-specific cost-ledger API.

## Not yet completed

- Full migration of every execution decision to one `EconomicQuote` instance. The supervisor still has a separate compatibility calculation path and must be unified in F2.
- Token mint identity is not yet carried by every `TokenAmount` / `CostEntry`; F1.2 must introduce denomination-aware amount types.
- `calculate_net_profit_raw` remains a legacy unsigned compatibility helper; callers should migrate to the signed evaluator.
- Kamino and mint account reads still have HTTP JSON-RPC call sites in legacy helpers; they have not been migrated to Yellowstone-only data.
- PumpSwap adapter, Pump fee tiers/creator fee/holder reward handling, full Raydium CLMM and Meteora DLMM account dependency traversal, Jito landing reconciliation, and the complete execution state machine remain unfinished.

## Verification limitation

This runtime has no `cargo`, `rustc`, `rustup`, or Docker executable. The source changes have not been compiled and the Rust test suite has not been run. Do not enable live execution based on this package. Required host-side gate:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Then run protocol fixtures and shadow-mode replay before any controlled live canary.
