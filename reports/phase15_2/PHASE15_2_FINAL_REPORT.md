# Sentinel Phase 15.2 — Final Report

**Date:** 2026-11-11  
**Phase:** 15.2 (close Phase 15.1 evidence gaps)  
**Verdict:** `BLOCKED — CORRECTNESS ISSUE`  
**Repository:** `D:\work_new`

---

## 1. Summary

Phase 15.2 produced all five required artifacts under `reports/phase15_2/`.
Every artifact was generated from live, observed data — nothing was synthesized,
stale data was never relabeled as success, and the two-slot freshness policy was
never relaxed. `cargo fmt`, `cargo check`, `cargo clippy (-D warnings)`, and
the full workspace test suite (`cargo test --workspace --all-targets`) all pass.

The final verdict is `BLOCKED — CORRECTNESS ISSUE`, driven by the unresolved
shadow economics ROI anomaly (implausible ~965% ROI on a 3-pool Meteora route)
plus declared evidence gaps (five-consecutive-bin-array snapshot unavailable;
Token-2022 transfer fee not exercised inside a DLMM quote). These do not
invalidate the artifacts themselves, but they prevent a `READY FOR FURTHER
SHADOW VALIDATION` verdict.

---

## 2. Required Artifacts

| # | Artifact | Status | Key finding |
|---|----------|--------|-------------|
| 1 | `mainnet_simulation_evidence.json` | ✅ Generated | `STALE_QUOTE_REJECTION` — not labeled as a successful swap |
| 2 | `sdk_differential_manifest.json` | ✅ Generated | 16/16 cases match pinned `@meteora-ag/dlmm@1.9.14`, diff = 0 |
| 3 | `optimizer_search_statistics.json` | ✅ Generated | Deterministic export, `result_kind = "best_observed_candidate"` |
| 4 | `shadow_process_health.json` | ✅ Generated | `PASS` — PID, start time, and safety flags verified against WMI |
| 5 | `PHASE15_2_FINAL_REPORT.md` | ✅ This document | — |

---

## 3. Verification Suite — Exact Results

| Command | Result | Details |
|---------|--------|---------|
| `cargo fmt --all -- --check` | PASS | No formatting diffs |
| `cargo check --workspace --all-targets` | PASS | All workspace crates compile |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS | Zero clippy warnings treated as errors |
| `cargo test --workspace --all-targets` | PASS | 670 passed, 0 failed, 2 ignored; 58 test binaries |
| `arb-sim::meteora_mainnet_simulation_test` | PASS | 1 test; emitted `mainnet_simulation_evidence.json` |
| `arb-engine::meteora_mainnet_sdk_differential` | PASS | 3 tests (incl. manifest emitter); 16 quote cases, diff = 0 |
| `arb-engine::trade_size_optimization_tests` | PASS | 2 tests; emitted `optimizer_search_statistics.json` |
| `arb-bot heartbeat unit tests` | PASS | 2 tests (UTC formatting, counter/status advance) |
| `scripts/phase15_2_shadow_verifier.py` | PASS | All blocking checks pass; start-time skew 0.24s |

Test counts are exact (not estimated). The phase is **not** marked complete merely because unit tests pass; each artifact was independently verified.

---

## 4. Artifact Detail

### 4.1 `mainnet_simulation_evidence.json` (schema v2)

**Outcome:** `STALE_QUOTE_REJECTION` (honestly retained — never relabeled as success).

```
classification: STALE_QUOTE_REJECTION
is_successful_swap_simulation: false
is_stale_quote_rejection: true
simulation_executed_on_chain: true (RPC simulation, non-broadcast)
```

**Slot provenance (corrected — uses actual pool state slot as quote basis):**

| Field | Value |
|-------|-------|
| requested_slot | 468068422 |
| quote_basis_slot | 468068433 (min(pool_a_state.slot, pool_b_state.slot)) |
| pool_a_state_slot | 468068433 |
| pool_b_state_slot | 468068433 |
| simulation_slot | 468068439 |
| gate_lag_slots | 17 (gate handed requested_slot) |
| true_lag_slots | 6 (vs actual state slot) |
| max_allowed_slot_lag | 2 |
| freshness_policy_relaxed | false |

The gate is conservative: `gate_lag (17) >= true_lag (6)`. The rejection is
correct — the gate rejected on the requested slot even though the true state
lag (6) already exceeds the max allowed lag (2).

**Execution:**
- Compute units consumed: 82,702
- Transaction size: 1,008 bytes (≤ 1,232 MTU)
- Program logs: 44
- Inner instructions: 8
- Program logs: 44
- fee_payer_lamport_delta: −9,000

**Realized deltas (per-mint, reconciled to zero):**

