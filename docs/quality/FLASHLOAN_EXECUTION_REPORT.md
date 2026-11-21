# FLASHLOAN_EXECUTION_REPORT — Kamino Flash-Loan Execution Integrity

**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4`
**Verification date**: 2026-11-21

---

## 1. Verified instruction semantics (Kamino Klend)

| Requirement | Verified in | Evidence this session |
|---|---|---|
| Atomics: `FlashBorrow → Swap_1..N → ProfitGuard → FlashRepay`, one transaction, never split across bundles | `crates/arb-tx/src/primitives/atomic_integrity.rs` (`verify_atomic_integrity` stays wired) | workspace suite green |
| `borrow_instruction_index` = position of flash-borrow in the **final** transaction (Kamino docs, checked 2026-11-21) | V1 compiler strips ComputeBudget no-ops then rewrites the repay's index against the executed list; stripping a borrow *target* is a hard `SanitizationError` | `adaptive.rs::compile_v1_message`; `test_v1_strips_compute_budget_and_requires_resource_limits`; E0659 trap (wrapper vs real builder) preserved via curated re-exports |
| Fee = `max(amount·sf, 2^60)` U68F60, ties-away rounding; `BorrowTooSmall` guard on raw bits | `arb-math/src/economic.rs:576–652` (canonical) | unit tests incl. boundary at `sf = 2^60 − 1` |
| Program ID `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | `arb-constants/src/lib.rs:463` | matches kamino.com architecture docs (2026-11-21) |
| Instructions-sysvar dependency for `flash_repay_checks` | encoded in builder account set | docs.rs/klend-interface (sysvar unchecked-account pattern) |

## 2. Borrow sizing bounds (fail-closed)

- Vault-cap primacy: borrow sizing is bounded by the **decoded live reserve
  supply-vault balance**, never by a config ceiling alone. Verified regressions:
  `borrow_above_the_kamino_ceiling_is_rejected`,
  `borrow_above_the_pool_capacity_is_rejected`,
  `test_matrix_06_insufficient_kamino_liquidity_rejected` — all green.
- If the liquidity snapshot is missing or stale, the candidate is rejected
  (`traversable_capacity_fails_closed_without_the_active_bin`,
  `empty_or_unowned_dependency_set_fails_closed`).
- Joint route × size evaluation never assumes monotonicity (PRs #94/#95):
  bounded multi-basin search, per-hop re-quotes, bin/tick boundaries, DLMM
  non-linear capacity, and `net_profit_clamp_never_flips_loss_into_profit`.

## 3. Sizing result contract

Every sizing candidate carries: borrow amount, expected output, repayment,
flash fee, DEX fees, price impact, network/priority fee estimates, Jito tip
budget, contingency, final net profit, profit bps, estimated CU, estimated
transaction size (see `crates/arb-engine/src/flash_loan_sizing.rs` and
`route_sizing.rs` result types; composition covered by the matrix tests
`test_matrix_*` in `arb-bot`).

## 4. Execution failure-path evidence (executed this session)

From the full workspace run (all green):

| Failure path | Test |
|---|---|
| First/mid swap reverts | `test_mainnet_non_broadcast_simulation_cross_dex_dlmm_cpmm_kamino_v1` matrix + atomic integrity casing |
| Insufficient output / repayment shortfall | floor policy tests (economic + sizing families) |
| Invalid instruction ordering / index | `atomic_integrity` ceilings test, V1 klend index-rewrite tests |
| Transaction size overflow | `test_compile_result_rejected_v1_oversized`, `challenge_wire_mtu_size_limit_and_high_account_count` |
| Account-limit exceedance | `test_scenario_h_v1_accounts_limit_exceeded` |
| CU / resource config invalid | `test_v1_sanitization_rules`, `test_v1_resource_config_bitmask_and_values` |

## 5. Failure-injection status

| Injection | In-process coverage | Live coverage |
|---|---|---|
| RPC 429 / quota exhaustion | `quota_exhaustion_tests` (fail-closed + healthy-endpoint failover) ✅ | BLOCKED (no egress this sandbox) |
| WS disconnect / reconnection | Phase 3 backoff/rotation tests ✅ | previously shadow-tested; soak pending |
| Jito rejection / timeout | selection scenario tests (`test_scenario_c_v1_jito_unsupported_when_flag_disabled`, ‑d, ‑h) ✅ | BLOCKED |
| Blockhash expiry | dispatch/recovery tests (expiry requires block-height evidence) ✅ | BLOCKED |
| Database failure / restart | store tests (provenance, idempotent reconciliation) ✅ | part of 24 h soak gate |

## 6. Live-readiness blockers

1. ISSUE-P0-04 `0x1773` same-snapshot replay (open).
2. 24 h Shadow Soak not completed on the current commit.
3. Realized-PnL evidence requires explicit operator authorization (`ARB_LIVE=0` remains).
