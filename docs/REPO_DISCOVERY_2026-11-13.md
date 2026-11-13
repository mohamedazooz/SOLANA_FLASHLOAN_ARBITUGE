# Repository Discovery & Re-Audit — 2026-11-13 (Directive 2)

Baseline: `main` @ `408c7b6` (Merge PR #6). Branch: `feat/unified-pipeline-webapp`.
Method: direct source inspection of every crate, call-graph tracing from the supervisor
run loop, env-key census, lock census, and live mainnet probes (Helius RPC + WSS + V1
simulation). Previous audit findings were re-validated against current code, not trusted.

## 1. Workspace dependency map

14 crates (~71.6k LOC src):

| crate | LOC (src) | role |
|---|---|---|
| arb-tx | 17,061 | tx builders (Kamino/DLMM/Orca/Raydium/Pump), V1 compiler, templates |
| arb-ingest | 12,961 | providers: Solami WS, Helius WS/LaserStream, Bitquery, GMGN, rpc_pool, registry, discovery |
| arb-bot | 9,720 | supervisor (run/dispatch/execution), config, telegram, jito_service, health, bins |
| arb-engine | 8,600 | quote engines, route graph, 3-leg pipeline, sizing, guarded quotes |
| arb-math | 7,471 | DLMM/CLMM math, fee models |
| arb-dex | 4,913 | protocol adapters/decoders |
| arb-types | 2,663 | PoolState, mints, shared types |
| arb-land | 2,545 | landing tracker, tip oracle |
| arb-sim | 2,586 | simulation harness |
| arb-store | 1,046 | SQLite/Postgres/Redis stores (used by supervisor) |
| arb-risk | 918 | RiskGate, KillSwitch, CircuitBreaker, Chronos EV |
| arb-mcp | 345 | **DEAD** — MCP server, zero references from any other crate |
| arb-metrics | 308 | Prometheus metrics server |
| arb-constants | 465 | program IDs, key PDAs |

Crate dependency flow: `arb-types/arb-constants` → `arb-math` → `arb-dex` → `arb-engine` →
`arb-tx` → `arb-ingest` → `arb-bot` (entry). `arb-risk`, `arb-store`, `arb-land`,
`arb-metrics`, `arb-sim` are leaves consumed by `arb-bot`.

## 2. Actual execution call graph (live 3-pool path)

```
main.rs → Supervisor::run (supervisor/run.rs:209)
  ├─ ingest stream (Solami WS / Helius WS) → account_store (fresh pool states)
  ├─ kamino_available_flash_liquidity (run.rs:19)  ← supply-vault decode from tracked store
  ├─ capture_three_leg_dlmm (run.rs:79, called at run.rs:814)
  │    └─ arb_engine::build_three_leg_meme_pipeline (three_leg_meme_pipeline.rs)
  │         ├─ candidate chain enumeration over fresh PoolStates
  │         ├─ size ladder filtered by kamino_available_liquidity (line 431:
  │         │    candidates above actual flash liquidity are DROPPED, error
  │         │    InsufficientFlashLiquidity if all exceed it)
  │         ├─ net_profit_of (integer, includes flash fee + network cost)
  │         └─ ExecutionOpportunity
  ├─ RiskGate.evaluate_flash_loan / evaluate_full (run.rs:1338/1354; kill switch run.rs:528)
  ├─ assemble_native_ixs (supervisor/execution.rs; pre-existing ATA RPC probe →
  │    skip_close_intermediate_mints)
  ├─ TxBuilder::assemble_flash_arbitrage (arb-tx/src/lib.rs) → dlmm_3leg_flash_arb
  ├─ compile: Tier-1/2 templates (patch_and_sign_with_swaps) or Tier-3 compile_v1_only
  │    (run.rs:1677) → adaptive::compile_v1_message (V1 wire, CU strip + klend
  │    borrow_instruction_index rewrite — proven live 2026-11-13)
  ├─ dispatch_opportunity (supervisor/dispatch.rs:61, mandatory preflight sim)
  └─ arb-land landing tracker / reconciliation
```

**The specialized 3-pool builder IS connected to the Supervisor** (run.rs:814 →
assemble_flash_arbitrage → dlmm_3leg_flash_arb). No bypass path found in the live loop.

## 3. Protocol integration matrix

| protocol | builder | decoder/adapter | quote math | live path status |
|---|---|---|---|---|
| Meteora DLMM | `arb-tx/dlmm_flash_arb.rs`, `dlmm_3leg_flash_arb.rs`, `dlmm_4leg_flash_arb.rs` | `arb-dex/meteora_dlmm.rs` | `arb-math/dlmm.rs` (1,515 L), `arb-engine/meteora_quote_engine.rs` | **ACTIVE** (3-leg) |
| Kamino klend | `arb-tx/kamino.rs` (borrow/repay, U68F60 fee) | reserve decode in run.rs/kamino.rs | `flash_loan_sizing.rs` fee | **ACTIVE** (funding) |
| Raydium AMM/CPMM/CLMM | `arb-tx` raydium builders | `arb-dex` | `arb-math/clmm.rs` | built + tested, not in the live 3-leg loop |
| Orca Whirlpool | `arb-tx` whirlpool builder | `arb-dex` | `arb-math` | built + tested, not in the live loop |
| Pump.fun / PumpSwap | `arb-tx` builders | `arb-dex` | `arb-engine` meme pipeline | built + tested, not in the live loop |
| Jupiter | `arb-tx/jupiter.rs` (`pub mod jupiter`, lib.rs:102) + `assemble_jupiter_flash_arbitrage` (lib.rs:3315) | — | — | **ORPHANED**: PR #4 claimed Jupiter removal but the module and assembler remain (used only by `bin/validate.rs` + `bin/live_scanner.rs` diagnostics) |

## 4. Duplicate functionality report

1. **Jupiter residue** — `arb-tx/src/jupiter.rs` + `assemble_jupiter_flash_arbitrage`
   (lib.rs:3315) survived the PR #4 removal; only diagnostic bins reference it.
   Classification D (duplicate/obsolete path). Action: remove module + assembler +
   diagnostic-only references, drop `JUPITER_API_KEY`/`JUP_API_KEY` env keys.
2. **Multiple quote entry points** in arb-engine (`meteora_quote_engine`,
   `cross_pool_dlmm`, `dlmm_flash_arbitrage`, `three_leg_meme_pipeline`, `guarded`).
   The live path uses `three_leg_meme_pipeline`; the others are reachable only from
   tests/other pipelines. Classification: consolidate behind one quote surface —
   keep `meteora_quote_engine` as the per-pool quoting primitive (used by the live
   pipeline), keep `guarded` pre-dispatch revalidation, mark the rest as
   pipeline-internal (no second discovery pipeline is wired in run.rs).
3. **Kamino fee implementations** — `flash_loan_sizing::calculate_kamino_flash_fee`
   (U68F60) vs arb-tx kamino fee math. Same formula verified against live reserve
   (fee_sf matches constants). Keep one canonical `kamino_fee` (arb-tx), have
   arb-engine re-use it. Classification D.
4. **Two pool-state → route conversions** (`adapters.rs` vs registry) — verified same
   source type; not a behavioral duplicate. Classification C (keep, single normalize
   point documented).

## 5. Dead / unused code report

- **`crates/arb-mcp`** — entire crate unreferenced by any other workspace member.
  Candidate for removal (or explicit exclusion from default members). Classification E.
- **`arb-tx/src/jupiter.rs`** — see §4.1.
- **Env keys with no live consumer**: `JUPITER_API_KEY`, `JUP_API_KEY` (only the
  orphaned jupiter module + validate bin), `CHRONOS_*` (used by RiskGate Chronos EV —
  active, keep), `BITQUERY_*`/`GMGN` (arb-ingest modules compiled but not subscribed in
  the live loop; keep behind default-off config).
- **`dashboard/index.html`** (442 lines, static) — superseded by the React app in this
  directive; retained only until the React build output replaces it.

## 6. Configuration dependency map (env census, 60 keys)

Operational groups: RPC/WSS (`SOLANA_RPC_URL`, `HELIUS_RPC_URL`, `HELIUS_SENDER_*`,
`SOLAMI_*`), V1 transport (`SOLANA_TX_V1_DIRECT_ENABLED`, `SOLANA_TX_V1_JITO_ENABLED`,
default false), live gating (`ARB_LIVE`, `LIVE`, `EXECUTE_LIVE`, `DRY_RUN`,
`BYPASS_RISK_GATE` non-live only), risk (`KILL_SWITCH_*`, `MAX_TRADE_SIZE_USD`,
`MIN_SOL_RESERVE_LAMPORTS`), stores (`DATABASE_URL`, `REDIS_URL`, `DATA_DIR`),
telegram (`TELEGRAM_*`), dashboard (`DASHBOARD_LISTEN`), ingest (`WS_ENCODING`,
`WS_COMMITMENT`, `RPC_MIN_INTERVAL_MS`, `RPC_MAX_CONCURRENT_REQUESTS`).
**Dead keys**: `JUPITER_API_KEY`, `JUP_API_KEY` (post-removal of the Jupiter path).
**Missing keys to add**: `HELIUS_WSS_URL`, `HELIUS_BETA_URL`, `SENTINEL_API_KEY`
(provider auth), `MIN_NET_PROFIT_USD`, web-auth keys (`WEB_ADMIN_PASSWORD_HASH`,
`WEB_SESSION_TTL_SECS`, `WEB_BIND`).

## 7. Hot-path lock map (supervisor)

| lock | field | hot path? | assessment |
|---|---|---|---|
| `parking_lot::Mutex<MarketGraph>` | supervisor.rs:66 | yes (every loop) | short critical sections, no I/O under lock — acceptable; document |
| `RwLock<MultiDexRegistry>` | supervisor.rs:70 | yes (reads) | read-mostly; candidate `ArcSwap` later; not a measured bottleneck |
| `RwLock<f64>` | supervisor.rs:78 | yes | trivial; fine |
| `Mutex<SlotClock>` | supervisor.rs:93 | yes | small; fine |
| `Mutex<Option<([u8;32], Instant)>>` | supervisor.rs:100 | per-dispatch | fine |
| `Mutex<HashMap<String,u32>>` measured_cu | supervisor.rs:105 | per-landing | fine |
| `Mutex<HashMap<String,PreWarmedTxTemplate>>` | supervisor.rs:108 | per-dispatch | fine |
| `Mutex<Vec<TradeSummary>>` ×3 | supervisor.rs:112-114 | telemetry | bounded vectors; fine |
| arb-ingest locks (registry.rs, rpc_pool.rs, helius_ws.rs, dual_ingest.rs) | — | ingest path | no network/disk I/O under lock observed; rpc_pool bounds concurrency |

No lock is held across network or disk I/O in the execution path; none of the audited
locks showed a replacement that would reduce latency without adding risk. The
`ArcSwap` dependency already exists in the workspace for future read-mostly swaps.

## 8. Test coverage map

924 tests / 0 failed at main @ `408c7b6` (`cargo test --workspace --all-targets`),
clippy `-D warnings` clean, fmt clean. Coverage concentrations:
- V1 wire format: `crates/arb-tx/tests/v1_property_fuzz.rs` (21 property/fuzz tests,
  boundaries 1,232/1,233/4,096/4,097, account/instruction limits, klend index rewrite).
- 3-pool assembly: `crates/arb-tx/tests/three_pool_flash_arb_tests.rs` (order
  borrow→3 swaps→repay, repay amount == principal, executed indices 3/7).
- Kamino: fee differential vs independent reference, reserve decode, liquidity gates.
- Economics: net-profit floors, insufficient-flash-liquidity rejection.
- Gaps: cross-DEX 3-pool integration, provider failover injection, web API tests
  (new surface in this directive), benchmark P50/P95/P99 report.

## 9. Re-validation of prior audit findings

- F1–F9 / O-1..O-17 (PIPELINE_AUDIT_2026-11-12): re-checked — all fixes present at
  `408c7b6` (arb-execution crate gone, Jupiter *discovery* path removed from the live
  loop — residue noted in §4.1, `expected_min_final_output` gate active, config.rs
  extracted, CI pinned).
- Kamino repay pairing (PR #6): re-verified live on Helius 2026-11-13 —
  `flash_borrow`+`flash_repay` V1 roundtrip simulates `err: null` (65,154 CU consumed),
  repay ix carries principal exactly, `borrow_instruction_index` rewritten to executed
  position 3 by the V1 compiler.
- O-16 (Yellowstone gRPC decision): still open by design; Helius WSS `slotSubscribe`
  now verified working (5 advancing slots) which unblocks the shadow-soak gap.
- Security audit (PR #5): no regressions found; secrets handling rules reaffirmed
  (user-provided keys stored only in git-ignored `.env`).

## 10. Live provider probe evidence (2026-11-13, Helius)

- RPC: solana-core **4.4.1**, slot 468,727,140; Kamino WSOL reserve discriminator OK,
  live `flash_loan_fee_sf = 11,529,215,046,068` == repo constant, liquidity mint WSOL,
  **available flash liquidity ≈ 230,445.14 SOL**; Meteora SOL/USDC LbPair owned by the
  DLMM program (904 bytes).
- WSS: `slotSubscribe` connected, 5 slots received, 4 advances — Helius WSS usable for
  market data subscriptions (previously all WS endpoints failed).
- V1 transport: both baseline (154 B) and Kamino roundtrip (638 B) V1 envelopes
  accepted by `simulateTransaction` with `maxSupportedTransactionVersion: 1`;
  roundtrip executed to completion (`err: null`, 65,154 CU).

## 11. Directive-2 remediation plan (file-level)

| # | item | files | class |
|---|---|---|---|
| R1 | Remove Jupiter residue + dead env keys | `arb-tx/src/jupiter.rs`, `arb-tx/src/lib.rs:102,108,3315-3342`, `bin/validate.rs`, `bin/live_scanner.rs`, `.env.example` | D |
| R2 | Remove `arb-mcp` from default workspace (dead) | `crates/arb-mcp/*`, `Cargo.toml` | E |
| R3 | Single canonical Kamino fee (arb-engine re-uses arb-tx) | `arb-engine/src/flash_loan_sizing.rs` | D |
| R4 | Helius provider keys + WSS in config/.env.example | `arb-bot/src/config.rs`, `.env.example` | C |
| R5 | Configurable min net profit (`MIN_NET_PROFIT_USD`) | `arb-bot/src/config.rs`, arb-engine floors | A |
| R6 | Unified control service (Telegram + web same commands) | new `arb-bot/src/control_service.rs`, `telegram.rs`, new `arb-bot/src/web_api.rs` | architecture |
| R7 | Axum REST + WS API with auth/sessions/CSRF/rate-limit/audit log, localhost bind | new `arb-bot/src/web_api.rs`, `main.rs` | architecture |
| R8 | React + TS + Vite app (sections A–I) served by the same Axum server | new `web/` | architecture |
| R9 | Supervisor module split (run.rs 2,134 L) | `supervisor/run.rs` → cohesive modules | maintainability |
| R10 | arb-tx/lib.rs split (4,799 L) | `arb-tx/src/lib.rs` | maintainability |
| R11 | Docker + CI + docs refresh | `Dockerfile`, `.github/workflows/*`, `docs/` | infra |
| R12 | Execution state persistence for safe restart/ambiguous-timeout recovery | `arb-store`, `supervisor/dispatch.rs` | C |

Items R1–R5, R6–R8 (core), R11, R12 are implemented in this branch; R9/R10 are done as
mechanical splits where they do not churn the hot path.

## 12. Honest status register (carried forward)

- No live mainnet broadcast has ever been performed (`ARB_LIVE=0`); all "simulation"
  claims are RPC `simulateTransaction` results, not landings.
- Shadow soak ≥24h: not yet run (WSS now available via Helius).
- Cross-DEX 3-pool live-loop integration: builders exist and are tested; the live loop
  currently enumerates same-protocol DLMM chains.
- No profit claim of any kind is made; economics are quote-time only.
