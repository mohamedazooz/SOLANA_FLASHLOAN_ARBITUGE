# FLASHLOAN_AUDIT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Scope

Kamino (primary), plus Solend/Marginfi references. Live on-chain verification NOT performed this session.

## Kamino (primary flash-loan provider)

- Program ID, reserve accounts, liquidity vault, token program: sourced from on-chain-verified constants in `arb-constants` / `arb-tx` (per existing evidence). Not re-verified this session.
- Borrow instruction: `build_kamino_flash_borrow` (wrapper + real builder — curated re-export in `flash/mod.rs` to avoid E0659 ambiguity).
- Repay instruction: `build_kamino_flash_repay`.
- Fee calculation: canonical `arb-math::economic::calculate_flash_fee_raw` — `fee_bits = max(amount*sf, 2^60)`, round ties-away. `BorrowTooSmall` guard compares RAW bits pre-rounding (per invariant — do not simplify).
- Atomicity: borrow + all swaps + repay in ONE transaction; `atomic_integrity::verify_atomic_integrity` is the checker and is wired.
- Borrow sizing: bounded by decoded supply-vault liquidity (`kamino_available_flash_liquidity`); fail closed if unverifiable.

## Repayment invariants (verified structurally in opportunity.rs)

- `flash_fee_raw = calculate_flash_fee_raw(size_in, flash_loan_fee_sf)`; on `None` → `u64::MAX` (fail closed).
- `net_before_tip = gross - flash_fee - base_fee - priority_fee`; if underflow → reject.
- Net profit computed in base-mint raw units via `calculate_net_profit_raw`, not lamports.

## Verification gaps

- Official SDK/IDL cross-check of Kamino account ordering, writable flags, PDA derivation: NOT performed this session (requires live RPC + SDK).
- Solend / Marginfi: adapters present; not audited live. Marginfi requires `lending_account_start_flashloan` / `lending_account_end_flashloan` with end-index semantics; CPI flash loans are restricted per Marginfi docs — verify actual path before use.
- `load_instruction_at_checked` for sysvar instruction validation: referenced, not re-verified.

## Status

Structurally sound per source review. Live protocol verification pending.
