# Three-Pool Meteora DLMM Flash Arbitrage — Net Economics + V1 Builder Hardening (2026-11-12)

Branch: `feat/v1-meteora-three-pool-flash-arbitrage`

## Scope

This change closes the evidence-based gaps found in the repository forensics pass
against the V1 three-pool Kamino-flash-funded Meteora DLMM directive. Existing
working components were reused; no duplicate quote engine, RPC provider,
transaction builder, or economic calculator was introduced.

## Findings and fixes

### F1 — Build broken on `main` (critical, fixed)

`cargo check --workspace --all-targets` failed on `main` HEAD (exit 101):

* `crates/arb-tx/src/cross_dex_flash_arb.rs` (test code): `WSOL_MINT` and
  `TOKEN_PROGRAM_ID` referenced without imports (20 × E0425).
* `crates/arb-tx/src/dlmm_4leg_flash_arb.rs` + `cross_dex_flash_arb.rs` (test
  code): `KaminoFlashParams::default_wsol()` referenced but never defined
  (5 × E0599).

Fix: added the missing test-module imports and a `default_wsol()` constructor
alias over the verified WSOL main-market `Default`.

### F2 — Kamino flash fee truncated (economic defect, fixed)

`arb_engine::flash_loan_sizing::calculate_kamino_flash_fee` truncated the
U68F60 product instead of round-half-away-from-zero, understating the fee
(9,999 vs 10,000 lamports per 1 SOL at the main-market WSOL rate). The
authoritative implementation lives in `arb_tx::kamino::calculate_kamino_flash_fee`
(klend `FeeCalculation::Exclusive`, verified against a 256-bit reference and
live mainnet accounts). The duplicate now delegates to it; the pre-existing
unit test that caught the divergence (`test_flash_fee_calculation`) passes.

### F3 — Available flash liquidity never validated (critical, fixed)

The directive requires `borrow_amount <= actual_available_flash_liquidity`
decoded from the real reserve supply vault. Previously:

* The WSOL supply vault was tracked via WebSocket
  (`supervisor/run.rs` `track_account`) but its balance was never decoded.
* `dlmm_flash_arbitrage.rs` carried a hardcoded
  `kamino_available_liquidity: 1_000_000_000` config default.

Fix:

* `arb_tx::kamino::decode_supply_vault_liquidity` — pure, fail-closed SPL
  Token/Token-2022 token-account balance decoder (mint-verified, layout
  offsets 0/32/64).
* `Supervisor::kamino_available_flash_liquidity` — reads the live Yellowstone
  snapshot of `KAMINO_WSOL_SUPPLY_VAULT`, enforces owner = SPL Token program,
  slot freshness (same 1,500-slot bound as the verified quote inputs), and
  decodes the balance. Missing/stale/undecodable → `None` (ceiling falls back
  to the configured ladder; the live path re-validates before dispatch).
* `ThreeLegMemeContextParams.kamino_available_liquidity: Option<u64>` — the
  3-leg size search drops every candidate above the ceiling and fails closed
  with `InsufficientFlashLiquidity` when no candidate remains.

### F4 — Size search optimized GROSS profit (economic defect, fixed)

`build_three_leg_meme_pipeline` ranked candidate borrow sizes by gross output
(`final_out - input`), ignoring the Kamino flash fee and network costs. The
standalone `flash_loan_sizing` helper that does model costs was dead code
(never called).

Fix: the 3-leg size search now maximizes NET profit:

```
net = gross − calculate_kamino_flash_fee(B, fee_sf) − network_cost_lamports
```

with checked `i64` conversions (`u64::MAX` disabled-reserve sentinel can no
longer wrap into a negative, profit-increasing value). Sizes below the
configured `min_profit_lamports` net floor fail closed
(`BelowNetProfitFloor`). New result fields: `net_profit_lamports`,
`flash_fee_lamports`; `ThreeLegMemeSizeSearchResult.tested_sizes` now carries
net values. `simulate_profitability_gate` gates on net.

### F5 — V1 builder accepted mismatched payer/signer (critical, fixed)

