# 18 — Final Audit Verdict

## Verdict: **A. BLOCKED — CRITICAL CORRECTNESS DEFECTS**

(Secondarily: **B. BLOCKED — INCOMPLETE EVIDENCE** for any non-live readiness claim. This is the
same blocking state; A dominates because the correctness defects are proven, not merely unproven.)

## Readiness classification (this audit, 2026-11-11)

| Category | Verdict | One-line evidence |
|---|---|---|
| Mathematical correctness | **FAIL** | transfer-fee double subtraction; min==expected; mixed units |
| Quote correctness | **FAIL** (DLMM cross-pool / 2022 path) — **PASS** on single-hop SDK-differential fixtures | 02, 06 |
| Pool discovery correctness | **FAIL** | fail-open slot stamping; oracle wiring; misordered DLMM |
| Route correctness | **FAIL** | quote/instruction array sets diverge |
| BinArray correctness | **FAIL** | offset-unaware span; 2-array cap; no bitmap scan |
| ALT correctness | **FAIL** | filtered-subset index bug; verification never runs; tips in ALT |
| Transaction compilation correctness | **FAIL** | live path legacy-only; V1 unsupported; >1232 B dropped |
| Wallet accounting correctness | **FAIL** | unit conflations; canonical-ATA destruction |
| Flash-loan atomicity | **PASS** (construction) | same-tx index-bound repay; Kamino on-chain checks |
| Jito integration correctness | **FAIL** (config & caps) / **PASS** (bid machinery) | dead override; dead tip cap; fail-open shipped config |
| Mainnet simulation evidence | **NOT VERIFIED** | no route ever simulated-then-verified in this audit |
| Shadow operational health | **NOT VERIFIED** | no shadow soak evaluated |

## Executive explanation (commissioned answers)

1. **How the engine works.** Ingestion (Yellowstone gRPC / Helius WS / HTTP registry pollers)
   snapshots pools into `MarketStateCache`/`AccountSnapshotStore`; a supervisor tick builds a
   `MarketGraph`, finds cycles (CPMM closed forms; DLMM cross-pool integer multi-basin search;
   DFS + Bellman-Ford fallback), runs economic gates, assembles an atomic flash-loan Kamino
   transaction (borrow→swaps→repay→tip), signs it as an inline **legacy** transaction, and either
   sends via SWQoS or as a single-transaction Jito bundle (bundleOnly), then polls landing.
2. **Pool discovery.** Decode account data per protocol into `PoolState` (mints, reserves, config,
   execution accounts); mint-equality wiring; ingest stamps slot freshness (fail-open); HTTP is the
   degraded fallback for the streaming feed.
3. **Direction.** From mint equality against decoded pool mints (X→Y/Y→X for DLMM); Whirlpool
   hard-codes `a_to_b = (mint_in == WSOL)`; CLMM/CPMM from reserve orientation — every path is
   mint-based except the Whirlpool shortcut and the DLMM catalog-vs-decode mismatch.
4. **Swap amounts.** Per-hop `quote_*` in arb-math (exact-in ceiling/floor conventions); CPMM
   closed-form sizing; DLMM bin-walking with Q64.64 pricing and per-bin fee math (SDK-equal);
   sizing by cross-pool coarse→fine search or closed form; leg outputs chained into `hop_inputs`.
5. **Fees.** DEX fees embedded in quotes (ceil); Kamino flash fee `ceil(B·sf/2⁶⁴)` from the live
   reserve (fail-closed); base 5_000 lamports; priority fee from an adaptive
   (p75×1.25, profit-capped) CU price; Jito tip as the final instruction — all base-SOL
   (WSOL-congruent) today.
6. **Profit.** `build_economic_quote` nets gross minus flash-fee/tip/etc. (but opportunity site
   passes 0 for network/priority and min==expected); dispatch re-derives with the actual bid
   (`calculate_net_profit_raw`) and the floor `min_net_profit_abs`; two divergent definitions coexist.
7. **Which mint holds profit.** WSOL in the payer's canonical WSOL ATA (native SOL if last hop is
   Pump.fun).
