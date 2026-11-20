> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# CPI Final Report — Atomic DLMM Swap2 / Kamino Flash-Loan Arbitrage

**Date:** 2026-11-16
**Branch in checkout:** `newrepair` @ `b9553c1e93769b5e2c04f179b8ef5596fe4dcbf9`
(directive states `main` @ `bf9d25cd…` — mismatch, see `CPI_PHASE0_BASELINE.md`).

---

## A. Executive Summary

Foundational work (repository forensics, official SDK/IDL research, and a line-cited
economic defect analysis) is **complete and documented**. **No production code was
implemented by this campaign, and no on-chain executor exists.** The required
Anchor program with generated Meteora `swap2` CPI was not created, because this
environment has no `solana` / `anchor` CLI (it cannot be built, CPI-tested, or
simulated), and because a second agent was observed editing the same source files
concurrently — applying overlapping edits would have caused conflicts.

What was delivered:
- `CPI_PHASE0_BASELINE.md` — repository forensics (note: this file was subsequently
  overwritten by a concurrent agent with a more detailed version; that version is
  retained because it is strictly more complete).
- `CPI_SDK_COMPATIBILITY.md` — verified upstream compatibility matrix.
- `CPI_ECONOMIC_REMEDIATION.md` — six confirmed + three suspected economic defects
  with worked numerical regressions and required fixes.

What was **not** done: on-chain program, Kamino SDK wiring, any test execution by
me, differential tests, mainnet simulation, feature branch, PR.

---

## B. Architecture

**Required (directive §3) — one atomic transaction:**
```
[0] Compute Budget
[1..n] ATA init
[n+1] Kamino FlashBorrow          <- TOP LEVEL (CPI forbidden by klend)
[n+2] Arbitrage Executor         <- TOP LEVEL
        CPI 1: Meteora DLMM swap2 -> balance-delta validation
        CPI 2: Meteora DLMM swap2 -> balance-delta validation
        CPI 3: Meteora DLMM swap2 -> final WSOL / profit validation
[n+3] Kamino FlashRepay           <- TOP LEVEL, principal only
[n+4] Jito tip (where applicable)
```

**Actual in this repository today:** the same top-level `borrow → 3×swap2 → repay`
ordering, but the three swaps are **top-level instructions**, not CPIs; there is
**no on-chain executor program** and therefore no on-chain balance-delta or
min-output validation. Atomicity is Solana transaction atomicity only. The
existing `swap2` instructions are hand-built (`arb-tx/src/protocols/meteora_dlmm.rs`)
with a golden-vector-tested discriminator — not generated IDL CPI interfaces.

Kamino CPI rejection was independently confirmed from upstream source
(`is_flash_forbidden_cpi_call()` → `FlashBorrowCpi` / `FlashRepayCpi`,
`sysvar::instructions`, stack height, `user_transfer_authority: Signer`).

---

## C. Changed Files (by this campaign)

| File | Reason | Impact |
|---|---|---|
| `CPI_PHASE0_BASELINE.md` | Phase 0 deliverable | Documentation only; later overwritten by a concurrent agent's more complete version |
| `CPI_SDK_COMPATIBILITY.md` | Phase 1 deliverable | Documentation only; no dependency or lockfile change |
| `CPI_ECONOMIC_REMEDIATION.md` | Phase 2 deliverable | Documentation only; no source fix applied by me |
| `CPI_FINAL_REPORT.md` | This report | Documentation only |

**Source files were edited by a different agent during this session** (observed
mtimes ~19:00–19:01): `crates/arb-tx/src/primitives/cu.rs`,
`crates/arb-tx/src/primitives/builder.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`,
`crates/arb-tx/tests/address_regression_guards.rs`, plus two `reports/phase15_2/*.json`.
I did **not** make, review-to-completion, compile, or test those edits. Their content
at one point contained a ceiling-formula error (`budget.div_ceil(1e6)*1e6/cu_limit`)
that was corrected before my next read; it is **not verified** whether the final
state passes the golden tests.

---

## D. SDK Matrix

Full matrix with source URLs is in `CPI_SDK_COMPATIBILITY.md`. Headline pins:

| Component | Pin |
|---|---|
| Kamino program | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`; klend repo HEAD `686913ee`, release 1.28.0 |
| `klend-interface` | **0.6.0** (crates.io; git master is 0.11.0 — pin the released version) |
| Kamino flash helper | `helpers::flash::flash_loan(...) -> (borrow_ix, repay_ix)` |
| Meteora DLMM program | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` |
| DLMM IDL | `idls/dlmm.json` **0.12.0**, last IDL commit `2f540c2107988e0f97efa5917100bb912c96c17a`; SDK repo HEAD `1532e540…` |
| DLMM TS SDK | `@meteora-ag/dlmm` 1.9.14 (reference only) |
| Official CPI example | HEAD `7f428b1f…`, IDL 0.8.2, **legacy `swap`** — not a source for `swap2` |
| Anchor | none in root workspace; `declare_program!` vs IDL 0.12.0 **NOT VERIFIED** |
| Transaction V1 | 4,096 B / 64 accounts / no ALT, active since epoch 1035; matches repo invariant |

No dependency was added. Any on-chain program must be an isolated workspace
(`programs/`, excluded from root `members`).

---

## E. Financial Corrections

Full before/after formulas and worked cases are in `CPI_ECONOMIC_REMEDIATION.md`.
Confirmed defects (as of the source state I inspected):

