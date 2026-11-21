# Master Findings Register

- Repository: `mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`
- Audit baseline: `0c370c16bdedce08d4cc302bd9b4d23b03d91149`
- Register status: incremental; not a declaration that the full audit is complete.
- Live broadcasting: must remain disabled.

## Phase 1A — Dashboard authentication and control transport

| ID | Severity | Source | Classification | Remediation | Regression evidence | Status |
|---|---:|---|---|---|---|---|
| WEB-001 | P1 | `web/src/components/Login.tsx` | CONFIRMED DEFECT | Removed automatic login and fixed credential attempt; login now occurs only from explicit form submission. | `npm run test:security` | FIXED IN PHASE 1A |
| CTRL-001 | P1 | `web/src/App.tsx`; `crates/arb-bot/src/control_service.rs` | CONFIRMED DEFECT | Dashboard now calls the canonical `/api/control/{action}` contract. Resume clears the emergency latch, then explicitly calls `start`; no optimistic local success state is set. | `npm run test:security`; backend route contract inspected | FIXED IN PHASE 1A |
| AUTH-002 | P2 | `web/src/App.tsx`; `web/src/components/Logs.tsx`; `web/src/api.ts`; `crates/arb-bot/src/web_api.rs` | CONFIRMED DEFECT | Both WebSocket consumers now request fresh single-use tickets through `api.wsUrl()`; long-lived session tokens are not placed in WebSocket URLs. | `npm run test:security`; backend single-use ticket tests exist | FIXED IN PHASE 1A |
| OPS-001 | P2 | `crates/arb-bot/src/main.rs` | CONFIRMED SECURITY ISSUE | Pending Phase 1B | Pending | OPEN |
| RISK-001 | P2 | `crates/arb-risk/src/lib.rs`; `crates/arb-bot/src/control_service.rs` | CONFIRMED DEFECT | Pending Phase 1B | Pending | OPEN |
| CPI-001 | P2 | CPI assembly paths | INTEGRATION GAP | Pending source-path parity audit | Pending | OPEN |
| CPI-002 | P2 | `programs/arb-executor/src/instructions/execute_three_leg.rs` | CONFIRMED DEFECT | Pending Phase 1C | Pending | OPEN |
| CPI-003 | P2 | Flash repayment builders | CONFIRMED ECONOMIC RISK | Pending Phase 1C | Pending | OPEN |
| ARCH-001 | P1 | Supervisor execution assembly | INTEGRATION GAP | Pending execution-path proof | Pending | OPEN |
| TOKEN-003 | P1 | ATA derivation call sites | CONFIRMED DEFECT | Repository-wide remediation pending | Pending | OPEN |

## Other previously reported findings

The remaining findings from the mission brief are not closed by this register entry. They remain subject to current-HEAD source verification, reproduction, regression testing, and runtime evidence. Do not interpret absence from the table as a pass.

## Phase 1A verification

- `node --test security-contract.test.mjs`: 3 passed, 0 failed.
- `npm run build`: passed with TypeScript and Vite.
- No backend or on-chain code changed in Phase 1A.
- No transaction was signed or broadcast.

## Phase 2.5 — Blocked-items unblock (SBF toolchain, LiteSVM harness milestone A, P0-04 capture)

Baseline: `main` at `a5f0c20` (merge of #112). Evidence verified 2026-11-21 in-sandbox.

| ID | Source | Classification | Remediation | Evidence | Status |
|---|---|---|---|---|---|
| S2.5-TOOLCHAIN | `programs/arb-executor` SBF build | BLOCKED → UNBLOCKED | Installed Agave toolchain (cargo-build-sbf 4.5.0, platform-tools v1.57); added missing `[lib] crate-type = ["cdylib", "lib"]` to `programs/arb-executor/Cargo.toml` (without it the end-to-end build produced no deploy artifact). | `target/deploy/arb_executor.so` sha256 `faf2b7e6ffd9aa8d748406bd10c4d39e3fc3c0fac3d55e725d9eecdac76e31b8` (64,728 bytes) | UNBLOCKED |
| S2.5-BLOBS | Mainnet klend/dlmm binaries | BLOCKED → UNBLOCKED | Fetched via RPC `getAccountInfo` (BPFLoaderUpgradeable programdata pointers); ELF slices hash-pinned. Programdata header is 45 bytes (`4B tag + 8B slot + 33B Option<authority>`), not 9. | klend ELF sha256 `6482390f659a6774b059768313b6f701b14819110f9e10db8fa651a311e9c97b`; dlmm ELF sha256 `d296c6771cec945601027613ca637c1be6721044c5859009d41c453477844c1f` | UNBLOCKED |
| S2.5-E2E | `crates/arb-sim/tests/litesvm_arb_executor_e2e.rs` (new) | CONFIRMED GAP | Milestone-A harness: real `arb_executor.so` deploys in LiteSVM and dispatches typed `Custom(6000)`; short-data guard; VM boot via system transfer; mainnet klend+dlmm blobs deploy. Feature-gated (`litesvm`); artifacts resolve from `ARB_EXECUTOR_SO` / `LITESVM_PROGRAM_BLOBS`; explicit SKIP when absent. | `cargo test -p arb-sim --features litesvm --test litesvm_arb_executor_e2e`: 4 passed, 0 failed | FIXED IN PHASE 2.5 |
| ISSUE-P0-04 | `crates/arb-sim/src/forensic.rs` (new) | CONFIRMED GAP | `ForensicBundle` deterministic-replay capture (sha256-pinned signed wire, per-hop provenance, bounded logs, emitting-program hint, `replay_locally` via fail-closed LiteSVM), wired into supervisor pre-flight failure paths writing `reports/forensics/`. Historical `0x1773` remains UNRESOLVED (no capture existed); future failures are capturable and replayable. | `cargo test -p arb-sim --features litesvm --lib`: 39 passed (incl. 5 forensic tests); `cargo test -p arb-bot`: 84 passed | CAPTURE DELIVERED; ROOT CAUSE STILL OPEN |

Remaining out of scope (BLOCKED): the full economic LiteSVM borrow→swap→repay proof (requires a consistent
mainnet account snapshot at one slot — Kamino reserve/market, DLMM pool/bin arrays, CPMM pool, vaults) and
any live broadcast / 24h shadow soak / Geyser feed coupling to external infrastructure. Live mode stays disabled.
