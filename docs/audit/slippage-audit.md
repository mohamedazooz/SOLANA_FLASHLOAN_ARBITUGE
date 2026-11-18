# Slippage Audit — P0 (2026-11-18)

Scope: directive §8 — a unified separation of price concepts and proof that
slippage is applied exactly once, as a protection floor, on fresh state.

## Price-concept separation (directive's 8 concepts)

| Concept | Where it lives | Audited behavior |
|---|---|---|
| 1. Spot price | `arb-dex` adapters, invariant-derived | used for pre-filtering / display only |
| 2. Mid price | spot consolidation for spread checks | display + pre-filter only |
| 3. Executable quote | adapter `quote(input)` per hop | the only number that flows into economics |
| 4. Expected execution price | composed route quote at B* | used for decision + min-out derivation |
| 5. Price impact | inherent in the curve quote (output net of trade) | never re-deducted |
| 6. Slippage tolerance | per-hop dynamic bound | protection bound, not cost |
| 7. Minimum output | `quote × (1 − tol)` per hop | passed into swap instructions |
| 8. Realized execution price | on-chain post-land balance deltas | recorded under MATH-01 fix (currently corrupted for losses) |

Each hop is quoted independently and composed: hop N+1 consumes hop N's actual
quoted output; no hop uses the first hop's price for downstream hops (audited:
composition is sequential output-based, no spot shortcuts) **[code]**.

## Double-counting checks (directive's 7 cases)

1. Quote net of impact + impact deducted again — **not found** (quotes are net; no
   second deduction exists; grep of cost ledger sites confirms) **[code]**.
2. Swap fee in quote + fee re-added — **not found** **[code]**.
3. Slippage deducted from expected output + min-out applies another deduction —
   **not found**: min-out derived once from quote × tolerance **[code]**.
4. Input-side vs output-side fee mismatch vs protocol — matches per-protocol
   reality (Pump per-direction bps; Raydium numerator/denominator; CLMM
   both-legs). **[code]**
5. Spot price used as executable quote — **not found** on the executable path
   (spot only pre-filters). **[code]**
6. First-hop price reused for second hop — **not found**. **[code]**
7. Decimals divergent between modules — not found; decimals carried in snapshot
   structs end-to-end. **[code]**

## Protections

- Execution refuses when: impact exceeds bound, expected slippage exceeds
  tolerance, pool state stale (`quote_readiness` freshness gate, fail-closed),
  or min-out cannot be represented by the protocol instruction.
- Dynamic tolerance: pool-type-aware and volatility-aware via config; not a
  global constant on the production path.
- Simulation does **not** refresh state; a passing sim is never proof of future
  profitability (semantics enforced in dashboards/labels; see full-pipeline doc).

## Open items

- MATH-01 (P0): realized execution price (concept 8) currently cannot hold
  losses — fix tracked in accounting PR.
- Fuzz/regression tests for "zero-liquidity malformed pool → min-out rejects"
  and property `min_out ≤ quoted_out` are specified; implemented in the P1/P2
  test phase (P10).
