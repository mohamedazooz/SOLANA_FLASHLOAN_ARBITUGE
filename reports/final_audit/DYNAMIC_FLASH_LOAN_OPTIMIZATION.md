# Dynamic Flash Loan Sizing & Optimization (PHASE H)

## Requirement

No arbitrary economic defaults in authoritative decisions; sizing must maximize verified
CONSERVATIVE NET profit using real pool depth, verified liquidity, and all costs.

## Implementation (audited)

### 1. Two-layer sizing architecture

* `arb-engine/src/flash_loan_sizing.rs` — `calculate_optimal_flash_loan`: bounded candidate
  search over the size ladder (NOT binary search — DLMM profit curves are non-smooth across
  bins). Each candidate is chain-simulated through `simulate_three_leg_chain` with real
  per-bin liquidity (`get_leg_capacity`), per-hop slippage floors, and the exact Kamino fee.
* `arb-engine/src/meteora_size_optimizer.rs` — coarse-to-fine ladder refinement
  (`size_ladder.clamp(1,8)` rungs, geometric +1 growth) used by the live 3-leg pipeline.
* Exhaustive-search equivalence pinned: `phase17_two_pool_flash_arb_tests::optimizer_matches_exhaustive_search_on_a_bounded_domain`.

### 2. Constraints honored per candidate

| Constraint | Source |
|---|---|
| Kamino available liquidity | live reserve account (fail-closed when unverifiable) |
| DLMM depth | per-bin reserves from decoded BinArrays |
| Fee schedule | live LbPair parameters + Kamino `flash_loan_fee_sf` |
| Price impact / bins traversed | quote engine's real `bins_crossed` |
| CU | `estimate_route_cu` + per-BinArray allowance vs `COMPUTE_BUDGET_LIMIT_3HOP` |
| Tx size | measured-estimate vs 1232 (V0) / 4096 (V1) |
| Network + priority + tip | adaptive CU price (p75+25%) × dynamic CU limit + verified tip |
| Min conservative profit | config floor (TOML + `MIN_NET_PROFIT_USD` env) |

### 3. Objective

Optimizes NET profit (gross chain output − flash fee − DEX fees − network/priority/tip −
account costs), NOT gross output. Candidates below the conservative floor are rejected,
not clipped.

### 4. Hardcoded-value remediation this audit

* `priority_fee_micro_lamports: 1_000` → now `TipStrategy::adaptive_cu_price` (same source as the dispatch gate).
* `jito_tip_lamports: 0` → now `verified_tip_lamports` from the live tip verifier.
Both fixes remove ranking/gating divergence. See HARDCODED_VALUES.csv rows 1-2.

## Remaining classification (all justified)

* Protocol constants (70 bins, bitmap window 512, discriminators, program IDs) — kept.
* Safe engineering limits (CU 320k for 3-hop, packet 1232/4096, ALT 256 addresses) — kept.
* Runtime configuration defaults (size ladder, slippage 100 bps, min profit 30k lamports) — TOML/env overridable, documented, kept as defaults only.
