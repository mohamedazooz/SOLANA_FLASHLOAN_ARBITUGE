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
