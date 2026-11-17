# TEST_RESULTS.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Commands attempted this session

None successfully. The shell tool could not reliably spawn `pwsh.exe` ("Access is denied"), preventing execution of:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- `cargo audit` / `cargo deny check`

## Results

- **Passed:** 0 (not run)
- **Failed:** 0 (not run)
- **Skipped:** entire workspace suite

## Reason

Environment toolchain blocker (pwsh spawn failure). No test results are fabricated.

## Historical evidence

Prior runs are recorded under `reports/` (e.g., reports/final_audit/, reports/phase15_2/, reports/v1_canary/). These are historical artifacts with timestamps; not re-verified this session. Evidence files are test-written (timestamps change) — use `git checkout -- <file>` to revert timestamp churn.

## Required before any status promotion

Re-run full suite on Linux (1.99.1) and record real pass/fail counts here.
