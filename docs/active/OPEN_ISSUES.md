# Forensic Defect Register & Open Issues

This register tracks all identified engineering defects, correctness gaps, and architectural vulnerabilities across the codebase. Issues are strictly prioritized and must be resolved before proceeding through phase exit gates.

---


## Resolved on 2026-11-25 — branch `fix/deep-quote-accounting-hardening-20261125` (merged to `main` via PRs #165 and #166 on 2026-11-26)

### QNT-CORR-2026-11-25-N1: Chronos Recomputed Success Costs and Used Floating-Point EV

- **Status**: Fixed locally.
- **Root cause**: Chronos received gross PnL and independently subtracted flash/network/priority/tip costs after `arb-math` had already produced the canonical fully-costed result. Its survival probability and EV approval also depended on `f64` calculations.
- **Fix**: Pass canonical net-success PnL in Lamports to Chronos; model failure costs separately (network + priority + failed-attempt overhead; no landed-only Jito tip); use a conservative Q32 lower bound for `exp(-lambda * age)` and integer EV/threshold arithmetic. Display-only float projections do not drive eligibility.
- **Tests**: Q32 monotonicity/lower-bound/staleness/overflow; exact success/failure PnL; max-age and lambda bounds.
- **Remaining**: Lambda calibration still requires a representative sample of observed quote-to-land latencies and failed/landed outcomes. The configured value is a guardrail, not a claim of empirically optimal calibration.

### QNT-CORR-2026-11-25-N2: RiskGate Compared Raw Base-Mint Units to Lamport PnL

- **Status**: Fixed locally.
- **Root cause**: The BPS hurdle used `TokenAmount` raw units as its notional while net profit was in Lamports. This only coincided for WSOL (9 decimals), and could misstate the hurdle for a 6-decimal base mint.
- **Fix**: Convert input notional to Lamports with ceiling using the same verified rational rate as canonical PnL conversion; keep PnL conversion rounded down; compute the BPS floor with ceiling division in one shared `arb-risk` helper used by both RiskGate and the read-only validator.
- **Tests**: Cross-mint USDC-to-Lamports regression and fractional-lamport BPS boundary.

### CFG-CORR-2026-11-25-N1: Config Unknown Keys and Environment Safety Overrides

- **Status**: Fixed locally.
- **Fixes**: Apply `deny_unknown_fields` to every typed TOML config struct; implement `load_strict` top-level allowlist; cap `ARB_MAX_IMPLAUSIBLE_ROI_BPS` at the configured ceiling; reject zero wallet reserve and bound Chronos decay lambda; clarify fee/concurrency/failure environment variable names while retaining legacy aliases.
- **Operator note**: Local `.env` is ignored and is not part of the branch; `.env.example` carries safe, non-secret variable names and explanations.

## Resolved during second-pass hardening (2026-11-23)

### TX-CORR-2026-11-23-N1: Jito Tip Account Must Stay Inline, Not ALT-Loaded

- **Status**: Fixed locally on `fix/local-economic-execution-hardening`.
- **Source rule**: Jito low-latency transaction documentation warns not to ALT-load tip accounts; tip account must remain a static/inline account.
- **Root cause**: `StaticMasterAlt::default_static_entries` previously inserted all eight canonical Jito tip accounts; one compiler test encoded that as an expected behavior.
- **Fix**: New ALT defaults omit all tip accounts; both `populate_from_entries` and `build_or_extend_master_alt_ixs` filter official tip addresses even when manually supplied or mislabeled; transaction account partitioning forces the selected tip inline even if a legacy on-chain ALT still contains it. Regression tests cover both fresh and legacy ALT contents.
- **Operator caveat**: Existing on-chain ALTs are append-only; the old entries cannot be removed in place. Provision a replacement clean ALT before relying on it for V0 compilation. The compiler inline override is active regardless.
- **Test**: `arb-tx::flash::compiler::test_partition_accounts_alt_vs_inline`; `arb-tx::transport::master_alt::test_manual_jito_tip_entries_are_filtered_from_both_provisioners`.

### TX-CORR-2026-11-23-N2: V0 ALT Lookup Indices and Read-Only ALT Validation

- **Status**: Fixed locally on `fix/local-economic-execution-hardening`.
- **Root causes**:
  1. V0 compiler built the lookup table from only the per-transaction filtered addresses. Solana lookup indices are positions in the complete on-chain ALT; compacting this list can resolve a different account.
  2. Required-address validation depended on in-memory `AddressCategory` tags, which are not stored in on-chain ALT accounts and are empty after read-only decoding.
  3. V0 compilation called a metadata activation check instead of verifying that the route snapshot slot was strictly later than `last_extended_slot`.
- **Fix**: V0 compilation validates membership, passes the complete on-chain address vector in its original order, checks required Pubkeys directly, and rejects same-slot ALT use via `verify_warmup(ctx.snapshot_slot)`.
- **Tests**: `test_v0_lookup_indices_use_full_onchain_alt_order`; `test_onchain_readonly_alt_verifies_required_pubkeys_without_categories`; `test_v0_compile_rejects_alt_extended_in_same_snapshot_slot`; read-only `from_onchain_readonly` V0 compiler regression.

### EXEC-CORR-2026-11-23-N1: Execution Min-Out Must Include the Gate's Contingency Buffer

- **Status**: Fixed locally on `fix/local-economic-execution-hardening`.
- **Root cause**: The Supervisor's bid ceiling subtracted both configured minimum net profit and the contingency buffer, while the actual native builder received only the graph's minimum-profit setting. The final-hop floor therefore did not explicitly reserve the same buffer used by the economic gate.
- **Fix**: Convert minimum net profit and contingency buffer separately from lamports into base-mint raw units using the verified rational rate and observed price slot; reject stale/overflow/unrepresentable results; checked-add them and pass that exact execution hurdle to both flash and non-flash native builders. WSOL is the active loan anchor, so the denomination is verified.
- **Related fail-closed change**: Signed Legacy/V0 RAM and Redis template fallback has been removed from the active Supervisor. The old RAM path patched a net-profit-only amount into swap min-out; the Redis loader did not resolve swap amount/min-out offsets. Active live compilation now requires the V1 enable flag AND provider/region ingress verification; otherwise it skips the opportunity. A future fast path must use typed unsigned instructions and protocol-specific offsets, not heuristic scans of signed transaction bytes.

---

## 1. Issue Priority Definitions

- **P0 — Critical / Blocker**: Causes false opportunities, inverted routes, incorrect borrow sizing, underfunded repayments, loss-making trades, or unsafe transactions. Blocks live execution.
- **P1 — High**: Latency degradation, incomplete state tracking, potential edge-case reverts, or unverified fee edge cases.
- **P2 — Medium**: Observability gaps, suboptimal data structures, or code cohesion refactoring.

---

## 2. P0 Defects (Phase 2 Correctness Priority)

### ISSUE-P0-01: Route Deduplication Discards Direction and Hop Sequence

- **Status**: Fixed in `fix/atomic-arb-mvp`; ordered route identity and best-quote retention are implemented in `supervisor/route_dedup.rs`. Regression tests cover direction, hop order, same pool set, and profit selection. The live profitability of any route remains unproven.

