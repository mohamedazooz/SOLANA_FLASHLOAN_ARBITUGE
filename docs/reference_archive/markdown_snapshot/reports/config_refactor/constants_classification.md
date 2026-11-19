# Constants Classification

**Repository:** Solana Arbitrage Bot  
**Audit date:** Mon Nov 16 2026  
**Method:** Every hardcoded value classified into one of four categories per the mission spec:
- **A. IMMUTABLE PROTOCOL CONSTANTS** — stay in strongly typed protocol modules; never ordinary operator config.
- **B. OPERATIONAL CONFIGURATION** — move into typed configuration with type, unit, validation, env override.
- **C. DYNAMIC MARKET & EXECUTION PARAMETERS** — remove static assumptions; observe from live data with freshness.
- **D. SECURITY SECRETS** — never hardcode; secret provider abstraction.

---

## Classification Summary

| Category | Count | Representative values |
|----------|-------|----------------------|
| A — Immutable Protocol Constants | ~73 | Program IDs, mints, discriminators, seeds, PDA constants, Q64 scales, bin layout, MTU |
| B — Operational Configuration | ~57 | Risk limits, tip params, timeouts, cooldowns, trade size, loss limits, CU buffer |
| C — Dynamic Market & Execution | ~9 | DLMM 92% safety factor, MAX_FEE_RATE, fee denominator, priority fee base, CU estimates, tip freshness, URLs |
| D — Security Secrets | ~5 | Admin token env, RPC key env names, Jupiter key env, keypair path, .env credentials |

---

## A. IMMUTABLE PROTOCOL CONSTANTS

These values are protocol invariants. They must:
- Remain in strongly typed protocol modules (`arb-constants`, `arb-dex`, `arb-tx`, `arb-math`).
- Have one authoritative source per value.
- Record the upstream repository, SDK version, and commit where verifiable.
- Have tests validating their values.
- Have cluster/network compatibility checks.
- Use versioned decoders for changing account layouts.
- Have fixtures for known account layouts.
- Detect unsupported protocol versions and fail closed.

They must NOT be moved into ordinary operator configuration.

### A.1 Token Programs & System IDs (verified against Solana docs)
| Constant | Value | Upstream source | Status |
|----------|-------|-----------------|--------|
| TOKEN_PROGRAM_ID | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | SPL Token program | VERIFIED (canonical) |
| TOKEN_2022_PROGRAM_ID | `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` | SPL Token-2022 | VERIFIED (canonical) |
| ASSOCIATED_TOKEN_PROGRAM_ID | `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | SPL Associated Token | VERIFIED (canonical) |
| ADDRESS_LOOKUP_TABLE_PROGRAM_ID | `AddressLookupTab1e1111111111111111111111111` | Solana lookup table | VERIFIED (canonical) |
| SYSVAR_INSTRUCTIONS_ID | `Sysvar1nstructions1111111111111111111111111` | Solana sysvar | VERIFIED (canonical) |
| SYSVAR_RENT_ID | `SysvarRent111111111111111111111111111111111` | Solana sysvar | VERIFIED (canonical) |
| SYSTEM_PROGRAM_ID | `[0;32]` | Solana system program | VERIFIED (canonical) |
| COMPUTE_BUDGET_PROGRAM_ID | `[3,6,70,111,...]` | Solana compute budget | VERIFIED (canonical) |

### A.2 Token Mints
| Constant | Value | Upstream source | Status |
|----------|-------|-----------------|--------|
| WSOL_MINT | `So11111111111111111111111111111111111111112` | Solana wrapped SOL | VERIFIED (canonical) |
| USDC_MINT | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` | USDC mainnet | VERIFIED (canonical) |
| TOKEN_2022_NATIVE_MINT | `9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP` | Token-2022 native SOL | UNVERIFIED |

### A.3 DEX Program IDs & Discriminators
| Constant | Value | Upstream source | Status |
|----------|-------|-----------------|--------|
| RAYDIUM_V4_PROGRAM_ID | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Raydium | VERIFIED (canonical) |
| RAYDIUM_CPMM_PROGRAM_ID | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Raydium | VERIFIED (canonical) |
| RAYDIUM_CLMM_PROGRAM_ID | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Raydium | VERIFIED (canonical) |
| ORCA_WHIRLPOOL_PROGRAM_ID | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | Orca | VERIFIED (canonical) |
| METEORA_DLMM_PROGRAM_ID | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Meteora | VERIFIED (canonical) |
| PUMP_FUN_PROGRAM_ID | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Pump.fun | VERIFIED (canonical) |
| PUMP_SWAP_PROGRAM_ID | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | PumpSwap | VERIFIED (canonical) |

