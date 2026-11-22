# Flash Loan Forensic Audit — Phase 2 (Mission §7)

- **Date:** 2026-11-22
- **Scope:** every flash-loan execution path in `crates/arb-tx/src/flash/`, `crates/arb-tx/src/primitives/`, `crates/arb-bot/src/supervisor/`, and `programs/arb-executor/`.
- **Method:** source-of-truth-is-code. Kamino semantics verified against a sparse clone of the official `Kamino-Finance/kleng`/`klend` repository (handlers + `klend-interface`), discriminators recomputed with `sha256(anchor_discriminator(name))` via python3 — not taken from repo comments. Solana/kLend cites: program `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`.
- **Verdict summary:** the borrow/repay instruction stream is byte-semantically correct against the official interface; the fee formula, borrow-index rewrite under V1 CB stripping, and on-chain delta guards are verified. One latent economic-floor gap was found and fixed (FL-F-2026-11-22-N1). ISSUE-P0-04 (0x1773) remains open — a reproduction requires a real captured failure, which does not exist yet.

---

## 1. Verified lifecycle (19 steps)

| # | Step | Implementation | Verification evidence | Status |
|---|------|----------------|----------------------|--------|
| 1 | Account preparation | `kamino.rs` builds the 12-account borrow/repay lists | Account order, writability flags and optional-referrer placeholder (readonly program id when `None`) match official `klend-interface` `optional_account(..., is_writable: false)` | VERIFIED |
| 2 | ATA creation | `builder.rs::assemble_*` / `dlmm_flash_arb.rs` `build_create_ata_idempotent_for_program` | Token-program-aware idempotent create (`Phase_1F` doc); settlement selection in `dlmm_flash_arb.rs:358` keeps principal/repay on one ATA | VERIFIED |
| 3 | Reserve validation | `decode_reserve_flash_fee_sf` etc. (`kamino.rs`) | Byte offsets empirically pinned 2026-11-04/17: `flash_loan_fee_sf` @ `8+4896`, `status` @ `8+4848`, supply vault @ `8+152`; decoders fail closed on short accounts | VERIFIED |
| 4 | Liquidity validation | borrow sizing bounded by decoded live supply-vault liquidity | `arb-engine` rejects `BorrowAboveKaminoLiquidity` / `BorrowAbovePoolLiquidity` (`dlmm_flash_arbitrage.rs`); fail closed | VERIFIED |
| 5 | Borrow instruction construction | `kamino.rs:85 build_kamino_flash_borrow` | Discriminator = `sha256("global:flash_borrow_reserves")` first 8 bytes, matches `arb-constants` constants and official `flash_ixs.rs` | VERIFIED |
| 6 | Borrow index determination | runtime `instructions_sysvar` scan (top-level only) | Official `flash_repay_checks` requires the borrow to be top-level at `borrow_instruction_index`; matches klend sysvar semantics | VERIFIED |
| 7 | Swap construction | per-DEX builders behind `build_swap_hop_instruction` / `build_swap2_exact_in` | Covered by DEX matrix (Phase 3); DLMM leg builders verified in `three_pool_flash_arb_tests` differential vs CPI assembly | VERIFIED (Phase 3 continues) |
| 8 | Account ordering | `BTreeMap` dedup, payer first; executor CPI explicit order | `adaptive.rs` compile path; klend 12-account order pinned; `cross_dex_flash_arb.rs` validates route closure | VERIFIED |
| 9 | Token balance tracking | delta-based: on-chain `execute_three_leg` computes `wsol_baseline = wsol_initial − leg1_amount_in` | `programs/arb-executor/src/instructions/execute_three_leg.rs` + tests; intermediate inventory isolation `IntermediateInventorySubsidized` | VERIFIED |
| 10 | Repay construction | `build_kamino_flash_repay_with_index` | Repays PRINCIPAL only; official handler transfers `flash_loan_amount_with_referrer_fee` then debits `reserve_origination_fee` separately → repo comment "never pass principal+fee as repay amount" is correct | VERIFIED |
| 11 | Fee calculation | `arb_math::economic::klend`: `fee_bits = max(amount*sf, 2^60)`, ties-away rounding | Canonical single source; on-chain recomputes from live refreshed reserve, so off-chain fee must come from live decode (it does) | VERIFIED |
| 12 | Repayment verification | official `flash_repay_checks` | Repay must be top-level, same reserve (`accounts[3]`), `liquidity_amount == borrow amount`, `borrow_instruction_index <= current_index`. Consequence enforced: same-reserve coupling; any CPI repay is rejected on-chain by design | VERIFIED |
| 13 | Final balance verification | two independent guards: (a) executor CPI delta guard `wsol_final ≥ wsol_baseline + min_final_wsol_delta`; (b) `build_profit_guard_instruction` absolute balance `≥ borrow + fee + min_profit` | Guard order: delta check precedes repay | VERIFIED |
| 14 | Jito tip | `build_marked_jito_tip_instruction`, tip last, `jitodontfront` memo; 8 tip accounts; min tip 1,000 lamports engine-enforced | Matches Jito docs (see `docs/active/research/JITO_LANDING_RESEARCH.md`) | VERIFIED |
| 15 | Transaction compilation | `adaptive.rs::compile_v1_message` (V1) / V0 builder | CB strip + `index_remap` + rewrite of repay's `borrow_instruction_index`; **fail-closed** error if a required index pointed at a stripped CB ix. V1 rules per verified SIMD-0385 (0x81, 4,096B, ≤64 accounts, no ALT, no silent downgrade) | VERIFIED |
| 16 | Simulation | preflight (gated by `ENABLE_PREFLIGHT_SIMULATION`) + `arb-sim` LiteSVM harness | ISSUE-P0-04 forensic capture bundles wire bytes + logs under `reports/forensics/` | PARTIAL — sim gate env-dependent, by design |
| 17 | Signing | `build_signed_transaction` → wire bytes; MTU check 1,232B for V0 | Signer identity validated | VERIFIED |
| 18 | Submission | RPC + Jito sendBundle transport selection | Non-atomic Jito mode rejected by `atomic_integrity.rs` | VERIFIED |
| 19 | Landing reconciliation | signature/bundle reconciliation ≤5 ids / 5-min lookback; ambiguous dispatch ≠ failure, reconcile-before-retry, never re-sign | `arb-land` + Jito research records | VERIFIED |