- **File**: `crates/arb-bot/src/supervisor/strategy.rs` (Lines 388–400)
- **Observed Behavior**:
  ```rust
  let mut seen_pool_sets: HashSet<Vec<Pubkey>> = Default::default();
  for o in &merged {
      let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
      ids.sort_unstable();
      seen_pool_sets.insert(ids);
  }
  ```
- **Root Cause**: Sorting pool public keys alone strips out the directed sequence of hops, intermediate tokens, and swap direction. If an opportunity evaluates $A \rightarrow B \rightarrow A$ and another evaluates $B \rightarrow A \rightarrow B$ over the same pool set, the second route is erroneously discarded as a duplicate.
- **Required Fix**: Implement a comprehensive `RouteSignature` preserving ordered hops, pool addresses, input mints, output mints, swap directions, and protocol IDs.
- **Regression Test**: Test verifying that forward and reverse cycles over identical pool sets are both preserved and evaluated independently.
- **Status (2026-11-20)**: Implemented in PR #93 with ordered route identity and keep-best duplicate selection. PR #93 CI is green; `main` remains blocked until review and merge.

---

### ISSUE-P0-02: Non-Monotonic Profit Assumption in Borrow Sizing

- **Status**: Graph-generated 2–6 hop paths now use bounded, multi-basin route-by-size search, including exact re-quotes and canonical net-profit evaluation; active bin/tick ceilings seed the search. The dedicated DLMM optimizer also refines the highest-profit basins first. Both report a best observed candidate, not a mathematically proven global optimum. Continue differential sizing tests against exhaustive small ranges and live snapshots before production use.

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: Coarse-to-fine integer search assumes that profit curves are quasi-concave or monotonic with respect to borrow size.
- **Root Cause**: In concentrated liquidity (Meteora DLMM bins, Orca Whirlpools), crossing tick boundaries or hitting empty bins introduces non-linear discontinuities and fee cliffs. A coarse sample can miss profitable narrow liquidity bins or overshoot into empty bins.
- **Required Fix**: Implement joint evaluation of $\text{Route} \times \text{Borrow Size}$ with boundary checks at active bin / tick liquidity limits. Always clamp maximum sizing by decoded live Kamino supply-vault balance.
- **Regression Test**: Multi-bin traversal test verifying that optimal size selection finds maximum net profit even when intermediate bins have zero liquidity.
- **Status (2026-11-20)**: DLMM local-peak ranking is in PR #94; bounded joint route-and-size search with per-hop re-quotes and bin/tick input boundaries is in PR #95. It returns the best observed candidate within its sample budget, not a proven global optimum. PR #95's CI exposed an Orca fixture exactly on a zero-capacity tick boundary; PR #97 corrects that fixture and has green CI. Live Kamino vault-cap decoding and production differential soak remain required.

---

### ISSUE-P0-03: Multi-Hop Route State Coherency and Stale Snapshot Skew

- **Status**: Existing runtime validation gathers all route dependency stamps together and enforces `max_slot_skew = 0`, with stale/future/mixed-provider/bank states rejected. The ingestion layer has a regression test for slot skew. This is verified in code; add an end-to-end route test if future dependency aggregation changes.

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: A candidate route with 2 or 3 hops can assemble pools whose snapshots were taken at divergent slots (e.g., Hop 1 at slot $S$, Hop 2 at slot $S-50$).
- **Root Cause**: While each individual pool satisfies `slot <= max_state_age_slots`, the inter-pool slot delta within a single candidate route was not strictly bounded. This creates phantom arbitrage opportunities from price divergence across time.
- **Required Fix**: Enforce route-level slot coherency: $\max_{h \in \text{hops}}(\text{slot}_h) - \min_{h \in \text{hops}}(\text{slot}_h) \le \Delta_{\text{max\_skew}}$ (default $\le 4$ slots).
- **Regression Test**: Reject opportunities where hop snapshots have divergent slot watermarks.
- **Status (2026-11-20)**: Runtime dependency validation passes `max_slot_skew = 0` for all route pools, stricter than the four-slot ceiling, and the ingest dependency-group test rejects divergent watermarks. No additional code change is pending.

---

### ISSUE-P0-04: Implicit Simulation Error Mapping & Slippage Revert 0x1773

- **Status**: Simulation fail-open path fixed; deterministic-replay capture plumbing delivered (see Required Fix 2, 2026-11-21); `0x1773` root cause remains open pending actual replay of a captured failure — no capture existed for the historical event. Slippage limits remain unchanged. See `reports/phase17_cross_dex/PHASE17_CORRECTION_2026-11-20.md`.

