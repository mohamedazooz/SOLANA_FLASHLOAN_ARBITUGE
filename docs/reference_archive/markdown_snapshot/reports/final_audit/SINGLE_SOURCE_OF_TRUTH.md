# Single Source of Truth (PHASE G)

## Authoritative implementations

| Concern | Single source | Duplicates found | Resolution |
|---|---|---|---|
| Net profit / ROI | `arb-math/src/economic.rs::calculate_net_profit_raw` | none (all callers delegate) | — |
| Kamino flash fee | `arb-math::economic::kamino_flash_fee_checked` (U68F60) | `arb-tx/flash/kamino.rs` re-export (delegate, no logic) | OK |
| BinArray index | `div_euclid(70)` | identical fn in arb-tx / arb-dex / arb-math | pinned byte-identical by `phase17_pda_differential.rs`; consolidation deferred to avoid cross-crate API churn (documented) |
| PDA derivation | `arb-tx/src/protocols/meteora_accounts.rs` | none | — |
| Bitmap Extension rule | `arb-constants::meteora_bin_array_index_requires_bitmap_extension` (NEW this audit) | was inlined wrongly in dlmm_bin_discovery (`indices.len() > 1`) | FIXED — all callers now use the constant |
| Priority fee | `arb-land::TipStrategy::adaptive_cu_price` | was hardcoded 1_000 in supervisor/run.rs | FIXED |
| CU estimate | `arb-tx::estimate_route_cu` + `calculate_dynamic_cu_limit` | per-BinArray static allowance is an input to it, not a duplicate | OK |
| Slippage | per-hop config (`slippage_bps`) → `min_amount_out` | none | — |
| Tx size | real serialization (`TxBuilder::compile_*` then measured `wire_bytes.len()`) | estimator exists ONLY as a pre-compile optimization gate; the measured size is authoritative | OK |

## Independent (test-only) implementations

* `audit/math_oracle/oracle.py` — Python, spec-derived, never imported by production.
* `crates/arb-math/tests/differential.rs` — property-based independent recomputation.
* `crates/arb-tx/tests/v1_header_differential.rs` — independent V1 encoder.

These remain intentionally separate to catch shared-bug correlation.

## Circular dependencies

None: `cargo modules`-style layering holds — arb-constants/arb-types (leaf) → arb-math →
arb-dex/arb-tx/arb-engine → arb-ingest/arb-sim → arb-bot. Grep-verified: no crate depends
on arb-bot.
