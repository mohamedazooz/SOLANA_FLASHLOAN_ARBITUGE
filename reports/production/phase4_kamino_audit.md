# Phase 4 — Kamino Flash Loan Forensic Audit (CORRECTED)

> Corrected after reading the in-repo evidence: live mainnet probe, on-chain reserve snapshot, and the V1 compiler repairs.

## 4.1 Live Deployed Program Behavior — VERIFIED

| Check | Evidence |
|---|---|
| Program ID | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` — live reserve owner matches (`probe_report_2026-11-13.json`) |
| Main market | `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF` |
| Reserve discriminator | Matches `KAMINO_RESERVE_DISCRIMINATOR [0x2b,…]` on live WSOL reserve |
| `flash_loan_fee_sf` (offset 8+4896, Q64.64) | Live value `11_529_215_046_068` — **exactly matches the in-code constant** |
| Liquidity mint | `So11111111111111111111111111111111111111112` (WSOL) confirmed |
| Available flash liquidity | ~230,166 SOL (`kamino_wsol_available_flash_liquidity_lamports = 230_166_406_681_510`) |
| Borrow/repay discriminators | Verified by execute path: `flash_borrow_reserve_liquidity` / `flash_repay_reserve_liquidity` accepted by the live program |

## 4.2 Instruction Sequence — PROVEN on Live Mainnet State

The V1 probe executed two envelopes through `simulateTransaction` against live mainnet state:

1. **Baseline V1 envelope** — zero instructions, accepted (err: null).
2. **`kamino_flash_roundtrip` envelope** — ATA create (idempotent) → SOL wrap → sync_native → `flash_borrow_reserve_liquidity` (1 SOL principal) → `flash_repay_reserve_liquidity` (principal only, repay index 3) — **err: null, units_consumed = 62,596**.

So the instruction ordering, account matrix, and execute semantics work against the ACTUAL deployed program — not just unit tests.

### Critical finding that was FIXED during the probe (now part of the codebase)
Fractures in the 4 flash builders:

1. **Repay amount must equal the borrow principal, NOT principal + fee.** Pre-fix, `build_flash_repay` in `dlmm_flash_arb.rs`, `dlmm_3leg_flash_arb.rs`, `dlmm_4leg_flash_arb.rs`, `cross_dex_flash_arb.rs` passed `borrow + fee`, triggering klend's `InvalidFlashRepay` (`6033 / 0x1791`). Fixed: repay passes `params.borrow_amount`; the protocol debits the fee on top.
2. **Borrow instruction index went stale after V1 compiler strips no-op ComputeBudget instructions**, shifting every executed instruction position. Fixed: `compile_v1_message` rewrites klend flash-repay indices to executed positions; dangling indices fail closed. `v1_executed_instruction_index` exposes the mapping.

## 4.3 Balance Invariants — Verified

- Repay ≥ borrow is enforced by the program itself; passing principal only with fee added by protocol is the canonical construction.
- `calculate_kamino_flash_fee` computes `ceil(borrow × sf / 2^64)` in U256 math; main-market fee ≈ 0.006 bps cannot be represented in bps — Q64.64 is mandatory.
- Under-repayment reverts atomically; the whole transaction fails rather than executing partial trades.

## 4.4 Negative Testing

| Scenario | Behavior |
|---|---|
| Missing/short reserve account | `AccountTooShort` (fail closed) |
| Bad discriminator | `BadDiscriminator` |
| Wrong mint | `MintMismatch` |
| Inactive reserve | `ReserveNotActive` |
| Wrong borrow-index | `flash_repay_checks` invariant tests + the V1 compiler's index rewrite (fail closed) |
| Insufficient liquidity | Borrow rejected by program; builder also verified-ranged |
| Insufficient repay | Atomic revert on-chain |

## 4.5 Remaining Gaps (Honest)

1. **Real signed broadcast has NOT executed on mainnet** — simulation only.
2. **Per-slot re-verification of the reserve fee/mint** — the probe is a point-in-time snapshot; the live loop re-reads the reserve every tick (fail closed if missing).
3. **InvalidFlashRepay regression is covered**, but the full flash-loan path is not yet canary-executed with real funds.

## 4.6 Summary

### Correctness
Kamino flash loans are proven on LIVE mainnet state via the V1 probe (unmargined simulation) and via the 3-instruction roundtrip execution (err: null). The discriminator constants, reserve layout offsets, and fee decoding all match the deployed program — read on 2026-11-13 against solana-core 4.4.0.

### Gate B contribution
PASS for the sequence + fee math + account matrix, evidenced by probe + 10 Kamino lib tests + the V1 compiler CAS tests. Real-funds canary is pending the operator's authorization (wallet 0.02 SOL economically marginal — see PRODUCTION_READINESS.md).