## 2. Official-source verification (Kamino)

Performed in this phase against a fresh clone of the official repository:

1. **Account order (borrow/repay, 12 accounts):** matches `klend-interface` `flash_borrow_reserve_instruction` / `flash_repay_reserve_instruction`. Writability confirmed account-by-account.
2. **Optional referrer:** `None` → placeholder readonly account = klend program id (`optional_account(..., true)`). Repo behavior identical.
3. **Discriminators:** recomputed locally via `python3 -c "import hashlib; hashlib.sha256(b'global:flash_borrow_reserves').digest()[:8]"` — identical to `arb-constants`.
4. **Fee semantics:** fee is debited from the user ATA by the repaying handler, recomputed on-chain from the freshly refreshed reserve (`flash_loan_fee_sf`), `U68F60` fixed point with `max(amount*sf, 2^60)` floor. The off-chain encoder and the on-chain copy use the same canonical formula from `arb-math` (single source of truth).
5. **`flash_repay_checks` constraints** (official `handlers`, `flash_ixs.rs`) reproduced and mapped to repository behavior; consequence: V1 CB stripping without index rewrite would deterministically break every repay (InvalidFlashRepay) — the rewrite in `adaptive.rs:380–480` is mandatory and implemented fail-closed.

## 3. Subsidy exposure audit ("no pre-existing balance can disguise a loss")

| Path | Entry point | Economically floored? | Finding |
|---|---|---|---|
| `TxBuilder::assemble_atomic_bundle` (generic hop builder) | supervisor native dispatch | ✅ `derive_swap_legs` final hop: `min_out = max(slippage_floor, borrow+fee+tip+min_profit)`, rejects `expected < floor` | OK |
| `TxBuilder::assemble_flash_arbitrage` (cross-DEX) | `cross_dex_flash_arb::assemble_cross_dex_opportunity` | ✅ same | OK |
| `TxBuilder::assemble_arbitrage` (non-flash) | supervisor | ✅ same (fee = 0) | OK |
| `assemble_split_flash_arbitrage` | supervisor `cfg(feature = "split-routing")` | ❌ was slippage-only → **FL-F-2026-11-22-N1 — FIXED in this phase** | FIXED |
| `assemble_flash_funded_two_pool` (DLMM engine, `arb-engine/src/dlmm_flash_arbitrage.rs`) | **not reachable from `arb-bot`** (tests/examples only) | ⚠️ Invariant 6 enforces `leg_b.min_out ≥ repay (principal+fee)` but not `+min_profit`/tip; `ProfitBelowFloor` gates at quote time only | FL-F-2026-11-22-N2 (P3, unreachable; see below) |
| `assemble_flash_funded_three_pool` / `_four_pool` / route-cpi | `arb_executor` assembly → on-chain delta guard preceding repay | ✅ on-chain `execute_three_leg` is delta-based (`wsol_baseline`), `IntermediateInventorySubsidized` isolates intermediates | OK |
| On-chain `arb-executor` (`execute_three_leg(_with_tip)`) | deployed program | ✅ delta guard + inventory isolation + tip-validated | OK |