- **Files**: `crates/arb-sim/src/lib.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Observed Behavior**: The independent Phase 17 audit records Meteora `ExceededAmountSlippageTolerance` (`0x1773`) during a non-broadcast simulation. That proves the attempted instruction exceeded its on-chain minimum under that simulated state; it does not identify whether quote drift, quote construction, pool state, or instruction data caused the mismatch.
- **Root Cause**:
  1. Simulation outcomes were represented as boolean `success` and string `error`, and the fallback path could accept incomplete responses.
  2. The precise source of the recorded slippage failure is not proven by the available artifacts.
- **Required Fix**:
  1. ✅ Introduce typed simulation statuses and fail closed on missing execution evidence, zero compute units, or stale context.
  2. ✅ (capture plumbing, 2026-11-21) Deterministic-replay forensic capture is implemented and wired: `crates/arb-sim/src/forensic.rs` (`ForensicBundle`: sha256-pinned signed wire bytes + per-hop quote/minimum-output provenance + bounded logs + emitting-program hint + context slot) records every pre-flight simulation failure (`crates/arb-bot/src/supervisor/opportunity.rs`) into `reports/forensics/` (override `FORENSIC_CAPTURE_DIR`), and `replay_locally()` reruns the captured wire through the fail-closed LiteSVM harness (milestone-A, `crates/arb-sim/tests/litesvm_arb_executor_e2e.rs`). NOTE: the HISTORICAL `0x1773` incident itself is still not reproduced — no forensic bundle/saved wire existed when it occurred, so the numeric `Custom(6003)` ambiguity remains open for generic logs; for THIS incident it is RESOLVED — the captured log names the failing program `LBUZKhRx...` and the error name `ExceededAmountSlippageTolerance` (verified against the pinned IDL `fixtures/dlmm.json` and the official error docs, 2026-11-22; see `docs/active/audit/0X1773_ROOT_CAUSE_INVESTIGATION.md`). The ROOT CAUSE of the historical event remains UNPROVEN — no replay inputs exist; every future failure WILL produce a replayable bundle. Live execution remains blocked until a live-consistent same-snapshot replay of a real failure is recorded.
  3. ✅ Keep current slippage limits unchanged until the failed assumption is identified and justified.
- **Regression Test**: Simulation harness verifying proper error categorization and bounded slippage behavior.
- **Status (2026-11-20)**: Typed outcomes and fail-closed dispatch preflight are in PR #93; the Phase 17 diagnostic's fake-bin fallback and simulation-error pass-through are removed in PR #96 (green CI). The opt-in mainnet diagnostic previously substituted active bin `-6542` when the pool read failed and returned success when `simulateTransaction` reported an RPC error or an on-chain revert; it now fails closed. Historical evidence shows the harness could report a revert as a pass while using a hardcoded first-hop minimum; it does not identify the exact on-chain price, fee, or state cause. Same-snapshot quote and replay remain outstanding, so this issue still blocks live execution.

### FL-F-2026-11-22-N1: Split-Route Return Hop Missing Economic Floor (Fixed)

- **Severity**: P2 at time of audit — the path was double-gated dead (`split-routing` feature off by default AND `split = None` hardcoded at `opportunity.rs:756` with a "disabled until proven" notice).
- **File**: `crates/arb-bot/src/supervisor/opportunity.rs` (split-route return hop).
- **Observed Behavior (pre-fix)**: return hop `min_out` was quote-relative slippage only; had split routing ever been re-enabled, price drift within the slippage band could let the Kamino repay succeed out of a pre-existing wallet balance.
- **Correction (this phase)**: `split_return_min_out(total_out, slip, borrow, fee, tip, min_profit) = max(slippage_floor, economic_floor)` is bound to the return hop; integer-only saturating math; gate parity with `TxBuilder::derive_swap_legs`.
- **Regression Tests**: `supervisor::opportunity::split_return_min_out_tests` (4 tests, `--features split-routing`): economic floor binds / slippage binds / zero slippage / u64 saturation.
- **Status**: FIXED in branch `vibe/flashloan-forensic-8ade4e`.

### FL-F-2026-11-22-N2: DLMM Two-Pool Engine Lacks On-Chain Retained-Profit Floor (P3)

- **File**: `crates/arb-engine/src/dlmm_flash_arbitrage.rs` + `crates/arb-tx/src/flash/dlmm_flash_arb.rs`.
- **Observed Behavior**: the dedicated two-pool builder now requires `leg_b.min_amount_out >= repay + minimum_retained_profit_raw`; the evaluator supplies a floor containing the network cost, the priority fee and configured minimum profit. Priority fee is derived from the actual ComputeBudget CU limit and micro-lamports-per-CU using ceiling arithmetic, with overflow failing closed. The route remains disconnected from `arb-bot`; the evaluator has no Jito tip instruction/cost input.
- **Remaining Required Fix**: before runtime wiring, add a verified Jito-tip liability when a tip instruction is assembled, load Kamino/priority/network costs from the same live configuration used by dispatch, and wire the route behind a simulation/dispatch safety gate.
- **Status**: Retained-profit min-out and priority-fee accounting are fixed in the isolated evaluator/builder; runtime integration and Jito-tip accounting remain OPEN (P3).

### DEX-F-2026-11-22-N1: Protocol-Specific State and Quote Engines for DAMM v2 / DBC / LaunchLab (P1)

- **Files**: `crates/arb-engine/src/adapters.rs`; `crates/arb-dex/src/meteora_damm_v2.rs`; `meteora_dbc.rs`; `raydium_launchlab.rs`; `crates/arb-ingest/` discovery and hydration paths.
- **Observed Behavior**: the `UniversalQuoteDispatcher` rejects incomplete snapshots, but the separate legacy route-graph adapters previously allowed DAMM v2 and DBC to enter generic CPMM math and DLMM Pro to enter generic CLMM math. The direct DBC and DLMM Pro `AmmPool` adapters also quoted partial state. DLMM Pro has since been removed from the active `DexProtocol` enum and runtime rather than retained as a candidate integration. A DAMM v2 parser field read from official Pool offset 296..328 was named `config`, although the official Pool field at that offset is `whitelisted_vault`; it must not be passed to the swap builder as a config account.
- **Safety correction (this branch)**: remove family-level fallbacks; register DAMM v2 / DBC as explicit quote-only adapters; make direct DAMM v2 / DBC quote methods refuse incomplete state; retire DLMM Pro from the active enum, math, decoder, hydration, and transaction paths; withhold their execution accounts; rename the DAMM field to `whitelisted_vault`; require LaunchLab's fee-config gate for execution readiness and reject Token-2022 until exact epoch-aware fee/hook context is available. Regression tests cover both the legacy route graph and direct `AmmPool` adapters.
- **Required before ANY enablement**: authoritative protocol-native state decoders and coherent pool/config/vault/tick/bitmap/oracle/mint snapshots; protocol-specific exact-in math and exact-out only where the official model supports it; dynamic/scheduled fees, migration/partial-fill semantics, epoch-aware Token-2022 transfer fees and transfer-hook accounts; sizing boundaries; exact instruction account/discriminator validation; official SDK differential vectors and non-broadcast mainnet simulation.
- **Status**: SAFETY GATE APPLIED; DLMM Pro is retired from the active runtime. DAMM v2, DBC, and LaunchLab remain NOT ENABLED. This issue stays open until the full integration gates pass for those remaining venues. See `docs/active/audit/DEX_SUPPORT_MATRIX.md` and `docs/active/research/DEX_PROTOCOL_ADAPTER_PLAN_2026-11-24.md`.

### DEX-F-2026-11-22-N2: Raydium CLMM Discovery Fallback Fee Is a Structural Constant (P3, telemetry)

- **File**: `crates/arb-ingest/src/decoders.rs:262` (`decode_pool_info` CLMM arm).
- **Observed Behavior**: discovery-time struct carries `fee_bps: 25` placeholder. Operative fee is correct: catalog `feeRate` (`discovery.rs:517`) + millionths→bps conversion with fail-closed exclusion of `fee_on`/`dynamic_fee_info` pools (`decoders.rs:692–701`). Risk only if a CLMM pool reaches the graph without hydration — log/telemetry improvement.

### ARB-F-2026-11-22-N1..N3 (P3, audit notations)

- N1: DLMM two-pool engine unreachable + quote-time-only floor (cross-ref FL-F-2026-11-22-N2).
- N2: BF f64 `-ln` weights are pre-screen only; integer re-derivation mandatory for pricing/sizing/gating (`bellman_ford.rs:404`) — keep the boundary.
- N3: Sizing returns best-observed candidate (budgeted multi-basin search), never claimed as global optimum — accepted limitation.

### TXC-F-2026-11-22-N1: V1 Transport Toggles Pending Live Landing Observation (P3)

- **File**: `crates/arb-tx/src/transport/selection.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`, `dispatch.rs`.
- **Behavior**: V1 submission requires BOTH an enable flag and a provider/region ingress-verification flag: `SOLANA_TX_V1_DIRECT_ENABLED` + `SOLANA_TX_V1_DIRECT_INGRESS_VERIFIED`, or `SOLANA_TX_V1_JITO_ENABLED` + `SOLANA_TX_V1_JITO_INGRESS_VERIFIED`. All default OFF. Generic Jito `/api/v1/sendBundle` documentation is not proof of transaction-format V1 ingress. Do not set the verification flags until a controlled, non-economic V1 canary has passed against that exact provider/endpoint/region and the returned signature is confirmed on-chain. The V1 wire parser now rejects malformed counts, masks, account indices, truncation, and trailing bytes before transport selection.
- **Direct RPC hardening (2026-11-24)**: the V1 direct send path now bypasses Helius Sender and uses only exact endpoint URLs listed in `SOLANA_TX_V1_RPC_ALLOWLIST` (semicolon-separated). The list is intersected with endpoints actually present in the RPC pool; unlisted endpoints are excluded from failover. If the list is empty or no listed endpoint exists in the pool, V1 direct dispatch fails closed. Generic legacy/non-V1 `send_raw_transaction` behavior is unchanged. Canary evidence for one provider does not authorize other providers; add a provider only after that exact URL/region has independently passed ingress + landing verification. Never place the allowlist in tracked config or logs because URLs may contain credentials.

### EXEC-F-2026-11-22-N1: Vanity Placeholder Program ID masquerading as Deployment Identity (P2)

- **File**: `crates/arb-constants/src/lib.rs:658` (`ARB_EXECUTOR_PROGRAM_ID = "ArbExec1111…111"`).
- **Observed Behavior**: the constant is a vanity placeholder, not a deployment identity. Read-only Mainnet `getAccountInfo` returned `null` on 2026-11-23 from SOLANA_RPC_PRIMARY, Helius, Solami, Shyft and Nodit. The former Anchor.toml ID was malformed and has been corrected to match the Rust placeholder; this does not mean the program is deployed.
- **Production-path remediation (2026-11-23)**: the active generic WSOL flash assemblers no longer emit the standalone `profit_guard` instruction. Their final-hop min-out now includes principal + flash fee + base fee + priority fee + tip + retained min profit. The optional arb-executor CPI feature remains inactive and must not be enabled.
- **Required before CPI activation**: record a real deployed program ID + upgrade authority; build/hash the SBF artifact; verify executable account + loader + upgrade authority and compare deployed binary hash against the local build from at least two RPC providers; add a live identity guard. Deployment/upgrade requires explicit operator approval.

### EXEC-F-2026-11-22-N2: SBF Build Unverified This Session (P3)

- `cargo-build-sbf` toolchain absent in the current sandbox; rebuild + hash-pin the workspace blobs before any deployment attempt.

---

### DIAG-F-2026-11-23-N1: Opaque `DLMM quote failed` on Pool 3gUa73kg (9hjBLBrc Pair) (P2, CLOSED AS CLASSIFIED)

- **Files**: `crates/arb-engine/src/dlmm_flash_arbitrage.rs` (gate order), `crates/arb-ingest/src/decoders.rs:745` (`function_type = data[35]`).
- **Observed Behavior**: live rehearsal rejected hop A/B with the detail-less string `DLMM quote failed`; neither the operator nor the log could tell whether the pool was broken, stale, or intentionally unsupported.
- **Root Cause (live evidence, rehearsal run)**: the pool's `StaticParameters.function_type == 2`. CORRECTED AFTER OFFICIAL-SOURCE VERIFICATION: the on-chain stored `function_type` is the `FunctionType` enum (`0 = Undetermined`, `1 = LiquidityMining`, `2 = LimitOrder` -- dlmm-sdk `commons/src/conversions/function_type.rs` `TryFrom<u8>`; TS SDK `ts-client/src/dlmm/constants/index.ts`). The meteora-invent `dlmm_config.jsonc` maps the CREATION-time `ConcreteFunctionType` argument (0 = LO, 1 = LM), a DIFFERENT enum -- the initial "value 2 undocumented" classification was wrong. `function_type = 2` = LimitOrder is a first-class pool type whose exact-in fill layers (MM -> processed -> open orders) were already implemented in `arb-math::dlmm::swap` mirroring the official `get_exact_in_fill_amount_result`. The real defects were: (1) the engine gate rejected first-class pools, and (2) the rejection was labeled with an opaque "DLMM quote failed".
- **Correction (final)**: gate widened to reject only values outside the official `FunctionType` set (`function_type > 2` -> `UnsupportedFunctionType`); the opaque-string failure replaced by the `QuoteGateRejection` taxonomy through `quote_failure_reason()` (single-sourced via `quote_inner()`); the repo decoder's `support_limit_order` match (`2 => true, 1 => false, 0 => reward-mints-default`) is VERIFIED IDENTICAL to the official `is_support_limit_order` (commons `extensions/lb_pair.rs`) -- no decoder change needed. Rails-comment conflict resolved: the two enums were conflated in the old gate doc comment; the decoder comment was correct all along.
- **Correction (superseded intermediate)**: an initial commit on this branch labeled the pool "undocumented and correctly rejected" and only fixed the taxonomy; official-source verification then showed the ON-CHAIN `FunctionType` set includes `2 = LimitOrder` (see Root Cause above), so the gate was subsequently widened to `> 2`. The `quote()` re-factoring into `quote_inner() -> Result<DlmmHopQuote, QuoteGateRejection>` with `quote_failure_reason()` as the single-sourced 13-variant taxonomy stands as first implemented.
- **Regression Tests**: `quote_gate_labels_each_rejection_precisely`, `quote_gate_order_puts_pair_state_before_fee_params`, `quote_and_quote_failure_reason_never_diverge`, `evaluate_surfaces_the_precise_quote_gate_reason` (arb-engine lib).
- **Status**: FIXED (taxonomy + gate widening, source-verified from dlmm-sdk Rust commons and TS SDK). Residual: live LimitOrder-pool fee-split parity (the LO-layer fee shares `split_fee`) affects fee attribution only, not the swap output; differential-quote LO fixtures with NON-zero limit-order layers are queued as a follow-up test phase.

### DIAG-F-2026-11-23-N2: USDC Borrows Always Died As `FLASH_MINT_RESERVE_MISMATCH` (P2, CLOSED AS CONFIG WIRING)

- **Files**: `crates/arb-sim/examples/full_path_rehearsal.rs`, `crates/arb-tx/src/flash/kamino.rs` (`select_kamino_flash_reserve`).
- **Observed Behavior**: every USDC-denominated 2-pool evaluation failed with `FLASH_MINT_RESERVE_MISMATCH` because the session wired exactly one (WSOL) Kamino reserve while `flash_mint_matches_kamino_reserve` correctly fails closed on `flash_is_wsol != reserve_is_wsol`.
- **Correction**: `select_flash_reserve_candidates_live` now sweeps+decodes ALL configured reserves; rehearsal builds a per-mint `HashMap<Pubkey, KaminoReserveCandidate>` for {env flash mint, WSOL, USDC}; every evaluation site (sweep loop, dry-run, executor assembly) applies the per-mint reserve override; mints with no selectable reserve are skipped with an explicit printed reason instead of failing per-quote. Engine semantics unchanged and still fail closed.
- **Status**: FIXED (wiring). USDC routes now either evaluate with a correctly-wired USDC reserve or are skipped with an explicit reason; no net-profit accounting unit mixing is possible because the reserve mint must still equal the flash mint.

---

### HFT-F-2026-11-22-N1: Yellowstone gRPC Compile-Ready but Not Decision-Wired (P3)

- **File**: `crates/arb-ingest/Cargo.toml`, `crates/arb-ingest/src/dual_ingest.rs`, `crates/arb-ingest/src/quote_readiness.rs`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: `yellowstone-grpc-client 17.0.0` is an optional dependency behind feature `yellowstone`; dual-ingest shadow harness (`dual_ingest.rs`) writes to an isolated `StreamingPoolCache` while HTTP polling (`MarketStateCache`) remains the trading source of truth; `QuoteReadinessContext.require_bank_identity` is the designed promotion gate for bank-id-bearing transports. Greedy client library exists; no wire-in or sustained A/B run.
- **Risk if ignored**: none today (conservative direction). Risk only if streaming source were switched without measured A/B: silent data-path regression.
- **Required action**: Run sustained dual-ingest A/B; promote streaming as decision source only with measured evidence; then wire `require_bank_identity=true` end-to-end.
- **Accepted criteria**: recorded A/B report + parity run + no regression in landing latency before any switch.

### HFT-F-2026-11-22-N2: No Ingest-Side Decode Histogram (P3)

- **File**: `crates/arb-ingest/src/helius_ws.rs`, `crates/arb-metrics/src/lib.rs`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: Stage ladder begins histogramming at supervisor stages; ingest→decode leg reaches the ladder only as inherited `t_ingest_to_quote_us` (not a registered histogram in `arb-metrics`).
- **Required action**: Add `INGEST_DECODE_LATENCY_US` histogram recorded at the WS apply path.
- **Accepted criteria**: metric registered + recorded + boundary test.

### HFT-F-2026-11-22-N3: Criterion Bench Coverage Limited (P3)

- **File**: `crates/arb-ingest/benches/`, `crates/arb-math/benches/`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: Only `dual_ingest_bench.rs` and `cpmm_bench.rs` exist; hydration batch fan, route discovery and exact-quote hot loops have no benches.
- **Required action**: Add benches before any optimization; baseline-first policy is binding.

---

### CFN-F-2026-11-22-N1: Legacy Root config.toml Auto-Selection (P3, RESOLVED)

- **File**: `crates/arb-bot/src/main.rs:42–50`, root `config.toml`
- **Severity**: P3
- **Status**: RESOLVED locally.
- **Evidence**: the default resolver now selects only `config/config.toml`; the tracked root file has been removed. The operator copy remains at the ignored canonical path.
- **Risk if ignored**: N/A; the resolver is canonical and fails closed when the operator file is absent.
- **Required action**: none; keep the canonical path and explicit `--config` override.
- **Acceptance criteria**: met by canonical-path resolver regression test and startup path log.

### CFN-F-2026-11-22-N2: No Testing Configuration (P3, RESOLVED)

- **File**: `config/`
- **Severity**: P3
- **Status**: RESOLVED locally.
- **Correction**: added standalone `config/testing.toml` with loopback validator RPC and isolated resources; launch with `ARB_MODE=simulate` and keep `ARB_LIVE` unset. The runtime mode is environment-controlled, not read from the TOML file.

### CFN-F-2026-11-22-N3: No Append-Only Config-Change Audit Log (P3, PARTIALLY RESOLVED)

- **File**: `crates/arb-bot/src/main.rs::append_config_audit`
- **Severity**: P3
- **Status**: PARTIALLY RESOLVED locally; field-level semantic change auditing remains open.
- **Correction**: every successful config load appends one JSONL provenance row to `data/config_load_audit.jsonl` with Unix timestamp, selected path, SHA-256 of the selected file, and runtime mode. It deliberately stores no config values or secret material. Audit write failures emit a warning and do not alter execution mode.
- **Remaining action**: if threshold-level change history is required, add a redacted semantic diff of non-secret fields (or explicitly accept hash-only provenance as the final policy).

### JTO-F-2026-11-22-N4: BAM / Proposer-Builder Separation Transition Unassessed (P3)

- **File**: `crates/arb-land/src/lib.rs` (bundle submission paths)
- **Severity**: P3
- **Status**: Open.
- **Evidence**: online research (2026-11-22) confirms Jito's Block Assembly Marketplace (PBS, TEE-based builders) live on mainnet since 2025-09-25; repo's submission path and landing model have not been re-evaluated against BAM semantics (verifiable ordering may change uncled-bundle/landing attribution).
- **Risk if ignored**: landing posteriors mix pre/post-BAM regimes once slope changes.
- **Required action**: monitor Jito announcements; re-measure per-rung posteriors after any BAM behavior change; keep proven ledger schema versioning for regime breaks.

---

### BCK-F-2026-11-22-N1: In-Memory Session Store (P3)

- **File**: `crates/arb-bot/src/web_api.rs`
- **Severity**: P3
- **Status**: Open (deliberate for single-operator).
- **Evidence**: sessions live in process memory; restart forces re-login; no persistence or revocation list.
- **Required action**: acceptable as-is; revisit only for multi-operator deployment.

### BCK-F-2026-11-22-N2: Client IP Trust For Rate Limiting (P3)

- **File**: `crates/arb-bot/src/web_api.rs:210+`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: per-IP limiter derives identity from proxy conventions; spoofable if ever exposed beyond loopback.
- **Required action**: keep loopback bind; document trusted-proxy requirement; consider X-Forwarded-For strict parsing behind a known proxy.

### BCK-F-2026-11-22-N3: Plain sha256 For Login Password (P3)

- **File**: `crates/arb-bot/src/web_api.rs:52,158–164`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: sha256 with constant-time compare; loopback-only default exposure keeps risk low.
- **Required action**: upgrade to Argon2id before any non-loopback or multi-operator exposure.

### FRT-F-2026-11-22-N1/N2: Frontend UX Minor Gaps (P3)

- **File**: `web/src/components/RiskControl.tsx`, `StrategyConfig.tsx`, `api.ts`
- **Severity**: P3
- **Summary**: minimal error messaging on failed control/config POSTs; token TTL is tab-bound by design.
- **Required action**: cosmetic; combine in a single UI polish pass later.

---

### WEB-F-2026-11-22-N1: CI Coverage Gap For Webhook Service (P2)

- **Component**: `webhook-service/` (standalone crate, own Cargo.lock, outside main workspace).
- **Root cause**: `.github/workflows/rust-ci.yml` ran only cargo-deny against the webhook service; build/test/fmt/clippy never executed by CI despite a 28-test suite existing locally.
- **Impact**: silent regression/rot of an independently deployed edge component.
- **Correction**: dedicated `webhook-service` CI job added in Phase 11 PR (fmt + build + test + clippy -D warnings + cargo-deny). Evidence: `docs/active/audit/WEBHOOK_SERVICE_AUDIT.md` §4-5.

### WEB-F-2026-11-22-N2: Per-Instance In-Memory Rate Limiter (P3)

- **Component**: `webhook-service/src/rate_limit.rs:26` — counters are process-local `DashMap`; effective limit multiplies by replica count behind horizontal scale.
- **Impact**: single-VPS deployment today; becomes real only if replicas > 1.
- **Recommended**: Redis-backed sliding window when scaling out; not hot path.

### WEB-F-2026-11-22-N3: Webhook Service Unwired From Main Bot (P3, by design)

- **Component**: `webhook-service/` vs `crates/arb-bot` — no consumption path exists; classification PRESENT-UNWIRED (discovery-only value, e.g. pump.fun events).
- **Impact**: none while unwired; any future pump.fun ingestion must pass the same `(slot, write_version)` coherence contract as the primary ingest (MARKET_DATA_PIPELINE.md).

## 3. P1 Defects (High Priority)

### ISSUE-P1-01: Token-2022 Transfer Fee Double Counting Audit

- **Files**: `crates/arb-math/src/economic.rs`, `crates/arb-dex/`
- **Current Behavior**: `PoolState` now carries a canonical per-mint Token-2022 fee context (`basis_points`, `maximum_fee`, `epoch`, `snapshot_slot`). Generic hydration refreshes Token-2022 mint bytes and Clock on each pass, verifies their response slots/owners, selects the older/newer schedule using the Clock epoch, and accepts only the TransferFeeConfig extension. The graph/Bellman-Ford projections preserve this context; the dispatcher receives direction-correct input/output fee configs and chains the net output once. Malformed/unknown extensions, stale/missing context, and unsupported paths fail closed. The legacy `quote_hop` and transaction builders still reject Token-2022; TransferHook account resolution and instruction/account parity are not implemented.
- **Status**: Partially addressed: epoch-aware quote-only support is enabled for coherent hydration/quote paths; live execution remains disabled. Do not enable Token-2022 transaction construction from this change alone.

---

### ISSUE-P1-02: Transport Ambiguity and Duplicate Submission Prevention

- **Files**: `crates/arb-bot/src/supervisor/recovery.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Current Behavior**: On network submission timeout, transactions may be re-evaluated.
- **Required Fix**: Strictly enforce invariant: on timeout, reconcile transaction signature against RPC before taking any action. Never re-sign or re-dispatch duplicate transactions. Only mark expired when blockhash progression proves expiration beyond `last_valid_block_height`.
- **Status**: Implemented in `dispatch.rs` and `recovery.rs`: the signed transaction signature and last-valid block height are persisted before send; transport errors poll that signature, unresolved intents stay pending, and recovery never resubmits. Expiration requires block-height evidence. Runtime soak remains a release gate.

