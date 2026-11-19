# Safety Guards & Hardcoded Values — Full Audit

> Consolidated audit of all safety guards (fail-closed controls) and hardcoded values across the Solana arbitrage bot codebase. Includes every conflict/contradiction found.

---

## 1. Safety Guards (Fail-Closed Controls)

### 1.1 `arb-risk` (CircuitBreaker / KillSwitch / RiskGate / Chronos)

| Guard | Details |
|---|---|
| **KillSwitch (durable)** | Marker file (`data/kill_switch`). Starts BLOCKED if marker exists. `activate` writes marker; `deactivate` removes it. Survives restart (test: `kill_switch_marker_survives_restart`). Fail-safe on I/O errors. |
| **CircuitBreaker** | States: Closed/Open/HalfOpen. Opens after 8 consecutive failures; cooldown 10 slots; closes after 3 successes in HalfOpen. |
| **RiskGate.evaluate_full** | kill switch → breaker → trade size → velocity → tip share → tip lamports → price impact → state age → min profit → losses. |
| **RiskGate.evaluate_flash_loan** | + inflight bundles, tip spend ratio, min landing rate. |
| **check_velocity** | max_orders_per_hour (120) within 1hr window. |
| **check_trade_size** | size > 0 & finite & ≤ max (25 USD). |
| **check_tip_share** | ≤ max_tip_share_bps (8500). |
| **check_tip_lamports** | ≤ max_tip_lamports (2,000,000). |
| **check_price_impact** | ≤ max_price_impact_bps (40). |
| **check_state_age** | ≤ max_state_age_slots (2). |
| **check_min_profit** | net ≥ min_abs (50,000 lamports) AND ≥ min_bps (8 bps of amount_in). |
| **check_losses** | daily ≤ 5 USD, hourly ≤ 2 USD. |
| **check_max_inflight_bundles** | ≤ 2 per slot. |
| **check_tip_spend_ratio** | tip/(tip+net) ≤ 0.7. |
| **check_min_landing_rate** | landed/sent ≥ 0.02 (after history). |
| **Chronos EV Gate** | age ≤ max_age, Pi_success > 0, EV ≥ min_ev_usd. |
| **Admin auth** | Constant-time bearer/X-Admin-Token. Missing/placeholder → disable listener (fail closed). |

