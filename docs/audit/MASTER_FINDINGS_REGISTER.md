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
