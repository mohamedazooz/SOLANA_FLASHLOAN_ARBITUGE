# تدقيق عكسي شامل (Final — merged) — كود ميت · أخطاء منطق · ميزات غير مستغلة

> **الريبو:** `D:\solana-arb-bot` · **HEAD:** `2f00378` · 14 crate + webhook-service.  
> **المصادر:** `docs/reports/FINDINGS.md` (Phase 0) + 3 تدقيقات متوازية (dead-code, logic-errors, wiring) + exploration يدوي.  
> **الدمج:** dead-code (ses_e4ea3c7b6ffe) + logic-errors (ses_e4ea3c7b5ffe) + underutilized (ses_e4ea3a357ffe) — كلها completed.

---

## 0) ملخص تنفيذي

- **كود ميت:** 24 عنصر (D-01…D-24)، أبرزها: module `errors` (dead)، `http_client` field، `recent_landed_cu` (never mutated)، `chronos` math module (865 LOC، 0 prod callers)، litesvm feature disabled، arb-dex decoders مكررة، 7 metric helpers ميتة، 7 config fields parsed-but-unused، 4 telegram/jito methods ميتة.
- **أخطاء منطقية:** 31 (L-01…L-09 + FIN-01…FIN-06 + TX-01/02 + CPMM-01 + DATA-01 + SEC-09 + M-01…M-10). **الأعلى تأثيراً:** M-01 (unit confusion في economic quote)، M-03/M-04 (fee truncation)، M-06 (CLMM tick stall)، M-07 (DLMM phantom prices)، FIN-01 (flash fee hardcoded).
- **ميزات غير مستغلة:** 10 (U-01…U-10). أبرزها: `arb-math::chronos` (dead، بينما `arb-risk::ChronosEvInput` wired)، litesvm (feature disabled)، arb-dex decoders (ingest يكررها)، metric helpers، arb-mcp (standalone).
- **الحالة:** Live execution لا يزال **disabled** (gates A-E). SEC-01 (key leak) + LIVE-01/02 (fabricated exec) لم تُغلق.

---

## 1) كود ميت (Dead Code) — D-01…D-24

