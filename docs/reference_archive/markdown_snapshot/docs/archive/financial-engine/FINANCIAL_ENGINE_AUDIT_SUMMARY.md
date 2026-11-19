# Financial Engine Audit Summary

**Status:** F0 audit complete; F1 implementation not yet applied.

Highest-severity confirmed issues in the current source:

1. `min_profit_raw` is documented as base-mint raw units but converted as Lamports.
2. Negative net PnL returns an arithmetic error instead of a signed loss.
3. `minimum_final_amount_raw` is not used in the execution profit gate.
4. SOL cost conversion floors liabilities and silently changes zero denominator to one.
5. CostLedger sums Lamports and TokenRaw in one untyped aggregate.
6. Pump.fun fee is hardcoded to 100 bps; current official bonding curve schedule is 125 bps, PumpSwap is tiered and requires a separate adapter.
7. Kamino fee reader still uses direct HTTP JSON-RPC in production paths, contrary to the agreed Yellowstone/WebSocket market-state architecture.
8. `live_single_exec` has a fixed size fallback, fixed minimum size, wrong notional basis, a fallback tip, and discards RiskGate's result.
9. Chronos freshness survival is treated as success probability without calibration or separate landing/failure states.
10. Jito bundle acknowledgement, bundle ID, transaction signature and finalized settlement need separate handling.

No Rust code was changed in this F0 audit package. This is intentional: changing signed-PnL types, unit conversions and quote-gate semantics touches multiple crates and must be done as one coordinated F1 migration with all call sites and tests updated. Applying isolated substitutions would create a false sense of correctness.
