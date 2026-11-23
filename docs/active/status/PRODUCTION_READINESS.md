# PRODUCTION_READINESS

**Final status: SIMULATION READY — SHADOW READY pending live 24 h soak.**
Not PRODUCTION READY and not CONDITIONAL PRODUCTION READY: no live merchantable opportunity has been observed on mainnet (all nets currently measured negative), and several P1/P2 items below are explicitly gated on operator actions.

## Checklist (mission §17 / Phase 17 gate)

| Gate | Status | Evidence |
|---|---|---|
| Complete P0 remediation | ✅ | phases 0–6 PRs; no open P0 in `OPEN_ISSUES.md` |
| Critical P1 remediation / accepted exceptions | ✅ exceptions registered (CFN-N1 legacy config precedence, EXEC-F-N1 placeholder program ID) | register |
| Clean build | ✅ | `cargo check --workspace --release` 0 errors (phases 12–14) |
| Clean clippy `-D warnings` | ✅ | all phases; CI enforces |
| Full required test suite | ✅ | workspace suites + 10 reliability suites (41 assertions) |
| Verified Anchor/SBF build | ⚠️ | not buildable in sandbox (no SBF toolchain); CI does not build the program — recorded |
| Verified program identity | ⚠️ | not deployed; identity check impossible (EXEC-F-N1 INACTIVE path) |
| Valid production configuration | ⚠️ | operator-supplied `config/config.toml` on deployment host; validator binary exists (`cargo run -p arb-bot --bin validate`) |
| Verified key management | ✅ | `docs/quality/SECURITY.md` (rotation, 600 perms, no echo) |
| Verified risk limits | ✅ | `RISK_CONTROL_POLICY.md`; fail-closed gates tested |
| Mainnet snapshot regression | ✅ | real-byte decode fixtures + QUOTE_PARITY + probe evidence |
| Successful transaction simulation | ✅ | V1 ladder; live simulate-accepted evidence 2026-11-13 (RPC-acceptance only) |
| Successful repayment verification | ✅ in sim | flash-loan lifecycle suites; NOT mainnet-executed |
| 24 h continuous Shadow Soak | ❌ | **operator action** (runbook: `SHADOW_MODE_RUNBOOK.md`) |
| No unresolved critical errors | ✅ | in scope of executed suites |
| No unexplained balance discrepancies | ✅ (sim) | intermediate-balance isolation suites |
| Operational recovery test | ✅ (doc+tests) | `DATABASE_RECOVERY.md` evolution/roundtrip tests |
| Verified monitoring | ✅ | Prometheus `:9100/metrics`, watchdog + logrotate, Telegram alerts |
| Verified shutdown procedure | ✅ | SIGTERM graceful stop, kill-switch file |

## What "SIMULATION READY" means here

- Everything short of broadcasting is verified: detection, analysis, sizing, compile, preflight, reconciliation, persistence.
- The odds are honest: with current market efficiency the engine will refuse to fire (optimal borrow = 0, fail-closed). To reach live dispatch, an opportunity must clear `net >= min_net` under integer math — nothing is loosened to make trades pass.
- The next stage (SHADOW READY → PRODUCTION) requires: (1) a 24 h live shadow soak without unexplained discrepancies, (2) creation of the `production` GitHub Environment approval gate, (3) explicit operator authorization per mission §4.5/rule §22.

## Blocked-by-operator items

1. Deploy `production` environment + required reviewers on GitHub.
2. Choose VPS region (Frankfurt/Amsterdam per measured Jito RTT).
3. On-cluster V1 transport evidence (TXC-N1).
4. Live mainnet pre-flight of the full dispatch path when an opportunity actually clears the gates.
