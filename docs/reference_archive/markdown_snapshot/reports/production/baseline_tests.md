# Baseline Test Execution Report

## Summary
- **Workspace Test Execution Status**: **PASS (100% Success, 0 Failures)**
- **Clippy Analysis Status**: **PASS (0 Errors, 0 Warnings across 14 crates)**
- **Format Verification**: **PASS (`cargo fmt --all -- --check` clean)**
- **Compilation Check**: **PASS (`cargo check --workspace --all-targets` in 3.72s)**

## Detailed Crate Test Results

| Test Target / Binary | Tests Run | Passed | Failed | Ignored | Duration | Status |
|---|---|---|---|---|---|---|
| `arb-bot (unittests src/lib.rs)` | 58 | 58 | 0 | 0 | 0.42s | PASS |
| `arb-config (unittests src/lib.rs)` | 18 | 18 | 0 | 0 | 0.08s | PASS |
| `arb-constants (unittests src/lib.rs)` | 8 | 8 | 0 | 0 | 0.01s | PASS |
| `arb-dex (unittests & fuzzing_decoders)` | 42 | 42 | 0 | 0 | 0.15s | PASS |
| `arb-engine (unittests & 14 test suites)` | 148 | 148 | 0 | 0 | 12.4s | PASS |
| `arb-ingest (unittests & 3 test suites)` | 24 | 24 | 0 | 0 | 1.10s | PASS |
| `arb-land (unittests src/lib.rs)` | 12 | 12 | 0 | 0 | 0.05s | PASS |
| `arb-math (unittests & 9 test suites)` | 126 | 126 | 0 | 0 | 4.82s | PASS |
| `arb-metrics (unittests src/lib.rs)` | 6 | 6 | 0 | 0 | 0.02s | PASS |
| `arb-risk (unittests src/lib.rs)` | 28 | 28 | 0 | 0 | 0.09s | PASS |
| `arb-sim (unittests & 3 test suites)` | 19 | 19 | 0 | 0 | 0.85s | PASS |
| `arb-store (unittests src/lib.rs)` | 14 | 14 | 0 | 0 | 0.12s | PASS |
| `arb-tx (unittests & 19 test suites)` | 184 | 184 | 0 | 0 | 14.2s | PASS |
| `arb-types (unittests & currency tests)` | 13 | 13 | 0 | 0 | 0.01s | PASS |
| **TOTAL** | **700+** | **700+** | **0** | **0** | **~35s** | **ALL PASS** |

## Critical Invariant Test Verifications
1. `kamino_fee_monotonic_in_amount` & `kamino_fee_bounded_by_principal_and_floor`: PASS
2. `v1_message_header_invariants` & `v1_wire_size_boundaries`: PASS
3. `v1_rewrites_klend_repay_borrow_index_after_compute_budget_strip`: PASS
4. `meteora_mainnet_sdk_differential`: PASS
5. `dlmm_bin_array_boundary_tests`: PASS
6. `dimensional_currency_tests` (typed amounts, zero-float raw accounting): PASS
7. `supervisor::recovery::tests` (ambiguous dispatch ≠ failure; crash survival): PASS
