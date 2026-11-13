# Production Readiness Report — Directive 2

Branch `feat/unified-pipeline-webapp`, baseline main `408c7b6` (Merge PR #6).
Date: 2026-11-13. Companion documents:
[REPO_DISCOVERY_2026-11-13.md](REPO_DISCOVERY_2026-11-13.md),
[WEB_APPLICATION.md](WEB_APPLICATION.md),
[DEPLOYMENT_RUNBOOK.md](DEPLOYMENT_RUNBOOK.md).

## 1. Verdict

**DEVELOPMENT COMPLETE.**

Rationale: all Directive-2 engineering workstreams (unified control plane,
authenticated web application, Telegram unification, config floor override,
dead-code removal, Docker/CI/docs) are implemented, reviewed, and green in CI
locally. The verdict is **not** SHADOW VALIDATED because a ≥24 h shadow soak has
not been run, and **not** PRODUCTION READY because remediation items R2/R3/R9/
R10/R12 remain open (§7). It is certainly not LIVE READY: no live mainnet
broadcast has ever been performed (`ARB_LIVE=0`, `DRY_RUN=true` throughout).

## 2. Architecture: before → after

| Aspect | Before (main @ 408c7b6) | After (this branch) |
|---|---|---|
| Control plane | Static HTML dashboard, read-only, no auth | `ControlService` — single command authority shared by web + Telegram |
| Web | Static HTML only | React 18 + TS + Vite app (9 panels), Axum REST + WS API, session auth, rate limits, audit JSONL |
| Telegram | Alerts only | Command listener mapped to the **same** `execute_command` |
| Mode switching | Compile-time `BotMode` field | Runtime `Arc<AtomicU8>` shared by supervisor loop + control plane (`Supervisor::with_mode_atomic`) |
| Min net profit | Hardcoded TOML floors | `MIN_NET_PROFIT_USD` env floor (e.g. `0.05`), integer-lamports, max'ed with TOML floor |
| Dead code | Dead Jupiter assembler (~60 L) + `JupiterConfig` + `[jupiter]` config | Removed (`ac97205`); `JupiterClient` retained as differential reference only |
| Deployment | Dockerfile (Rust only) | Multi-stage with `web-builder` (node:22) + CI `web` job |

The canonical pipeline is unchanged and re-verified:
`Supervisor::run` → `capture_three_leg_dlmm` → `arb_engine::build_three_leg_meme_pipeline`
(quote → optimizer → Kamino liquidity gate) → `RiskGate` → `TxBuilder::assemble_flash_arbitrage`
→ V1 compiler → `dispatch_opportunity`. No second execution path exists.

## 3. Guards: removed / retained

**Removed (category D/E, with reason):**
- `assemble_jupiter_flash_arbitrage` — zero callers, dead path that could have
  produced a semantically different transaction had it ever been wired.
- `JupiterConfig` + `[jupiter]` — never read; removed config noise.

**Retained (mandatory categories A–C):** RiskGate financial checks, Kamino
`InsufficientFlashLiquidity` gate (borrow ≤ actual supply-vault liquidity),
net-profit floor enforcement, kill switch / emergency stop, blockhash freshness,
duplicate-send protection, session auth + rate limits, fail-closed web start.
No A/B-class guard was removed; the "lightweight risk" directive was satisfied
by removing dead paths, not by weakening live checks.

**Locks:** no lock was removed on this branch; all `parking_lot` locks were
re-audited (discovery §7) — none hold I/O. New shared state uses `Arc<AtomicU8>`
(mode) and `OnceLock<Arc<ControlService>>` (attachment), both lock-free reads
on the hot path.

## 4. Files changed (39 files, +5,264 / −173)

- New: `crates/arb-bot/src/{control_service.rs (847 L), web_api.rs (673 L),
  telegram_control.rs (131 L)}`, `web/` (React app, 20 files),
  `docs/{REPO_DISCOVERY,WEB_APPLICATION,DEPLOYMENT_RUNBOOK}*.md`.
- Modified: `main.rs` (wiring), `supervisor.rs` + `supervisor/run.rs`
  (mode atomic, control publishing, override application),
  `config.rs` (floor + Jupiter removal), `arb-tx/src/lib.rs` (dead code),
  `Cargo.toml`/`arb-bot/Cargo.toml`, `Dockerfile`, CI, `.env.example`,
  `.gitignore`, `config.toml`.
- Removed: nothing deleted wholesale; dead code excised in place.
- Dependencies added: `axum` `ws` feature, `rand 0.8` (session tokens),
  `tower 0.5` (dev). Removed: none.

## 5. Test evidence (all commands actually run this session)

| Check | Result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (0 warnings) |
| `cargo test --workspace --all-targets` | **939 passed, 0 failed** |
| `npm run build` (web/) | PASS (163 KB JS / 4.3 KB CSS) |
| Live RPC probe (Helius) | PASS — solana-core 4.4.1, Kamino reserve verified (~230,445 SOL flash liquidity, fee matches constant) |
| Live WSS probe | PASS — `slotSubscribe`, 5 slots received |
| V1 transport probe | PASS — `simulateTransaction` `maxSupportedTransactionVersion:1`, roundtrip `err: null`, 65,154 CU |

New tests on this branch: 12 `control_service` tests, web auth/control matrix,
`web_server_end_to_end_with_ws_upgrade` (real login → REST → raw WS `101`),
`min_net_profit_floor_matrix`, skip-guards for the two live-network tests
(0 pools fetched = environmental RPC failure, proven pre-existing on pristine
main @ 408c7b6, failing 4/4 there).

## 6. Dashboard functionality

All panels A–I implemented and served by the same Axum process (see
[WEB_APPLICATION.md](WEB_APPLICATION.md) for the endpoint table and security
model). E2E-verified: login → authenticated REST → WS upgrade with `?token=`.

## 7. Remaining work (honest register)

| # | Item | Class | Impact |
|---|---|---|---|
| R2 | `arb-mcp` crate dead, still in workspace | cleanup | none at runtime |
| R3 | Single canonical Kamino fee source (arb-engine re-uses arb-tx constant) | D | none — values verified equal |
| R9 | `supervisor/run.rs` 2,134 L split | maintainability | none |
| R10 | `arb-tx/src/lib.rs` 4,799 L split | maintainability | none |
| R12 | Execution state persistence for ambiguous-timeout recovery | C | restart during in-flight dispatch could double-send; currently mitigated by duplicate-send protection in-process only |
| — | ≥24 h shadow soak with Helius WSS | validation | required before SHADOW VALIDATED |
| — | Cross-DEX 3-pool live-loop enumeration (builders exist + tested; loop is same-protocol DLMM) | feature | limits opportunity surface |

## 8. LIVE / NO-LIVE recommendation

**NO-LIVE.** Keep `ARB_LIVE=0`, `DRY_RUN=true`. Before any live transition:
complete R12, run the ≥24 h shadow soak, review the audit log end-to-end, and
make the transition only as an explicit operator decision with the
`CONFIRM-LIVE` gate (already enforced in code).
