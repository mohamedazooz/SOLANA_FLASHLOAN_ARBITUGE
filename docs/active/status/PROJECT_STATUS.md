# PROJECT_STATUS

**Date:** 2026-11-23 · **Branch lineage:** phases 0–14 merged / in-flight PRs (#145, #146, #147, #148)

## One-paragraph status

The workspace is a Rust-2024 (pinned 1.99.1) HFT arbitrage engine around a Kamino flash loan in ONE atomic transaction. The full pipeline (ingestion → quote → sizing → risk gate → V0/V1 compile → preflight simulation → dispatch → landing reconciliation → durable store) is implemented, tested and integrated; the defect backlog from the forensic audit is registered and mostly remediated. The system is **fail-closed everywhere**: unknown units, stale state, missing configuration, or unpriced routes stop execution rather than guessing.

## Verified capabilities (implemented + tested in-repo)

| Capability | Where | Evidence class |
|---|---|---|
| Multi-DEX decode (Raydium v4/CPMM/CLMM, Orca Whirlpool, Meteora DLMM, PumpSwap, Kamino reserve) | `arb-ingest`, `arb-dex` | byte-level fixtures + real mainnet decode (4 ns/op) |
| Integer quote math + optimal borrow (closed-form / GSS / split-routing) | `arb-math`, `arb-engine` | criterion 194 ns/op, exhaustive-differential tests |
| Kamino flash-loan assembly (borrow→swaps→repay, single reserve, fee exact-formula) | `arb-tx/flash/*` | lifecycle + isolation suites; 0x1773 classified (min-out enforcement revert, pricing-class) though its root cause REMAINS UNPROVEN — see `docs/active/audit/0X1773_ROOT_CAUSE_INVESTIGATION.md` |
| V0/V1 compiler (SIMD-0385 context), 4096-byte ladder | `arb-tx` | v1 size ladder + property/fuzz tests; V1 simulate-accepted evidence 2026-11-13 |
| Risk gates, kill switch, quota/failover, WS backpressure & reconnect | `arb-risk`, `arb-ingest` | 41 fault-injection assertions green |
| Dispatch: SWQoS + Jito bundles, intent-before-send, reconciliation-never-blind-retry | `arb-bot/supervisor`, `arb-land` | cross-dex supervisor suites |
| Durable records: full economic execution + bundle-id tracing | `arb-store` (PR #146) | 15/15 store tests, legacy-DB evolution test |
| Web API + React dashboard + Telegram control (single control service) | `arb-bot`, `web/` | security-contract tests, CI builds |
| Webhook service (standalone, token+HMAC, dedup, rate limit) | `webhook-service/` | 28/28 standalone tests, own CI job |
| CI/CD: SHA-pinned actions, weekly advisory re-scan, manual-only pinned-commit deploy w/ auto-rollback | `.github/*` (PR #147) | YAML-validated, operator-dispatch only |

## Known field verdict (economics)

All currently visible 2-pool WSOL routes measure **economically negative on mainnet** (bid–ask executability exceeds mid-price divergence; per-leg fee + priority fee + flash fee > divergence). Optimal borrow computes to 0 and the engine correctly reports a fail-closed dry-run. This is market efficiency, not an engine bug — the scanner/gates are functioning as designed and the engine refuses unprofitable execution. Live rehearsal logs (2026-11-23) show 0 QUOTE_FAILED after the LimitOrder-gate correction.

## Open workstreams

- TXC-N1: V1 transport toggles stay OFF until on-cluster routing evidence.
- EXEC-F-N1: vanity placeholder program ID (on-chain CPI path is INACTIVE; `INACTIVE_FEATURES.md`).
- Phase-15 residual: full live 24 h shadow soak (operator runbook exists).
- Root config precedence (CFN-N1/N2/N3), on-chain program deployment verification (blocked: not deployed).
