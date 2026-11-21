# Phase 1F — Token-Program-Aware ATA Derivation (TOKEN-003)

## Scope

Remove implicit legacy-SPL ATA derivation from generic swap execution paths where each hop already carries token-program metadata. Preserve the two-argument ATA helper only for explicitly SPL-only contexts such as WSOL and legacy protocol builders.

## Changes

- Document `get_associated_token_address` as legacy SPL Token-only; generic/mint-dynamic paths must use `get_associated_token_address_with_program_id`.
- Update `build_swap_hop_instruction` to derive input/output ATAs using each hop's declared token programs for Orca, Meteora DLMM, Raydium CPMM/CLMM, Meteora DAMM/DBC/DLMM-Pro, and LaunchLab branches.
- Orca's instruction layout has one token-program account: set it to the shared mint program and fail closed when a hop mixes SPL Token and Token-2022.
- Update generic `TxBuilder::assemble_arbitrage` intermediate ATA creation, idempotent initialization, and close-account instructions to preserve each mint's token program.
- Fix PumpSwap V1 builder to use its existing `token_program_base` and `token_program_quote` arguments for user, fee-recipient, and creator-vault ATA derivation. V2 was already program-aware.
- Add `KaminoFlashParams::liquidity_token_program` and `from_candidate_with_token_program`; the selected program now controls both the borrow ATA and Kamino token-program account. Existing `from_candidate` remains the explicit legacy-SPL convenience path.
- Add regression tests for SPL-vs-Token-2022 ATA derivation, Orca Token-2022 ATA selection and mixed-program rejection, PumpSwap V1 program-aware ATAs, and Kamino candidate program selection.

## Remaining SPL-only paths (explicit limitations)

- WSOL/USDC convenience ATAs in the transaction builder and cross-DEX path use the canonical legacy SPL Token program.
- `pump_fun` and `pump_fun_raydium` legacy builders remain SPL-only; their current account APIs do not carry a Token-2022 mint-program selection.
- `KaminoFlashParams::from_candidate` defaults to SPL Token for backward compatibility. A caller handling a Token-2022 reserve must use `from_candidate_with_token_program` with the verified mint account owner.
- `wallet_probe` is an SPL-only example helper; it is not used by production execution.

## Verification

- `cargo test -p arb-tx`: 377 passed, 0 failed, 2 ignored.
- `cargo test -p arb-tx --test swap_hop_direction_tests`: 14 passed, 0 failed.
- `cargo clippy -p arb-tx --all-targets`: passed.
- `cargo check -p arb-bot`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Safety

No transaction signing, mainnet broadcast, deployment, or asset movement was performed. Review-only; do not merge without explicit operator authorization.
