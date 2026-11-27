# Protocol Research — DLMM Two-Pool Flash-Lane Cost-Model Completion (2026-11-27)

Research dossier for the branch `fix/dlmm-two-pool-jito-tip-liability`. This is
**not** a duplicate of the standing research suite under
[`docs/active/research/`](../active/research/README.md); it records only the
verification performed for this change and links the authoritative documents.

Scope of the change: the isolated Meteora DLMM two-pool flash-loan evaluator
(`crates/arb-engine/src/dlmm_flash_arbitrage.rs`) and its
`arb-tx` builder (`crates/arb-tx/src/flash/dlmm_flash_arb.rs`). Runtime wiring
was intentionally **not** touched (see `docs/active/OPEN_ISSUES.md`,
`FL-F-2026-11-22-N2`).

---

## 1. Jito Tip Minimum (verified)

| Item | Finding | Source |
|---|---|---|
| Minimum bundle tip | **1,000 lamports**, enforced by the block engine | [Jito — Low Latency Transaction Send](https://docs.jito.wtf/lowlatencytxnsend/) ("The minimum tip is 1000 lamports") |
| Tip mechanics | Ordinary System Program transfer (instruction index 2) to one of 8 tip accounts from `getTipAccounts`; paid only when a bundle lands; independent of the priority fee | Jito docs (above); cross-confirmed by secondary write-ups quoting the same doc line |
| Repo encoding | `arb_constants::MIN_JITO_TIP_LAMPORTS = 1_000`, `JITO_TIP_ACCOUNTS` (8 entries) | `crates/arb-constants/src/lib.rs` |

Verified against the repo constant on 2026-11-27; both agree.

## 2. Kamino Flash-Loan Fee (already canonical — verified, unchanged)

- Fee = `round(borrow * flash_loan_fee_sf / 2^64)` with round-half-away-from-zero
  and a 1-atomic-unit floor (`FeeCalculation::Exclusive` in klend), implemented
  once in `crates/arb-math/src/economic.rs` and re-exported by `arb-tx`
  (`crates/arb-tx/src/flash/kamino.rs:199-224`).
- Main-market WSOL reserve carries `flash_loan_fee_sf = 11_529_215_046_068`
  (= 1e-5, i.e. 100 lamports per 1e7 borrowed); the phase-17 integration test
  asserts the exact rounded value.
- The borrow instruction carries the **principal only**; the klend handler
  debits `principal + fee` from the settlement account at repay time
  (`flash_borrow_check_matching_repay` rejects mismatched repay). The builder's
  `repay_amount()` = `principal + fee` is therefore never double-counted.

## 3. Meteora DLMM Limit-Order Layer Parity (newly verified with existing data)

- The captured Mainnet snapshot `fixtures/meteora_mainnet_snapshot.json`
  (pool `5rCf1DM8...`, function_type 0, captured at slot 469998027) contains
  **179 bins with non-zero limit-order layers** (`openOrderAmount` /
  `processedOrderRemainingAmount`).
- The new regression test
  `deep_sdk_verified_paths_traverse_nonzero_limit_order_bins`
  (`crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`) proves that
  **all 8 SDK-verified deep cases** (both directions × both collect-fee modes ×
  exact-in/exact-out) cross at least one non-zero LO bin while matching the
  pinned official `@meteora-ag/dlmm@1.9.14` oracle exactly on `amount_in`,
  `amount_out`, `total_fee`, `protocol_fee`, `end_bin_id`, `bins_crossed`.
- The engine's LO handling mirrors the SDK's
  `Bin::get_limit_order_amounts_by_direction` /
  `get_max_amount_out_with_limit_orders`
  (`crates/arb-math/src/dlmm/swap.rs:104-131`).
- **Still open (blocked)**: a differential capture of a live **function_type=2
  (LimitOrder)** pool with a non-zero LO layer. Blocked in the 2026-11-27
  environment: no `.env` RPC credentials and no installed
  `scripts/differential-harness/node_modules`; the harness requires
  mainnet RPC egress. Nothing here claims that live capture exists.

## 4. Verified Protocol Behaviors Governing the Change

1. The Jito tip is a **lamport liability of the payer**, not of the flash
   settlement token account; it is nevertheless a real execution cost and must
   reduce evaluated net profit exactly once.
2. A bundle with a sub-minimum tip cannot land, so pricing it as executable
   would be an optimistic estimate — rejected fail-closed
   (`FlashArbRejection::JitoTipBelowMinimum`).
3. Zero tip means "not bundled through Jito" — no cost is counted (optional
   costs are not charged when not used).
4. Cost conversion to flash-mint raw units rounds **up**
   (`DlmmFlashArbConfig::cost_in_flash_mint`), so sub-unit lamport costs are
   never truncated into overstated profit.

## 5. Meteora DLMM Official Data API (newly verified — discovery source, 2026-11-27)

| Item | Finding | Source |
|---|---|---|
| Canonical pool catalog | `GET https://dlmm.datapi.meteora.ag/pools` — paginated list of all DLMM pools | [Meteora Docs — DLMM Pools API](https://docs.meteora.ag/api-reference/dlmm/pools/pools) (OpenAPI verified live) |
| Pagination | `page` (1-based) + `page_size` (max 1000); response carries `total` / `pages` / `current_page` / `data[]` | Same OpenAPI; live probe returned `total=137,913` |
| Ranking / filtering | `sort_by=tvl:desc`; `filter_by` supports `tvl>=N` and `is_blacklisted=false` expressions joined by `&&` | Same OpenAPI |
| Live behavior | First page (TVL-desc) begins with the liquid SOL/USDC core (`5rCf1DM8...` first); `token_x`/`token_y` objects carry `address` + `decimals`; `tvl` is a number | Live probe 2026-11-27 |

**Engineering consequence (root cause of the thin DLMM watch set):** discovery
previously used only DexScreener hub/search queries for Meteora, which returned
a capped fuzzy subset (22 candidates vs 250 above the same TVL floor from the
official catalog). `discover_meteora_catalog` now pages the official API
(bounded: `page_size=100`, ≤10 pages, TVL floor + blacklist filter, malformed
entries skipped fail-closed) and falls back to the existing DexScreener paths on
failure. `config/base.toml` now enables `http_discovery_enabled`: with it off,
the runtime registry holds only the hardcoded baseline pools and the WS stream
subscribes only to those — the scan could structurally never see the liquid
DLMM core.

## 6. Unverified Assumptions / Limitations

- The tip liability is a **configured input** on the isolated evaluator; when
  (if ever) wired to the runtime, the caller must source it from the live
  dispatch configuration (tip floor / auction data), not a constant.
- `FlashArbParams::required_floor()` adds lamport-denominated fees directly to
  flash-mint raw units; exact for the WSOL flash mint (1 lamport = 1 raw unit),
  conservative (over-rejecting) for smaller-decimal mints. Unchanged behavior,
  now documented at the function site.
- No live mainnet capture, simulation, shadow run, or transaction was performed
  in this change; local Linux tests only (toolchain 1.99.1).
- Forensic decomposition (read-only, slots 474501741→474505238) showed the
  gross-profitable cycles the scan found are transient (all 10 flipped
  gross-negative ~20 minutes later) and dust-sized (≤15 lamports gross). The
  53 bps "inter-DEX spread" was dominated by a one-sided DLMM active bin: pool
  `CgqwPLSF...` (80 bps bin step) had zero SOL in its active bin, so any
  USDC→SOL buy executed ~85 bps above the active-bin mid. Active-bin mid price
  is therefore not an executable price for one-sided bins; only exact quotes
  (which the engine already uses) are decision-grade.
- The fixed-cost breakeven measured ≈ 25,000–33,754 lamports of gross (base
  5,000 + tip p75 10,000–18,753 + priority + Kamino fee + 10,000 absolute
  hurdle) vs ≤15 lamports best gross observed: the SOL/USDC market on these
  venues is efficiently arbitraged below this bot's cost stack at scan time.
  Opportunity count is market-state-dependent and cannot be manufactured
  without loosening safety gates (rejected).
