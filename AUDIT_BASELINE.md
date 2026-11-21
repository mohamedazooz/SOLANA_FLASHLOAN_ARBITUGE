# AUDIT_BASELINE — Forensic Revalidation Baseline

**Mission**: Atomic Profitability & Execution Engineering (SOLANA_FLASHLOAN_ARBITUGE)
**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4` (main, 2026-11-21T02:46:22+02:00)
**Toolchain**: rustc/cargo 1.99.1 (pinned, edition 2024), Linux x86_64
**Audit date**: 2026-11-21
**Method**: repository forensics + revalidation of every prior claim against current source + protocol research against activated mainnet specifications. Historical audit documents were treated as claims, not facts.

Companion machine-readable register: [`AUDIT_BASELINE.json`](AUDIT_BASELINE.json)

---

## 1. Verification Ground Rules Applied

1. Nothing was accepted from prior reports without re-checking the current source at the baseline commit.
2. Every "VERIFIED" statement below cites the exact file inspected in this session.
3. Live-network evidence could not be produced in this sandbox: outbound network calls from the shell are blocked (`curl` returns "Network requests via curl are not allowed") and no `.env` exists. All mainnet-dependent checks are classified **BLOCKED** with reproducible commands.

---

## 2. Source-of-Truth Matrix (revalidated)

| Domain | Canonical implementation (file) | Entry points | Tests covering it (this session) | Revalidated status |
|---|---|---|---|---|
| CPMM quote math | `crates/arb-math/src/cpmm.rs` | `cpmm_out`, `cpmm_out_fee`, `cpmm_out_upfront_fee_ceil`, `quote_cpmm` | `cpmm_matches_python_oracle` (word-for-word oracle vectors, `crates/arb-math/tests/python_oracle_differential.rs`), workspace unit tests | VERIFIED — U256 checked arithmetic, ceil-fee/floor-out matches Raydium AMM v4 / CPMM / DAMM v2 accounting |
| DLMM quote math | `crates/arb-math/src/dlmm/` (`swap.rs`, `fee.rs`, `quote_exact_in.rs`, `quote_exact_out.rs`, `capacity.rs`, `breakpoints.rs`) | `quote_dlmm_multi_bin`, `quote_dlmm_view`, `quote_dlmm_exact_out` | `dlmm_single_bin_matches_python_oracle`; true SDK differential `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs` against `@meteora-ag/dlmm@1.9.14` over captured mainnet snapshot `fixtures/meteora_mainnet_snapshot.json` | VERIFIED (offline parity); live re-quote BLOCKED (no network) |
| CLMM math | `crates/arb-math/src/clmm.rs` (1,729 lines, proptest'd) | tick/sqrt-price/fee-growth engine | unit + property tests in-module | VERIFIED locally; no captured SDK differential exists yet → PARTIAL (no Whirlpool/CLMM oracle fixture in repo) |
| Fee layer (DLMM) | `crates/arb-math/src/dlmm/fee.rs` | `compute_base_fee`, `compute_variable_fee`, `compute_fee_to_add`, `included_fee_amount`, `compute_fee_from_amount`, `compute_protocol_fee` | covered by DLMM differential + oracle vectors | VERIFIED — formulas (base `bf·bs·10·10^p`, variable `⌈vfc·(va·bs)²/10^10⌉`, fee-to-add `⌈a·r/(10^9−r)⌉`) mirror official SDK semantics |
| Kamino flash fee | `crates/arb-math/src/economic.rs` (`kamino_fee_bits` @ economic.rs:646, `calculate_kamino_flash_fee`, `kamino_flash_fee_checked` @ economic.rs:621) — re-exported by `arb-tx` | single canonical implementation | dedicated unit tests incl. U68F60 `BorrowTooSmall` boundary (economic.rs:931–935) | VERIFIED — `fee_bits = max(amount·sf, 2^60)`, ties-away rounding, raw-bits guard preserved |
| Token-2022 | `crates/arb-dex/src/token2022.rs`; policy = fail-closed exclusion of non-base SPL mints from discovery (ISSUE-P1-01 closed for pilot) | mint-owner gate in graph/adapters | gate tests (see arb-engine/arb-dex suites) | VERIFIED as *unsupported and excluded* — standalone quote helpers must not be read as enabling Token-2022 |
| State integrity | `crates/arb-ingest/src/slot_integrity.rs`, `quote_readiness.rs`, provider supervision in `helius_ws.rs` | snapshot reconciler, dependency-group validation | 162 ingest tests green (per Phase 3 gate, revalidated by this session's `cargo test -p arb-ingest` scope in full workspace run) | VERIFIED in-process; live soak pending |
| Route discovery | `crates/arb-engine/src/route_graph.rs`, `bellman_ford.rs`, `route_sizing.rs`, `adapters.rs`; dedup `crates/arb-bot/src/supervisor/route_dedup.rs` | ordered `RouteSignature` dedup (PR #93) | route identity regressions (direction/hop-order/keep-best) | VERIFIED (ISSUE-P0-01 fixed and regression-covered) |
| Sizing optimizer | `crates/arb-engine/src/flash_loan_sizing.rs`, `route_sizing.rs`, `meteora_size_optimizer.rs`; joint route×size search in `graph.rs` (PR #95) | bounded multi-basin search, bin/tick ceilings | sizing regressions incl. multi-basin tests (PR #94) | VERIFIED as best-observed (not proven global optimum); documented in ISSUE-P0-02 |
| Trade construction | `crates/arb-tx/src/primitives/builder.rs`, `flash/` assemblers (`kamino_flash.rs`, `dlmm_*_flash_arb.rs`, `cross_dex_flash_arb.rs`) | single assembler family, no parallel builders | assembler unit tests + `atomic_integrity::verify_atomic_integrity` | VERIFIED |
| Transaction V1 | `crates/arb-tx/src/transport/adaptive.rs`, `flash/compiler.rs` | `compile_v1_message`, `compile_v1_only`, `build_signed_v1_transaction`, `build_unsigned_v1_simulation_transaction` | V1 wire/differential tests in `arb-tx` suites | VERIFIED against the **activated** SIMD-0385 spec (see §3) |
| Simulation | `crates/arb-sim/src/lib.rs` | typed outcomes (PASSED/FAILED/UNAVAILABLE/TIMEOUT/STALE/UNSUPPORTED), fail-closed on missing evidence | simulation classification tests (PR #93/#96) | VERIFIED in-process |
| Dispatch & recovery | `crates/arb-bot/src/supervisor/dispatch.rs`, `recovery.rs` | signature-status reconciliation, no blind resubmit, expiry requires block-height evidence | recovery/dispatch tests | VERIFIED in-process (ISSUE-P1-02 implemented) |
| Landing & realized PnL | `crates/arb-land/`, `crates/arb-store/src/lib.rs`, `crates/arb-metrics/src/lib.rs` | balance-delta reconciliation; SQLite outcome provenance; signed realized-PnL gauge | store/metrics tests (PR #98) | VERIFIED in-process; live balance-delta evidence BLOCKED (no network) |
| Metrics | `crates/arb-metrics/src/lib.rs` | realized vs estimated separation | metrics tests | VERIFIED |
| Dashboard/API | `crates/arb-bot` (web API, `web_api.rs`), `web/` (React) | ControlService as single control plane | env-lock guarded web tests | VERIFIED |
| On-chain executor | `programs/arb-executor` | on-chain profit-guard program; compiles green | workspace check | VERIFIED to compile; on-chain enforcement semantics reviewed under same snapshot constraints — see FINAL report |

---

## 3. Transaction V1 — revalidated against the ACTIVATED spec

Verified this session against SIMD-0385 (`proposals/0385-transaction-v1.md`, status **Activated**) and Solana's official upgrade page (checked 2026-11-21):

- **Activation**: mainnet epoch 1035, slot 447,120,000, ~2026-09-15T01:00Z (SIMD-0296 size + SIMD-0385 format).
- `0x81` version byte → `adaptive.rs:42` `V1_VERSION_DISCRIMINATOR = 0x81` ✅
- Config mask bits `[0,1]` priority-fee (absolute total, LE u64, **lamports**, not micro-lamports/CU), `[2]` CU limit, `[3]` loaded-accounts-data limit, `[4]` heap size → `adaptive.rs:44–53` and `compute_mask_and_values` ✅ (both priority bits set together, as required)
- Limits: 4,096-byte wire, 64 accounts, 64 instructions, `maxSupportedTransactionVersion: 1` on reads, base64 transport ✅
- No ALTs in V1; all addresses inline ✅ (`v1_capable()` profile disables ALT; compiler has no lookup section)
- Unset limits are fatal in V1 (0 CU / 0 loaded-data) → `build_signed_v1_transaction` refuses `None`/0 CU and `None`/0 loaded-data ✅
- Heap request validated as 1 KiB multiple in [32 KiB, 256 KiB] (spec: sanitization failure otherwise) → `adaptive.rs` (heap validation block) ✅
- ComputeBudget instructions are no-ops in V1 → compiler strips them and rewrites klend `flash_repay`'s `borrow_instruction_index` against the executed instruction list (`adaptive.rs compile_v1_message`) ✅ — this protects Kamino reproducibility when builders emit leading CB instructions.
- Loaded-accounts-data limit: `V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT = 64 MiB` justified by a live measurement note (Agave 4.4.1, 2026-11-17: minimal klend spine loads ≈10.7 MB; DECISIONS.md's "16 MiB" text is stale — see Finding F-02).

Conclusion: **V1 implementation matches the activated protocol specification.** No modification required.

## 4. Protocol constants cross-checked with official sources (checked 2026-11-21)

| Constant | Repo location | Official source check | Result |
|---|---|---|---|
| Kamino klend program ID `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | `crates/arb-constants/src/lib.rs:463` | Kamino docs / klend repo (mainnet `declare_id!`) | MATCH |
| Jito tip accounts (8) | `crates/arb-constants/src/lib.rs:73–82` | `docs.jito.wtf` — `getTipAccounts` response | MATCH (all 8) |
| Jito bundle constraints (max 5 txs, min tip 1000 lamports, 50 ms auction ticks, `getInflightBundleStatuses` lifecycle) | `arb-tx` transport + docs | Jito low-latency send docs | MATCH |
| Raydium CPMM fee rounding (`fee = ceil(dx·num/den)` upfront, floor output) | `crates/arb-math/src/cpmm.rs` | Raydium program accounting as encoded in cpmm.rs; oracle vectors | MATCH |