| Mint | Owner | Delta (raw) | Decimals |
|------|-------|-------------|----------|
| USDC (`EPjF...`) | payer | −1,242 | 6 |
| USDC (`EPjF...`) | pool_b vault | −98,758 | 6 |
| USDC (`EPjF...`) | pool_a vault | +100,000 | 6 |
| WSOL (`So11...`) | payer | +12,051 | 9 |
| WSOL (`So11...`) | pool_a vault | −1,205,017 | 9 |
| WSOL (`So11...`) | pool_b vault | +1,192,966 | 9 |

- `all_mints_conserved`: **true** (every mint's sum of deltas = 0)
- Token-2022 transfer fee: **not applicable** (all mints are SPL Token)
- Transfer fees accounted: 0 lamports

**Mode flags (never changed):** `ARB_LIVE=0`, `LIVE=false`, `EXECUTE_LIVE=false`, `DRY_RUN=true`, broadcast=false, jito_bundle=false, alt_deployed=false.

---

### 4.2 `sdk_differential_manifest.json`

**Verdict:** `PASS_WITH_DECLARED_GAPS`

| Field | Value |
|-------|-------|
| SDK package | `@meteora-ag/dlmm` |
| SDK version | `1.9.14` |
| SDK bundle SHA-256 | `362b3677...ac929` |
| IDL SHA-256 | `34c60a50...890c7a6e0` |
| IDL instructions | 76 |
| IDL accounts | 12 |
| IDL types | 86 |
| Program ID | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` |
| Pool address | `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6` |
| Snapshot RPC slot | 467,704,169 (consistent with quotes slot) |
| Active ID | −6,179 |
| Bin step | 4 bps |
| Collect fee mode (declared) | 0 (InputOnly) |

**Coverage:**
- Quote cases: 16/16 (2 fee modes × 2 directions × exact-in/out × small+deep)
- Field comparisons: 121 (amount_in, amount_out, total_fee, protocol_fee, lp_fee, end_bin_id, bins_crossed, plus fee_on_input and Q64.64 prices)
- Max absolute diff: **0 raw units** (all diffs zero)
- PDA checks: 5/5 (bin arrays −90, −89, −88; bitmap extension; oracle) — all match
- Bin arrays spanned by any case: **2** (deep cases cross array boundary)

**PDA derivation verified against SDK:**
- Bin array PDAs for indices −90, −89, −88: match
- Bitmap extension PDA: `DArpuuqJxNLRGQ8xq5ebZbobyjxSWWsPq8MqSZ2fUZLE` — match
- Oracle PDA: `59YuGWPunbchD2mbi9U7qvjWQKQReGeepn4ZSr9zz9Li` — match

**Declared unsupported cases (7):**
1. `five_consecutive_bin_array_live_snapshot` — NOT_COVERED_ON_LIVE_SNAPSHOT (snapshot has only 3 existing arrays)
2. `token_2022_transfer_fee_inside_dlmm_quote` — PARTIAL_STANDALONE_ONLY
3. `synthetic_variable_fee_sweep` — NOT_COVERED
4. `bitmap_extension_account_decoding` — PDA_DERIVATION_ONLY
5. `collect_fee_mode_1_live_pool` — SIMULATED_ONLY (mode overridden on mode-0 pool data)
6. `quotes_outside_captured_bin_range` — NOT_COVERED
7. `instruction_encoding_and_flash_loan_cpi` — OUT_OF_SCOPE (quote arithmetic only)

---

### 4.3 `optimizer_search_statistics.json`

**Direction A (profitable):**
- Optimal amount in: 20,000,000 raw units
- Expected gross profit: 2,548,539
- Expected net profit: 2,546,919 (1,273 bps)
- Estimated CU: 95,000
- Required bin arrays: 2
- Evaluations: 42
- Coarse points: 16
- Candidate basins: 1
- Fine search intervals: 1
- `result_kind`: `"best_observed_candidate"`
- `stopping_condition`: coarse logarithmic+linear grid, multi-basin local maxima, bounded fine sweep; best observed candidate selected
- `bounded_early`: false

**Direction B (not profitable):**
- Optimal: 10,000 raw units, net −2,753 (−2,753 bps), `is_profitable: false`

**Global best feasible opportunity:** Direction A, 20,000,000 raw units in, 2,546,919 net profit.

---

### 4.4 `shadow_process_health.json`

**Verdict:** `PASS`

| Check | Result |
|-------|--------|
| heartbeat JSON valid | PASS |
| schema = `sentinel.heartbeat.v1` | PASS |
| PID present in heartbeat | PASS (13,964) |
| Process alive in WMI | PASS |
| Process name = arb-bot.exe | PASS |
| CommandLine contains `--mode shadow` | PASS |
| CommandLine contains arb-bot | PASS |
| heartbeat start time present | PASS |
| WMI CreationDate matches heartbeat (skew 0.24s) | PASS |
| Uptime consistent | PASS (non-blocking) |
| heartbeat_seq ≥ 1 | PASS (seq = 2, advanced from 1) |
| Status not STALLED | PASS (status = OK, loop_started = true) |
| Tick fresh (< 10s) | PASS (2,686 ms) |
| Safety flags correct | PASS |

**Safety flags:** `ARB_LIVE=0`, `LIVE=false`, `EXECUTE_LIVE=false`, `DRY_RUN=true`.

**RPC pool health:** 9 endpoints total, 0 healthy, 1 quota exhausted, 1 in cooldown.

**Method:** The verifier (`scripts/phase15_2_shadow_verifier.py`) cross-checks
`data/heartbeat.json` against Windows WMI `Win32_Process` (PID, CreationDate,
CommandLine, Path). Heartbeat is written atomically (tmp + rename) every second
by an independent task; `heartbeat_seq` advances once per loop iteration, so a
wedged loop is visible as a frozen sequence.

---

## 5. Classification Matrix

| Requirement | Classification | Evidence |
|-------------|---------------|----------|
| Mainnet non-broadcast simulation | PASS | `STALE_QUOTE_REJECTION`, CU 82,702, deltas reconciled |
| Freshness policy not relaxed | PASS | `freshness_policy_relaxed: false`, max lag 2 enforced |
| Stale quote not relabeled as success | PASS | `is_successful_swap_simulation: false` |
| Pinned SDK differential coverage | PASS | 16/16 cases, diff = 0, SDK 1.9.14 pinned |
| Bitmap PDA validation | PASS | 5/5 PDA checks match SDK |
| Five-array snapshot availability | NOT APPLICABLE (unavailable) | Snapshot has 3 arrays (−90, −89, −88); declared gap |
| Optimizer diagnostics | PASS | All diagnostic fields exported, deterministic JSON |
| Shadow process integrity | PASS | PID/start-time/safety verified against WMI |
| Token-2022 inside DLMM quote | NOT APPLICABLE (partial) | Standalone only; declared gap |
| Full test suite (exit codes/counts) | PASS | 670/0/2 across 58 binaries |
| cargo fmt / check / clippy | PASS | All clean |

---

## 6. Open / Unresolved Issues

### 6.1 Shadow economics ROI anomaly (drives BLOCKED — CORRECTNESS ISSUE)

The shadow runner logged an implausible opportunity:

```
size=3415629 real_net_profit=32985083 tip=89960 lamports
Gross Revenue: 0.036530 SOL
Net Profit: +0.033026 SOL (~$2.739)
ROI %: +978.7678%
```

A ~965–979% ROI on a 3-pool Meteora route is not credible. Candidate root causes:
- `graph_max_age_slots = 300` (2-minute horizon) with fail-open starvation fallback
  in `crates/arb-bot/src/supervisor/run.rs` (~lines 456-464) can inject stale pools
- Possible mixed-token unit accounting in route profit display
- `max_state_age_slots = 60` vs `graph_max_age_slots = 300` mismatch

This is a **correctness issue** in the shadow economics/path, not in the Phase 15.2
evidence artifacts themselves. It requires escalation and a fix before shadow
validation can proceed to live readiness.

### 6.2 Five-consecutive-bin-array Mainnet snapshot unavailable

`fixtures/meteora_mainnet_snapshot.json` contains only 3 existing bin arrays
(−90, −89, −88). The deepest exercised quote spans 2 arrays. A 5-array
hysteresis/boundary claim cannot be evidenced. Declared as `NOT_COVERED_ON_LIVE_SNAPSHOT`.

### 6.3 Token-2022 transfer fee not inside a DLMM quote

Token-2022 transfer fee is verified as standalone fee math
(`crates/arb-engine/tests/meteora_sdk_differential_tests.rs`), but the captured
Mainnet pool uses SPL Token, so no DLMM quote applies a Token-2022 fee.
Variable-fee coverage comes from the live snapshot's non-zero accumulator,
not a synthetic matrix. Declared as `PARTIAL_STANDALONE_ONLY`.

---

## 7. Final Verdict

**`BLOCKED — CORRECTNESS ISSUE`**

Rationale: All five Phase 15.2 artifacts are complete, honest, and verified.
The freshness policy was never relaxed; stale quotes are never relabeled as
success; no live execution was enabled. However, the unresolved shadow
economics ROI anomaly (~965% implausible ROI) is a genuine correctness issue
that must be fixed and re-validated before the system can be considered ready
for further shadow validation. Declared evidence gaps (five-array snapshot,
Token-2022 inside quote) are documented but do not themselves drive the
blocking verdict.

### Path to `READY FOR FURTHER SHADOW VALIDATION`
1. Investigate and fix the ROI anomaly (graph max age / fail-open fallback /
   token-unit accounting).
2. Re-run the shadow runner and confirm profitable opportunities are credible.
3. (Optional) Capture a Mainnet pool with ≥5 consecutive bin arrays and regenerate
   SDK quotes for five-array coverage.
4. (Optional) Capture a Token-2022 DLMM pool and obtain official SDK quotes for
   in-quote transfer-fee coverage.
5. Re-run the full verification suite and re-issue this report.
