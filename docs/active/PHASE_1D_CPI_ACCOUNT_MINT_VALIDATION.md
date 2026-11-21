# Phase 1D — CPI User Token Account and Mint-Chain Validation (CPI-002)

## Scope

Harden the on-chain three-leg ArbExecutor account validation. The canonical processor previously validated user authority and token-program ownership but passed `None` for expected mints, allowing a user token account with the wrong mint to pass the initial account gate.

## Changes

- Resolve the actual route mint chain from the three DLMM pool mint-account pairs: WSOL → Meme A → Meme B → WSOL.
- Reject a pool pair when the expected input mint is absent or duplicated.
- Require mint-account owner/program identity to remain consistent across adjacent pools and on the closing WSOL hop.
- Validate each user settlement/intermediate token account against the exact resolved mint account and require its token-account owner to match the owning SPL Token or Token-2022 program.
- Require the complete 165-byte token-account base layout and state byte 108 == 1 (initialized, not frozen).
- Keep `execute_three_leg_with_tip` delegated to the canonical processor; its initial WSOL check also inherits the initialized/non-frozen validation.
- Add tests for correct mint-chain resolution, broken route mint continuity, token-program mismatch, and initialized/frozen account state.

## Verification

- `cargo test -p arb-executor`: 25 passed, 0 failed.
- `cargo clippy -p arb-executor --all-targets`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Safety and limitations

- No transaction signing, mainnet broadcast, deployment, or asset movement was performed.
- This PR validates expected mints/programs for user WSOL and intermediate accounts before swaps.
- CPI-003 checked repayment arithmetic remains a separate follow-up.
- PR is review-only and must not be merged without explicit operator authorization.
