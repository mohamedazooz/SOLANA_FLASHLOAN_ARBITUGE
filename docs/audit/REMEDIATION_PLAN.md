# Remediation Plan

## Operating constraints

- Baseline: `0c370c16bdedce08d4cc302bd9b4d23b03d91149`.
- Preserve the pre-existing local branch `fix/comprehensive-hft-remediation`; it contains five local commits and 73 changed paths relative to `origin/main`.
- Perform isolated remediation work in phase-specific branches/worktrees.
- Never enable Mainnet broadcasting or transfer assets.
- Do not merge pull requests without operator authorization.

## Pull-request sequence

| Work package | Scope | Branch | Status |
|---|---|---|---|
| Phase 1A | WEB-001, CTRL-001, AUTH-002 | `fix/hft-phase-1-safety-controls` | Implemented; validation passed; PR pending |
| Phase 1B | OPS-001, RISK-001 | To be created from verified base | Not started |
| Phase 1C | CPI-001, CPI-002, CPI-003, ARCH-001, TOKEN-003 | To be created from verified base | Not started |
| Phase 2 | ECON-002, SIZE-001, QUOTE-001, QUOTE-002 | Pending | Not started |
| Phase 2.5 | S2.5 toolchain unblock, LiteSVM e2e harness milestone A, ISSUE-P0-04 forensic capture | `vibe/blocked-unblocks-a9e0e5` | Implemented; verified locally (arb-sim 39 lib tests + 4 e2e tests, arb-bot 84 tests, clippy `-D warnings` clean); PR pending |
| Phase 3 | DLMM and Token-2022 findings | Pending | Not started |
| Phase 4 | Transaction compiler conformance | Pending | Not started |
| Phase 5 | Supervisor recovery and control consistency | Pending | Not started |
| Phase 6 | HFT profiling and optimization | Pending | Not started |
| Phase 7 | Final verification and release assessment | Pending | Not started |

Each work package must have a focused diff, regression tests, exact command output, and a separate PR. PRs are not merged automatically.