8. **Which wallet.** The bot keypair only — no fee recipient, referrer, operator, or vault anywhere
   in the flow.
9. **BinArray selection.** Geometric around the active bin — **not bitmap-driven** — plus a
   buggy directional span formula in the instruction path; quote and instruction never share a set.
10. **Master ALT.** Operator-provisioned static manifest (11 static + 3 bot ATAs + 8 tip accounts);
    parsed defensively; warm-up/activation checked; **never referenced at runtime** — live path is
    legacy-only; tip accounts inside the manifest violate the inline-tip rule.
11. **V0.** Implemented (compiles + unit-tested), **incorrect** (filtered-subset ALT indices) and
    unreachable; would fail closed if reached (verify_activation present).
12. **V1.** Wire-format implementation exists and is unit-tested; **UNSUPPORTED** end-to-end:
    no `maxSupportedTransactionVersion:1` anywhere, Jito rejects >1232 B, SIMD-0385 exists but this
    repo has no transport for it; `v1_fallback` has a tip-strip bug.
13. **Jito.** Single-transaction bundles, `bundleOnly`, live REST+WS tip-floor oracle with
    stale-refusal, EV rung pricing on a Wilson ledger, atomic all-or-nothing; profit counted only on
    `Landed` with RPC cross-check.
14. **Why bundles fail.** Oversize legacy txs (never reach Jito); guaranteed-revert content (close
    pre-borrow WSOL ATA; sf=0 repay; SPL-derived ATA for 2022 mints); unknown leaders tipped by the
    shipped fail-open config; 7-second verify window books late landings as failures and suppresses
    future bids; duplication of the tip with an uncapped dynamic CU-price addend; blockhash expiry
    untracked post-hoc; expired bundles corrupted EV posteriors.
15. **Confirmed financial-risk defects.** ARB-001…009 (CRITICAL: ALT indices, 1232 B silent drop,
    transfer-fee double count, pump.fun-first revert, sf=0 repay, bitmap/empty-bin mis-handling,
    enabled/only-Y mis-pricing, fail-open freshness, oracle wiring) — any of these independently
    guarantees wrong quotes, reverting bundles, or unshippable transactions. Seven more HIGH
    defects are cited in §11_JITO/09; **23 CRITICAL+HIGH items are listed with full detail in
    `16_COMPLETE_BUG_REGISTER.csv`.**
16. **Missing proofs.** No end-to-end simulation of any live-path route; no sf≠0 fixture; no 2022-in-quote
    test; no coherent-snapshot proof of the live gate; no on-wire ALT resolution test in CI (only an
    `#[ignore]`d RPC test); no two-basin sizing test; no adversarial suite for the hot path.
17. **Fix first.** §17_REMEDIATION_PLAN: §0 disarm live firing → §1 items 4-11 (V0 fix, bin-array
    span, extension gating, transfer-fee double count, ATA close ordering, sf=0, unit mixing,
    ingest honesty) → §2 decisioning/accounting → §3 evidence gates 17-22.
18. **Benchmark next.** After §0-2: measure (a) sim-pass rate over 1,000 historical routes replayed
    through the fixed path; (b) wire sizes of real 3-leg routes pre/post ALT eviction and tip inline;
    (c) CU consumed vs `estimate_route_cu` calibration; (d) landing-vs-tip ladder calibration using
    re-polled statuses; (e) deadline throughput of the ingest stamping fix.

## Closing statement

The engine contains genuinely impressive components — the DLMM math core is byte-equal to the
official SDK, the flash-loan pair is properly atomic by construction, and the Jito bid/landing
machinery is unusually thoughtful. It does **not** contain a working path from "opportunity found"
to "bundle landed": the signing path emits legacy-only transactions that the system itself drops at
its own size gate, the instruction builder under-accounts required BinArrays, the cross-pool quote
engine prices Token-2022 pools nonsensically, the ingest stamps stale data as fresh, and the shipped
configuration is armed for live firing with the safety floor inverted. **Do not run this against
mainnet value. Fix, prove, then re-audit.**
