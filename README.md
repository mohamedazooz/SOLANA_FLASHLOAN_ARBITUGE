# Solana Flash-Loan Arbitrage Engine

> Atomic multi-DEX arbitrage on Solana: 100 % Kamino flash-loan funded, single atomic transaction, integer-only economics, fail-closed by construction. **Status: SIMULATION READY · SHADOW pending soak. NOT live-trading authorized.**

## 1. Project overview

A Rust workspace (edition 2024, pinned toolchain 1.99.1) implementing the complete arbitrage pipeline: market-data ingestion, route discovery, integer quote math, research sizing, risk gates, V0/V1 transaction compilation, preflight simulation, dispatch (SWQoS / Jito bundles), landing reconciliation and durable persistence — plus a React control dashboard, Telegram command surface and a standalone webhook service.

## 2. Current project status

- **Verified in-repo:** decode (byte-exact fixtures + live matrices), quoting (criterion-benched, 17 ns/hop), closed-form optimal sizing (194 ns), flash-loan lifecycle, V1 4096-byte ladder, fault-injection reliability suites (41 assertions), store reconstructability.
- **Measured 2026-11-23:** full deterministic local path ≈ 15 µs; network dominates (RPC RTT 31–39 ms, Jito Amsterdam 52 ms).
- **Honest economics:** all currently visible 2-pool WSOL opportunities on mainnet measure net-negative; optimal borrow = 0 and the engine fail-closes. This is correct behavior against efficient markets, not a defect.
- Live execution requires operator authorization; nothing broadcasts from this codebase by default.

## 3. Safety warning

> **This software can move funds.** It ships in shadow mode (`ARB_LIVE=0`, `DRY_RUN=true`, enforced). Run it live only on infrastructure you own, with funds you can lose, after completing [`docs/active/status/PRODUCTION_READINESS.md`](docs/active/status/PRODUCTION_READINESS.md). Do not trust quotes, simulations, bundle acceptances or RPC acceptance as realized profit. There is no support and no warranty.

## 4. Architecture (actual code path)

```
Helius WS / gRPC / HTTP reconcile ──► arb-ingest (freshness, slot + write-version)
  ──► arb-engine (route graph, discovery, guarded quotes, sizing)
  ──► arb-math  (integer AMM/CLMM/DLMM math, canonical economics)
  ──► arb-risk  (gates, kill switch)
  ──► arb-tx    (instruction builders, V0/V1 compiler, flash-loan assemblers)
  ──► arb-sim   (preflight simulation, optional litesvm)
  ──► arb-bot   (supervisor: intent-before-send dispatch, SWQoS/Jito, web API, Telegram)
  ──► arb-land + arb-store (landing reconciliation, SQLite-WAL/Postgres durability)
```

## 5. Core capabilities

Detection → analysis → sizing → compilation → preflight → dispatch decision → reconciliation → persistence, with single-unit accounting doctrine (base-mint raw atomic units; lamports for network costs), integer-only financial acceptance, and NULL-never-zero unknowns.

## 6. Supported DEX protocols — Quote vs Execution distinction

| Protocol | Decode | Quote | Sizing | Instruction build | In preflash sim | Mainnet verified |
|---|---|---|---|---|---|---|
| Meteora DLMM | ✅ | ✅ (incl. LimitOrder function_type pools) | ✅ bin-exact | ✅ | ✅ | reading + dry-run only |
| Raydium v4 / CPMM | ✅ | ✅ | ✅ | ✅ | ✅ | reading + dry-run only |
| Raydium CLMM | ✅ | ✅ | ✅ | ✅ | ✅ | reading + dry-run only |
| Orca Whirlpool | ✅ | ✅ | ✅ | ✅ | ✅ | reading + dry-run only |
| PumpSwap | ✅ | ✅ | ✅ | ✅ | ✅ | reading only |
| Jupiter (consultative quotes) | ✅ cross-check | parity/validate | n/a | ❌ not used in prod path | n/a | read-only |

Full matrix: `docs/active/audit/DEX_SUPPORT_MATRIX.md`. "Mainnet verified" above means read-only validation — **never** a funded swap.