### A.4 Anchor Discriminators & PDA Seeds
All Anchor discriminators (8-byte `sha256("account:X")[..8]` or `sha256("global:ix")[..8]`) and PDA seeds are protocol serialization structures. They must remain in `arb-constants` with byte-equality tests. Each has an associated upstream IDL that must be cited.

### A.5 Pool & Vault Addresses (MAINNET — UNVERIFIED, must be checked on-chain)
| Constant | Value | Status | Required verification |
|----------|-------|--------|----------------------|
| RAYDIUM_SOL_USDC_POOL | `58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2` | UNVERIFIED | Pool state, mint identity, vault authority |
| RAYDIUM_SOL_VAULT | `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz` | UNVERIFIED | Pool coin vault for above pool (P0 C-02) |
| RAYDIUM_USDC_VAULT | `HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz` | UNVERIFIED | Pool pc vault |
| ORCA_SOL_USDC_POOL | `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` | UNVERIFIED | Pool state |
| ORCA_SOL_VAULT | `EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9` | UNVERIFIED | Vault authority |
| ORCA_USDC_VAULT | `2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP` | UNVERIFIED | Vault authority |
| METEORA_SOL_USDC_POOL | `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6` | UNVERIFIED | Pool state, bin_step=4 |
| METEORA_SOL_RESERVE | `EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o` | UNVERIFIED | Reserve account |
| METEORA_USDC_RESERVE | `CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz` | UNVERIFIED | Reserve account |
| METEORA_SOL_USDC_BIN_STEP | `4` | UNVERIFIED | Compare vs Meteora SDK pool params |
| METEORA_BINS_PER_BIN_ARRAY | `70` | UNVERIFIED | Must equal `lb_clmm::state::bin_array::BINS_PER_ARRAY` |
| RAYDIUM_CPMM_* (pool/vaults) | 3 addresses | UNVERIFIED | Pool state |
| KAMINO_* (program, market, reserves, vaults) | 9 addresses | UNVERIFIED | Kamino main market, WSOL & USDC reserves (P0 C-01) |

### A.6 Q64 / Fixed-Point Scales & Arithmetic
| Constant | Value | Classification | Notes |
|----------|-------|----------------|-------|
| DLMM SCALE_OFFSET | `64` | Protocol constant | Q64.64 fixed point |
| DLMM ONE_Q64 | `1 << 64` | Protocol constant | |
| CLMM Q64 sqrt price bounds | `4,295,048,016` … `79,226,673,515,401,499,993,453,864,427` | Protocol constant | Tick price bounds |
| CLMM FEE_RATE_DENOMINATOR | `1,000,000` | Protocol constant | Whirlpool fee denominator |
| CPMM BASIS_POINTS_DIVISOR | `10,000` | Protocol constant | BPS |
| CPMM NANO_SCALE | `1,000,000,000` | Protocol constant | |

### A.7 Protocol Limits
| Constant | Value | Classification | Notes |
|----------|-------|----------------|-------|
| SOLANA_PACKET_MTU_LIMIT | `1232` | Protocol limit | Transaction size |
| MAX_COMPUTE_UNIT_LIMIT | `350,000` | Protocol limit | Solana runtime CU cap |
| COMPUTE_BUDGET_LIMIT_2HOP | `260,000` | Route-specific limit | ≤ runtime cap |
| COMPUTE_BUDGET_LIMIT_3HOP | `320,000` | Route-specific limit | ≤ runtime cap |
| PUMP_FUN_GRADUATION_LAMPORTS | `85,000,000,000` | Protocol constant | Verify vs deployed program |
| Bps MAX | `10,000` | Type invariant | 100% cap |
| ComputeUnits MAX | `1,400,000` | Protocol limit | Solana runtime cap (C-16 UNVERIFIED) |

---

## B. OPERATIONAL CONFIGURATION

Move into typed configuration. Every field needs: explicit type, unit, validation, documented meaning, safe operational range, env override support.