### A. Modules / fields
| # | الموقع | العنصر | الدليل | الإجراء |
|---|--------|--------|--------|---------|
| D-01 | `main.rs:13` + `errors.rs` (49 LOC) | `pub mod errors;` entire module | `crate::errors` never referenced | **حذف** module + decl |
| D-02 | `supervisor.rs:80-81` | `http_client: reqwest::Client` | `grep self.http_client` = 0 | **حذف** field + init |
| D-03 | `supervisor.rs:71` | `recent_landed_cu: Vec<u64>` | initialized once, never mutated | fix (mutate) أو حذف |
| D-04 | `route_worker.rs:58,325` | `RouteArchitectWorker.rpc_pool` + `decode_account_data` | `#[allow(dead_code)]` | worker معطل (start_worker_loop no-op) |
| D-05 | `supervisor.rs:72` | `#[allow(dead_code)]` على `simulator` | actually used @2005/2022 | allow stale (بلا ضرر) |
| D-06 | `arb-math/src/chronos.rs` (865 LOC) | module entire | 0 prod callers (`arb_math::chronos`/`LatencyWindow`/etc. = 0) | dead (PLANNED Phase 3) — note: `arb-risk::ChronosEvInput` IS wired |
| D-07 | `arb-sim` (litesvm feature default=[]) | `simulate_local` | returns "litesvm feature disabled" | no-op في الإنتاج |
| D-08 | `arb-tx/src/alt.rs` | ALT helpers | tests only | intentionally disabled (P0 #1) |
| D-09 | `arb-mcp` (binary) | MCP server | not spawned anywhere | standalone surface |
| D-10 | `arb-metrics` helpers | `set_grpc_lag`, `set_state_age`, `inc_reconnects`, `inc_updates`, `set_tip_share_bps`, `inc_pump_opportunities`, `set_pump_spread_bps`, `inc_graduation_events` | defined, never called in prod | مقاييس ميتة |
| D-11 | `arb-ingest` Raydium/Orca decoders | `arb-dex::raydium_v4`, `orca_whirlpool` | production uses `registry.rs` own decoders | تكرار (drift risk) |
| D-12 | `arb-math/src/lib.rs` orphan file `sed9c7SlT` (31 KB) | corrupted copy of chronos.rs | not declared in lib.rs | orphan (HYG-01) |
| D-13 | `verified_config.rs:64` | `is_fresh` | never called in supervisor (uses own TTL) | minor |
| D-14 | `arb-sim/src/lib.rs:24` | `shadow_check` | no production caller | minor |

### B. Unused pub functions / methods
| # | الموقع | العنصر | الدليل |
|---|--------|--------|--------|
| D-15 | `verified_config.rs:119` | `lamports_per_base_raw(decimals)` | tests only |
| D-16 | `verified_config.rs:267,271` | `usdc_mint()`, `usdc_reserve()` | never called in prod/bins |
| D-17 | `telegram.rs:200` | `alert_opportunity` (plain variant) | never called (`alert_opportunity_with_telemetry` used) |
| D-18 | `telegram.rs:424` | `alert_simulation_opportunity` | never called |
| D-19 | `jito_service.rs:190` | `JitoService::plan` (non-context) | tests only |
| D-20 | `jito_service.rs:284,383,422` | `ledger_json`, `poll_until_terminal`, `minimum_tip` | never called |
| D-21 | `jito_service.rs:67` | `ExecutionPlan::is_executable` | tests only |

### C. Config fields parsed but never read
| # | الموقع | Field | Issue |
|---|--------|-------|-------|
| D-22 | `main.rs:~343-347` | `BotConfig.canary` | unused |
| D-23 | `main.rs:~217-225` | `GrpcConfig` (only `primary_url` logged) | ingest uses env/helius URL, not this |
| D-24 | `main.rs:~271-276` | `JupiterConfig` (base_url, api_key_env, timeout_ms) | JupiterClient uses env |
| D-25 | `main.rs:339` | `AdminConfig.token_env` | admin auth not implemented |
| D-26 | `main.rs:317` | `RiskConfig.max_attempts_per_slot` | not passed into `RiskLimits` |
| D-27 | `main.rs:282` | `WalletConfig.min_sol_balance_lamports` | never enforced |
| D-28 | `main.rs:236` | `JitoConfig.tip_floor_percentile` | never read |

### D. bin/ files (dev/ops, not shipped)
Dockerfile builds only `arb-bot` + `alchemy-webhook`. Rest are dev probes:
- `validate.rs`, `discover.rs`, `replay.rs` — read-only (legitimate).
- `live_scanner.rs`, `live_single_exec.rs`, `latency_bench.rs` — contain stale hardcoded pool constants (e.g. `RAYDIUM_SOL_USDC_POOL`) and overlap with supervisor; arguably superseded but still compile.

### E. Cross-crate wiring (confirmed)
- `arb-mcp`: **not** a dep of `arb-bot` (absent from Cargo.toml); standalone MCP server.
- `arb-sim`: **wired** in production path (`Simulator::simulate_local`/`simulate_async` called before dispatch), but `litesvm_sim` is thin wrapper; litesvm feature disabled → `simulate_local` no-op.
- `arb-store`: **wired** — PostgresStore/SqliteStore/RedisStore constructed; `record_attempt`/`record_outcome`/`set_json`/`try_dedupe` called.
- `arb-metrics`: **wired** — MetricsServer started in main.rs; `inc_*`/`add_*` called in supervisor.
- `chronos`: **wired in production** — `RiskGate::with_chronos` + `evaluate_chronos_gate` invoked on hot path (arb-risk side). The `arb-math::chronos` module (865 LOC richer types) is NOT used.

---

## 2) أخطاء منطقية (Logic Errors) — L/M/FIN/TX/CPMM/DATA/SEC

### A. Split-routing path (جديد — commit `7080afb`)
| # | الموقع | المشكلة | الأثر |
|---|--------|---------|-------|
| L-01 | `supervisor.rs:1359-1372` | `effective_price_impact_bps = opp.price_impact_bps / num_legs` — heuristic غير مُتحقق | risk gate يرى impact منخفضاً → يمر opp عالي-impact |
| L-02 | `supervisor.rs:1480-1500` | split path يستخدم `first_hop.mint_in/out/fee_bps` لكل legs (حتى لـ pools برسوم مختلفة) | min_out محسوب برسوم خاطئة لـ legs غير first |
| L-03 | `supervisor.rs:1359+` | economics gate (`real_net_profit`) محسوب على opp أحادي-pool، لا على split | gate يمر بناءً على أحادي-pool، تنفيذ split مختلف |
| L-04 | `route_worker.rs:240` `maybe_split_allocation` | يُعامل جميع parallel pools كـ CPMM (`cpmm_output`) — CLMM/DLMM approximation | output estimate خاطئ لـ CLMM/DLMM |
| L-05 | `supervisor.rs:1050-1070` | stale-pools fallback: if `filtered.len() < 2` → uses all pools (incl. stale) | phantom cycles ممكنة |
| L-06 | `supervisor.rs:1050-1070` | `max_graph_age_slots = max_state_age*4` — 임의 배수، no justification | arbitrary threshold |
| L-07 | `telegram.rs:291-307` | cooldown map grows unbounded within 5-min windows (prune only on call) | memory growth (minor) |

### B. من FINDINGS.md (ACTIVE، لم تُغلق)
| ID | الملف | المشكلة | الأثر |
|----|-------|---------|-------|
| FIN-01 | `arb-constants`, `arb-tx`, `supervisor`, `route_worker`, `live_scanner` | Kamino flash fee hardcoded 5 bps؛ `flashLoanFeeSf` غير مقروء on-chain | under/over-repay → bundle fail أو خسارة |
| FIN-02 | `supervisor.rs:646-720` | ad-hoc economics بدلاً من `EconomicQuote` | تناقض discovery vs execution |
| FIN-03 | `route_worker.rs`, `live_scanner.rs`, `arb-math::pick_tip` | tip = fraction of profit (50%/55%) | too low/high in competitive window |
| FIN-04 | `live_single_exec.rs:187`, fixtures | `base_decimals: 9` hardcoded | USDC (6) mis-priced 10³ |
| FIN-05 | `arb-types/src/lib.rs:652-658` | `fee_bps()` defaults to 25 bps when denominator 0 | mispriced quotes |
| FIN-06 | `arb-engine/src/lib.rs:338`, `bellman_ford.rs:433`, `arb-math` | raw token units stored in `Lamports` | USDC misreport 10³ |
| TX-01 | `supervisor.rs:816-817,830-831` | split legs slippage 50 bps vs config 10 bps | floor below profit floor |
| TX-02 | `supervisor.rs:660-661 vs 686-688` | `net_before_tip` subtracts 10k، gate subtracts 5k | gate 5k lamports optimistic |
| CPMM-01 | `arb-math/src/cpmm.rs:58-59,74` | overflow saturates → 0 / u64::MAX | phantom liquidity |
| DATA-01 | `supervisor.rs:439-480` | DexScreener USD-liquidity-derived reserves + decimals=0 | discovery noise |
| SEC-09 | `arb-tx/src/error.rs:44` | profit floor hardcoded in error text | misleading msg |

### C. أخطاء منطقية إضافية (من subagent2 — core math audit)
| # | الموقع | المشكلة | الأثر |
|---|--------|---------|-------|
| M-01 | `arb-math/src/economic.rs` `build_economic_quote:272-329` | `min_profit_raw` (base-mint raw) treated as lamports → re-converted via `lamports_to_base_raw` → double conversion | profit floor scaled incorrectly for non-WSOL base; `meets_min_profit` wrong |
| M-02 | `arb-math/src/cpmm.rs:528-543` | `clamp_at_marginal_ceiling_two_pool`: guard `clamped > optimal` impossible (clamped ≤ optimal) → `Some(optimal)` branch dead | guard logically broken; should test `candidate_size > optimal` |
| M-03 | `arb-engine/src/lib.rs:382` | fee conversion `as u16` truncation (vs `try_from().unwrap_or(10_000)` in `fee_bps()`) | wrong fee_bps stored on MarketEdge |
| M-04 | `arb-engine/src/bellman_ford.rs:57-62,126-129` | same `as u16` truncation + no `is_fee_valid()` skip → fabricated 25 bps for invalid pools | contradicts "reject not guess" contract |
| M-05 | `arb-math/src/clmm.rs:76-88` | `div_ceil_u64`: saturating_add keeps u64::MAX → `try_from` succeeds → returns u64::MAX instead of overflow | understates ceiling; swap appears to fit |
| M-06 | `arb-math/src/clmm.rs:649` | `find_next_tick_boundary` a_to_b uses `<=` → can re-select current tick → zero-progress / infinite loop | stall in `compute_swap` |
| M-07 | `arb-math/src/dlmm.rs:208` | `pow_q64`: overflow → `unwrap_or(0)` → base silently zero → phantom prices | wrong (possibly huge) price |
| M-08 | `arb-math/src/economic.rs:185-187` | `is_profitable()` uses truncated u64 `net_profit_raw > 0` while `meets_min_profit` uses U256 — representation mismatch | potential mask case |
| M-09 | `arb-engine/src/lib.rs:594,824-828,1070` | `hop_inputs` / `final_out` u128→u64 truncation without checks | inconsistent `min_amount_out` downstream |
| M-10 | `arb-math/src/cpmm.rs:506-524` | `marginal_profit_derivative` (discrete) computes average return `out/size`, not marginal derivative | wrong sign near peak → bad truncation decision |

### D. أخطاء إضافية (من exploration)
| # | الموقع | المشكلة |
|---|--------|---------|
| L-08 | `verified_config.rs:93-114` | `derive_rational_rate` for decimals<9 uses `price * 10^decimals` rounded — precision loss possible (mitigated by None return) |
| L-09 | `cpmm.rs` / `clmm.rs` / `dlmm.rs` | fee direction (in vs out) per-protocol — potential double-fee or no-fee (need verify) |

---

## 3) ميزات غير مستغلة (Underutilized / Misplaced) — U-01…U-10

| # | الميزة | الوضع | أين يجب أن تُستخدم |
|---|--------|-------|-------------------|
| U-01 | `arb-math::chronos` (LatencyWindow, VolatilityEstimator, ExecutionTimeline, chronos_edge_weight) | dead في الإنتاج | supervisor opportunity ranking (RiskAdjustedScore) قبل go/no-go |
| U-02 | `arb-sim` litesvm local sim | feature disabled (default=[]) | enable `litesvm` feature in arb-bot (blocked by external conflict) |
| U-03 | `arb-dex` Raydium V4 / Orca Whirlpool decoders | production uses `registry.rs` own | ingest should delegate to arb-dex (avoid drift) |
| U-04 | `arb-tx::alt` (ALT) | tests only | intentionally disabled (P0 #1) — keep disabled |
| U-05 | `arb-mcp` binary | not deployed | operator/AI-agent surface — deploy separately |
| U-06 | metric helpers (grpc_lag, state_age, reconnects, updates, tip_share_bps, pump_*) | defined, never called | wire into ingest (grpc) + supervisor (state_age, tip_share) + pump discovery |
| U-07 | `verified_config::is_fresh` | never called | supervisor should use it instead of own TTL |
| U-08 | `arb-sim::shadow_check` | no caller | (low priority) |
| U-09 | ARCH-01 (economic engine disconnected) | PARTIALLY FIXED (verified inputs now wired @730-760) | still: test-only `with_economic_config` paths; production sets config per tick ✓ |
| U-10 | `arb-metrics::SIMULATED_NET_PROFIT_TOTAL` | tracked but pump metrics dead | wire pump metrics |

### ميزة مستغلة بشكل صحيح (للتأكد):
- `verified_config` ✓ (wired, allow justified)
- `arb-store` (postgres/sqlite/redis) ✓
- `arb-land` (jito lander + tip oracle + leader gate + ledger) ✓
- `arb-tx` (kamino + jupiter) ✓
- discovery (gmgn/dexscreener/pumpfun) ✓
- `arb-risk::ChronosEvInput` + `evaluate_chronos_gate` ✓ (wired on hot path)
- `arb-sim` (Simulator) ✓ (wired, but litesvm feature disabled → simulate_local no-op)
- `arb-metrics` (MetricsServer + inc_*/add_*) ✓ (wired; helpers unused)

---

## 4) prioritas (HFT senior view)

**CRITICAL (live-blockers):** FIN-01 (flash fee), FIN-04 (decimals), SEC-01 (key leak), LIVE-01/02 (fabricated exec), ARCH-01 (partially fixed).

**HIGH (mispricing):** FIN-02, FIN-03, FIN-06, DATA-01, TX-01, TX-02, M-01 (unit confusion), M-03/M-04 (fee truncation), L-01 (split heuristic), L-02 (fee mismatch in split).

**MEDIUM (correctness):** FIN-05, CPMM-01, SEC-09, M-02, M-05, M-06, M-07, M-09, M-10, L-03, L-04, L-05, L-06.

**DEAD (cleanup):** D-01 (errors module), D-02 (http_client), D-06 (chronos math module), D-07 (litesvm), D-10 (metrics helpers), D-11 (duplicate decoders), D-12 (orphan file), D-15…D-21 (unused methods), D-22…D-28 (unused config fields).

**UNDERUTILIZED (wiring):** U-01 (chronos math), U-03 (arb-dex), U-05 (mcp), U-06 (metrics), U-07 (is_fresh).

---

## 5) توصيات (Ponytail — minimal)

1. **حذف** `errors` module + `pub mod errors;` (D-01) — 2 edits.
2. **حذف** `http_client` field + init (D-02) — 2 edits.
3. **حذف** orphan `sed9c7SlT` (D-12) — confirm first.
4. **إصلاح** M-01 (unit confusion: use min_profit_raw directly, no double conversion) — `economic.rs:272-329`.
5. **إصلاح** M-03/M-04 (fee capping: `try_from().unwrap_or(10_000)` + skip invalid pools) — `lib.rs:382`, `bellman_ford.rs:57-62,126-129`.
6. **إصلاح** M-05/M-06/M-07 (clmm div_ceil checked, tick boundary strict, dlmm pow_q64 propagate error).
7. **إصلاح** M-09 (hop_inputs u64::try_from) — `lib.rs:594,824-828,1070`.
8. **إصلاح** M-10 (rename/replace marginal_profit_derivative) — `cpmm.rs:506-524`.
9. **إصلاح** M-02 (guard condition `candidate_size > optimal`) — `cpmm.rs:528-543`.
10. **إصلاح** L-02 + L-03 (split-path fee per-leg + recompute net for split).
11. **إصلاح** TX-01 (slippage 50→10 bps) + TX-02 (cost basis unified) — centralize cost list.
12. **ربط** `arb-dex` decoders في ingest (U-03) — حذف تكرار، تقليل drift.
13. **ربط** chronos math في ranking (U-01) — فقط إذا أُقرّت Phase 3.
14. **ربط** metric helpers (U-06) — ingest grpc + supervisor state_age/tip_share + pump.
15. **إصلاح** FIN-01 (on-chain flash fee reader) — already exists in verified_config; supervisor uses verified value @741-752 ✓.
16. **إصلاح** FIN-05 (reject zero-denom pools) + CPMM-01 (checked arithmetic).
17. **إصلاح** DATA-01 (mark DexScreener-derived pools `is_active: false` until real decoder).
18. **إصلاح** SEC-01 (rotate Helius key + scrub history) — operator action.
19. **إصلاح** LIVE-01/02 (remove fabricated exec / label heuristic) — operator gating.

> **ملاحظة:** هذا التقرير final (merged). Live execution لا يزال **disabled** (gates A-E من FINDINGS.md). أي تغيير في الكود يتطلب approval per AGENTS.md §2 (Strict Phasing).