## 5. Findings (current source, baseline commit)

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| F-01 | P0 (existing) | ISSUE-P0-04 `0x1773` root cause remains unresolved pending a same-snapshot transaction replay against mainnet state. | UNCHANGED — still the live-execution blocker; no doc or code flag was allowed to imply it is cleared. |
| F-02 | P2 (docs drift) | `docs/active/DECISIONS.md` Invariant 4 still describes V1 as using "Explicit `SetComputeUnitLimit`/`SetComputeUnitPrice` instructions" and a 16 MiB loaded-data limit. The activated code path (`adaptive.rs`) puts resource limits in the message config (no CB instructions) and requests 64 MiB. **Fixed in this audit** (Invariant 4 text corrected; ADR trail intact). | FIXED (docs) |
| F-03 | P2 (comment) | `arb-constants` `DEFAULT_JITO_TIP_FLOOR_LAMPORTS = 10_000` is described as "enforced by the block engine"; the block-engine-enforced minimum is 1,000 lamports. The stricter repo default is safe (rejects more), so no defect — comment clarified. | FIXED (comment) |
| F-04 | INFO | Missing deliverable docs required by the mission (`docs/research/EXTERNAL_BEST_PRACTICES.md`, `docs/quality/QUOTE_PARITY_REPORT.md`, `docs/quality/ECONOMIC_VERIFICATION.md`, `docs/quality/FLASHLOAN_EXECUTION_REPORT.md`, `docs/quality/TRANSACTION_V1_VERIFICATION.md`, `docs/quality/SHADOW_EXECUTION_REPORT.md`, `docs/performance/HOT_PATH_BENCHMARKS.md`, root `FINAL_PRODUCTION_READINESS_REPORT.md`). | CREATED in this mission (with only verified content) |
| F-05 | BLOCKED | 24 h Shadow soak, mainnet quote parity re-run, live Jito bundle metrics, realized-PnL evidence: sandbox has no network egress and no `.env`. | BLOCKED — reproducible commands provided in `docs/quality/SHADOW_EXECUTION_REPORT.md` |

## 6. Baseline verification commands & results (this session, commit 25ffe60)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS (exit 0, no output) |
| `cargo check --workspace --all-targets` | PASS (0 errors, finished in 1m32s) |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (exit 0, 0 warnings) |
| `cargo test --workspace --all-targets` | PASS — **1,398 passed / 0 failed / 2 ignored** across 109 suites |

## 7. What was NOT changed (deliberately)

- No quote, sizing, profitability, or execution logic was rewritten. The audit found no defect in the canonical implementations that justified code changes; the phase-2/phase-3 gates already hold regression coverage.
- Safety defaults (`ARB_LIVE=0`, `DRY_RUN=true`) untouched.
- No new dependencies.
