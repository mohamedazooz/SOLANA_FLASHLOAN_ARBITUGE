# PR24 Forensic Repair Log — Kamino × Meteora DLMM Pipeline

Branch: `fix/pr24-dlmm-flashloan-forensic-repair` · PR: #24 (draft → ready)
Policy: READ-ONLY network validation. No financial broadcast performed.

## Round 3 — §2.2 / §2.3 / §4 / §5 / §6 (commit d38a3cd + this commit)

### §2.3 Conservative sequential chaining — FIXED (P0)

**Root cause.** `quote_chain` (crates/arb-engine/src/three_leg_meme_pipeline.rs) chained
leg N+1 at leg N's OPTIMISTIC output (`amount = quote.out[i]`). On-chain, leg N only
guarantees its `min_amount_out` floor; any shortfall between expected and floor would
have been silently bridged from wallet inventory.

**Fix.** Conservative chaining: `in2 = floor(out1·(10000−s)/10000)`, requote leg 2 at
`in2`, `in3 = floor(out2·(10000−s)/10000)`, requote leg 3 at `in3`. Slippage applied
exactly once per hop boundary (no double count). `ChainQuote` now carries `inputs[]`;
`build_leg_records` consumes them. Net profit, gross profit, and the repayment floor
are all computed on the conservative chain.

**Tests.** `quote_chain_inputs_are_conservative_min_outs` (leg-record level),
pipeline contract test updated to assert `legs[1].amount_in == floor(legs[0].amount_out)`.

### §2.2 Pre-existing inventory subsidy — FIXED (P0)

**Root cause.** `capture_three_leg_dlmm` built params via
`ThreeLegMemeContextParams::for_discovery(payer)`, hardcoding
`meme_a_ata_balance = 0, meme_b_ata_balance = 0` without ever fetching the payer's
real ATA balances. A pre-existing intermediate or WSOL balance could subsidize a leg
shortfall or the flash-loan repayment, breaking the proceeds-only solvency invariant.

**Fix (directive "Alternative" fail-closed provenance policy).**
1. Pipeline: new `wsol_ata_balance` param + `PreExistingInventorySubsidy` error.
   `build_context` rejects any non-zero WSOL/meme-A/meme-B balance.
2. Live capture: `capture_three_leg_dlmm` is now async and fetches the payer's REAL
   balances via `getMultipleAccounts` (SPL/Token-2022 amount at byte offset 64..72 LE):
   - missing account → provably 0 (pass);
   - undecodable account → `u64::MAX` (fail-closed reject);
   - RPC fetch failure → reject (zero-balance unprovable);
   - any non-zero → reject with a `warn!` carrying all three balances.
   ATAs derived with the mint's actual token program from the pool's X/Y slots.

**Tests.** `build_context_rejects_pre_existing_inventory` (wsol/meme_a/meme_b ×123
rejected; all-zero passes to the next gate).

### §4 Bitmap extension — FIXED (P0)

**Root cause.** registry.rs derived the extension PDA and populated
`Some(extension)` unconditionally, without proving the account exists. Passing a
nonexistent writable PDA fails the swap2 on-chain; conversely a required-but-absent
extension was never detected downstream.

**Fix.**
1. Registry: the extension PDA is fetched in the same derived-account batch;
   `Some(ext)` is populated ONLY when the account actually exists on-chain
   (`derived_data_map.contains_key`), else `None` (official program-ID sentinel).
   New helper `bitmap_extension_pda` (seeds `[b"bitmap", lb_pair]`, official SDK).
2. `build_context`: when `derive_directional_bin_arrays` reports
   `bitmap_extension_required` (any selected array index outside `-512..=511`),
   a missing extension now fails closed with `BitmapExtensionRequired { leg }`
   instead of silently passing the sentinel.

**Tests.** `build_context_fails_closed_when_bitmap_extension_required_but_absent`
(out-of-range active bin 35 840 → array 512 → rejected without extension; passes
with an existence-proven extension).