### Finding FL-F-2026-11-22-N1 (P2 at time of audit — double-gated dead path; fixed regardless)

- **Component / location:** `crates/arb-bot/src/supervisor/opportunity.rs`, split-route return hop (inside `#[cfg(feature = "split-routing")]`).
- **Reproduced pre-fix state:** return hop `min_out = expected_total_out − (expected_out × slip_bps / 10_000)` — quote-relative only; module also forbidden by the existing split-rail gate: `split = None` is hardcoded while `enabled` remains disabled, so the path was **double-dead** (feature flag off by default + runtime `None`).
- **Impact if the path were ever re-enabled:** a stale quote within the slippage band could make the final swap output less than `borrow + fee + tip + profit`; Kamino's repay would still succeed because it debits the fee from the same ATA *after* crediting the swap output — i.e. the borrower's pre-existing WSOL balance subsidizes repayment. Loss bounded by the slippage band per event.
- **Correction (this branch):** extracted `split_return_min_out(total_out, slip, borrow, fee, tip, min_profit) = max(slippage_floor, economic_floor)` and used it for the return hop. Integer-only, saturating arithmetic.
- **Regression tests:** `supervisor::opportunity::split_return_min_out_tests` (4 tests): economic floor binds over slippage; slippage binds when stricter; zero slippage; u64 saturating extremes. Requires `--features split-routing` (CI runs exactly this combination).

### Finding FL-F-2026-11-22-N2 (P3)

- **Component / location:** `crates/arb-engine/src/dlmm_flash_arbitrage.rs` + `crates/arb-tx/src/flash/dlmm_flash_arb.rs` (`assemble_flash_funded_two_pool`).
- **Fact:** `estimate_tx_bytes` sets `leg_b.min_amount_out = repay` (principal+fee) — repayment sufficiency guaranteed, no subsidized repayment possible, but no on-chain retained-profit floor (min_profit enforced only at quote time). Not reachable from `arb-bot`; used by `arb-engine` tests and `arb-sim` rehearsal only.
- **Action:** P3 — when (and only when) the DLMM two-pool engine is wired into supervision, raise `leg_b.min_amount_out` to `max(repay, borrow+fee+tip+min_profit)` and add a tip. Tracked in `docs/active/OPEN_ISSUES.md`.

## 4. ISSUE-P0-04 (0x1773 slippage revert) — status: OPEN, unchanged by this phase

- Forensic capture plumbing (wire bytes + per-hop quote provenance + bounded logs + slot) ships in `opportunity.rs::capture_simulation_forensics`.
- **No live-consistent capture exists yet.** We refuse to clear this blocker without a captured failure; we refuse to widen slippage or lower min-out thresholds as a workaround, per the mission's non-negotiable rules.
- Options recorded in `docs/active/OPEN_ISSUES.md`: (a) run shadow capture until one failure is captured; (b) construct a deterministic LiteSVM fixture using captured mainnet reserve/pool account snapshots once any capture exists.

## 5. Verification performed on this branch

| Command | Result |
|---|---|
| `cargo test --workspace` | **1,626 passed / 0 failed / 6 ignored** (baseline 1,622 + 4 new floor tests) |
| `cargo clippy --workspace --all-targets -- -D warnings` | EXIT=0 |
| `cargo clippy -p arb-bot --features split-routing --all-targets -- -D warnings` | EXIT=0 (feature-gated code lints clean) |
| `cargo check -p arb-bot --features split-routing` | EXIT=0 |
| `cargo fmt --all -- --check` | EXIT=0 |
| Kamino builder differential vs official clone | semantic equivalence confirmed (accounts, writability, optional placeholder, discriminators, fee) |
| LiteSVM `litesvm_arb_executor_e2e` | not run this phase — SBF blobs / `cargo-build-sbf` toolchain not present in this sandbox session; classed **SKIP (not a pass)**. Prior session hash-pinned blobs were not available here. |

## 6. Regression matrix

See `docs/active/audit/FLASH_LOAN_REGRESSION_MATRIX.md`.
