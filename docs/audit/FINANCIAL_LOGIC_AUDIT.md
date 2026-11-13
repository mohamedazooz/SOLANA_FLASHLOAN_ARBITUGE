# Financial Logic Audit — Kamino fee, sizing, net profit (2026-11-13)

Classification: **Verified** items ran in this campaign's test suite;
**Partially verified** items are code-inspected with partial test coverage.

## 1. Kamino flash-fee formula — Verified

Canonical owner: `crates/arb-math/src/economic.rs`
(`calculate_flash_fee_raw` / `calculate_flash_fee_checked`), re-exported by
`arb-tx::kamino`. Semantics (RAW bits, pre-rounding):

- `fee_bits = max(amount * origination_fee_sf, 2^60)` — the 2^60 floor is the
  Kamino minimum-fee semantic (1/2^60 fraction).
- `BorrowTooSmall` iff `fee_bits >= amount << 60` (fee would consume the
  principal) — checked variant, fail-closed.
- Rounding: `(fee_bits + 2^59) >> 60` — round-half-up on the RAW fraction.
- Disabled sentinel: `origination_fee_sf == u64::MAX` ⇒ fee disabled (0).
- Property test in arb-math enforces arb-tx/arb-math agreement (R3).

No hardcoded fee percentage exists anywhere in the execution path; the live
`origination_fee_sf` is read from the verified reserve snapshot.

## 2. Repayment enforcement — Verified

`supervisor/run.rs` (capture gate step 5):

```
total_required = size_in + flash_fee          (checked_add, overflow → reject)
expected_out >= total_required  else reject
```

The final on-chain guarantee is structural: Kamino `FlashRepay` is the last
instruction of the same atomic transaction; if the final balance cannot cover
repayment, the whole transaction reverts (no partial settlement).

## 3. Borrow sizing — Verified (bounded, non-monotonic-safe)

Owner: `arb-engine::three_leg_meme_pipeline` (3-leg) and
`arb-engine::MarketGraph::golden_section_size` (graph path).

- Candidate ladder built from `tuning.input_size` (×3 growth) bounded by
  `max_sizing` (risk-gate output) and `tuning.max_input`; web/Telegram
  overrides `min_borrow_lamports` / `max_borrow_lamports` clamp the ladder at
  the tuning boundary (`supervisor/run.rs`).
- Candidates ABOVE verified `kamino_available_flash_liquidity` are DROPPED,
  never clamped; if all candidates are dropped ⇒ `InsufficientFlashLiquidity`
  error (fail closed) —
  `three_leg_net_economics_tests::candidates_above_available_flash_liquidity_rejected`.
- Available liquidity source: actual supply-vault SPL balance
  (`decode_supply_vault_liquidity`), freshness-bounded (1500 slots), owner- and
  mint-checked (`supervisor/run.rs::kamino_available_flash_liquidity`).
- Objective is NET profit (gross − flash fee − network cost), never gross
  output. Non-monotonic safety: the 3-leg search evaluates an explicit discrete
  candidate set (bin transitions/dynamic fees make the quote function
  discontinuous; a blind binary search is not used there). The graph path uses
  golden-section search over a smooth CPMM-model curve, appropriate for that
  model.
- Network cost model (`three_leg_network_cost_lamports`): base fee + Jito tip +
  adaptive priority fee + WSOL settlement-ATA rent ONLY (intermediate meme
  ATAs are created and closed in-tx and reclaim rent). No double counting:
  DLMM fees are embedded in the curve quotes; the flash fee is added once.

## 4. Net-profit units — Verified

All amounts are raw integer token units in the flash-loan denomination (WSOL
lamports). USD conversion happens only at the reporting/override boundary via
the last verified live SOL price; a stale/zero price fails closed
(`min_profit_lamports` keeps its TOML floor).

## 5. Cross-pool route consistency — Verified

`quote_chain` passes each hop's actual output as the next hop's input (raw
integers); mint continuity is asserted by the graph DFS (from/to mint match)
and tested in `cross_dex_three_pool_live_loop.rs` (exact mint continuity +
per-protocol hops).

## 6. Partially verified / open

- Per-bin fee breakdown (base vs variable fee split per traversed bin) is
  modeled in `arb-math::dlmm` and differentially tested against SDK snapshots,
  but a live per-bin audit on a high-volatility pool has not been run in this
  environment (needs the ≥24 h shadow soak).
- Failure-related fees (failed-onchain tx cost) are counted in operational PnL
  via `risk_gate.record_loss`; a dedicated reconciliation of failed-tx fee
  totals against chain data is pending the soak.
