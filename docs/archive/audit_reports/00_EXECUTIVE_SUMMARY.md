# 00 — Executive Summary

Independent forensic audit of the Solana atomic-arbitrage workspace at `D:\work_new`.
Date: 2026-11-11. Auditor: independent (no prior reports trusted). Mode: strictly non-live.
No transaction was broadcast, no ALT was created/extended/deactivated, no funds were moved,
no production credential was used or printed.

## Safety-constraint verification

| Control | Required | Observed on disk | Enforced by code? |
|---|---|---|---|
| ARB_LIVE | 0 | **`ARB_LIVE=1` in `.env` (untracked by git, present on disk)** | **No — no code reads `ARB_LIVE`** |
| LIVE | false | **`LIVE=true`** | **No — not read by any crate; live mode is selected via `config.toml [bot] mode = "live"`** |
| EXECUTE_LIVE | false | **`EXECUTE_LIVE=true`** | **No — not read by any crate** |
| DRY_RUN | true | not present | n/a (mode/config not DRY_RUN either) |

The live-execution gate is `config.toml` `[bot] mode = "live"` (config.toml:5), **not** the env
flags. `ENABLE_SOLANA_JITO`, `MAX_JITO_TIP_SOL`, `ARB_LIVE`, `LIVE`, `EXECUTE_LIVE` are **dead
knobs** — zero references in `crates/` (see `audit_work/11_JITO.md` §8). The shipped
`config.toml` also sets `leader_gate_enabled = false` → `tip_on_unknown_leader = true`,
inverting the code's fail-closed default (supervisor.rs:386-390). **The repository, as shipped,
is configured to fire real bundles with real tips.** This audit did not run `.env`; it inspected
it as evidence.

## Verification commands (recorded)

| Command | Result |
|---|---|
| `cargo check --workspace --all-targets` | **PASS** (exit 0, 1m 52s) — `check_audit.log` |
| `cargo fmt --all -- --check` | **FAIL** — diffs in `arb-execution/src/merger.rs`, `arb-execution/src/v0_compiler.rs`, `arb-tx/tests/v1_header_differential.rs`, `arb-tx/tests/v1_header_unit.rs` |
| `cargo clippy --workspace --all-targets -- -D warnings` | **FAIL** — first run errored on `v1_header_differential.rs` (E0308/E0507, files edited on disk mid-audit by an external process, mtime 14:57); re-run `cargo clippy -p arb-tx --all-targets -- -D warnings` still **fails**: 10 lint errors (`u8 -> u8` cast, 8× `.clone()` on `Copy` type, `single_match`) |
| `cargo test --workspace` (first attempt) | Aborted — **`no space on device`** during linking; 27.7 GB of `target/debug/incremental` removed to proceed (regenerates) |
| `cargo test --workspace --no-fail-fast` | **73 suites executed; 683 passed, 1 failed, 3 ignored** — `test_audit3.log`. The 1 failure is a debug-mode performance assertion, not a logic defect: `test_yellowstone_synthetic_burst_throughput_and_stale_rejection` got 9,487 updates/s vs required 10,000 (`crates/arb-ingest/tests/yellowstone_stress_and_pipeline.rs:92`) |
| Ignored tests | 3 — `mainnet_alt_integration_tests.rs:111,198` (`#[ignore = "live RPC; run with --ignored"]`) + 1 other `#[ignore]` (0-run suite). Live-RPC ALT checks **never run in CI** |

These results are not evidence of correct execution. A green build proves nothing about on-chain
behavior; the failing perf test and the fmt/clippy failures are reported, not hidden.

## Headline defects (confirmed, severity CRITICAL/HIGH)

1. **V0 ALT compilation is deterministically wrong.** `try_compile_v0` (compiler.rs:967-970) and
   `v0_compiler::compile_v0` (v0_compiler.rs:183-191) pass the *referenced-only subset* as the
   lookup table's `addresses`; `compile_v0_message` uses position-in-subset as the on-wire ALT
   index (lib.rs:3552-3554). Any ALT-loaded account resolves to the **wrong on-chain account**.
   Only the `#[ignore]`d mainnet integration test uses the full table correctly.
2. **The live path never uses ALT, V0, V1, or the versioned compilers at all.** `arb-bot`
   signs an inline **legacy** message (lib.rs:3347/3471) and silently drops any route whose
   transaction exceeds 1232 bytes (dispatch.rs:170). Real 3-leg routes (~45–55 accounts) cannot
   pass this gate: the system is structurally unable to ship the route shapes its engine finds.
3. **Cross-pool DLMM quote engine double-subtracts Token-2022 transfer fees** —
   `apply_transfer_fee_ceil` returns the *net* (arb-math/src/lib.rs:69-77); callers treat it as
   the fee and subtract again (`meteora_quote_engine.rs:118-123, 134-139`). For any pool with
   `transfer_fee_bps > 0`, quoted input/output collapses to the fee itself. Latent in CI (all
   fixtures use `bps = 0`).
4. **Bin-array under-provisioning in the instruction path.** `meteora_bin_array_indices` /
   `derive_directional_bin_arrays` compute required arrays as `bins_traversed/70 + 1`,
   ignoring the in-array start offset (lib.rs:1059-1070; meteora_dlmm.rs:254). Boundary
   counter-examples produce instructions missing required BinArray accounts → guaranteed revert.
   `meteora_bin_array_indices` has **zero unit tests**. The quote and instruction builder never
   use the same array set.
5. **Pump.fun-first flash routes are dead on-chain.** The canonical WSOL ATA is closed *while it
   holds the borrowed flash-loan WSOL*, before the swaps (lib.rs:3165-3167; same defect at
   :2968-2970). SPL CloseAccount requires zero balance ⇒ 100 % revert.