`build_flash_funded_three_pool_transaction` assembled instructions for
`params.payer` but signed with `signer`. When the two differ, the V1 message
requires two signatures (sanitizer rejection) and the V0 path would sign a
message the key cannot verify. Fix: fail-closed
`FlashArb3LegError::PayerSignerMismatch` guard.

### F6 — Supervisor wiring

`Supervisor::capture_three_leg_dlmm` now receives the slot-verified Kamino
`flash_loan_fee_sf` (from `build_verified_inputs_from_snapshots`), the live
supply-vault ceiling, and a conservative network-cost estimate
(base fee + verified Jito tip ceiling + adaptive priority-fee estimate at the
3-hop flash-funded CU shape). The pipeline's profitability gate therefore
reflects the same cost model as dispatch.

## V1 transaction format (unchanged, re-verified)

The existing adaptive compiler already implements the required V1 wire format
and is now covered by the new test suite:

* `0x81` discriminator at byte 0, trailing signatures.
* SIMD-0160 (≤ 64 instructions) and SIMD-0385 (≤ 64 inline addresses) enforced.
* 4,096-byte MTU enforced on the message and again on signed wire bytes.
* No ALT in V1; no legacy ComputeBudget instructions in V1 messages.
* CU limit / total-lamports priority fee / loaded-accounts data limit encoded
  in the message config (mask + values), not as legacy instructions.
* V1 is selected only when `v1_transport_supported`; otherwise V0+ALT within
  1,232 bytes, and oversize is rejected, never silently downgraded.

## Tests added (all executed and passing)

`crates/arb-tx/tests/three_pool_flash_arb_tests.rs` (17 tests):

* instruction ordering (2 CU + 3 ATA + borrow@5 + 3 swaps + repay@9 + tip),
  repay amount = borrow + fee, repay carries the real borrow index
* fail-closed: duplicate pools, broken mint chain, non-closing route,
  insufficient final/mid output, zero borrow, payer/signer mismatch
* V0+ALT path fits 1,232 bytes and parses as V0
* V1 inline route > 1,232 and ≤ 4,096 bytes, parses as V1, `0x81` at byte 0,
  Ed25519 signature verifies
* SIMD-0385 address-limit rejection (> 64 inline addresses)
* 4,096-byte boundary accepted exactly; 4,097 rejected
* V1 priority fee encoded as total lamports (micro-lamports/CU converted with
  ceil), no legacy ComputeBudget instructions in the V1 message
* supply-vault liquidity decoder roundtrip/mint-mismatch/short-account
* klend Exclusive fee regression (10,000 lamports per 1 SOL, not 9,999)

`crates/arb-engine/tests/three_leg_net_economics_tests.rs` (8 tests):

* net = gross − flash fee − network cost (exact identity)
* size search ranks by net; best size is the net argmax
* gross-positive/net-negative route fails closed
* disabled-reserve fee sentinel (`u64::MAX`) fails closed (catches the
  `u64::MAX as i64` wrap hazard)
* candidates above available flash liquidity rejected; ceiling clamps the
  tested ladder
* profitability gate operates on net profit
* standalone sizing helper charges the exact klend fee

`crates/arb-tx/src/kamino.rs` (4 unit tests): supply-vault decoder.

## Commands executed (this branch)

* `cargo check --workspace --all-targets` — exit 0 (after F1 fix; exit 101 before)
* `cargo test --workspace --all-targets --no-fail-fast` — see commit message / CI
* `cargo clippy --workspace --all-targets -- -D warnings`
* `cargo fmt --all -- --check`

## Remaining limitations (explicit)

* V1 transport remains gated behind `v1_transport_supported` (default false).
  No mainnet broadcast was performed (ARB_LIVE=0); endpoint/validator V1
  compatibility must be established by the operator before enabling.
* The 3-leg capture path compiles V0 via the prewarmed-template pipeline;
  `build_flash_funded_three_pool_transaction` (V1-capable) is exercised by
  tests and ready for the operator-authorized live path.