- **B1/B2 — hop-input contract + double slippage.** Live path passes
  `[input, floor(o1), floor(o2), o3]` where the builder contract requires
  `[borrow, o1, o2, o3]`. Worked case (borrow 1e9 lam, s=1%, o1=1.01e9):
  correct `min_out_0 = 999,900,000`; wire `min_out_0 = 989,901,000`, while
  next hop needs 999,900,000 → violates `prev.min_out >= next.amount_in` by
  **9,999,000 lamports**.
- **B3 — optimistic profit ranking.** `three_leg_meme_pipeline` ranks on `out3`,
  `flash_loan_sizing` on `floor(out3·(1−s))`.
- **B4 — priority-fee double charge.** Builder added `min(gross/25k, 500k)` µlam/CU
  on top of the supervisor's already-capped price. Case: gross 1e8 lam,
  cu_limit 200k → deducted 1,000,000 lam, wire paid 1,000,800 lam. At gross
  12.5 SOL the gap reaches 100,000 lam. (I observed source edits addressing this,
  but did not verify them by test.)
- **B5 — unchecked Kamino fee** in the authoritative sizing path; `BorrowTooSmall`
  is not enforced off-chain.
- **B6 — no canonical `HopExecutionPlan`**; `PlannedHop` exists but is unused.

No fix was applied by me and no regression test was run by me.

---

## F. CPI Evidence

**None.** There is no on-chain executor program, no `declare_program!`, no
`CpiContext`, and no CPI integration test. No generated Swap2 CPI execution was
demonstrated. Any claim of "CPI integration tested" would be fabricated.

---

## G. Kamino Evidence

No new evidence generated. Existing repo evidence:
- `docs/production/kamino_flash_loan.md` and `arb-constants` pin program ID and
  discriminators (which **disagree** with each other — see D).
- `arb-tx/tests/three_pool_flash_arb_tests.rs` (per prior docs) asserts order
  borrow→3 swaps→repay, repay == principal, executed indices 3/7.
- Upstream source confirms top-level-only flash, principal-only repay, separate
  protocol-fee transfer, and `Signer` authority.

No borrow/repay instruction was generated, signed, simulated, or landed by me.

---

## H. Performance

**Not measured.** No simulation or build was run by me. Note: `cu.rs` uses
fabricated per-hop CU constants (e.g. DLMM 120,000) with a 15% buffer; the code
comment itself says the "measured" path is a reserve-depth proxy pending real
`simulateTransaction`. Directive §9's requirement for measured, route-specific CU
is **not satisfied**.

---

## I. Security

Controls confirmed present and required to remain intact: `ARB_LIVE=0`,
`DRY_RUN=true`, kill switch, loss/size/stale-state limits, fail-closed builder
gates, durable dispatch intents, no blind resubmit, fail-closed web auth. No safety
default was weakened; no secret was read or logged.

Designed-but-not-implemented on-chain validations (directive §4.2): program ID
allowlist, pool ownership, mint/authority/vault relationships, PDA derivation,
writable/signer flags, account uniqueness, ordered Bin Arrays, bitmap extension,
remaining-account rejection, mint continuity, actual balance deltas, final WSOL
sufficiency. **None of these exist on-chain.**

---

## J. Test Matrix

All test groups in directive §11 are **NOT RUN BY ME** (no reliable shell;
concurrent agent activity). No test result is claimed.

| Group | Status |
|---|---|
| A — Unit tests (SDK/fee/sizing/slippage/PDA…) | NOT RUN by me |
| B — CPI tests (19 cases) | NOT POSSIBLE — no compiled program/Solana toolchain |
| C — Kamino integration | NOT RUN by me |
| D — Mainnet non-broadcast rehearsal | NOT RUN by me; prior evidence on another branch covers the old top-level-swap architecture only |
| `cargo fmt/clippy/test` full CI | NOT VERIFIED by me |

---

## K. Remaining Blockers

1. **Baseline mismatch:** directive `main @ bf9d25cd…` vs local `newrepair @ b9553c1e…`; no fetch was possible.
2. **No feature branch / PR.** Suggested branch not created; no PR opened.
3. **No Solana/Anchor toolchain** → no on-chain program can be built, CPI-tested, or simulated.
4. **No on-chain executor** — the central deliverable of the directive.
5. **Economic defects B1–B6** not verified fixed; source edits by a concurrent agent are uncommitted and untested by me.
6. **No SDK dependency pinned**; `declare_program!` vs IDL 0.12.0 uncompiled.
7. **DBC / DLMM Pro** have execution builders in `swap_hop.rs` but no independent SDK differential tests or execution simulation — directive §12 says keep such protocols disabled; this needs explicit operator decision.
8. **Kamino discriminator doc/code discrepancy** unresolved.
9. **CU values are estimates, not measurements** (directive §9).
10. **Concurrent agent** editing the same working tree; changes must be reviewed and reconciled before any commit.

---

## L. Pull Request

- Branch: **none created** (`newrepair` checkout retained).
- Commit SHA: none (no commit made by me).
- PR URL: **none**.
- CI status: **not verified by me**.

---

## M. Production Verdict

**NOT IMPLEMENTED.**

Rationale: the required on-chain executor does not exist; no generated `swap2` CPI
was executed; no economic fix was verified by compilation or test; no branch or PR
was created. Foundational research and analysis are complete, but none of the
Definition-of-Done conditions that require compilation, CPI execution, simulation,
or a PR can be honestly checked. Live trading was neither enabled nor requested;
`ARB_LIVE=0` remains intact.