---

### ISSUE-P1-03: Realized PnL vs Quoted PnL Metric Separation

- **Files**: `crates/arb-metrics/src/lib.rs`, `crates/arb-store/src/lib.rs`
- **Current Behavior**: SQLite stores outcome provenance and the daily PnL query already excludes estimated landed rows from realized PnL. However, estimated landed rows were omitted from the estimate bucket, and the `net_profit_total` metric also received estimates when balance metadata was unavailable. It was a Prometheus counter, which cannot represent signed realized losses.
- **Status**: Fixed in source: signed realized PnL is a gauge fed only by on-chain balance deltas; estimates for landed transactions have their own gauge and are included in the estimated PnL bucket. Database rows remain in the shared outcomes table with enforced provenance filters; table-level separation is not required for correct aggregation.

---

## 4. P2 Defects (Medium Priority)

### ISSUE-P2-01: Unbounded Ingest Memory Allocation During High-Vol Shocks

- **File**: `crates/arb-ingest/src/helius_ws.rs`
- **Current Behavior**: The referenced `helius_ws.rs` no longer contains an unbounded Tokio channel (repository search found no `unbounded_channel` or `UnboundedReceiver` usage under `crates/`). This register entry is stale and does not describe the current implementation.
- **Status**: Closed as not present in current source. Continue to monitor bounded ingestion queues if a channel-based decoder pipeline is introduced.


