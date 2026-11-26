# Baseline Verification & Test Results (`origin/main` @ `b9b516c`)

**Executed On**: 2026-11-26  
**Branch**: `phase/00-baseline-audit` (`b9b516c`, clean checkout of `origin/main`)  
**Environment**: Windows x64, `rustc 1.99.1`, `cargo 1.99.1`, `node v24.18.0`, `npm 11.16.0`

---

## 1. Summary Matrix

| Check | Command | Outcome | Notes |
|---|---|---|---|
| Formatting | `cargo fmt --all -- --check` | **PASS** (exit 0) | Clean across workspace |
| Default compilation | `cargo check --workspace --all-targets` | **PASS** (exit 0) | Completed in 1m 26s |
| Feature compilation | `cargo check --workspace --all-targets --features arb-bot/extra-ingest,arb-ingest/extra-ingest,arb-bot/split-routing,arb-engine/split-routing,arb-sim/litesvm` | **PASS** (exit 0) | Completed in 2m 29s |
| Shadow-only feature | `cargo check -p arb-bot --features shadow-only` | **PASS** (exit 0) | Completed in 28.29s |
| Strict Clippy | `cargo clippy --workspace --all-targets --features arb-bot/extra-ingest,arb-ingest/extra-ingest,arb-bot/split-routing,arb-engine/split-routing,arb-sim/litesvm -- -D warnings` | **PASS** (exit 0) | 0 warnings across all targets & enabled features |
| Full Workspace Tests | `cargo test --workspace --all-targets` | **PASS** (exit 0) | **151 test binaries: 1,844 passed, 0 failed, 7 ignored** |
| Webhook Service | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` in `webhook-service/` | **PASS** (exit 0) | **6 test binaries: 28 passed, 0 failed** (`11 + 0 + 3 + 6 + 4 + 4`) |
| Web Typecheck & Build | `npx tsc --noEmit` & `npm run build` in `web/` | **PASS** (exit 0) | `vite v6.4.6` production bundle built in 12.84s |
| Web Security Contract | `node --test web/security-contract.test.mjs` | **PASS** (exit 0) | **3 passed, 0 failed** |
| `cargo-audit` | `cargo audit` | **NOT INSTALLED** | Binary absent in local Windows toolchain |
| `cargo-deny` | `cargo deny check` | **NOT INSTALLED** | Binary absent in local Windows toolchain (enforced in CI `.github/workflows/rust-ci.yml`) |
| Anchor / SBF Build | `cargo build-sbf` / `anchor build` | **NOT AVAILABLE** | Neither `solana` CLI nor `anchor` CLI installed on Windows host |

---

## 2. Ignored Workspace Tests (Exact Recorded List)

All 7 ignored tests from `cargo test --workspace --all-targets` are opt-in live Mainnet RPC / simulation diagnostics requiring live RPC cluster stability and explicit `--ignored` execution:

1. `test_fresh_state_meteora_cross_pool_cycle` (`ignored, opt-in mainnet live simulation; requires live cluster stability`)
2. `test_live_mainnet_meteora_dlmm_cross_pool_simulation` (`ignored, manual opt-in Mainnet RPC simulation; deterministic test signer only; no broadcast`)
3. `test_live_alt_compiles_v0_and_measures_bytes` (`ignored, live RPC; run with --ignored`)
4. `test_live_alt_validates_and_warmup` (`ignored, live RPC; run with --ignored`)
5. `test_mainnet_non_broadcast_simulation_cross_dex_dlmm_cpmm_kamino_v1` (`ignored, opt-in mainnet diagnostic; fixed diagnostic thresholds are not live quotes (see OPEN_ISSUES.md ISSUE-P0-04)`)
6. `test_mainnet_non_broadcast_simulation_orca_whirlpool` (`ignored, opt-in mainnet live simulation; requires live cluster stability`)
7. `test_mainnet_non_broadcast_simulation_raydium_v4_v2` (`ignored, opt-in mainnet live simulation; requires live cluster stability`)

---

## 3. Forensic Interpretation

Green CI and 1,844 passing tests confirm strong coverage on covered invariants, **not** the absence of production defects. Forensic code inspection in Phase 0 identified multiple concrete safety, protocol, configuration, and telemetry defects (`PUMPSWAP-01`, `SWAP-01`, `CPI-003`, `CPI-001`, `TOKEN-003`, `SIZE-001`, `CFN-01`, `TM12`) documented in [`INITIAL_FINDINGS.md`](INITIAL_FINDINGS.md).
