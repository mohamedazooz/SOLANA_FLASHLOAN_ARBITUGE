# Phase 1C — Shared Three-Pool Route Validation (CPI-001)

## Scope

Make the direct three-pool assembler and ArbExecutor-CPI assembler enforce one identical route/solvency validation contract. This PR does not change CPI account validation (CPI-002), checked repayment arithmetic (CPI-003), signing policy, dispatch wiring, or live execution.

## Changes

- Extracted `validate_flash_funded_three_pool_params` as the shared fail-closed validation function.
- Both direct and CPI assemblers now call the shared validator before instruction construction.
- Preserved the CPI-only Agave runtime stack-depth validation as an additional CPI invariant.
- Shared validation covers distinct pools, non-zero borrow principal, repayment coverage, closing flash-mint route, closing token-program identity, both inter-hop mint and token-program links, and each hop's conservative minimum-output solvency.
- Added parity regression coverage comparing the direct and CPI assemblers on valid input and invalid borrow, duplicate pool, non-closing mint, mint mismatch, token-program mismatch, closing-program mismatch, and underfunded-hop inputs.

## Verification

- `cargo test -p arb-tx`: 373 passed, 0 failed, 2 ignored.
- `cargo test -p arb-tx --test arb_executor_cpi_tests`: 6 passed, 0 failed.
- `cargo clippy -p arb-tx --all-targets`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Safety and limitations

- No transaction signing, broadcast, deployment, or asset movement was performed.
- This PR does not claim the standalone three-pool builder is wired into the live supervisor dispatch path.
- CPI-002 (expected-mint/token-account validation) and CPI-003 (checked repayment arithmetic) remain separate follow-up work packages.
- PR is review-only and must not be merged without explicit operator authorization.
