# Kamino KLend Flash Loan Audit (PHASE E)

Program: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (repo constant `KAMINO_KLEND_PROGRAM_ID`).

## Fee economics — EXACT, not guessed

* Fee source: `ReserveConfig.liquidity.fees.flash_loan_fee_sf` — a raw U68F60 bit pattern decoded live from the reserve account (`arb-tx/src/flash/kamino.rs:161-218`).
* Formula reproduces klend `ReserveFees::calculate_fees(.., FeeCalculation::Exclusive, has_referrer=false)`: `fee = round(borrow × sf / 2^60)`, minimum 1 atomic unit, ties away from zero, `BorrowTooSmall` guard surfaced via `KaminoFeeError`.
* Main-market WSOL reserve: `sf = 11_529_215_046_068` → exactly 0.1 bps (10,000 lamports per 1 SOL). A bps integer cannot represent this value; the code correctly never uses a bps approximation.
* Fail-closed variant `calculate_kamino_flash_fee` returns `u64::MAX` for a disabled reserve (`sf == u64::MAX`) so any cost comparison rejects the route.

## Lifecycle (atomic, single transaction)

```
[0] SetComputeUnitLimit        (V0)  — stripped and re-encoded natively in V1
[1] SetComputeUnitPrice        (V0)
[2] Kamino flash_borrow_reserve_liquidity   ← borrow
[3..] swap legs (Meteora DLMM swap2 × N)    ← arb path
[N+1] Kamino flash_repay_reserve_liquidity  ← repay, borrow_instruction_index = 2
```

* Discriminators verified: `flash_borrow_reserve_liquidity` / `flash_repay_reserve_liquidity` (sha256("global:...")[..8], `arb-constants:336-338`).
* `flash_repay` carries `borrow_instruction_index` pointing at the executed borrow instruction; the V1 compiler remaps it when ComputeBudget instructions are stripped (`transport/adaptive.rs:418-443`) — otherwise klend's `flash_repay_checks` reverts with `InvalidFlashRepay`.
* Repay amount: the builder passes `principal`; klend collects the fee separately at repay — passing `principal + fee` as the instruction amount would double-charge (pinned by `bundle_tests.rs`, `challenger_m2_tests.rs`).
* Atomicity: `primitives/atomic_integrity.rs` verifies borrow↔repay pairing, ordering (borrow first, repay last), and rejects non-atomic assemblies before signing.
* Rollback: any swap failure (slippage floor, missing bin array, insufficient liquidity) reverts the whole transaction including the borrow — enforced by the runtime, proven by `phase17_two_pool_flash_arb_tests::transaction_is_borrow_swaps_repay_in_one_atomic_unit`.

## Sizing constraints (dynamic)

* Borrow ≤ verified available liquidity (`kamino_available_liquidity` threaded from the live reserve account into `capture_three_leg_dlmm`); unverifiable liquidity fails closed ("flash funding enabled but Kamino supply-vault liquidity unverifiable" → skip, no guess).
* Borrow bounded by the size ladder (`input_size..max_input`, TOML/env-overridable, optimizer-driven).
* Repay solvency: `quote_b.amount_out ≥ borrow + fee` enforced before dispatch; conservative leg minimums chain hop-to-hop (`hop_two_input_is_bound_to_hop_one_minimum_output`).

## Known gap (documented)

Slot-race between liquidity verification and execution: the reserve can change between
verify and land. Mitigation is atomic revert (transaction fails, no funds at risk beyond
fees) — no capital loss path exists because the borrow and repay are in one transaction.