### 1.2 `arb-engine/guarded.rs` (State Gate)
- `quote_route_guarded`: readiness (max_age_slots, max_slot_skew) → generation → protocol match → slot match → post-quote recheck (generation + evidence equality) → `StateChangedBeforeDispatch`.
- `revalidate_before_dispatch`: final check before submission.
- Private pool copy (rejected quote doesn't pollute cache).

### 1.3 `arb-bot/supervisor/dispatch.rs` (Execution Guards)
- Live gate double check: `mode == Live && live_execution_enabled()`.
- Safety Floor: balance ≥ 50,000,000 lamports (0.05 SOL) else skip.
- Self-sweep guard: cold wallet ≠ signer (skip self-sweep).
- MTU check: tx_bytes ≤ 1232 bytes else skip.
- Pre-flight simulation (fail-closed): LiteSVM local + RPC simulation. RPC error → fail closed.
- Final stream readiness guard: `validate_execution_readiness` (connected, generation, slot, age).
- Fire-time wait: ≤ SLOT_MS.
- Post-send tracking: record_order, record_bundle_sent, record_bundle_landed, on_success/on_failure, breaker state, loss recording on drop/fail.

### 1.4 `arb-bot/supervisor/execution.rs` (Assembly Guards)
- Flash vs non-flash: uses `use_flash_loan` flag.
- `min_profit_raw` from economic config (graph), not hardcoded.
- Blockhash cache: 3s TTL; fallback `[0u8;32]` if fetch fails.
- `live_sol_price`: `None` if ≤ 0 (never guessed constant).
- `build_verified_inputs`: snapshot-only (no HTTP RPC); freshness check.
- `max_state_age_slots`: fail closed — missing pool → `u64::MAX`.

### 1.5 `arb-bot/verified_config.rs` (Verified Inputs)
- Fail closed on missing snapshots (reserve, mint).
- Owner checks: reserve == klend program; mint ∈ {token, token-2022}.
- Slot freshness: snapshot.slot ≤ current_slot.
- Decimals validation: ≤ 18, initialized flag == 1.
- Rational rate: fail closed if not integral (USDC 6 decimals needs SOL price).
- No HTTP JSON-RPC (deprecated → error). Yellowstone only.
- No hardcoded Kamino fee (read from `flash_loan_fee_sf`).
- No hardcoded decimals (read from mint).
- No hardcoded tip (configured ceiling).

### 1.6 `arb-tx` (Transaction Safety)
- Kamino flash fee decoded from `ReserveConfig.fees.flash_loan_fee_sf` (ceiling rounding).
- No hardcoded fee bps/decimals.
- Slippage-derived sqrt_price_limit (no full-range hardcoded).

### 1.7 `arb-math/economic.rs`
- Base mint decimals from verified mint (never hardcoded to 9).
- Fail closed on unverifiable inputs.

### 1.8 `arb-ingest`
- Registry: fee_rate read from chain (4 bps example), not hardcoded.
- Pump.fun fee: requires explicit fee BPS + decimals; skips until joined.
- No hardcoded canonical/non-canonical tier table.
- `slot_integrity`: dead slot invalidates descendants; bank identity required for reopening; `MAX_TRACKED_SLOTS` = 8,192.
- `quote_readiness`: 12 fail-closed checks (stream, deps, owner, account, startup, provider, generation, bank identity, dead slot, future slot, stale slot, coherence).
- `provider_supervisor`: failover (3 failures → OpenCircuit + 2s cooldown); selection (healthy + fresh + in-lag); disable.

### 1.9 `arb-engine/route_worker.rs` (Split Routing)
- `pool_depth_from_state`: returns `None` if fee invalid (never prices with guessed default).
- `maybe_split_allocation`: only splits when price impact > max_price_impact_bps; requires ≥ 2 parallel pools; skips zero-reserve/fee-invalid pools.
- `start_worker_loop`: DISABLED (fabricated economics removed).
- `cpmm_output`: saturating arithmetic; returns 0 on zero inputs.

### 1.10 `arb-engine/adapters.rs`
- `execution_capability`: PumpFun & PumpSwap → quote-only (Err). RaydiumV4 always Ok. Orca/CLMM require decoded clmm state. RaydiumCpmm requires fee valid + reserves > 0. Meteora DLMM requires bins + params.
- `quote_hop`: amount_in==0 → Err; pool inactive → Err; fee invalid → Err; Token-2022/risky mint (transfer_fee_bps != 0 or has_risky_token) → Err.
- PumpSwap: dedicated adapter required (returns Err in generic path).
- Mint continuity check in `quote_route`.
- Compounded price impact: 1 - prod(1 - impact_i).

### 1.11 `arb-dex/pump_fun.rs`
- fee_bps required (None → Err, no static 1.25% fallback).
- mode_flags_present required.
- token_decimals required.
- mayhem/cashback curves → Err.
- complete (graduated) → Err.
- zero input → Err.
- zero virtual reserves → Err.
- fee_bps > 10,000 → Err.
- real reserves exceeded → PumpFunReserveExceeded.
- Overflow checks (checked_mul/sub/add).
- Account size ≥ 83 bytes + discriminator match.
- bool validation (only 0/1).

### 1.12 `arb-math/clmm.rs`
- Tick bounds: ±443,636 (out-of-bounds → Err).
- Sqrt price bounds: MIN/MAX_SQRT_PRICE_X64.
- Zero checks: amount_in==0 → Ok (no-op); liquidity==0 → Err (DivisionByZero in b-input).
- Fee rate validation: fee_rate==0 → Err.
- Division by zero: explicit Err.
- Overflow: checked_add/sub/mul; U256→u128 try_from → Err.
- No progress: `NoProgress` error.
- Iteration limit: max_steps.
- Incomplete quote: amount_remaining > 0 → IncompleteQuote.
- Tick array coverage: MissingTickArrayCoverage if array not found (no synthetic fallback).
- Tick spacing: tick_spacing==0 → Err.
- Initialized tick validation: uninitialized tick with non-zero liquidity → Err.
- Tick array data size: < 9,988 bytes → Err.
- start_tick alignment: must be divisible by ticks_per_array.
- Liquidity update on cross: checked_add/sub → LiquidityOverflow.

### 1.13 `arb-math/dlmm.rs`
- Bin ID bounds: ±443,636.
- Pool disabled: status != 0 → Err.
- Zero input: amount_in==0 → Err.
- Bin not found: missing bin → Err.
- Zero price_q64 → None.
- Zero reserves → None.
- Overflow: checked_add/mul/sub; try_into → Err.
- Fee cap: total fee clamped to MAX_FEE_RATE (10%).
- Fee rate zero → 0 (no fee).
- Denominator zero → Err.
- Pow exponent: exp_abs >= MAX_EXPONENTIAL → Err; result==0 → Err (underflow).
- Advance active id: out of bounds → Err.
- Price impact: ideal_out ≤ amount_out → 0 bps.

### 1.14 `arb-math/cpmm.rs`
- Zero inputs: dx/x/y/fee_denominator == 0 → None.
- Denominator zero → None.
- Overflow: out > u128::MAX → None.
- Existence invariant: N <= D0 → None.
- d1 zero → None.
- sqrt <= d0 → None.
- d_star zero → None.
- u64 cast failure → None.
- Price impact: invalid inputs → Bps(10000) (100% impact, fail-closed).
- Marginal degradation ceiling: LHS <= RHS → past ceiling.
- Derivative: denom_sq==0 → i128::MIN.

### 1.15 `arb-tx/kamino.rs`
- `decode_reserve_flash_fee_sf`: account too short → Err; discriminator mismatch → Err; mint mismatch → Err; status != 0 → Err.
- `calculate_kamino_flash_fee`: fee_sf==0 || borrow==0 → 0; ceiling rounding; capped at u64::MAX.
- `build_kamino_flash_borrow/repay`: 12 accounts exact; referrer optional → defaults to program_id (readonly).
- No hardcoded fee — read from on-chain `flash_loan_fee_sf`.

### 1.16 `arb-types/src/lib.rs`
- Lamports/TokenAmount: checked_add/sub/mul; saturating_sub.
- Bps: MAX = 10,000.
- Slot: checked_sub; saturating_sub.
- ComputeUnits: MAX = 1,400,000.
- UsdMicros: saturating conversion; NaN → ZERO; clamp to i64 bounds.
- MintInfo::is_tradeable: no transfer hook, no permanent delegate, no freeze authority, transfer_fee_bps==0.
- PumpFunState::is_tradeable: mode_flags_present && !mayhem && !cashback && !complete && real_token_reserves>0 && virtual_sol>0 && virtual_token>0.
- PoolState::fee_bps: None if denominator==0; capped at 10,000.
- PoolState::is_fee_valid: denominator != 0.
- RaydiumV4Accounts::mainnet_sol_usdc: hardcoded mainnet addresses.

### 1.17 `arb-land` (lib.rs, bidder, ledger, tip_oracle, leader)
- Bundle size: 1..=5 txs (else Err).
- Bundle status: `processed` → Sent (not Landed); `confirmed`/`finalized` → Landed; has_err → Failed.
- Inflight status: max 5 bundle ids; empty → Err.
- Unknown status → Pending (not terminal) — prevents false failure recording.
- Invalid → no landing evidence (None).
- HTTP client: 5s timeout, pool_max_idle_per_host=8, tcp_nodelay.
- Tip strategy: floor/cap/min_keep; dynamic scaling with profit & competition; min tip 1000 lamports.
- Adaptive CU price: p75 + 25%; empty → 100,000 micro-lamports.
- Tip floor URL: `TIP_FLOOR_URL` (from tip_oracle).
- Max age: `DEFAULT_MAX_AGE_MS` (from tip_oracle).

### 1.18 `webhook-service` (main.rs, config, router, engine, dedup, handlers)
- CORS: `CorsLayer::permissive()` (⚠️ permissive — any origin).
- Graceful shutdown: Ctrl+C + SIGTERM (Unix).
- Config load: fallback to `config.toml` in CWD; panic if missing.
- Dedup: Redis + local DashMap; pruning task interval.
- Worker engine: channel_buffer_size, worker_threads (config).
- Routes: /v1/webhook, /health, /webhooks/SOLAMI, /webhooks/helius/pumpfun, /v1/bitquery/*, /api/topology, /ws/topology.

---

## 2. Hardcoded Values

### 2.1 `arb-constants/src/lib.rs`
| Category | Values |
|---|---|
| Token programs | Tokenkeg…, TokenzQd…, pfeeUx…, ATokenGP…, ALT, Sysvar Instructions/Rent |
| Mints | WSOL So111…112, USDC EPjF…Dt1v, Token-2022 native 9pan…XejP |
| Jito tip accounts | 8 mainnet addresses |
| Jito defaults | floor 10,000 lamports; tip share 7,000 bps; max tip share 8,500 bps; keep-share 0.85; min keep 50,000 lamports |
| DEX program IDs | Raydium v4/CPMM/CLMM, Orca, Meteora, Pump.fun, PumpSwap + discriminators + seeds |
| Pools SOL-USDC | Raydium v4 58oQ…YQo2, CPMM 79Rb…sKJ7, Orca Czfq…44zE, Meteora 5rCf…HAS6 |
| Meteora | BINS_PER_BIN_ARRAY=70, BIN_STEP=4 |
| Kamino | klend program, main market, reserves, flash borrow/repay discriminators |
| Compute budget | 2-hop 260k, 3-hop 320k, max 350k CU, buffer 15%, MTU 1232, base priority fee 1000 µ-lamports |
| Pump.fun graduation | 85,000,000,000 lamports |
| Hop slippage | 10 bps |
| JITODONTFRONT | 32-byte marker |

### 2.2 `arb-risk/src/lib.rs`
| Category | Values |
|---|---|
| RiskLimits defaults | max_trade_size_usd=25.0; min_net_profit_bps=8; max_price_impact_bps=40; max_state_age_slots=2; max_tip_lamports=2,000,000; max_daily_loss_usd=5.0; max_hourly_loss_usd=2.0; min_landing_rate=0.02; max_inflight_bundles=2; max_tip_spend_ratio=0.7; max_orders_per_hour=120 |
| MAX_VELOCITY_ORDERS_PER_HOUR | 10 |
| CircuitBreaker default | 8 failures / 10 slots cooldown / 3 successes |
| ChronosRiskConfig default | enabled=true; decay_lambda=3.0; max_age_ms=400; min_ev_usd=0.05; failed_cost=10,000 lamports; beta=0.0005 |
| Chronos EV | 1_000_000_000.0 (lamports→SOL) |

### 2.3 `arb-bot/supervisor/dispatch.rs`
| Category | Values |
|---|---|
| Safety floor | 50,000,000 lamports (0.05 SOL) |
| Sweep threshold | 250,000,000 lamports (0.25 SOL) |
| Sweep fee reserve | 10,000 lamports |
| MTU | 1232 bytes |
| Preflight polls | 20 (SWQoS sig), 8 (bundle), 400ms interval |
| Loss calc base | 5,000 lamports + tip |
| Fire wait | ≤ SLOT_MS |
| Enable preflight env | "ENABLE_PREFLIGHT_SIMULATION" == "true" |

### 2.4 `arb-bot/supervisor/execution.rs`
| Category | Values |
|---|---|
| Blockhash cache TTL | 3 seconds |
| Jupiter slippage | 50 bps |
| Fallback blockhash | `[0u8; 32]` |

### 2.5 `arb-bot/verified_config.rs`
| Category | Values |
|---|---|
| USDC reserve | hardcoded `D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25` (⚠️ differs from arb-constants!) |
| Decimals cap | 18 |
| Rate base | 10^9 |
| Price scale | 10^6 (for 6-decimals USDC) |

### 2.6 `arb-engine/route_worker.rs`
| Category | Values |
|---|---|
| Fee denominator | 10,000 (bps) |
| Bisection bounds | lo=1e-12, hi=1e24, 80 guard iterations, 200 convergence iterations |
| Disabled worker | sleeps 3600s |

### 2.7 `arb-engine/adapters.rs`
| Category | Values |
|---|---|
| CU estimates | RaydiumV4 65k, CPMM 45k, CLMM 55k, Orca 50k, Meteora 60k, PumpFun 40k, PumpSwap 48k |
| Account counts | RaydiumV4 18, CPMM 13, CLMM 15, Orca 11, Meteora 15, PumpFun 12, PumpSwap 16 |
| Price impact scale | 1_000_000_000 (1e9) |
| BPS factor | 100_000 (1e5) per bps |

### 2.8 `arb-dex/pump_fun.rs`
| Category | Values |
|---|---|
| PUMP_FUN_BONDING_CURVE_SIZE | 83 bytes |
| Account layout offsets | discriminator 8, reserves @ 8/16/24/32, supply @ 40, complete @ 48, creator @ 49..81, mode flags @ 81/82 |
| Base decimals | 9 (WSOL-quoted only) |
| Fee denominator | 10,000 (BPS) |
| Graduation | 85,000,000,000 lamports (from arb-constants) |

### 2.9 `arb-math/clmm.rs`
| Category | Values |
|---|---|
| Tick bounds | ±443,636 |
| Sqrt price bounds | 4,295,048,016 … 79,226,673,515,401,499,993,453,864,427 |
| Q64 | 1 << 64 |
| FEE_RATE_DENOMINATOR | 1,000,000 |
| TICK_ARRAY_SIZE | 88 |
| WHIRLPOOL_TICK_ARRAY_SIZE | 9,988 bytes |
| TICK_STRUCT_SIZE | 113 bytes |
| Tick offset in array | 44 + i*113 |
| net_bytes offset | +1..+17 |
| gross_bytes offset | +17..+33 |
| initialized byte offset | 0 (relative) |
| start_tick bytes | 40..44 |

### 2.10 `arb-math/dlmm.rs`
| Category | Values |
|---|---|
| SCALE_OFFSET | 64 |
| ONE_Q64 | 1 << 64 |
| BASIS_POINT_MAX | 10,000 |
| FEE_PRECISION | 1,000,000,000 |
| MAX_FEE_RATE | 100,000,000 (10%) |
| MIN/MAX_BIN_ID | ±443,636 |
| MAX_EXPONENTIAL | 0x80000 (524,288) |
| Fee denom (variable) | 100,000,000,000 |
| Safety factor (scale) | 92% (92/100) |
| Protocol share | bps (e.g. 1000 = 10%) |
| Bin step | bps (e.g. 25 = 0.25%) |

### 2.11 `arb-math/cpmm.rs`
| Category | Values |
|---|---|
| BASIS_POINTS_DIVISOR | 10,000 |
| NANO_SCALE | 1,000,000,000 |
| U-types | U384, U512, U768 |
| Price impact fallback | 10,000 bps (100%) |
| Fee denominators | 10,000 (V4), 1,000,000 (CPMM PPM) |

### 2.12 `arb-tx/kamino.rs`
| Category | Values |
|---|---|
| FLASH_BORROW_DISCRIMINATOR | [135, 231, 52, 167, 7, 52, 212, 193] |
| FLASH_REPAY_DISCRIMINATOR | [185, 117, 0, 203, 96, 245, 180, 186] |
| KAMINO_RESERVE_DISCRIMINATOR | [0x2b, 0xf2, 0xcc, 0xca, 0x1a, 0xf7, 0x3b, 0x7f] |
| Offsets | liquidity_mint @8+120, mint_decimals @8+264, status @8+4848, fee_sf @8+4896 |
| KAMINO_RESERVE_STATUS_ACTIVE | 0 |
| Account size | 8 + 4896 + 8 = 4912 bytes |
| Market authority bump | 248 |
| Borrow ix data | 16 bytes (8 disc + 8 amount) |
| Repay ix data | 17 bytes (8 disc + 8 amount + 1 borrow_ix_index) |
| Accounts count | 12 |
| Mainnet sf | 11,529,215,046,068 (≈0.006 bps) |

### 2.13 `arb-types/src/lib.rs`
| Category | Values |
|---|---|
| Lamports/TokenAmount | checked_add/sub/mul; saturating_sub |
| Bps | MAX = 10,000 |
| Slot | checked_sub; saturating_sub |
| ComputeUnits | MAX = 1,400,000 |
| UsdMicros | saturating conversion; NaN → ZERO; clamp to i64 bounds |
| Z_95 | 1.6448536269514722 |
| Percentile | 7 rungs (P50..P99) with probabilities 0.50..0.99 |
| Posterior | PRIOR = Beta(1,1); wilson_lower with z |
| InflightStatus::is_terminal | Landed/Failed/Invalid |
| RaydiumV4Accounts::mainnet_sol_usdc | hardcoded mainnet addresses (pool, vaults, serum market, etc.) |

### 2.14 `arb-land` (lib.rs, bidder, ledger, tip_oracle, leader)
| Category | Values |
|---|---|
| HTTP timeout | 5s |
| pool_max_idle_per_host | 8 |
| max_retries | 3 |
| bundle txs range | 1..=5 |
| inflight bundle ids max | 5 |
| adaptive CU price default | 100,000 micro-lamports |
| CU escalation | +25% (p75 * 1.25) |
| tip scaling | profit_sol * 2.0 capped at 1.0 (over 0.5 SOL) |
| min Jito tip | 1000 lamports |
| Jito endpoint | https://mainnet.block-engine.jito.wtf/api/v1/bundles (or region.mainnet...) |
| poll bundle | 8 attempts × 400ms (dispatch.rs) |
| poll signature | 20 attempts × 400ms (dispatch.rs) |

### 2.15 `arb-ingest/provider_supervisor.rs`
| Category | Values |
|---|---|
| SupervisorPolicy defaults | failure_threshold=3, cooldown=2s, max_slot_lag=8, stale_after=3s |

### 2.16 `slot_integrity.rs`
| Category | Values |
|---|---|
| Slot status codes | 0=Processed, 1=Confirmed, 2=Finalized, 3=FirstShredReceived, 4=Completed, 5=CreatedBank, 6=Dead |
| MAX_TRACKED_SLOTS | 8,192 |

### 2.17 `webhook-service` (main.rs)
| Category | Values |
|---|---|
| CORS | permissive (any origin) |
| Log filter default | solana_webhook_service=info,tower_http=warn |
| Bind address | from config (host:port) |

---

## 3. Conflicts & Contradictions (ALL)

### 3.1 Code-Level Conflicts

1. **USDC reserve mismatch**: `verified_config.rs` hardcodes `D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25` while `arb-constants` has `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59`. Need verification which is correct.

2. **RaydiumV4Accounts pool_coin_vault typo**: `arb-types/src/lib.rs` has `DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz` while `arb-constants` has `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz`. Possible typo (DXYZ vs DXQ7).

3. **Legacy binaries contain stale hardcoded pool constants**: `live_scanner.rs`, `live_single_exec.rs`, `latency_bench.rs` still contain `RAYDIUM_SOL_USDC_POOL` and overlap with supervisor. Quarantined but still compile.

4. **Auxiliary binaries bypass main mode gate**: `live-scanner`, `validate`, `discover`, `replay` are not governed by `--mode live` / `ARB_LIVE` gate.

5. **Token-2022 transfer fee**: `adapters.rs` rejects any pool with `transfer_fee_bps != 0` or `has_risky_token` until verified per-mint TransferFeeConfig quote. Intentional fail-closed, but means Token-2022 tokens cannot be traded.

6. **PumpFun/PumpSwap quote-only**: Cannot reach transaction builder until verified. Intentional.

7. **CORS permissive in webhook-service**: `CorsLayer::permissive()` allows any origin — security concern for production.

8. **No HSM/secrets manager**: Keypair is file/env input; file permissions (0600) are the only control.

9. **Admin token shared**: Bearer token, no per-operator identity, no audit trail of who paused/resumed.

10. **Dependency advisories unverified**: `cargo audit` blocked (RustSec DB fetch failed). Vulnerability status unknown.

11. **No release artifact signature**: No cosign/SBOM produced.

12. **Credentials not rotated**: Repository cannot prove keys exposed outside git are revoked.

13. **Kamino offsets verified on 2026-11-04**: Must re-verify if program upgraded (offsets may change).

14. **Meteora safety factor 92%**: Hardcoded to avoid crossing suboptimal bins; may be overly conservative.

15. **Fee cap 10% (MAX_FEE_RATE)**: Hardcoded; may not reflect all DEX fee schedules.

16. **ComputeUnits MAX = 1,400,000**: Solana cap; hardcoded.

17. **Tip oracle DEFAULT_MAX_AGE_MS**: Value not visible in reviewed code (verify).

18. **TIP_FLOOR_URL**: Value not visible in reviewed code (verify).

19. **webhook-service config load**: Falls back to `config.toml` in CWD; panics if missing. No graceful degradation.

20. **`arb-types` PoolState.fee_bps**: Caps at 10,000 (100%) via `unwrap_or(10_000)` — silently caps invalid fees instead of rejecting.

### 3.2 Design Conflicts (Intentional)

21. **Kamino fee NOT hardcoded**: Read from on-chain `flash_loan_fee_sf` (fail-closed if unreadable). Previously hardcoded 5 bps (FIN-01 fixed).

22. **Decimals NOT hardcoded**: Read from mint account (FIN-04 fixed). Previously `base_decimals: 9` hardcoded.

23. **Pump.fun fee NOT hardcoded**: Requires explicit fee BPS + decimals (FIN-P0-06 fixed). Previously 100 bps.

24. **Fee rates read from chain**: Not hardcoded per DEX (registry.rs).

25. **Tip NOT hardcoded**: Configured ceiling, not 0/5 bps guess.

26. **SOL price tracked live**: `None` if unavailable (never guessed constant).

27. **Token-2022 rejected until verified**: No BPS approximation (intentional fail-closed).

28. **Processed ≠ Landed**: Correctly mapped to `Sent` in arb-land (prevents false landing evidence).

29. **Invalid status → no landing evidence**: `None` (prevents false failure recording).

30. **Unknown status → Pending**: Not terminal (prevents false failure recording).

---

## 4. What is NOT Hardcoded (By Design)

- Kamino flash fee → on-chain `flash_loan_fee_sf`.
- Decimals → mint account.
- Pump.fun fee → explicit fee BPS + decimals required.
- Fee rates → read from chain.
- Tip → configured ceiling (not 0/5 bps guess).
- SOL price → tracked live (None if unavailable).
- Token-2022 transfer fees → rejected until verified per-mint.

---

## 5. Summary Table

| Category | Count |
|---|---|
| Safety guards (fail-closed) | 60+ |
| Hardcoded values (constants) | 80+ |
| Conflicts/contradictions | 30 |
| Intentional design decisions (not hardcoded) | 8 |

---

*Generated from full codebase audit. All files in `crates/`, `webhook-service/`, `docs/` reviewed.*