### B.1 Risk limits (arb-risk RiskLimits)
| Field | Type | Unit | Current default | Safe range | Env override |
|-------|------|------|-----------------|------------|--------------|
| max_trade_size_usd | f64 | USD | 25.0 | >0, finite | MAX_TRADE_SIZE_USD |
| min_net_profit_abs | u64 (Lamports) | lamports | 50,000 | >0 | — |
| min_net_profit_bps | u64 (Bps) | bps | 8 | 0..=10_000 | — |
| max_price_impact_bps | u64 (Bps) | bps | 40 | 0..=10_000 | — |
| max_state_age_slots | u64 (Slots) | slots | 2 | >0 | — |
| max_tip_share_bps | u64 (Bps) | bps | 8,500 | 0..=10_000 | — |
| max_tip_lamports | u64 (Lamports) | lamports | 2,000,000 | >0 | — |
| max_daily_loss_usd | f64 | USD | 5.0 | >0 | — |
| max_hourly_loss_usd | f64 | USD | 2.0 | >0, ≤ daily | — |
| min_landing_rate | f64 | ratio | 0.02 | 0.0..=1.0 | — |
| max_inflight_bundles | usize | count | 2 | >0, bounded | — |
| max_tip_spend_ratio | f64 | ratio | 0.7 | 0.0..=1.0 | — |
| max_orders_per_hour | usize | count/hr | 120 | >0 | — |
| max_consecutive_failures | u64 | count | 8 | >0 | — |
| cooldown_slots | u64 (Slots) | slots | 75 | >0 | — |

### B.2 Jito / tip config
| Field | Type | Unit | Current default | Safe range | Env override |
|-------|------|------|-----------------|------------|--------------|
| tip_floor_percentile | enum | p50/p75/p90/p99 | p50 | — | — |
| tip_share_bps | u64 (Bps) | bps | 5,500 | ≤ max_tip_share_bps | — |
| max_tip_lamports | u64 | lamports | 2,000,000 | >0 | — |
| max_tip_share_bps | u64 (Bps) | bps | 7,000 | 0..=10_000 | — |
| min_keep_lamports | u64 | lamports | 10,000 | >0 | — |
| keep_share | f64 | ratio | 0.60 | 0.0..=1.0 | — |
| ladder_growth | f64 | multiplier | 1.6 | >1.0 | — |
| tip_floor_max_age_ms | u64 | ms | 5,000 | >0 | — |
| tip_floor_poll_ms | u64 | ms | 1,000 | >0, ≥1000 (1 rps) | — |
| leader_gate_enabled | bool | — | true | — | — |
| ledger_path | String | path | data/adaptive_ledger.json | — | — |
| leader_refresh_secs | u64 | secs | 60 | >0 | — |

### B.3 Strategy config
| Field | Type | Unit | Current default | Safe range | Env override |
|-------|------|------|-----------------|------------|--------------|
| base_tokens | Vec<String> | mint addresses | 32 tokens | non-empty | — |
| min_net_profit_abs | u64 | lamports | 50,000 | >0 | — |
| min_net_profit_bps | u64 | bps | 5 | 0..=10_000 | — |
| max_price_impact_bps | u64 | bps | 40 | 0..=10_000 | — |
| max_state_age_slots | u64 | slots | 2 | >0 | — |
| graph_max_age_slots | Option<u64> | slots | 8 | ≥ max_state_age_slots | — |
| use_flash_loan | bool | — | true | — | — |
| max_pools | usize | count | 1,000 | >0 | — |
| discovery_refresh_secs | u64 | secs | 15 | >0 | — |
| discovery_min_tvl_usd | f64 | USD | 25,000.0 | ≥0 | — |

### B.4 Chronos config
| Field | Type | Unit | Current default | Safe range | Env override |
|-------|------|------|-----------------|------------|--------------|
| enabled | bool | — | true | — | — |
| decay_lambda_per_sec | f64 | /sec | 3.0 | ≥0 | CHRONOS_DECAY_LAMBDA_PER_SEC |
| max_age_ms | u64 | ms | 400 | 100..=5,000 | CHRONOS_MAX_AGE_MS |
| min_ev_usd | f64 | USD | 0.05 | ≥0 | CHRONOS_MIN_EV_USD |
| failed_attempt_cost_lamports | u64 | lamports | 10,000 | ≥0 | — |
| beta_risk_factor | f64 | factor | 0.0005 | ≥0 | — |

### B.5 Other operational
| Field | Type | Unit | Current default | Safe range | Env override |
|-------|------|------|-----------------|------------|--------------|
| Jupiter timeout_ms | u64 | ms | 400 | >0 | — |
| min_sol_balance_lamports | u64 | lamports | 0 | ≥0 | — |
| Safety floor lamports | u64 | lamports | 50,000,000 | >0 | — |
| Sweep threshold lamports | u64 | lamports | 250,000,000 | > safety floor | — |
| Sweep fee reserve lamports | u64 | lamports | 10,000 | ≥0 | — |
| Blockhash cache TTL | u64 | ms | 3,000 | >0 | — |
| Metrics listen | SocketAddr | host:port | 127.0.0.1:9100 | — | — |
| Admin listen | SocketAddr | host:port | 127.0.0.1:9101 | — | — |

