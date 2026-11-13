# Session History — SOLANA_FLASHLOAN_ARBITUGE (Complete Work Log)

Chronological, evidence-backed record of every phase of work performed in this session
series, from the first audit to the live V1 transport canary. Every claim carries its
artifact path or command.

---

## Phase 12–15 — Forensic audit campaign (PRs #13, #14 — MERGED)

* Full 15-phase audit per the master directive (repository baseline → final certification).
* Deliverables under `docs/audit/*` (BASELINE, ARCHITECTURE_AUDIT, SOURCE_CODE_AUDIT,
  MATHEMATICAL_VERIFICATION, PROTOCOL_VERIFICATION, DATA_INTEGRITY_AUDIT,
  FLASHLOAN_AND_ROUTING_AUDIT, EXECUTION_AUDIT, SECURITY_AUDIT, TESTING_REPORT,
  PERFORMANCE_REPORT, BEST_PRACTICES_RESEARCH, FINAL_REGRESSION,
  FINAL_PRODUCTION_READINESS_REPORT).
* Independent Python math oracle `audit/math_oracle/oracle.py` + `audit/test_vectors/`:
  Rust↔Python differential on every critical AMM/CLMM/DLMM/fee equation — 0 mismatches on
  the initial 26-vector suite (later expanded).
* CI pipeline established: fmt → check → test → clippy `-D warnings`, **default features
  only** (the `--all-features` trap broke the Live path once and was never used again).

## Phase 16 — Production readiness (PRs #16–#21 — MERGED by user via UI)

* CI failures fixed iteratively: unused variable in `arb-ingest/examples/clmm_probe.rs`,
  fmt diffs, cfg-trap in tests (test-only code compiled under non-test cfg).
* RPC switched to `https://solana-rpc.publicnode.com` / wss (user instruction).
* Funded keypair placed in gitignored `.env` (`SOLANA_PRIVATE_KEY`, 64-byte base58).
  Wallet `2Cby3x…`, ~0.02 SOL. Key never printed, logged, or committed.
* **Zero-opportunities bug root-caused and fixed**: multiple hardcoded zeros and wrong
  denominators in the opportunity/profit path (user's diagnosis: "في مكان في الكود القيم ب 0"
  — confirmed). All economic defaults made dynamic from runtime inputs.
* **Fake-ROI bug fixed**: ROI was computed against the wrong denominator (gross output /
  total fees instead of Net Profit / Borrow Principal). Unified on
  `ROI = Net Profit / Borrow Principal × 100` with expected vs conservative variants.
* Live 4-minute simulate run (mode=simulate, no broadcast): 163 pools, 245 scans,
  p50 scan latency 370 ms, borrow 0.1 USDC → net −1,906 raw units → honest rejection.
  CU 76,549/400k; wire 942/1232 B; slot lag 1; zero broadcast. Logs shown to user.

## Shadow-mode removal (PR #22 — CI green, awaiting user UI merge)

* Shadow mode removed entirely from 12+ files. Mode control is env-only:
  `ARB_MODE ∈ {simulate, live, canary}`. Fail-closed default = simulate.

## MASTER EXECUTION PROMPT — three-agent audit (PR #23, branch `vibe/final-audit-3adf0b`, commit `eaa0111`)

* **BUG-001 (HIGH) fixed** — bitmap-extension rule: `requires_bitmap_extension` was
  `indices.len() > 1` (wrong). Correct protocol rule: any planned bin-array index outside
  the default window **[−512, 511]** requires the extension
  (`arb-constants/src/lib.rs`: `METEORA_BIN_ARRAY_BITMAP_SIZE = 512`,
  `meteora_bin_array_index_requires_bitmap_extension`). Verified against the cloned
  MeteoraAg/dlmm-sdk IDL (`MAX_BIN_PER_ARRAY=70`, `BIN_ARRAY_BITMAP_SIZE=512`, seeds
  `"bin_array"`/`"bitmap"`). Engine passes the extension only when required
  (`arb-engine/src/dlmm_flash_arbitrage.rs:814/830`). 5 boundary regression tests added
  (±511/±512/±513). 11/11 tests pass in `dlmm_bin_discovery.rs`.
* **BUG-002 fixed** — hardcoded `priority_fee_micro_lamports: 1_000` →
  `TipStrategy::adaptive_cu_price` (dynamic).
* **BUG-003 fixed** — hardcoded `jito_tip_lamports: 0` → `verified_tip_lamports`.
* 16 deliverables written under `reports/final_audit/` (inventory, math audit + 26-vector
  CSV 0 mismatches, bin-array audit, Kamino audit, three-pool audit, V1 audit, CU audit,
  freshness audit, HARDCODED_VALUES.csv, BUG_REGISTER.csv, TEST_RESULTS.md,
  PRODUCTION_READINESS.md, FINAL_ENGINEERING_REPORT.md …).