* Fork simulation and live mainnet read-only observation require operator
  infrastructure and are not part of this change.

## Review round 2 (same day): ATA derivation + accounting audit — 7 findings fixed

Independent review pass over the accounting method and ATA handling found and
fixed the following defects. All fixes are on this branch, all tests re-run.

### ATA findings

1. **Wrong settlement ATA derivation** (`crates/arb-tx/src/dlmm_3leg_flash_arb.rs:577`):
   `get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_mint)`
   passed the WSOL *mint* where the *token program* belongs, producing a wrong
   PDA in `FlashThreeLegArbitrageTx.settlement_account` metadata (wire bytes
   were unaffected — assembly used the correct program). Fixed to derive with
   `leg_1.token_in_program()`, matching the 4-leg builder.
2. **Token-2022 intermediate ATAs in the live path** (`assemble_flash_arbitrage`,
   `crates/arb-tx/src/lib.rs`): intermediate ATAs were derived/created/closed
   with SPL-only helpers, ignoring `hop.token_programs` — a Token-2022
   intermediate would get a wrong PDA and the swap would fail on-chain. Fixed:
   per-mint program from `(mint_in, mint_out)` hop programs, using
   `get_associated_token_address_with_program_id`,
   `build_create_ata_idempotent_for_program`, and
   `build_close_token_account_for_program`.
3. **ATA rent not accounted** (`three_leg_network_cost_lamports`,
   `crates/arb-bot/src/supervisor/run.rs`): the WSOL settlement ATA is created
   when missing and never closed (it holds the profit), so its
   2,039,280-lamport rent is a real cost. It is now charged when the ATA is
   absent from the account snapshot store. Intermediate meme ATAs are created
   AND closed inside the same transaction (rent reclaimed in-tx) and are
   therefore not charged.

### Accounting findings

4. **`simulate_three_leg_chain` used `MintId::default()` placeholders for all
   mints** (`crates/arb-engine/src/flash_loan_sizing.rs`): `quote_single_hop`
   rejects mint mismatches, so every quote returned `None` and the final output
   was always zero — `calculate_optimal_flash_loan` could never produce a valid
   size. Fixed: route mints are now derived from the pool states anchored on
   the real WSOL mint, with cycle-shape validation (leg2 must bridge A<->B).
5. **Final output was leg 3's INPUT in mint-B units** — `out3` was discarded
   and `amounts[2]` (leg-3 input) was compared against the WSOL borrow. Fixed:
   the expected final WSOL output is captured and returned
   (`expected_final_output` on `FlashLoanSizingResult`).
6. **DEX fees double-counted**: DLMM quote outputs are already net of
   protocol/LP fees (fee taken from the swap input before bin traversal), but
   `total_costs` subtracted `dex_fees` again. Fixed: fees are informational
   only; net = conservative final floor − borrow − exact klend fee − network
   cost. Net is now computed on the slippage-haircut floor, never the
   optimistic expected output.
7. **Repayment guarantee enforced + single accounting source**: the pipeline
   now fails closed with `RepaymentNotGuaranteed` when the slippage floor of
   the final output cannot cover principal + flash fee; and a canonical
   `flash_loan_sizing::net_profit_lamports` is the single net-profit function
   shared by the size optimizer and the 3-leg pipeline (previously three
   parallel accounting models). The weak `sizing_helper_charges_exact_klend_fee`
   test was strengthened to assert quoted outputs and the full net identity,
   and a new `route_fails_closed_when_repayment_not_guaranteed` test was added.

Note: `crates/arb-engine/src/pnl.rs` is an orphan module (not declared in
`lib.rs`, therefore never compiled) — flagged for follow-up, not wired in this
change.

### Verification (this round)

* `cargo check -p arb-tx -p arb-engine -p arb-bot -p arb-execution --all-targets` — 0 errors
* `cargo clippy --workspace --all-targets -- -D warnings` — exit 0
* `cargo test --workspace --all-targets --no-fail-fast` — 954 passed, 0 failed
* `cargo fmt --all -- --check` — clean
