# Phase 16 — Production Readiness & Mainnet Execution Verification

**Date:** 2026-11-13
**Baseline:** `main` @ `093b20e`; audited tree = PR #13 head `b01b0c4`
**Audit branch:** `vibe/phase16-production-readiness-9d81d9`
**Verdict:** ❌ **NOT PRODUCTION READY** — see §6. Development-grade
correctness is strong (962/964 tests pass offline), but the two requirements
that cannot be delegated — a **24-hour shadow soak** and an **authorized mainnet
broadcast with on-chain confirmation** — have no evidence.

> This report separates **VERIFIED** (reproduced in this audit), **CITED**
> (recorded by an earlier run, not re-claimed), and **NOT VERIFIED / BLOCKED**.
> A green offline test does not prove mainnet execution.

---

## 1. Repository integrity

| Item | Result |
|---|---|
| PR #13 diff scope | `.env.example`, `arb-sim/src/profitability_gate.rs`, `arb-tx/tests/v1_size_ladder_tests.rs`, docs, `reports/phase15_5/V1_SIZE_LADDER.json` |
| Protocol/execution code changed by PR #13 | **No** — no regression surface in adapters or transport |
| Working functionality removed | None found |
| `cargo check --workspace --all-targets` | PASS (0 errors) |
| `cargo fmt --all -- --check` | PASS (exit 0) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS (exit 0) |
| Full test suite | **962 passed, 2 failed, 2 ignored** (964 executed) |

The two failures are the only non-green tests and are analysed in §5.1. The
PR body's "964 tests" matches the 964 executed tests exactly; it does not mean
964 *passed*.

## 2. Atomic flash-loan verification

Verified at the instruction-builder and gate level (full evidence in
[PHASE_16_FLASH_LOAN_EVIDENCE.md](PHASE_16_FLASH_LOAN_EVIDENCE.md)):

- Instruction layout is `borrow → swap A → swap B( → swap C) → repay → tip`
  (`flash/dlmm_flash_arb.rs`, `flash/dlmm_3leg_flash_arb.rs`; tests
  `layout_is_borrow_swaps_repay`, `layout_is_borrow_three_swaps_repay_tip`).
- `repay_amount == borrow_amount + flash_fee` is enforced; under-repayment is
  rejected before signing (`insufficient_final_output_rejected`,
  `final_output_below_repayment_is_rejected`, `route_fails_closed_when_repayment_not_guaranteed`).
- Borrow-before-repay and repay-after-swap ordering are enforced by
  `verify_atomic_integrity`.
- Rollback is modeled and tested: `test_failed_second_hop_rollback_preserves_initial_balance`,
  `test_intermediate_isolation_case_4_failed_first_hop_reverts_whole_tx`.
- A single Solana transaction gives runtime-level all-or-nothing atomicity;
  Jito bundles are correctly **rejected** as non-atomic for multi-leg
  arbitrage (`test_jito_bundle_is_not_atomic`).

**Caveat (documentation corrected in this PR):** `verify_atomic_integrity`
enforces mode, borrow/repay presence and ordering, repayment coverage, and a
*legacy lower-bound* size estimate. Its account-meta and token-program checks
are advisory no-ops, and mint-chain / compute-budget checks are delegated to the
route builders. The previous doc comment claimed 9 enforced checks; the header
now states the real set.

## 3. Multi-pool DLMM

- 2-pool and 3-pool routes assemble and self-verify
  (`phase17_two_pool_flash_arb_tests.rs`, `three_pool_flash_arb_tests.rs`).
- Bin-array planning is exact and directional, never truncated
  (`flash/dlmm_bin_discovery.rs`; 8 unit tests + 12 boundary tests).
- **Measured** serialized sizes (never estimated), real 3-pool Kamino→DLMM×3
  routes, from `reports/phase15_5/V1_SIZE_LADDER.json`:

| bins/leg | accounts | wire bytes | V1 |
|---:|---:|---:|:--:|
| 1 | 38 | 1574 | ✅ |
| 5 | 50 | 1970 | ✅ |
| 10 | 65 | 2465 | ✅ |
| 11 | 67 | — | ❌ `SIMD-0385 address limit exceeded: 67 > 64` |

- SIMD-0385 is **not bypassed**: the 64-inline-address ceiling rejects growth
  at 11 bins/leg. The V1 4,096-byte MTU is enforced separately (4,097 rejected).
- ALT/compression: `transport/adaptive.rs` is V1-only when the profile supports
  V1 and never silently downgrades; V0-with-ALT is used only for explicitly
  non-V1 profiles.
- **Honesty constraint:** the size-ladder "real routes" use
  `Pubkey::new_unique()` bin-array addresses. They are structurally complete
  and signable, but **not mainnet-executable as written**.

## 4. V1 transport

Full evidence in [PHASE_16_V1_TRANSPORT_EVIDENCE.md](PHASE_16_V1_TRANSPORT_EVIDENCE.md).

- Real signed V1 wire bytes: 0x81 header, one real signature, ComputeBudget
  config; signature verified in `adaptive_compiler_tests.rs` and
  `three_pool_flash_arb_tests.rs`.
- Synthetic capacity (1232/1500/2000/3000/3500/4000) is recorded under
  `instrument: "wire_capacity"` and kept **separate** from executable routes.
- No silent downgrade: `test_v1_capable_profile_never_downgrades_small_payload`,
  `compile_adaptive` rejects non-V1 when the profile is V1.