## 7. Flash loan support

Kamino KLend single-reserve flash loan, exact provider fee formula, borrow/repay in one atomic transaction, dynamic borrow bounded by validated reserve supply-vault liquidity (never hardcoded). Marginfi/Solend studied as alternatives, not integrated (adding a provider requires demonstrated advantage per mission rules — `docs/active/research/FLASH_LOAN_RESEARCH.md`).

## 8. Transaction formats

- **V0** with ALTs: supported, tested round-trip + index remapping.
- **V1 (SIMD-0385 context, 4096 bytes):** production primary path; size ladder `arb-tx/tests/v1_size_ladder_tests.rs`; live simulate-accepted evidence 2026-11-13. Transport toggles for V1 dispatch remain OFF until on-cluster evidence (TXC-N1) — safe V0 fallback retained.

## 9. Market data architecture

WS/gRPC streaming + HTTP polling with provider supervision, dedup ring, slot/write-version freshness gating, dead-slot invalidation, bounded queues, disconnect→backoff→RPC-fallback. Streaming stays secondary until runtime consistency is proven; HTTP polling remains source of truth.

## 10. Jito architecture

Bundle tip selection, regional engines, leader-aware dispatch, reconciliation (accept ≠ landed ≠ finalized), no blind retries. Measured RTTs favor Frankfurt/Amsterdam.

## 11. Simulation

Preflight with real live state (`arb-sim`), litesvm behind an optional feature, mainnet account snapshots for decode differential tests; dry-run verdicts fail-closed on any insufficient-output.

## 12. Risk management

Integer profit gate with all costs (flash fee, DEX fees, priority fee, tip, base fee, safety margin), max-size/max-loss/slippage/state-age limits, kill switch file, provider/RPC circuit breakers, quota-exhaustion cooldowns. Policy: `docs/active/operations/RISK_CONTROL_POLICY.md`.

## 13. Repository structure

13 crates (`arb-types` … `arb-bot`), `programs/arb-executor` (on-chain program — INACTIVE: not deployed; see `docs/active/status/INACTIVE_FEATURES.md`), `web/` (dashboard), `webhook-service/` (standalone crate, out-of-workspace), `scripts/` (VPS + watchdog + logrotate), `config/`, `docs/` (active/inactive/archive doctrine).

## 14. Technology stack

Rust 2024 (1.99.1), tokio, SQLite-WAL/Postgres, Solana Agave SDK crates, Anchor (program), React+Vite+TypeScript, Node 22 (web + webhook), GitHub Actions SHA-pinned, Docker multi-stage (pinned rust:1.99.1-slim-bookworm).

## 15. Prerequisites

Linux (or WSL2/Docker — `yellowstone-grpc-client` is Unix-only), Rust 1.99.1 via `rust-toolchain.toml`, Node 22 for `web/`, pkg-config + OpenSSL, optional `gh` CLI.

## 16. Installation

```bash
git clone https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git
cd SOLANA_FLASHLOAN_ARBITUGE
cp .env.example .env            # fill values; chmod 600; never commit
cp config/config.example.toml config/config.toml
```

## 17. Configuration

`config/config.example.toml` is the tracked reference (copy it to the ignored `config/config.toml`). Default startup uses only that canonical path; the legacy root `config.toml` is not auto-selected. Local tests can use `--config config/testing.toml` with `ARB_MODE=simulate` and `ARB_LIVE` unset (loopback RPC, isolated DB/ports). Configuration selection and environment override behavior: `docs/active/security/CONFIGURATION_SECURITY.md`.

## 18. Environment variables

Documented exhaustively in `.env.example` (Helius RPC/WS, provider keys, Telegram, ARB_LIVE, DRY_RUN, KILL_SWITCH_FILE, …). Secrets stay out of git; gitleaks runs in CI on full history.

## 19. Development commands

```bash
bash scripts/verify-linux.sh    # one-shot: fmt, check, clippy, tests
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo bench -p arb-math
```

## 20. Testing

