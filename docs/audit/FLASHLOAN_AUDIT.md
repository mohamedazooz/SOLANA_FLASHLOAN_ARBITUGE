# Kamino Flash Loan Audit

**Program:** klend (Kamino Lending), mainnet. **Reserve:** configured WSOL
reserve. All findings verified against the deployed account layout and an
independent big-integer reference implementation — not blog posts or assumed
percentages.

## Verified facts

| Item | Verification | Evidence |
|---|---|---|
| Fee source | Read from the deployed reserve: `ReserveConfig.fees.flash_loan_fee_sf`, U68F60 raw `Fraction` bit pattern at account offset `8 + 4896` | `crates/arb-tx/src/flash/kamino.rs:221-278` |
| Fee semantics | Reproduces klend `ReserveFees::calculate_fees(amount, flash_loan_fee_sf, ...)`: `fee = amount * rate_bits >> 60`, then rounding per klend rules | `kamino.rs:178-201` |
| Fee parity | `flash_fee_matches_bigint_reference` — matches an independent big-integer reference across fixtures | `kamino.rs` tests |
| Repay amount | Borrow principal **exactly** — klend's `flash_borrow_check_matching_repay` rejects any other value | `dlmm_3leg_flash_arb.rs:237,450,864` |
| Instruction ordering | `[5] flash_borrow_reserve_liquidity` → swaps → `[9] flash_repay_reserve_liquidity` with `borrow_instruction_index = 5` | `dlmm_3leg_flash_arb.rs:18-22,418,454` |
| Reserve validation | Decoder rejects wrong discriminator, wrong mint, inactive reserve, short account | `kamino.rs` tests (`decode_rejects_*`) |
| Available liquidity | Read from the reserve's supply vault (liquidity vault) token balance, incl. Token-2022 base layout | `supply_vault_liquidity_*` tests |
| Atomicity | Borrow + 3 DLMM swaps + repay in ONE transaction; any failure reverts everything | `assemble_flash_funded_three_pool` + compile tests |

## Explicitly rejected mistakes

- **Ordinary borrow APY is not the flash-loan fee.** The fee is the reserve's
  dedicated `flash_loan_fee_sf` field.
- **Total reserve deposits ≠ borrowable liquidity.** Sizing uses the supply
  vault's actual token balance.
- **No hardcoded fee percentage.** Percentage is per-reserve, on-chain,
  re-read from live account data.

## Remaining gaps

- Live mainnet borrow/repay broadcast is policy-gated (R-B); current evidence
  is unit + differential + snapshot-simulation level.
- A persistent forked-validator run (R-H) would strengthen repayment evidence.

See also: `audit/FINANCIAL_LOGIC_AUDIT.md`, `reference/SDK_PARITY.md`.