- Live simulate: **CITED** from `reports/v1_mainnet_probe/probe_report_2026-11-13.json`
  — solana-core 4.4.0, V1 `simulateTransaction` accepted, 62,596 CU. Not
  re-run in this audit.
- **Broadcast: NOT VERIFIED.** No authorized send, no signature, no on-chain
  evidence.

## 5. Execution infrastructure

| Requirement | Status | Evidence |
|---|---|---|
| Jito bundle submission | VERIFIED (code) | `arb-land::send_bundle_multi_region`, region fan-out |
| Jito confirmation | VERIFIED (code) | `getBundleStatuses` **and** `getSignatureStatuses` cross-check |
| Priority-fee estimation | VERIFIED (code) | `opportunity.rs` adaptive CU price, capped by profit share |
| Jito tip optimization | VERIFIED (code) | `AdaptiveBidder` + `TipOracle` + `LandingLedger`; floor 1,000 lamports, `max_tip_share` |
| Leader/Jito gating | VERIFIED (code) | Kobe registry ⋈ `getLeaderSchedule`; non-Jito leader → plain RPC |
| Compute budget | VERIFIED (code) | version-aware limit/price builders; V1 strips legacy CB ix |
| Blockhash freshness | VERIFIED (code) | cached `getLatestBlockhash` + `lastValidBlockHeight` |
| Slot latency | VERIFIED (code/math) | `SlotClock` scheduling, p95+3σ lead, one-slot cap |
| Expiration / retry | VERIFIED (code) | recovery proves expiry by block height, never slot arithmetic |
| Duplicate-submission prevention | VERIFIED (code) | durable `dispatch_intents` written **before** send; recovery never resubmits |
| Velocity / in-flight caps | VERIFIED (code) | 10 orders/hr, max 2 in-flight bundles per slot |
| Fire-time kill switch | VERIFIED (code) | re-checked after simulation and fire-wait |

### 5.1 The two failing tests

`crates/arb-tx/tests/mainnet_non_broadcast_simulation.rs` runs a **real,
read-only** mainnet `simulateTransaction` for Raydium V4 and Orca. In this
environment both fail with:

```
Simulation Execution Error: Some("InvalidAccountForFee")
Simulation logs must confirm invocation of ... program on mainnet: logs=[]
```

Cause: no `SOLANA_PRIVATE_KEY` is available, so the test falls back to
`KeypairSigner::from_seed(&[99u8; 32])` — an unfunded account. The RPC URL was
available (from `../.env`), so the test attempted mainnet with an unfunded
fee payer. This is an **environment/harness limitation**, not a DEX math or
execution-path defect:

- the test does not skip when a funded payer is absent;
- it asserts on program logs that cannot exist for an unfunded payer.

**Required fix (tracked as blocker B-3):** skip with a clear message when no
funded payer is configured, or fund a dedicated simulation-only key. Do not
weaken the assertion.

## 6. Shadow soak

**NOT RUN.** A ≥24 h continuous shadow run is impossible in this sandbox:
no `.env` in the repo (only a parent-directory RPC URL), no funded key, no
persistent 24 h host, and live mode is unset. The protocol, required metrics,
and the store-schema gap are documented in
[PHASE_16_SHADOW_SOAK_REPORT.md](PHASE_16_SHADOW_SOAK_REPORT.md).

Consequence: opportunity counts, gross/net expected profit, quote freshness,
simulation success rate, CU consumption, fee/tip estimates, rejection reasons,
and slot/execution latency have **no 24 h evidence**.

## 7. Safety

- `ARB_LIVE` unset; shadow is the effective mode.
- No mainnet transaction was signed or broadcast by this audit.
- No capital deployed.
- No claim of execution without a confirmed signature is made anywhere.

## 8. Verdict and gates

| Gate | Status |
|---|---|
| Code/tests (fmt, clippy, offline suite) | ✅ PASS |
| Atomic flash-loan lifecycle (builder + gate) | ✅ VERIFIED offline |
| 2-/3-pool DLMM + SIMD-0385 + measured sizes | ✅ VERIFIED offline |
| V1 signed wire format + no downgrade | ✅ VERIFIED offline |
| V1 live simulate | ⚠️ CITED only |
| Execution infra (Jito/fees/tip/latency/recovery) | ✅ VERIFIED in code |
| 24 h shadow soak | ❌ NOT RUN |
| Authorized mainnet broadcast + confirmation | ❌ NOT PERFORMED |

**Do not mark PRODUCTION READY.** See
[PHASE_16_REMAINING_BLOCKERS.md](PHASE_16_REMAINING_BLOCKERS.md) for the
exact, evidence-gated list of what must close first.

## 9. Documents in this phase

- [PHASE_16_PROTOCOL_MATRIX.md](PHASE_16_PROTOCOL_MATRIX.md)
- [PHASE_16_FLASH_LOAN_EVIDENCE.md](PHASE_16_FLASH_LOAN_EVIDENCE.md)
- [PHASE_16_V1_TRANSPORT_EVIDENCE.md](PHASE_16_V1_TRANSPORT_EVIDENCE.md)
- [PHASE_16_SHADOW_SOAK_REPORT.md](PHASE_16_SHADOW_SOAK_REPORT.md)
- [PHASE_16_REMAINING_BLOCKERS.md](PHASE_16_REMAINING_BLOCKERS.md)
- [PHASE_16_REPRODUCIBLE_COMMANDS.md](PHASE_16_REPRODUCIBLE_COMMANDS.md)
