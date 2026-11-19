# Phase 3 — Meteora DLMM Forensic Audit (CORRECTED)

## 3.1 Protocol Identification — VERIFIED

| Item | Value |
|---|---|
| Program ID | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`) |
| `METEORA_BINS_PER_BIN_ARRAY` | `70` — must equal on-chain `BINS_PER_BIN_ARRAY`; test-verified |
| `METEORA_BIN_STEP` | Pool-specific; SOL-USDC pool step = 4 |
| `METEORA_SWAP2_DISCRIMINATOR` | `[65, 75, 63, 76, 235, 91, 91, 136]` |
| `METEORA_SOL_USDC_POOL` | `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6` (bin_step 4) |
| `METEORA_SOL_RESERVE` / `METEORA_USDC_RESERVE` | `EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o` / `CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz` |
| Compute budget for swap2 | `COMPUTE_BUDGET_LIMIT_2HOP = 260_000`, `MAX_COMPUTE_UNIT_LIMIT = 350_000` |

## 3.2 Bin Array Correctness — VERIFIED

- `bin_array_index_for_bin(bin_id)` uses `div_euclid(METEORA_BINS_PER_BIN_ARRAY)` → correct for negative bin IDs.
- `derive_meteora_bin_array_pda` uses seeds `[b"bin_array", lb_pair, bin_array_index.to_le_bytes()]` with **i64** index.
- **BinArray PDA derivation verified against official SDK** (`MeteoraAg/dlmm-sdk`); test mirrors SDK `pda.rs` semantics.
- Empty bins, zero liquidity bins, and missing arrays are rejected (test: `test_multi_bin_traversal_y_to_x`, `test_dlmm_active_bin_max_input`).
- **`bitmap extension` vs `default window`**: `BUG-001` (FIXED) — the bitmap extension requirement was wrongly inferred from array count instead of the official rule; now derived from `meteora_bin_array_index_requires_bitmap_extension` over every planned index (default window, per IDL `BIN_ARRAY_BITMAP_SIZE=512`). Two regression tests pass.

## 3.3 SDK Differential Testing — EXECUTED, NOT JUST THEORETICAL

`crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`:

```
mainnet_snapshot_prices_match_official_sdk                  ... ok
mainnet_snapshot_quotes_match_official_sdk                  ... ok
phase15_2_sdk_differential_manifest                          ... ok
```

Coverage:
- Collect-fee mode 0 (input only) and mode 1 (both legs): both tested per swap direction.
- Price impact, fee-on-input, variable fees, multi-bin traversal, and final active bin all match the captured on-chain snapshot (read 2026-11-13 through public mainnet RPC, slot 468,722,672).
- Per direction × per fee-mode quote comparison is exact at raw-unit granularity for matching inputs.

## 3.4 The 92% Safety Factor — LOCATION AND STATUS

`scale_dlmm_three_hop_input` in `arb-math/src/dlmm.rs` scales the input to `92%` of the bottleneck active-bin liquidity to avoid crossing into suboptimal bins. This is:
- an approximation, not a replacement for actual bin traversal;
- retained only as a conservative overlay after the full `quote_dlmm_multi_bin` traversal determines the bottleneck;
- NOT used to mask a quote mismatch (mismatches are explained, not haircut).

Per the directive's requirement to replace approximations only after differential proof: the 92% node is a legacy scaling heuristic layered on top of actual traversal; it is a candidate for removal/discussion, but removing it without an operator decision would alter live behavior. **Escalation to operator required** — do NOT change silently.

## 3.5 DLMM 3-Hop Sizing Bottlenecks

- Two-sided (`swap_for_y = true` and `false`) traversal both execute without double-counting fees (fee = `compute_fee_from_amount(amount_in, fee_rate)` on input only).
- `end_active_id` and `end_volatility_accumulator` are propagated correctly through traversals.
- `price_impact_bps` = `(ideal_out - amount_out) * 10_000 / ideal_out` with ideal computed from starting spot price in Q64.64.

## 3.6 Summary

### Correctness
DLMM math is Q64.64 fixed-point, no floating point. Bins traversal, fee dynamics, and volatility accumulator all validated. Meteora SDK differential testing confirms raw agreement on captured mainnet state.

### Outstanding

1. **92% scaling factor** — review tag in code; operator decision on removal/retention recommended. Not a correctness bug if the conservative overlay is preserved on top of the verified traversal.
2. **SDK shape without capture refresh** — fixtures are from the 2026-11-13 mainnet snapshot; if Meteora upgrades the program (Lemigator), re-capture.
3. **No standalone fuzzing for bin arrays** — property tests exist for economic invariants but not bin-array edge fuzzing at the extremes (-443636, +443636).

### Gate B contribution
PASS — differential testing executed against the official captured SDK outcome, and price equality verified in both directions and fee modes.