### §5 X/Y ordering + token programs — FIXED (P0)

**Root cause (two-sided contract violation).**
- `SwapHop.token_programs` is documented as `(mint_in, mint_out)`, but the 3-leg
  capture producer passed `pool.token_programs` = `(base, quote)` — reversed for any
  leg trading quote→base.
- `build_swap_hop_instruction` (DLMM branch) mapped `token_programs[0]/[1]` straight
  into the swap2 `token_x_program`/`token_y_program` slots — inverted for every
  Y→X hop. Harmless while both sides are SPL Token, fatal for any Token-2022 side.

**Fix.**
- Producer (run.rs): flip to `[in, out]` when `leg.from_mint == pool.quote_mint`.
- Consumer (swap_hop.rs): derive `(token_x_program, token_y_program)` from the hop
  direction: `swap_for_y ? (in, out) : (out, in)`.

**Tests.** `dlmm_reverse_hop_maps_programs_to_pool_xy_slots` (Y=2022 → slot 11 = SPL,
slot 12 = 2022) and `dlmm_forward_hop_maps_programs_to_pool_xy_slots`.

**Verified consistent producers (no change needed):** MarketGraph edges
(crates/arb-engine/src/lib.rs:494,518 — reverse edge already flips),
Bellman-Ford (bellman_ford.rs:457), multi-hop (lib.rs:729/740/972/985/998),
opportunity.rs:670 (copies first hop).

### §3.1 Bin-array planner matrix — AUDITED (no defect)

| Planner | Crate | Runtime use | Semantics |
|---|---|---|---|
| `derive_directional_bin_arrays` | arb-tx/meteora_dlmm | 3-leg pipeline (live) | active array first, direction-walk, ≤2-array policy, bitmap flag |
| `plan_bin_arrays` | arb-tx/dlmm_bin_discovery | 2-pool flash arb (`dlmm_flash_arbitrage`) | exact first→last index walk, no arbitrary ceiling |
| `dlmm_bin_array_pdas` | arb-ingest/registry | ingestion snapshot window (~500 bps size-aware) | fetch warm-up only; build_context ignores it (`bin_array_pdas: _`) |
| `meteora_bin_array_indices` | arb-tx/meteora_accounts | legacy + parity reference in differential tests | equal to `plan_bin_arrays` (asserted by phase17_pda_differential) |
| `bin_array_index_for_bin` ×4 (arb-dex/arb-math/arb-tx/arb-ingest) | all | all `div_euclid(70)` | deliberately independent implementations cross-verified by `phase17_pda_differential` + `dlmm_bin_array_boundary_tests` (incl. -35910→-513, 35770→511) |

The duplication is intentional for differential-test independence; all four index
functions are proven equivalent by cross-crate tests. The runtime 3-leg planner
consumes the quote's actual `bins_traversed` (quote depth == execution depth).

### §6 Depth-based sizing — AUDITED (live wiring confirmed)

- 2-pool DLMM: `optimize_size` (cross_pool_dlmm.rs:235,421) — live, coarse+fine basin search.
- 3-leg: pipeline size search over `size_search_sizes` (run.rs ladder, capped by
  `max_sizing` and `tuning.max_input`, filtered by `kamino_available_liquidity`) — live.
- Per-leg depth: candidates exceeding executable depth quote to zero/fail → dropped;
  graph-edge capacity uses multi-bin `dlmm_multi_bin_max_input` (commit 2bae863).
- `calculate_optimal_flash_loan` (flash_loan_sizing.rs) is NOT on the live path:
  exported for the independent-reference tests (`three_leg_net_economics_tests`).
  It is evidence/reference code, not dead silent authority — the live sizing is the
  pipeline search above.

## Test totals

- `cargo test --workspace --all-targets`: **1003 passed, 0 failed**.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.

## Evidence classification

- Unit/integration tested: all fixes above.
- Mainnet-read tested: registry existence-proof path exercises live RPC batches
  during normal (read-only) monitoring runs.
- Financial broadcast: NONE (policy: read-only).