### DEX-F-2026-11-25-N2: Raydium CPMM Official-SDK Quote Differential and Fee Attribution (P2)

- **Files**: `crates/arb-ingest/src/decoders.rs`, `crates/arb-ingest/src/hydration.rs`, `crates/arb-dex/src/engines/raydium_cpmm.rs`, `crates/arb-engine/src/adapters.rs`
- **Current behavior**: exact `trade_fee_rate / 1_000_000` is preserved in quote math; accrued protocol/fund/creator counters are removed from raw vault balances; pool, vault and AmmConfig owner/mint checks are enforced; snapshots must share one slot; Raydium CPMM Token-2022 and creator-fee-enabled pools remain excluded from its specialized ingestion path. `MarketEdge` retains the exact fee numerator/denominator alongside rounded BPS metadata. Bellman-Ford screen weights use the native ratio (including direction-aware CPMM creator fee), and its bounded size search evaluates candidate amounts through the exact protocol quote path instead of BPS-derived gamma. Generic fee component fields remain unattributed when live allocation rates are unavailable. Raydium V4/Orca fee decoders now round sub-bps rates up and reject missing denominators; Raydium discovery skips missing/invalid API fee rates; the supervisor no longer emits a zero-fee hop when fee state becomes invalid.
- **Status**: Partially addressed; full support remains gated.
- **Decoder parity hardened (2026-11-25)**: the legacy `arb-dex` `RaydiumCpmmPool` and `DecodedRaydiumCpmmPool` paths now require the full 637-byte PoolState and discriminator, reject default keys/unsupported token programs/invalid creator flags/decimals, and require the full 236-byte AmmConfig with its discriminator and bounded fee allocations. A shared Mainnet-snapshot regression test compares all protocol/fund/creator accrued counters between the legacy decoder and `arb-ingest`, and checks truncated/wrong-discriminator rejection.
- **Mainnet fixture captured (2026-11-25)**: `fixtures/raydium_cpmm_mainnet/sol_usdc_confirmed_2026-11-25.json` stores the verbatim PoolState, AmmConfig, both vaults and both mint accounts from one `getMultipleAccounts` response at context slot `473731445` (confirmed). The regression test verifies the 637/236/165/82-byte layouts, account owners, vault mints, mint decimals, fee rates and accrued-fee reserve sufficiency.
- **Official SDK differential completed (2026-11-25)**: 18 exact-input quote vectors from the confirmed Mainnet snapshot cover both directions and nine input sizes. Another 36 synthetic vectors cover all three `CreatorFeeOn` modes, both directions and six input sizes. All 54 Rust outputs match `@raydium-io/raydium-sdk-v2@0.2.87-alpha` (source calculator commit `44d856ae7cfd2a25fa3b62724a564502978d7589`). The synthetic creator-fee vectors deliberately zero captured accrued counters so the tested reserves are exactly the SDK golden reserves.
- **Remaining required action**: extend differential coverage to exact-output quotes, real Mainnet pools with creator fees enabled, Token-2022 fee-cap/epoch boundary cases on verified mainnet snapshots, and multiple AmmConfig schedules. Add verified fee-component attribution and retain the execution gate for Token-2022/creator-fee-enabled pools until instruction parity and account resolution are complete.

