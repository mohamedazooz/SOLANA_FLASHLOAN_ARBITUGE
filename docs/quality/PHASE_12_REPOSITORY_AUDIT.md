# Phase 12 — Repository Forensic Audit (2026-11-13)

> Scope: full forensic re-audit of `main` @ `093b20e` (merge of PR #11) as the
> baseline for the Phase 12→15 campaign. Every claim below was re-verified
> against the actual tree on 2026-11-13 — nothing is carried forward from
> earlier reports on trust. Cross-references: [REPO_DISCOVERY_2026-11-13.md](REPO_DISCOVERY_2026-11-13.md),
> [FINAL_VERIFICATION_2026-11-13.md](FINAL_VERIFICATION_2026-11-13.md),
> [PRODUCTION_READINESS_2026-11-13.md](PRODUCTION_READINESS_2026-11-13.md).

## 1. Repository state at audit start

| Item | Value |
|---|---|
| Branch | `main`, clean working tree, up to date with `origin/main` |
| HEAD | `093b20e` — Merge PR #11 (store fail-closed, R-I) |
| Workspace | 13 crates (`arb-types` … `arb-bot`), edition 2024, pinned 1.99.1 |
| Source files | 186 `.rs` files under `crates/`, 49 test-target files |
| Open PRs | #12 `vibe/ops-monitoring-9d81d9` (DRAFT — panic hook, systemd watchdog, log rotation; touches `.env.example` Helius section only) |
| CI | `.github/workflows/rust-ci.yml` (fmt/clippy/test/web) |
| Dependency audit | `deny.toml` present; prior RUSTSEC findings resolved (`prometheus 0.14`, `rusqlite 0.42`, `tokio-postgres`) |

## 2. Verification run (this audit, this machine)

`cargo fmt --all -- --check` → clean.
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace --all-targets` → results recorded in
[PHASE_15_FINAL_REPORT.md](PHASE_15_FINAL_REPORT.md) §Testing (filled from the
actual run on the merged state; not copied from prior reports).

## 3. Findings

Severity scale: CRITICAL (fund loss / soundness) > HIGH (wrong behavior
reachable in production config) > MEDIUM (misleading docs / dead surface) > LOW.

### F-1 — Profitability gate could pass a structural loss when the hurdle is 0 — FIXED
- **Where:** `crates/arb-sim/src/profitability_gate.rs` (net-profit step, was line 235).
- **Root cause:** `net_profit = gross_edge.saturating_sub(total_costs)` maps a
  guaranteed loss (`gross_edge < total_costs`) to `0`; with
  `strict_profit_hurdle == 0` the check `0 >= 0` passes the gate. Today every
  construction site uses the 50,000-lamport default, so the bug was latent —
  but the invariant "a loss can never pass" must be structural, not incidental.
- **Impact:** potential execution of money-losing routes if a future
  configuration sets the hurdle to 0.
- **Fix:** explicit `NetProfitNegative { gross_edge, total_costs }` rejection
  before the hurdle comparison; subtraction is now provably non-clamping.
- **Tests:** `test_profitability_gate_rejects_structural_loss_even_with_zero_hurdle`,
  `test_profitability_gate_zero_hurdle_passes_exact_breakeven` (same file).

### F-2 — 69 dead/phantom environment keys in `.env.example` — FIXED
- **Where:** `.env.example` (128 keys → 59).
- **Root cause:** template accreted keys from abandoned architectures
  (Sentinel admin/WS, HuggingFace/S3, Solami Beam/gRPC/webhook, webhook domain,
  Jupiter-adjacent price feeds) and from providers that were never part of the
  execution path: **1inch, OKX, Binance, Arbitrum/EVM, Flashbots, Titan,
  Beaver**. Also `ENABLE_ARBITRUM_FLASHLOANS=true` — an unused key defaulting
  to `true`, and `HOST=0.0.0.0` for a control plane that is documented
  localhost-bind.
- **Method:** every key was grep-verified against `crates/`, `web/`,
  `dashboard/`, `scripts/`, `config/`, compose files, `Dockerfile`, and CI —
  with exact-quoted-string matching for short keys (`ENV`, `PORT`, `HOST`,
  `LIVE`, `DRY_RUN`) to eliminate substring false negatives. A key was removed
  only with **zero** consumers.
- **Kept deliberately:** `HELIUS_LASERSTREAM_ENDPOINT` (reserved future ingest
  path, documented in PR #12 and Remaining_Risks) and `LIVE`/`EXECUTE_LIVE`
  (real gates in `main.rs:140-142`).
- **Impact:** operators were invited to provision credentials for providers the
  bot never reads — pure secret-sprawl risk, zero function.

### F-3 — `DRY_RUN` is not a real gate — DOCUMENTED
- **Where:** `.env.example` (was `DRY_RUN=false`); sole consumer is
  `crates/arb-bot/src/heartbeat.rs:245` (heartbeat reporting only).
- **Risk:** the template implied a dry-run switch that does not gate dispatch.
  The actual gates are `ARB_LIVE` (`main.rs:76-79`), `LIVE`/`EXECUTE_LIVE`
  (`main.rs:140-142`), and `ARB_MODE` (CLI > env > config > `shadow`).
- **Fix:** template now ships `DRY_RUN=true` with an explicit comment that it
  is heartbeat-reported only; recorded in KNOWN_LIMITATIONS.

### F-4 — `SOLANA_TX_V1_JITO_ENABLED` documented but unwired — FIXED (doc-side)
- **Where:** `.env.example` V1 block claimed a Jito-ingress V1 toggle with a
  "version-aware size gate in `validate_jito_bundle`". Exact-string grep shows
  **no** consumer; only `SOLANA_TX_V1_DIRECT_ENABLED` is read
  (`supervisor/dispatch.rs:248`, `supervisor/opportunity.rs:766`).
- **Impact:** an operator could believe V1-over-Jito is configurable. It is not.
- **Fix:** key removed; comment corrected; limitation recorded in
  [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md).

### F-5 — Duplicated `MIN_NET_PROFIT_USD` with conflicting values — FIXED
- **Where:** `.env.example` had `MIN_NET_PROFIT_USD=0.05` and, later,
  `MIN_NET_PROFIT_USD=0.10` under "Target Pairs & Thresholds".
- **Impact:** whichever line the operator's dotenv loader keeps last silently
  doubles the profit floor; two sources of truth for one economic parameter.
- **Fix:** duplicate removed; single canonical entry (0.05) remains.

## 4. Re-verification of prior claims (spot checks, this audit)

| Claim | Method | Result |
|---|---|---|
| Kamino flash borrow/repay discriminators | `sha256("global:flash_borrow_reserve_liquidity")[:8]` computed independently | `[135, 231, 52, 167, 7, 52, 212, 193]` and `[185, 117, 0, 203, 96, 245, 180, 186]` — byte-identical to `crates/arb-constants/src/lib.rs:337-339` ✅ |
| Flash loan is a real klend borrow/repay, not a token transfer | Read `crates/arb-tx/src/flash/kamino.rs` + `kamino_flash.rs`: program `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`, 12-account matrix, reserve decode with discriminator check, U68F60 fee from live reserve | Confirmed ✅ |
| `saturating_sub` in money paths | Full-crate grep + manual review | All remaining uses are either (a) fee-complement math (`10_000 - fee_bps`, cannot underflow by construction), (b) slot-lag deltas, or (c) signed-`i64` profit paths where negatives are preserved (`flash_loan_sizing.rs:net_profit_lamports`). The one loss-masking use is fixed by F-1 ✅ |
| Jito tip accounting cannot mask a loss | `crates/arb-land/src/bidder.rs` | `ceiling = min(edge − hurdle, share cap)`; refuses when `ceiling < MIN_JITO_TIP_LAMPORTS`; margin saturation is unreachable in the valid range ✅ |
| Store fail-closed (R-I) | `supervisor.rs:141,148,263` | `DATABASE_URL`-driven, explicit fail-closed errors, no silent in-memory fallback ✅ |

## 5. Agent work split and file ownership (conflict control)

| Agent | Files touched | Files forbidden |
|---|---|---|
| A1 (on-chain/flashloan) | `arb-tx/src/flash/*`, `arb-math/src/economic*` | `arb-engine`, `arb-land` |
| A2 (searcher/financial) | `arb-engine/*`, `arb-ingest/*`, `arb-sim/src/profitability_gate.rs` | `arb-tx` builders |
| A3 (tx/ops) | `arb-tx/src/transport/*`, `arb-land/*`, `.env.example`, docs | `arb-math` |
| Supervisor (this audit) | `.env.example`, `arb-sim/src/profitability_gate.rs`, `docs/quality/*`, `docs/operations/ROLLBACK_PLAN.md` | — |

PR #12 (`ops-monitoring`) touches `.env.example` only in the Helius comment
block (lines ~84-100); this campaign's `.env.example` edits are elsewhere in
the file — no textual overlap, both merge cleanly.

## 6. Reproduction

```bash
git checkout 093b20e
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
# F-1 regression tests:
cargo test -p arb-sim profitability_gate
```