Workspace unit + integration suites (incl. protocol byte fixtures, lifecycle, economics, compiler LTO/size, fault injection). Selected evidence: `docs/quality/SHADOW_EXECUTION_REPORT.md`, `docs/quality/ECONOMIC_VERIFICATION.md`, `docs/active/performance/RELIABILITY_REPORT.md`.

## 21. Simulation mode

Default dev loop: `cargo run -p arb-bot --bin [<live_scanner|replay|validate>]` — all read-only. `validate` cross-checks our on-chain math against Jupiter venue-restricted quotes on live reserves. Nothing broadcasts.

## 22. Shadow mode

Full pipeline, never broadcasts (`--mode shadow` / `ARB_LIVE=0`+`DRY_RUN=true`). Soak procedure + exit criteria: `docs/active/operations/SHADOW_MODE_RUNBOOK.md`.

## 23. Deployment prerequisites

VPS (Ubuntu/Debian), GitHub secrets VPS_HOST/USER/PORT/SSH_KEY, `production` GitHub Environment with required reviewers enabled, funds in a dedicated deploy wallet only.

## 24. Security model

Fail-closed defaults; secrets only in 600-perm `.env`; SSH via pinned deploy key; manual-only `workflow_dispatch` deploys pinned to the exact commit; action SHAs pinned; gitleaks+cargo-deny CI; provenance doctrine: no claim without recorded command output. Model: `docs/active/security/CONFIGURATION_SECURITY.md`, `docs/SECURITY.md`.

## 25. Known limitations

- On-chain CPI program NOT deployed (INACTIVE; vanity placeholder ID).
- All scans normal-efficiency-mainnet yield net-negative routes (expected).
- No Postgres/Redis chaos tests; perf_event unavailable in sandbox.
- Yellowstone streaming consistency unproven at runtime (TXC-N1).
- Live 24 h shadow soak not yet executed.

## 26. Roadmap

`(1)` operator-run 24 h shadow soak → SHADOW READY · ` (2)` V1 transport on-cluster probe (TXC-N1) · `(3)` multi-provider flash loans only if measured advantage · `(4)` streaming promotion if runtime-consistency evidence lands · `(5)` live authorization solely per §22 checklist + explicit operator approval.

## 27. Documentation index

| Area | Entry |
|---|---|
| Docs index | [`docs/README.md`](docs/README.md) · active tree `docs/active/README.md` |
| Status | [`docs/active/status/PROJECT_STATUS.md`](docs/active/status/PROJECT_STATUS.md) · [`PRODUCTION_READINESS.md`](docs/active/status/PRODUCTION_READINESS.md) · [`KNOWN_ISSUES.md`](docs/active/status/KNOWN_ISSUES.md) · [`INACTIVE_FEATURES.md`](docs/active/status/INACTIVE_FEATURES.md) |
| Architecture | `docs/architecture/`, `docs/active/architecture/` |
| Audits | `docs/active/audit/` (flash-loan forensic, DEX matrix, quote parity, engine audit, compiler audit) |
| Security | `docs/active/security/`, `docs/quality/SECURITY.md` |
| Operations | `docs/active/operations/` (deployment architecture, VPS runbook, rollback, shadow runbook, incident response, DB recovery, risk policy) |
| Performance | `docs/active/performance/` (baseline + reliability) |
| Archive | `docs/archive/` (historical superseded reports; provenance only) |

## 28. Contribution guidelines

Small focused PRs; every claim backed by a fresh command; never mark unimplemented features complete; CI must be green on its own evidence, not descriptive names; tests prove invariants (repay-guaranteed, stale-balance isolation, fail-closed unknowns). Match the established cargo gate: fmt → check → clippy `-D warnings` → tests. Integration for external-service work lands via PR review; high-risk and production-affecting changes require explicit operator approval.

## 29. Operate without reading history

A senior engineer needs exactly three files to operate this repo: this README, `docs/active/status/PROJECT_STATUS.md` (what works, what's proven), `docs/active/operations/VPS_RUNBOOK.md` (how to run/safeguard). Everything else is depth-on-demand.

## 30. License

No license file currently tracked in the repository root — all rights default to repository owner; clarify license before any third-party use.