6. **`flash_loan_fee_sf = 0` in `build_kamino_bundle`** (lib.rs:2831-2833) — repay always short
   of the live Kamino fee; all fixtures use sf = 0, so it is untested.
7. **Exit-critical accounting weaknesses:** opportunity-level net profit ignores base/priority
   fees; `make_economic_opportunity` passes `minimum_final == expected_final` (illusory
   conservatism) and DEX/transfer fees = 0; mixed-unit subtraction `gross(lamports) −
   flash_fee_raw(base raw)` (run.rs:676-679); `Lamports(base_raw)` casts fed into lamport-typed
   risk gates/metrics (run.rs:1048/1059/1629, dispatch.rs:110, arb-risk/lib.rs:592); configuring
   tip ceiling ≠ actual bid; `t_max` cap path exists but `check_tip_lamports` (2_000_000 cap) is
   **never called**; `$0.20 net floor` comment vs enforced 50_000 lamports (~$0.009).
8. **Incoherent-snapshot policy is decorative.** `slot_integrity.rs`/`quote_readiness.rs` are
   fail-closed but never called on the live hot path; ingest stamps every pool with
   `max(watermark, fetched_account_slot)` (registry.rs:604-834, helius_ws.rs:588-592) so the
   300-slot age filter (run.rs:488-500) sees age 0 for arbitrarily old data. One fresh pool +
   one stale pool in a multi-hop route is indistinguishable to the live gate.
9. **PumpSwap `sell_v2` discriminator is wrong** in `arb-dex` (stored `global:sell` hash); every
   PumpSwap V2 sell would fail the on-chain check. **MeteoraDammV2 and RaydiumLaunchLab adapters
   are dead code** (missing from protocol dispatch); LaunchLab discriminators are defined but
   the builder inlines pump.fun's. Orca whirlpool `a_to_b` derived from `mint_in == WSOL`
   (breaks non-SOL whirlpools); CLMM swap-disable check reads bit 16 instead of bit 4.
10. **No `MAX_ACCOUNTS_PER_TX` / loaded-accounts-data check** on the live path (Solana locks =
    64); `pos > 255` silently inlines an over-range ALT member (lib.rs:3553-3565) instead of
    erroring.
11. **`.env` ships live arming flags** (`ARB_LIVE=1`, `LIVE=true`, `EXECUTE_LIVE=true`) that no
    code reads; the real arm switch is `config.toml mode = "live"` + fail-open leader gate.
    `.mcp.json` contains a live-looking Helius API key in plaintext.
12. **ALT contents**: tip accounts (all 8) are inside the master ALT manifest (master_alt.rs:537-548);
    ALT authority/activation/deactivation verification exists but is **never executed at runtime** —
    V0 compilers that would fail closed are unreachable from `arb-bot`.

See `16_COMPLETE_BUG_REGISTER.csv` for the full, severity-ranked register (confirmed vs suspected).

## Per-category verdict

| Category | Verdict | Evidence |
|---|---|---|
| Mathematical correctness | **FAIL** | Transfer-fee double-count; mixed units; i64 overflow risks; min==expected conservatism gap |
| Quote correctness | **FAIL** (cross-pool DLMM / Token-2022 path); PASS within 1 raw unit on the single-hop CPMM/CLMM/DLMM SDK-differential fixtures | meteora_quote_engine.rs; sdk differential tests |
| Pool discovery correctness | **FAIL** (fail-open slot stamping; DLMM direction/reserves mismatch for misordered pools; oracle = pool id wiring bug) | audit_work/04 |
| Route correctness | **FAIL** (quote/instruction array sets diverge; direction OK in mint-equality form) | audit_work/06 |
| BinArray correctness | **FAIL** (offset-unaware span formula; no bitmap scan; extension `None` unconditionally) | audit_work/06 |
| ALT correctness | **FAIL** (filtered-subset index bug; never enforced at runtime; tips in ALT) | audit_work/07 |
| Transaction compilation correctness | **FAIL** (live path legacy-only; V0 wrong; V1 unsupported; MTU gate drops) | audit_work/07, 16 |
| Wallet accounting correctness | **FAIL** (unit mixing; canonical-ATA closes; base=WSOL masks it) | audit_work/09 |
| Flash-loan atomicity | **PASS** (borrow/repay same tx, index-bound) — but the assembled instruction sequence is often **non-executable** (pump.fun-first routes) and never simulated | audit_work/09 §6 |
| Jito integration correctness | **PARTIAL/FAIL** (excellent bid/landing machinery; dead override knob, dead tip-cap, fail-open config, 7 s window books late landings as failures) | audit_work/11 |
| Mainnet simulation evidence | **NOT VERIFIED** (no broadcast permitted; `simulateBundle` never called; one LiteSVM/RPC sim path exists but routes that cannot compile never reach it) | dispatch.rs:200-271 |
| Shadow operational health | **NOT VERIFIED** | logs/ and config reviewed; no live soak in audit |

## Final verdict

**A. BLOCKED — CRITICAL CORRECTNESS DEFECTS**

Evidence of critical correctness defects is overwhelming and reproduced locally. A second,
independent blocker exists: **B-level evidence incompleteness** — no on-chain simulation of a
real route has ever passed in this audit (none was permitted or possible). Even after fixing the
critical defects, the system cannot reach "ready" without the missing evidence chain (see
`17_REMEDIATION_PLAN.md` §Gates and `18_FINAL_AUDIT_VERDICT.md`).

**This audit does not, and cannot, issue a LIVE-READY verdict.** The repository as configured
on disk is armed for live firing. Until the verdict upgrades, `mode` must be set to a non-live
value, the shipped fail-open flags must be restored to fail-closed, and the critical defects
must be fixed and re-audited.
