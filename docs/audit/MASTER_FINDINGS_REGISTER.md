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
| OPS-001 | P2 | `crates/arb-bot/src/main.rs` | CONFIRMED SECURITY ISSUE | Fail-closed config discovery (config/config.toml first, explicit bail, never auto-selects example config) | `cargo test -p arb-bot` 85+1+6/0 | FIXED IN PHASE 1B (PR #136) |
| RISK-001 | P2 | `crates/arb-risk/src/lib.rs`; `crates/arb-bot/src/control_service.rs` | CONFIRMED DEFECT | Half-open single-probe admission + immediate re-trip on probe failure + stale-probe release; 3 new regression tests | `cargo test -p arb-risk` 25/0 | FIXED IN PHASE 1B (PR #136) |
| CPI-001 | P2 | CPI assembly paths | INTEGRATION GAP | Shared `validate_flash_funded_three_pool_params` forces identical solvency/route contract on direct and CPI assemblers | `cargo test -p arb-tx` 373/2i; `--test arb_executor_cpi_tests` 6/0 | FIXED IN PHASE 1C (`1a5d526`) |
| CPI-002 | P2 | `programs/arb-executor/src/instructions/execute_three_leg.rs` | CONFIRMED DEFECT | On-chain mint-chain resolution + user token-account/mint ownership validation (`968a`) + token-account maintenance/authority checks (Phase 1D); token-program header-ID gate added Phase 1C-2 | `cargo test -p arb-executor` 37/0 incl. Program wallet admin gate tests | FIXED IN PHASE 1D + 1C-2 |
| CPI-003 | P2 | Flash repayment builders | CONFIRMED ECONOMIC RISK | `repay_amount` checked, typed `RepaymentOverflow`, saturating coverage checks removed (Phase 1E); Kamino fee math parity verified vs klend `FeeCalculation::Exclusive` at `kamino.rs:685` | `three_pool_flash_arb_tests` 18/0 | FIXED IN PHASE 1E |
| ARCH-001 | P1 | Supervisor execution assembly | INTEGRATION GAP | Execution-path proof (Phase 1C-2): supervisor production path = `compile_3leg_meme_dlmm_arb` (direct 3-leg, no flash) via `three_leg_meme_pipeline.rs` + `dlmm_flash_arb`/`meteora_atomic_builder` (flash 2-hop) via `dlmm_flash_arbitrage.rs`; flash-funded three-pool assembler + `arb-executor` CPI path remain unwired (no arb-engine/arb-bot callers) | repo-wide caller trace | OPEN — integration gap proven with evidence |
| TOKEN-003 | P1 | ATA derivation call sites | CONFIRMED DEFECT | Token-program-aware ATA derivation across Orca/DLMM/CPMM/CLMM/DAMM/LaunchLab/PumpSwap/Kamino paths; legacy 2-arg helper documented SPL-only and restricted to WSOL/USDC contexts | `cargo test -p arb-tx` (ATA divergence tests) | FIXED IN PHASE 1F |

## Phase 1B-2 — capture-rejection observability (wallet-state precondition)

| ID | Severity | Source | Classification | Remediation | Regression evidence | Status |
|---|---:|---|---|---|---|---|
| OPS-002 | P2 | `crates/arb-bot/src/supervisor/run.rs` (3-leg subsidy guard) | CONFIRMED GAP | The subsidy guard silently rejected ALL 3-leg captures when the payer WSOL/intermediate ATAs hold any balance — invisible in dashboards. Added `inventory_rejections_total` counter (arb-metrics -> ObservabilitySnapshot -> web api.ts -> Overview card) and an actionable warn naming the exact remedy (unwrap WSOL / close intermediate ATA). Economic threshold unchanged — this is observability, not gate loosening. | `cargo test -p arb-metrics -p arb-bot` 85+1+6/0; `npm run build` (tsc+vite) pass | FIXED IN PHASE 1B-2 |

NOTE (operator action): a payer WSOL ATA with any wrapped-SOL balance blocks every 3-leg capture BY DESIGN (flash-funded execution requires a zero pre-inventory settlement account). This is one likely contributor to "no opportunities seen" on wallets holding wrapped SOL dust.

## Other previously reported findings

The remaining findings from the mission brief are not closed by this register entry. They remain subject to current-HEAD source verification, reproduction, regression testing, and runtime evidence. Do not interpret absence from the table as a pass.

## Phase 1A verification

- `node --test security-contract.test.mjs`: 3 passed, 0 failed.
- `npm run build`: passed with TypeScript and Vite.
- No backend or on-chain code changed in Phase 1A.
- No transaction was signed or broadcast.

## Phase 2 — canonical economics, sizing, and quote-engine rejection bounds

Finding IDs ECON-002, SIZE-001, QUOTE-001, QUOTE-002 originate from the archived
audit register (`origin/feat/remediation-r2-r12` baseline `0c370c16`), mapped to
the ARB-xxx register as follows. Every item below was re-verified against the
current HEAD source before remediation, per the audit-baseline rule.

| ID | ARB mapping | Severity | Current-HEAD verification at `a5f0c20` | Remediation in this phase | Regression evidence | Status |
|---|---|---:|---|---|---|---|
| QUOTE-001 | ARB-003 (transfer-fee double subtraction: `apply_transfer_fee_ceil` NET return subtracted again, quoting only the fee as input) | P0 | ALREADY FIXED at HEAD: `meteora_quote_engine.rs` rejects any pool with `pool.transfer_fee_bps != 0` / `has_risky_token` (lines 62, 133) instead of pricing an incomplete Token-2022 schedule — a stricter fail-closed fix than the register's original suggestion. | No code change needed; added the missing rejection regressions. | `crates/arb-engine/tests/meteora_quote_engine_reject_tests.rs` (positive control + transfer-fee reject on both boundary pools) | FIXED IN PHASE 2 |
| QUOTE-002 | ARB-009 (hardcoded `status`/`collect_fee_mode` values) | P0 | ALREADY FIXED at HEAD: `build_dlmm_pool_and_bins` rejects `dlmm.status != 0`, `function_type > 2`, `collect_fee_mode > 1` (`meteora_quote_engine.rs:58-66`) and propagates the decoded values. | No code change needed; added the missing rejection regressions. | `crates/arb-engine/tests/meteora_quote_engine_reject_tests.rs` (status 1/2/7 and collect_fee_mode 2/5/255 rejected; modes 0/1 quoted) | FIXED IN PHASE 2 |
| ECON-002 | ARB-021 (`minimum_final == expected_final`, no slippage haircut; worse at HEAD: `expected_min_final_output` was the `0` sentinel, disabling the supervisor's conservative repayment gate for engine-discovered opportunities) | P0 | CONFIRMED DEFECT at HEAD: `make_economic_opportunity` (`arb-engine/src/lib.rs`) and the Bellman-Ford fallback constructor (`arb-engine/src/bellman_ford.rs`) both passed `expected_final` as `minimum_final_amount_raw` and set `expected_min_final_output: 0`, so the conservative gate `is_profitable()` (which uses `minimum_net_profit_signed_raw`) never saw a slippage haircut. | Added canonical integer helper `arb_math::economic::slippage_haircut_min_final` (compound per-hop ceiling deductions; checked math; >10_000 bps rejected). Both constructors now pass the haircut as `minimum_final_amount_raw` and carry it in `expected_min_final_output`, activating the supervisor's min-vs-repayment gate. | `arb-math` golden vector (2 hops @ 500 bps → 902_500 < 1_000_000, exact per-hop ceilings, bounds hard-error); `arb-engine/src/lib.rs::opportunities_carry_slippage_haircut_min_final_output` (floor > 0, < expected, equals canonical helper) | FIXED IN PHASE 2 |
| SIZE-001 | ARB-026 (golden-section f64 sizing optimizers on monetary paths) | P1 | CONFIRMED REMNANT at HEAD: pub `golden_section_size` (`arb-engine/src/lib.rs:280`) was f64 but production-dead (only caller was the CI test `tests/pipeline.rs:96`); production sizing at HEAD is the bounded integer joint search (`optimize_route_candidate`) and the DLMM multi-basin optimizer, both integer. Live-f64 sizing remains in `three_pool_router.rs` and `pump_raydium_sizing.rs`/`flash_loan_sizing.rs`, all production-unreachable from `arb-bot` at HEAD (verified by caller search) — recorded for the later phase that owns those modules. | Deleted `golden_section_size`; the pipeline regression now exercises the exact quote engine through an integer coarse-grid argmax, preserving the interior-peak intent without floating point. | `crates/arb-engine/tests/pipeline.rs::cycle_profit_peaks_at_an_interior_size` (integer grid, still asserts interior optimum beating both bounds) | FIXED IN PHASE 2 (dead-f64 removal); live-path f64 in unreached modules tracked below |

### Phase 2 verification (HEAD pre-remediation `a5f0c20`)

- `cargo test -p arb-math --lib`: 161 passed, 0 failed (includes new haircut golden vector).
- `cargo test -p arb-engine --lib`: 70 passed, 0 failed (includes new ECON-002 regression).
- `cargo test -p arb-engine --test meteora_quote_engine_reject_tests --test pipeline --test hop_slippage_once_tests`: 14 passed, 0 failed.
- Workspace `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`: recorded in the Phase 2 PR description.
- No transaction was signed or broadcast; `ARB_LIVE=0` / `DRY_RUN=true` untouched.

### Known remaining (tracked, not closed by this phase)

- `three_pool_router.rs`, `pump_raydium_sizing.rs` (`size_golden_section`/`size_brents_method`), and `flash_loan_sizing.rs` (Kelly) still contain f64 sizing code, verified production-unreachable from `arb-bot` at HEAD. They must be integerized or deleted when (or before) any wiring makes them reachable — a decision belonging to the later sizing/profiling phases.
- Bellman-Ford edge weights are f64 by documented design (discovery/ranking only; acceptance is re-quoted through the canonical integer economic gate).

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
