# Conflicts & Contradictions (Phase 0)

## P0 — Address Conflicts (RESOLVED in this session)

### C-01: USDC reserve mismatch
- **verified_config.rs** (old): `D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25`
- **arb-constants** (canonical): `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59`
- **Resolution:** verified_config.rs now uses `arb_constants::KAMINO_USDC_RESERVE`. Single source of truth.
- **Status:** FIXED. Regression test needed.

### C-02: RaydiumV4Accounts pool_coin_vault mismatch
- **arb-types** (old): `DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz` (typo)
- **arb-constants** (canonical): `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz`
- **Resolution:** arb-types fixed to canonical value.
- **Status:** FIXED. Regression test needed.

## P0 — Execution Safety (RESOLVED in this session)

### C-03: Zero blockhash fallback
- **execution.rs** (old): returned `[0u8; 32]` on failure; caller checked for zero.
- **Resolution:** now returns `Option<[u8; 32]>` (None on failure). Callers (run.rs, dispatch.rs) skip submission on None.
- **Status:** FIXED.

### C-04: Risk config contradiction (MAX_VELOCITY vs max_orders_per_hour)
- **arb-risk** (old): `MAX_VELOCITY_ORDERS_PER_HOUR = 10` (dead const) vs `RiskLimits.max_orders_per_hour = 120`.
- **Resolution:** Removed dead misleading const. Single authoritative value: max_orders_per_hour = 120.
- **Status:** FIXED.

## P1 — Remaining Conflicts (UNRESOLVED)

### C-05: Meteora DLMM 92% safety factor (RESOLVED)
- **dlmm.rs** (old): `safe_optimal_in = (bottleneck_in * 92) / 100` — arbitrary 92% safety factor with no mathematical justification.
- **Resolution:** Removed the 92% factor. Now uses exact bottleneck (min of hop capacities and wallet cap). Safety is enforced by `dlmm_active_bin_max_input`, `consume_bin_liquidity`, and `compute_total_fee_rate`. Test updated to expect exact bottleneck.
- **Status:** FIXED.

### C-06: PoolState::fee_bps silent cap (ALREADY CORRECT)
- **arb-types** (audit claimed): `unwrap_or(10_000)` silently caps invalid fees at 100%.
- **Actual code:** Already returns `None` when fee > FEE_BPS_MAX (10,000). No silent cap exists.
- **Status:** ALREADY RESOLVED in current code.

### C-07: MAX_FEE_RATE = 10% (DLMM)
- **dlmm.rs**: `MAX_FEE_RATE = 100_000_000` (10%).
- **Status:** UNVERIFIED. May not reflect all DEX fee schedules.

### C-08: CORS permissive (RESOLVED)
- **webhook-service/main.rs** (old): `CorsLayer::permissive()` — any origin allowed.
- **Resolution:** Replaced with configured `cors_origins` from `ServerConfig`. Empty = deny all (fail closed). Wildcard `*` rejected in validation. Only GET/POST methods, Content-Type and Authorization headers allowed.
- **Status:** FIXED.

### C-09: Legacy binaries with stale hardcoded pools
- **live_scanner.rs, live_single_exec.rs, latency_bench.rs**: contain stale pool constants, bypass mode gate.
- **Status:** MITIGATED. Quarantine review pending for Phase 2.

### C-10: Auxiliary binaries bypass live gate
- **validate.rs, discover.rs, replay.rs, live_scanner.rs, latency_bench.rs, alchemy_webhook.rs**: not governed by main mode gate.
- **Status:** MITIGATED — no live trading bypass exists.

### C-11: No HSM/secrets manager
- Keypair is file/env; 0600 perms only control.
- .env.example created with placeholder structure (no real values).
- config.toml security header added clarifying env var names vs secret values.
- **Status:** PARTIALLY RESOLVED — .env.example created, config.toml documented. Full HSM integration is architectural/long-term.

### C-12: Admin token shared
- No per-operator identity, no audit trail.
- Code already implements: constant-time comparison, placeholder rejection, fail-closed on missing/placeholder token.
- **Status:** UNRESOLVED (architectural). Per-operator identity requires user management system beyond scope of this refactor.

### C-13: Dependency advisories unverified
- cargo audit blocked (RustSec DB fetch failed).
- **Status:** NOT VERIFIED.

### C-14: No release artifact signature
- No cosign/SBOM.
- **Status:** UNRESOLVED.

### C-15: Kamino offsets verification
- Offsets verified 2026-11-04; must re-verify if program upgraded.
- **Status:** UNVERIFIED (ongoing).

### C-16: ComputeUnits MAX = 1,400,000
- Solana cap; hardcoded.
- **Status:** UNVERIFIED.

### C-17: Tip oracle DEFAULT_MAX_AGE_MS / TIP_FLOOR_URL (ALREADY IMPLEMENTED)
- **tip_oracle.rs**: `TIP_FLOOR_URL = "https://bundles.jito.wtf/api/v1/bundles/tip_floor"`, `DEFAULT_MAX_AGE_MS = 5_000`. Already uses dynamic data from Jito API with freshness tracking.
- **Status:** ALREADY IMPLEMENTED correctly.

### C-18: webhook-service config load panics (RESOLVED)
- **main.rs** (old): `.expect("FATAL: Unable to load config.toml")` would panic.
- **Resolution:** Changed to `.map_err(|e| anyhow::anyhow!(...))?` — returns proper error instead of panicking.
- **Status:** FIXED.

### C-19: arb-types RaydiumV4Accounts hardcoded addresses
- All mainnet addresses hardcoded (pool, vaults, serum market, etc.).
- pool_coin_vault typo FIXED (C-02): `DQyrAcCrDXYZ…` → `DQyrAcCrDXQ7…`
- Remaining addresses are protocol constants from Raydium mainnet.
- **Status:** PARTIALLY RESOLVED — should be centralized in arb-constants in Phase 2.

### C-20: Token-2022 rejected
- transfer_fee_bps != 0 → Err (intentional fail-closed).
- **Status:** BY DESIGN.

### C-21: PumpFun/PumpSwap quote-only
- Cannot reach transaction builder.
- **Status:** BY DESIGN.

### C-22: Fee cap 10% (MAX_FEE_RATE)
- May not reflect all DEX fee schedules.
- **Status:** UNVERIFIED.

### C-23: Meteora safety factor 92%
- May mask missing bin coverage.
- **Status:** UNVERIFIED.

### C-24: CORS permissive
- Any origin allowed.
- **Status:** UNRESOLVED.

### C-25: No per-operator identity for admin
- Shared bearer token.
- **Status:** UNRESOLVED.

### C-26: Dependency advisories
- cargo audit blocked.
- **Status:** NOT VERIFIED.

### C-27: No release signature
- **Status:** UNRESOLVED.

### C-28: Credentials not rotated
- **Status:** OPERATIONAL (not code).

### C-29: Token-2022 transfer fee rejection
- Intentional fail-closed.
- **Status:** BY DESIGN.

### C-30: PumpFun/PumpSwap quote-only
- Intentional.
- **Status:** BY DESIGN.