---

## C. DYNAMIC MARKET & EXECUTION PARAMETERS

These must NOT remain static. They must be observed from live data with freshness and validity checks.

| Parameter | Current static value | Problem | Dynamic replacement | Freshness source |
|-----------|---------------------|---------|---------------------|------------------|
| DLMM safety factor | 92% (dlmm.rs:305) | Masks missing bin coverage; systematic underquoting | Remove; use actual bin data + exact fee calc; reject incomplete quotes | Meteora SDK on-chain state |
| MAX_FEE_RATE | 10% (dlmm.rs) | May not reflect all DEX fee schedules | Read on-chain fee params; typed error if missing/invalid | Meteora pool params |
| PoolState::fee_bps fallback | unwrap_or(10_000) = 100% | Silently caps invalid fees | Return None/Err; never cap | On-chain pool state |
| Base priority fee | 1,000 µ-lamports | Static guess; not market-aware | Recent fee-market observations; percentile CU pricing | Solana fee market |
| CU estimates per protocol | 65k/45k/55k/50k/60k/40k/48k | Static guesses | Transaction simulation + historical CU consumption | Simulation |
| Tip floor max age | 5,000 ms | Universal timeout | Per-source freshness policies | Tip oracle |
| Tip floor URL | hardcoded | Configurable | Configurable URL; approved schemes | Jito API |
| DLMM fee denominator | 100,000,000,000 | Static | Read on-chain parameters | Meteora SDK |
| Jupiter base URL | hardcoded | Configurable | Configurable URL | Jupiter API |

### C.1 DLMM 92% safety factor — disposition required (P1)
`scale_dlmm_three_hop_input` applies `(bottleneck_in * 92) / 100` at dlmm.rs:305. This is a static economic assumption that:
- May mask missing bin coverage (uses only active-bin max_in, ignores full bin depth)
- Creates systematic underquoting (8% haircut with no market basis)
- Causes incorrect route profitability (scaled profit = gross * 0.92)
- Interacts incorrectly with fees and slippage (linear scaling ignores fee curve)

**Required:** Compare against official Meteora SDK quote results for identical bin state. If unjustified, remove the approximation and reject incomplete quotes instead.

---

## D. SECURITY SECRETS

Never hardcode. Implement secret provider abstraction.

| Secret | Current location | Classification | Required control |
|--------|-----------------|----------------|------------------|
| Admin bearer token | config.toml `token_env = "SENTINEL_ADMIN_KEY"` | Secret | Secret provider; env-based local dev; constant-time compare (existing); rotation; audit trail; no plaintext in config |
| Helius API key | config.toml `primary_token_env = "HELIUS_API_KEY"` | Secret | Secret provider; no plaintext |
| Shyft API key | config.toml `failover_token_env = "SHYFT_API_KEY"` | Secret | Secret provider; no plaintext |
| Jupiter API key | config.toml `api_key_env = "JUPITER_API_KEY"` | Secret | Secret provider; no plaintext |
| Keypair file | config.toml `keypair_path = "solana_keypair.json"` | Secret (key material) | Key file permission checks (0600); secret provider; no secret in Debug output |
| .env contents | `.env` (modified, untracked) | Secret (likely credentials) | Ensure never committed; .env.example only; secret scan |

**Security rules:**
- No private keys, admin tokens, RPC API keys, webhook secrets, DB credentials, or external API tokens in config.toml.
- No secret values in Debug output.
- No secrets in generated reports.
- Redaction in tracing and error messages.
- Explicit failure if required credentials are missing.
- Production secret manager integration where available.

---

## Cross-Category Consistency Rules (to enforce)

1. **No operator config overrides protocol invariants.** Risk config cannot change a program ID or discriminator.
2. **No silent fallbacks for missing economic values.** Missing blockhash → error (already fixed). Missing fee → Err (not 10% cap).
3. **No guessed defaults.** Missing SOL price → None (already respected). Missing tip floor → absent, not reused.
4. **Single authoritative source.** Risk limits live in one place (arb-risk RiskLimits); TOML defaults must match.
5. **Per-source freshness.** Pool snapshots, SOL price, Jito tips, priority fees, blockhashes, landing statistics each have independent freshness policies — no universal timeout.
