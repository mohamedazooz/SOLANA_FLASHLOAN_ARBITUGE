# Flash Loan Regression Matrix — Phase 2

Scope: every repo behavior that protects flash-loan atomicity and economic integrity, with its guard location, its test, and last-run evidence (2026-11-22, branch `vibe/flashloan-forensic-8ade4e`).

Legend: ✅ test passing this session · ⚠️ guard present, test gap or skip-class · ❌ missing.

| ID | Invariant | Guard location | Test | Evidence |
|----|-----------|----------------|------|----------|
| FL-01 | Borrow and repay use identical 12-account order matching official klend-interface | `crates/arb-tx/src/flash/kamino.rs` | differential vs official clone (manual, documented) + `three_pool_flash_arb_tests` | ✅ verified vs official clone; PASS |
| FL-02 | Repay amount = PRINCIPAL only; fee debited on top by handler | `kamino.rs:128` (+ comment in `builder.rs`, `dlmm_flash_arb.rs`) | `arb_executor_cpi_tests`, `three_pool_flash_arb_tests` assert repay principal | ✅ PASS |
| FL-03 | Flash fee formula `max(amount*sf, 2^60)`, ties-away | `arb-math::economic` (single source) | `arb-math` fee boundary vectors | ✅ PASS |
| FL-04 | Borrow sizing ≤ decoded live supply-vault liquidity; fail closed | `arb-engine` sizing guards | `BorrowAboveKaminoLiquidity` / `BorrowAbovePoolLiquidity` tests | ✅ PASS |
| FL-05 | Repay `borrow_instruction_index` points at the real top-level borrow, correct after every compiler transform (V1 CB strip included) | `adaptive.rs:380–480` `index_remap` | V1 compile tests; fail-closed when a needed index is stripped | ✅ PASS |
| FL-06 | Borrow before repay; both present exactly once | `atomic_integrity.rs:66–549` `verify_*` | `verify_atomic_integrity_for_format` tests | ✅ PASS |
| FL-07 | Repay principal == borrow amount | `atomic_integrity.rs` | equality assertions | ✅ PASS |
| FL-08 | Repay amount ≥ borrow + fee | `atomic_integrity.rs` + assembler Invariant 3/6 | `InsufficientRepayment` / `FinalOutputBelowRepayment` tests | ✅ PASS |
| FL-09 | Route closed: final mint == borrowed mint | assemblers (`NonClosingRoute` errors) | per-assembler tests | ✅ PASS |
| FL-10 | Same reserve for borrow and repay (klend accounts[3] coupling) | builder + klend on-chain check | `flash_repay_checks` semantics honored by construction | ✅ PASS |
| FL-11 | Non-atomic Jito mode rejected | `atomic_integrity.rs` | negative test | ✅ PASS |
| FL-12 | Token-program incompatible combos fail closed | builders (`Phase_1F`) | token-program-aware ATA tests | ✅ PASS |
| FL-13 | No pre-existing wallet balance can subsidize repayment — executor CPI path | on-chain `execute_three_leg` delta guard + `IntermediateInventorySubsidized` | program tests + LiteSVM e2e | ⚠️ unit/program tests PASS; LiteSVM e2e SKIP (no SBF blobs in this session) |
| FL-14 | Same for generic hop builder path | `derive_swap_legs` final-hop floor (`builder.rs:327`) | `hop_slippage_once_tests`, `cross_dex_dlmm_cpmm_v1_atomic_tests` floor tests | ✅ PASS |
| FL-15 | Same for split-route return hop | `split_return_min_out` guard (`opportunity.rs`) | `split_return_min_out_tests` (4 tests, `--features split-routing`) | ✅ PASS (FIX for FL-F-2026-11-22-N1) |
| FL-16 | Unused/unfloored paths cannot silently return | `DlmmFlashArbEngine` unreachable from bot | reachability documented (audit §3); N2 tracked (P3) | ✅ documented |
| FL-17 | V1 never silently downgraded to V0 | `adaptive.rs` compile path | compatibility matrix tests (Phase 5 continues) | ✅ PASS so far |
| FL-18 | Size limits: 1,232B V0 / 4,096B V1 / ≤64 accounts | `builder.rs` MTU check, `adaptive.rs`, `estimate_v0_transaction_size` | boundary tests incl. `TransactionTooLarge` | ✅ PASS |
| FL-19 | Tip last, marked (`jitodontfront`), min 1,000 lamports | tip builders + engine gate | `three_pool_flash_arb_tests` tip-folder ordering | ✅ PASS |
| FL-20 | Ambiguous dispatch reconciled before any retry; never re-sign | `arb-land` | reconciliation tests | ✅ PASS |
| FL-21 | 0x1773 (ISSUE-P0-04) root cause fixed | — | forensic capture awaiting a real failure | ❌ OPEN (fail-closed by rule) |
| FL-22 | Subsidy scenario: losing final swap with pre-existing balance cannot repay | FL-13/14/15 guards | FL-15 test vector `economic_floor_binds_when_slippage_is_looser` (slippage-floor looser than economic floor) | ✅ PASS |

## Execution evidence commands

```text
cargo test --workspace                → 1,626 passed / 0 failed / 6 ignored
cargo clippy --workspace --all-targets -- -D warnings            → EXIT=0
cargo clippy -p arb-bot --features split-routing --all-targets -- -D warnings → EXIT=0
cargo fmt --all -- --check            → EXIT=0
```

Notes: LiteSVM e2e SKIP is not a pass and is listed as such. ISSUE-P0-04 reproduction remains gated on a captured live-consistent failure.