### FEE-F-2026-11-25-N1: Conservative Fee Metadata and Missing-Fee Fallbacks (P2)

- **Files**: `crates/arb-types/src/pool.rs`, `crates/arb-engine/src/lib.rs`, `crates/arb-engine/src/bellman_ford.rs`, `crates/arb-engine/src/route_worker.rs`, `crates/arb-engine/src/meteora_pair_discovery.rs`, `crates/arb-dex/src/raydium_cpmm.rs`, `crates/arb-dex/src/raydium_v4.rs`, `crates/arb-dex/src/engines/pump_swap.rs`, `crates/arb-ingest/src/decoders.rs`, `crates/arb-ingest/src/discovery.rs`, `crates/arb-bot/src/supervisor/run.rs`
- **Current behavior**: graph projections use the shared ceiling-rounded `PoolState::fee_bps()`; Raydium V4/CPMM adapter metadata also rounds up. PumpSwap preserves exact PPM fee components in its dedicated adapter. Raydium V4 and Orca fee decoders round sub-bps values up; invalid denominators fail closed. Raydium API discovery skips missing/invalid fee rates, and supervisor hop construction skips a route if fee metadata is unavailable. The generic PumpSwap quote engine rejects missing fee ratios, applies buy fees to quote input and sell fees to quote output, adds a two-raw-unit upper bound for component-wise ceiling rounding, computes curve price impact, and does not fabricate an 80/20 fee split. Ingestion requires effective quote reserve to equal independently decoded spendable quote reserve before publishing a generic PoolState.
- **Status**: Exact fee ratios are now carried through `MarketEdge` and used by Bellman-Ford's conservative graph pre-screen and exact candidate re-quoting. Remaining gap is attribution of PumpSwap generic fee components and independent mainnet fixtures for PumpSwap and remaining catalog-derived CLMM paths.
- **Required action**: add verified mainnet fee-config fixtures and component-attribution differential coverage; do not enable a venue solely because its graph metadata is now precise.
### CFG-F-2026-11-25-N1: Dual Configuration Schemas and Permissive Legacy Keys (P2)

