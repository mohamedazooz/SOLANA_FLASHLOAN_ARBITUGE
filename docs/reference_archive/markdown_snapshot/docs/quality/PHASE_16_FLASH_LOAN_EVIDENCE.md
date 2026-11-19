# Phase 16 — Flash Loan Atomicity Evidence

**Date:** 2026-11-13
**Scope:** Kamino WSOL flash loan funding a 2- or 3-leg Meteora DLMM arbitrage
inside **one** Solana transaction.

## 1. Required lifecycle

```
ComputeBudget(limit, price)
  → Kamino flash_borrow(principal)
  → swap A (exact-in, min_out_A)
  → swap B (exact-in, min_out_B)
  → [swap C (exact-in, min_out_C)]          # 3-leg routes
  → Kamino flash_repay(principal, borrow_ix_index)
  → Jito tip
```

## 2. Source evidence

| Property | Location | Test |
|---|---|---|
| Instruction order 2-leg | `flash/dlmm_flash_arb.rs` (`assemble_flash_funded_two_pool`) | `layout_is_borrow_swaps_repay` |
| Instruction order 3-leg | `flash/dlmm_3leg_flash_arb.rs` | `layout_is_borrow_three_swaps_repay_tip` |
| Repay index points at real borrow | `build_kamino_flash_repay_with_index` | `v1_rewrites_klend_repay_borrow_index_after_compute_budget_strip` |
| Repay amount = principal + fee | `KaminoFlashParams::repay_amount` | `insufficient_final_output_rejected`, `final_output_below_repayment_is_rejected` |
| Route closes to WSOL before repay | route invariant | `non_closing_route_rejected` |
| Distinct pools / mint chain | route invariant | `non_distinct_pools_rejected`, `mint_chain_mismatch_rejected` |
| Borrow-before-repay + amount coverage | `primitives/atomic_integrity.rs` | `test_flash_loan_borrow_before_repay`, `test_insufficient_repayment_reverts`, `test_repay_before_borrow_rejected` |
| Bundle ≠ atomic | `atomic_integrity` | `test_jito_bundle_is_not_atomic` |
| Full-tx rollback on hop 2 failure | runtime model | `test_failed_second_hop_rollback_preserves_initial_balance` |
| First-hop failure reverts all | runtime model | `test_intermediate_isolation_case_4_failed_first_hop_reverts_whole_tx` |
| Profit gate rejects gross < costs | `arb-sim/src/profitability_gate.rs` | `test_profitability_gate_rejects_structural_loss_even_with_zero_hurdle` |

## 3. Fee model

- Kamino flash fee uses the on-chain `fee_sf` (scale factor
  `115292150460684`), verified against the live reserve in the 2026-11-13
  probe; the constant and live value agree.
- `calculate_kamino_flash_fee` is covered by
  `test_calculate_kamino_flash_fee`, `…_checked_guards`, and the BigInt
  reference `flash_fee_matches_bigint_reference`.
- Repayment failure is **not** caught locally and swallowed: if the on-chain
  handler rejects, the whole transaction reverts. Local checks make that
  impossible to reach by construction (fail before signing).

## 4. What is NOT proven

The above proves the **shape and guards** of an atomic route. It does **not**
prove that a real Kamino borrow + real DLMM swaps + real repay succeed on-chain:

- no funded simulation payer was available (see B-3);
- no signed transaction was broadcast (live disabled);
- the size-ladder "real routes" use synthetic `Pubkey::new_unique()` bin arrays.

Therefore atomicity is **VERIFIED at the builder/gate level** and **NOT VERIFIED
end-to-end on mainnet**.

## 5. Reproduce

```bash
cargo test -p arb-tx --test three_pool_flash_arb_tests
cargo test -p arb-tx --test intermediate_balance_isolation_tests
cargo test -p arb-engine --test phase17_two_pool_flash_arb_tests
cargo test -p arb-engine --test three_leg_net_economics_tests
cargo test -p arb-tx primitives::atomic_integrity
cargo test -p arb-sim profitability_gate
```
