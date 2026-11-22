# HFT Remediation — Repository Baseline

**Date:** 2026-11-22 · **Commit SHA at baseline:** `0f0707c` (main) · **Branch for this work:** `vibe/hft-remediation-spec-8ade4e`

## 1. Baseline facts (REPOSITORY_VERIFIED)

- `git status --short` before any change: empty working tree on `main` @ `0f0707c`.
- Workspace crates audited in Phases 0–11 (1,626/0 workspace tests at Phase 2 exit; per-crate evidence in `docs/active/audit/BASELINE_TEST_RESULTS.md`).

## 2. Source-document mapping (CRITICAL DISCREPANCY)

The task instructs: "Inspect the actual repository and current Git state … The original document is a proposal, not proof."

**Fact: `docs/audit/HFT_BEST_PRACTICES_AND_REMEDIATION_SPEC.md` does not exist anywhere in this repository** (verified by `find . -name "HFT_BEST_PRACTICES*"` → zero results; `docs/audit/` contains only `EXECUTION_PATH_MAP.md`, `MASTER_FINDINGS_REGISTER.md`, `REMEDIATION_PLAN.md`).

Therefore the corrected spec is produced **from the primary evidence registers that do exist**, and the requested finding IDs are mapped to their actual equivalents:

| Requested ID | Actual equivalent (verified location) |
|---|---|
| CPI-003 | `docs/audit/MASTER_FINDINGS_REGISTER.md:19` — "Flash repayment builders — CONFIRMED ECONOMIC RISK — Pending Phase 1C" |
| CPI-001 | same register, line 17 (INTEGRATION GAP, OPEN) |
| ECON-002, QUOTE-001, QUOTE-002, SIZE-001 | same register, Phase 2 table (lines 43–46) — each already carries a current-HEAD verification + regression evidence |
| DLMM-003 / DLMM-NEW-002 / DLMM-NEW-003 | no register entry under these exact IDs; equivalent verified code paths: `crates/arb-dex/src/dlmm_validation.rs:87`, `crates/arb-tx/src/flash/dlmm_bin_discovery.rs`, `crates/arb-tx/src/protocols/meteora_accounts.rs:37,83`, `crates/arb-constants/src/lib.rs:359` |
| SIMD-0385 | `docs/active/audit/TRANSACTION_COMPILER_AUDIT.md` + `TRANSACTION_COMPATIBILITY_MATRIX.md` |

**No archive is produced** because there is no original to archive; `docs/audit/archive/` is created as the drop point. Archiving a non-existent file would be fabrication.

## 3. Feature/tooling state at baseline

- Toolchain pin: `1.99.1` (`.github/workflows/rust-ci.yml`), feature gates: `extra-ingest`, `split-routing`, `litesvm` (main CI combination).
- `reports/` exists (history: cleanup, daily, final_audit, phase15_*, phase16, phase17_*); `reports/audit/` was created by this work.
