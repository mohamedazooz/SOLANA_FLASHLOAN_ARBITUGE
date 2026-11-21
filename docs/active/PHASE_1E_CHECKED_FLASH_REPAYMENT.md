# Phase 1E — Checked Three-Pool Flash Repayment Arithmetic (CPI-003)

## Scope

Remove saturating repayment arithmetic from `FlashArb3LegParams` and ensure both direct and ArbExecutor-CPI three-pool builders fail closed when principal plus flash fee cannot fit in `u64`.

## Changes

- Change `FlashArb3LegParams::repay_amount` to return `Result<u64, FlashArb3LegError>` using `checked_add`.
- Add typed `RepaymentOverflow { borrow, fee }` error with explicit values.
- Propagate the typed error through direct and CPI assemblers, atomicity verification, and the final transaction result.
- Replace the three-pool repayment coverage check's saturating addition with checked arithmetic.
- Update all three-pool call sites and tests for the fallible API.
- Add regression coverage proving both direct and CPI instruction assemblers reject `u64::MAX + 1` rather than silently saturating.

## Verification

- `cargo test -p arb-tx`: 373 passed, 0 failed, 2 ignored.
- `cargo test -p arb-tx --test three_pool_flash_arb_tests`: 18 passed, 0 failed.
- `cargo clippy -p arb-tx --all-targets`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Safety and limitations

- This change is scoped to the three-pool flash-funded path (`FlashArb3LegParams`). Other two-pool, four-pool, and cross-DEX parameter types retain their own repayment APIs and are not modified in this PR.
- No transaction signing, mainnet broadcast, deployment, or asset movement was performed.
- Review-only; do not merge without explicit operator authorization.