- **Files**: `crates/arb-bot/src/config.rs`, `crates/arb-config/src/lib.rs`, `crates/arb-config/src/loader.rs`, `config/*.toml`
- **Current behavior**: the runtime accepts the legacy `[bot]/[rpc]` schema and an alternate top-level `[mode]/[provider]` AppConfig schema. Both load one standalone file; no automatic profile merge exists. The legacy parser remains permissive toward unknown TOML keys.
- **Status**: Open follow-up; canonical path selection and secret-safe provenance are fixed in this branch, but schema consolidation is not included.
- **Required action**: migrate runtime to one schema, provide one canonical example/testing template, reject unknown keys at every nested level, and add compatibility/migration tests before removing the legacy parser.

### LEGACY-2026-11-25-N1 — Pump/Raydium legacy engine is not buildable when explicitly enabled (P2)

- **Status:** Not part of the default build or active runtime; do not enable `pump-raydium-legacy-engine`.
- **Evidence:** `cargo test -p arb-engine --features pump-raydium-legacy-engine` currently fails with duplicate public `ArbitrageCycle` export, stale sizing return/error handling (`?` used in non-Result functions), integer-width mismatches, and mutable graph borrow conflicts. Default workspace tests do not compile this gated legacy module.
- **Resolution path:** Port the needed route logic into the active `MarketGraph`/quote dispatcher and delete the legacy feature, or separately repair and test its complete public API before any reactivation. No changes from this pass are claimed as fixing that disabled engine.
### DEX-F-2026-11-25-N3 — Raydium CLMM fee configuration is not live-joined (P1)

- **Status:** Safety gates applied locally; Raydium CLMM remains discoverable for telemetry but is not priceable or executable.
- **Root cause:** Discovery catalog `fee_bps` is not authoritative. Raydium CLMM stores `trade_fee_rate` in its separate program-owned `AmmConfig` (denominator `1_000_000`), and pool state may include dynamic-fee state. The former hydration and WS path promoted catalog fee metadata without joining/subscribing to the config account.
- **Mitigation:** Hydration skips CLMM; WS refresh cannot revive it; `MarketGraph` rejects stale cached CLMM `PoolState`; `UniversalQuoteDispatcher` rejects exact-in and exact-out; route adapters and execution capability refuse CLMM.
- **Re-enable requirements:** Batch-fetch AmmConfig by pool config key; verify owner, Anchor discriminator, config index, tick spacing and fee bounds; require same-slot pool/config/tick-array snapshots; preserve exact fee ratio; model dynamic fees conservatively; subscribe to config changes; add mainnet fixtures and official SDK differential vectors in both directions.
- **References:** `https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/config.rs` and `https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/math.mdx`.
- **2026-11-27 builder/hydration update:** the instruction builder now accepts a variable tick-array list plus an optional bitmap-extension remaining account, derives the extension PDA with the official seed, and rejects empty/default/duplicate account keys. The production hydration path still does not fetch/validate these dependencies or live AmmConfig/dynamic-fee state at one coherent slot; CLMM remains unpriceable and unexecutable. No quote parity, simulation, or live execution is claimed.

### FEE-F-2026-11-25-N2 — Dynamic fee graph bounds and PumpSwap exact PPM (P1)

- **Status:** Fixed locally with regression coverage.
- **Meteora DLMM:** Candidate graph metadata now computes the protocol total fee at configured maximum volatility and rounds up; exact route quoting continues to use live volatility and bin traversal. This is intentionally conservative and may under-rank opportunities.
- **PumpSwap:** Ingestion preserves exact `total_ppm / 1_000_000`; the dedicated pool adapter computes each LP/protocol/creator component directly in PPM and graph metadata rounds up to BPS. The generic quote engine uses direction-correct fee placement and a two-raw-unit conservative margin because `PoolState` does not carry component allocation. Pools whose effective and spendable quote reserves differ are withheld from the generic state path.
- **Other venues:** Orca/Raydium V4 sub-bps graph metadata rounds up; missing/invalid fee ratios fail closed; Orca no longer receives a guessed 5-bps default.
- **Remaining limitation:** `MarketEdge` still stores integer BPS and generic PumpSwap results still cannot attribute the total fee to individual components. Carry exact ratios and verified allocation metadata through graph edges/results before removing conservative bounds.
### DATA-COVERAGE-2026-11-25-N1: Live Pool Decode Coverage and Same-Venue Quote Differential