* Verified: fmt clean, **994/994 tests**, clippy clean, oracle self-verify PASS.
* PR #23 opened and CI green.
* Final ZIP (secrets excluded, scan clean): `/workspace/SOLANA_FLASHLOAN_ARBITUGE_final_audit.zip`.
* **Verdict: SIMULATION READY.**

## V1 transport canary (BUG-004) — RESOLVED this session

Tool: `crates/arb-tx/examples/v1_canary_broadcast.rs` — non-financial SPL-Memo V1 canary,
real blockhash, preflight ON, no V0 fallback, JSON evidence per attempt.

| Attempt | Config | Result | Root cause |
|---|---|---|---|
| 1 | CU 10k, data cap 64 KiB | preflight reject: max loaded accounts data size cap | 64 KiB too small |
| 2 | CU 10k, data cap 64 MiB | preflight reject: `Instruction 0: Program failed to complete` | CU 10k < memo BPF load (17,724 CU measured) |
| 3 | **CU 200k**, data cap 64 MiB | **LANDED — finalized** | — |

On-chain evidence (independent `getTransaction`, `maxSupportedTransactionVersion=1`):

* Signature: `SmR6uhE4dr3PoimphULfC2WQpxA7aTQ7ztu9at2mqvm6VAXzMo1GRWn8KGbwnKivr2aUgUrm1gtHzUcsxhbZgMg`
* `version: 1`, `status: Ok`, `fee: 6000` lamports (5,000 base + 1,000 priority)
* Slot **469,007,459**, memo log present, transport `https://solana-rpc.publicnode.com`
* Evidence JSON: `reports/v1_canary/canary_1794590368.json` (+ attempts 1–2 retained)

Canary hardening added in the fix: preflight `simulateTransaction` stage (logs + CU
consumed recorded before send), `ARB_CANARY_PAYLOAD=system` alternative (native 0-lamport
self-transfer, zero BPF load) for transport-only isolation, CU 200k.

Operational consequence: `v1_transport_supported` may be enabled via
`ARB_V1_TRANSPORT_SUPPORTED=1` (env-gated, fail-closed default unchanged).

## Flash-loan × single-DLMM-platform review (this session)

Full review in `reports/final_audit/FLASH_LOAN_SINGLE_PLATFORM_REVIEW.md`. Highlights:

* Borrow/repay metas (12 accounts each) verified against the KLend Anchor IDL
  (`kamino.rs:85-119`); layout pinned by test `layout_is_borrow_swaps_repay`
  (borrow idx 4 / repay idx 7 in the 2-pool path, 5/9 in the 3-leg path).
* Repay amount = principal only; fee debited separately by the handler — the coverage
  invariant (`principal + fee`) is enforced against leg-3 min output separately.
* V1 borrow-index remap verified end-to-end (`adaptive.rs:418-443`,
  `dlmm_3leg_flash_arb.rs:655-665`).
* Settlement-account isolation (borrow destination = pool-3 output = repay source; leg 1
  consumes the principal in full → no inventory subsidy).
* No SyncNative needed (wrapped WSOL throughout).
* Bin-array selection verified against `MeteoraAg/dlmm-sdk`:
  `getBinArrayForSwap` walks from the active array in the swap direction (traversal order);
  our `plan_bin_arrays` matches; `binIdToBinArrayIndex` floor-division matches;
  the SDK passes caller-ordered bin arrays through unmodified in `swap()` — the program
  consumes them in traversal order, which is what we emit.
* `swap_for_y` derived per leg from each pool's **on-chain** X/Y ordering
  (`dlmm_flash_arbitrage.rs:154-162`); reversed-ordering test passes.
* Per-bin depth → borrow: `dlmm_active_bin_max_input` (arb-math/dlmm.rs:303, Q64.64/U256,
  direction-aware, fails closed on empty output reserve) → `get_leg_capacity` →
  `simulate_three_leg_chain` (conservative min-output chaining, bottleneck cap, 92% safety
  factor) → net-profit maximization (`net_profit_lamports`: output − borrow − U68F60 fee −
  network cost). `bins_crossed` per leg feeds `plan_bin_arrays`, closing the loop between
  borrow size and account footprint.

## Outstanding (unchanged)

1. 24-hour shadow/simulate soak (user shortened earlier runs to 3–5 min; the 24h soak
   remains a pre-live recommendation, not a code blocker).
2. Kamino reserve liquidity freshness at fire time (checked at sizing; re-check at
   dispatch recommended).
3. Arbitrage broadcast authorization: **not granted** — only the non-financial V1 canary
   was authorized and executed. Live trading stays disabled by default
   (`ARB_MODE=simulate` until the user explicitly flips it).
4. PR #22 awaiting user UI merge.