- **Status**: Partially addressed; the latest live snapshot hydrated all 9 registered pools, but sustained coverage and exact quote parity remain release gates.
- **Historical evidence**: Read-only validation at slot `473859686` decoded 5 of 9 pools. The new diagnostics then isolated the four rejects as Orca Whirlpool pools whose tick-array accounts did not share the pool context slot.
- **Diagnostics added (2026-11-27)**: `MultiDexRegistry::last_hydration_skips()` exposes one record for every registered pool omitted by the latest hydration pass. The read-only `validate` binary prints `POOL_HYDRATION_REJECTED` with protocol, full pool ID and a stable reason code for instrumented gates. Remaining early exits are reported as `unclassified_validation_gate`; this fallback is visible but is not a precise root cause. The diagnostics add no RPC calls to the normal quote path beyond the existing hydration pass and do not relax any quote gate.
- **Orca coherent snapshot fix (2026-11-27)**: Removed the separate stale tick-array fetch. The hydrator now batches each Orca pool with its three derived tick arrays in the same account request context, validates owner/slot/layout/start index, and re-derives the expected array window from the refreshed pool state. A moving `tick_current` is accepted only if the required PDA window is unchanged; a changed window or incoherent dependency set remains fail-closed.
- **Latest live evidence**: Read-only validation at slot `474365075` priced 9/9 registered pools (4 Orca Whirlpool, 3 Meteora DLMM, 1 Raydium CPMM, 1 Raydium V4). It built 18 graph edges; six gross-positive two-hop candidates were rejected by the verified economic gate and zero were execution-eligible. No transaction was signed or submitted. This is one successful snapshot, not a sustained-soak result.
- **Stage 5 live validation (2026-11-27, slot `474379079`)**: After the tip-parity correction, read-only validation again priced 9/9 pools and built 18 edges. The configured probe cap was `0.020000 SOL`, minimum net hurdle `10,000` lamports / `1` bps, and hop slippage `10` bps. The refreshed Jito p75 was `5,567` lamports (below the configured cap); the canonical cycle scan found 0 gross-positive 2-hop routes, 0 gross-positive 3-hop routes, 0 economic-gate rejections, and 0 execution-eligible routes. The supplemental reserve-based ranking from this run is not treated as valid evidence: it applied the CPMM formula to CLMM/DLMM pools and was corrected in the later Stage 5 cross-check. Jupiter same-pool differential was skipped because its venue reference was not the exact same single Whirlpool pool. No transaction was signed or submitted. This is one snapshot, not a sustained-soak result.
- **Official SDK differential (verified 2026-11-27)**: `cargo test -p arb-engine --test whirlpool_mainnet_sdk_differential -- --nocapture` passes against four frozen Mainnet pool snapshots captured at slots `472209044`, `472209365`, `472209368`, and `472209372`. All 16 exact-in cases (4 pools × 2 directions × small/deep sizing) match the official `@orca-so/whirlpools-core` `3.1.1` fixture oracle exactly for input consumed, output received, and trade fee. The test also re-derives every tick-array PDA, checks the raw pool layout and classic-fee gate, parses all captured tick-array bytes with the engine parser, and verifies tick-start probes. Fixture oracle version is now pinned in the test. This proves parity for those immutable classic-pool snapshots; it does not prove adaptive-fee, Token-2022 transfer-hook, exact-out execution, or live-state parity.
- **Route-rejection diagnostics (2026-11-27, Stage 3)**: `BaseCycleScanStats` now retains up to 32 strictly gross-positive routes rejected by the verified economic gate, including route input, every hop boundary amount, per-hop DEX/Token-2022 fee components only when the canonical dispatcher exactly reproduces the optimizer's hop outputs, and a dispatch error/mismatch explanation otherwise. The read-only `validate` binary prints `REJECTED_CYCLE` and `REJECTED_HOP` records. Diagnostic collection does not feed back into sizing, quote selection, or execution eligibility; it is bounded and does not add network calls to the hot path. A mismatch deliberately leaves fee fields unset rather than attributing fees to a different quote.
- **Validation tip parity correction (2026-11-27, Stage 5)**: The read-only `validate` graph previously received `Lamports(0)` for its Jito tip while the later explicit SOL/USDC edge report fetched and deducted a live p75 tip. This understated the canonical cycle-gate liability and made `REJECTED_CYCLE` economics inconsistent with the same run's edge report. Validation now fetches one tip estimate before graph construction, caps p75 at `config.jito.max_tip_lamports`, uses the configured maximum on oracle failure (rather than the protocol minimum), and passes/reuses that exact estimate for both reports. Dispatch still reprices independently. Added a regression assertion that the verified tip survives into `EconomicConfig`. This fixes diagnostic consistency; it does not yet classify the six historical rejects or establish new live results.
- **Route diagnostic amount-ladder correction (2026-11-27, Stage 5)**: `quote_route_input_ladder` already returns `[route_input, hop_1_output, ..., final_output]`. The Stage 3 rejected-route sampler prepended `route_input` a second time, so its canonical-dispatch comparison falsely reported a first-hop output mismatch and withheld otherwise attributable fee fields. At slot `474379935`, route `40` had matching dispatcher input, final output, and hop count; the reported first-hop mismatch was `expected=2,812,500` (the duplicated input) versus `actual=202,817`. The sampler now stores the existing ladder unchanged. A regression test verifies the ladder contains the input once and that a two-hop dispatcher reproduces each output for a CPMM fixture. This explains that diagnostic mismatch; it is not evidence of an adapter pricing mismatch. A same-slot Whirlpool replay is still needed to confirm fee attribution on that exact historical candidate.
- **Reserve-based cross-check correction (2026-11-27, Stage 5)**: The supplemental `cpmm_out` ranking previously applied constant-product math to every SOL/USDC pool, including Orca Whirlpool and Meteora DLMM. It now includes only Raydium V4 and Raydium CPMM (`PoolKind::Cpmm`), explicitly labels itself a conservative BPS cross-check, and excludes CLMM/DLMM curves. The native-adapter `MarketGraph` scan remains the authoritative multi-venue result.
- **Stage 5 live validation (2026-11-27, slot `474380722`)**: The corrected read-only validator priced 9/9 pools, built 18 graph edges, and observed 0 gross-positive 2-hop routes, 0 gross-positive 3-hop routes, 0 economic-gate rejections, and 0 execution-eligible routes. Jito p75 was `10,000` lamports; the verified Kamino WSOL fee scale remained `11,529,215,046,068`; configured cap `0.020000 SOL`, minimum net hurdle `10,000` lamports / `1` bps, slippage `10` bps. The CPMM-only secondary check included 2/9 pools and both directions were negative: net `-20,652` and `-23,257` lamports at `0.001 SOL` probe size, with costs `15,010` lamports (flash `10` + tip `10,000` + base `5,000` + priority `0`). Jupiter same-pool differential was skipped because its venue reference was not the exact same single Whirlpool pool. No transaction was signed or submitted. This is one snapshot, not a sustained-soak result.
- **Candidate economics (slot `474379935`, route `40`)**: The captured gross-positive edge was only `+5` lamports on a `2,812,500`-lamport borrow. With two 10-bps per-hop haircuts (each share rounded up), the conservative final output is `2,806,882` raw, a `5,623`-lamport haircut; the verified flash fee is `28` lamports, base signature fee `5,000`, Jito p75 `7,826`, and priority fee `0`. Total costs are `12,854` lamports; expected net is `-12,849`, conservative minimum net is `-18,472`, versus a required `+10,000` minimum-net hurdle. This candidate is therefore rejected for economic reasons, not because of a raw amount-conversion error. DEX fees are already embedded in the exact hop outputs; no separate Token-2022 fee was present. The diagnostic mismatch on this candidate was the duplicated input described above, and exact same-slot Whirlpool fee attribution remains unverified.
- **Next action**: Capture repeated read-only snapshots across distinct slots; when gross-positive candidates recur, verify the now-correct amount ladder and exact dispatcher parity, then classify flash fee, base/priority fee, tip, conservative slippage haircut, min-profit hurdle, and any checked-conversion failure. Extend frozen-snapshot Whirlpool differential coverage to additional pool states/fee tiers, then run a sustained hydration-coverage soak. Keep adaptive-fee and Token-2022 pools fail-closed until their state/context is modeled and differentially verified. Do not widen slippage or relax the economic gate.
- **Safety**: Keep any pool fail-closed when its pool/tick-array snapshot is incomplete, owner-invalid, or incoherent.
