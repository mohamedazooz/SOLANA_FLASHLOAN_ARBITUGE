# Hardcoded Values Inventory

**Repository:** Solana Arbitrage Bot  
**Audit date:** Mon Nov 16 2026  
**Scope:** `crates/`, `webhook-service/`, root config files  
**Method:** grep + manual source review (AST-aware where useful); each entry tagged with file:line, existing value, purpose, classification, risk, recommended action, verification source, and required tests.

---

## A. IMMUTABLE PROTOCOL CONSTANTS (arb-constants / protocol modules)

These are protocol invariants and must NOT be moved into ordinary operator configuration. They are centralized in `arb-constants` with one authoritative source per value.

| # | File:line | Existing value | Purpose | Classification | Risk | Recommended action | Verification source | Required tests |
|---|-----------|----------------|---------|----------------|------|--------------------|---------------------|----------------|
| A1 | `crates/arb-constants/src/lib.rs:16` | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | SPL Token Program ID | Immutable protocol constant | Low (canonical, well-known) | Keep; add on-chain ownership test | Solana SPL Token program docs | Valid Pubkey parse test |
| A2 | `crates/arb-constants/src/lib.rs:18` | `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` | Token-2022 Program ID | Immutable protocol constant | Low | Keep | SPL Token-2022 docs | Valid Pubkey parse test |
| A3 | `crates/arb-constants/src/lib.rs:20` | `pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ` | Pump Fees program ID | Immutable protocol constant | Medium (Pump program-specific) | Keep; verify against Pump.fun SDK | Pump.fun program docs | Valid Pubkey parse test |
| A4 | `crates/arb-constants/src/lib.rs:22` | `[143,52,146,187,219,123,76,155]` | Pump Fees FeeConfig Anchor discriminator | Protocol serialization structure | Medium | Keep; versioned decoder | Pump.fun IDL | Discriminator byte test |
| A5 | `crates/arb-constants/src/lib.rs:24` | `[149,8,156,202,160,252,176,217]` | PumpSwap GlobalConfig Anchor discriminator | Protocol serialization structure | Medium | Keep | PumpSwap IDL | Discriminator byte test |
| A6 | `crates/arb-constants/src/lib.rs:26` | `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | Associated Token Program ID | Immutable protocol constant | Low | Keep | SPL docs | Valid Pubkey parse test |
| A7 | `crates/arb-constants/src/lib.rs:28` | `AddressLookupTab1e1111111111111111111111111` | Address Lookup Table Program ID | Immutable protocol constant | Low | Keep | Solana docs | Valid Pubkey parse test |
| A8 | `crates/arb-constants/src/lib.rs:30` | `Sysvar1nstructions1111111111111111111111111` | Sysvar Instructions account | Immutable protocol constant | Low | Keep | Solana docs | Valid Pubkey parse test |
| A9 | `crates/arb-constants/src/lib.rs:32` | `SysvarRent111111111111111111111111111111111` | Sysvar Rent account | Immutable protocol constant | Low | Keep | Solana docs | Valid Pubkey parse test |
| A10 | `crates/arb-constants/src/lib.rs:34` | `[0;32]` | System Program (zero address) | Immutable protocol constant | Low | Keep | Solana docs | Zero-pubkey test |
| A11 | `crates/arb-constants/src/lib.rs:36-39` | `[3,6,70,111,...]` (32 bytes) | Compute Budget Program ID | Immutable protocol constant | Low | Keep | Solana docs | Byte-array test |
| A12 | `crates/arb-constants/src/lib.rs:46` | `So11111111111111111111111111111111111111112` | WSOL mint | Immutable protocol constant | Low | Keep | Solana WSOL docs | Valid Pubkey parse test |
| A13 | `crates/arb-constants/src/lib.rs:48` | `9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP` | Token-2022 native SOL mint | Immutable protocol constant | Medium | Keep; verify | Token-2022 docs | Valid Pubkey parse test |
| A14 | `crates/arb-constants/src/lib.rs:50` | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` | USDC mint | Immutable protocol constant | Low | Keep | USDC mainnet docs | Valid Pubkey parse test |
| A15 | `crates/arb-constants/src/lib.rs:57-66` | 8 Jito tip accounts | Jito tip accounts (Mainnet) | Protocol-specific | Medium | Keep; round-robin + distinctness test | Jito docs | Distinctness + valid parse test (existing) |
| A16 | `crates/arb-constants/src/lib.rs:110-113` | 32-byte `JITODONTFRONT` marker | Jito sandwich-protection marker | Protocol-specific | Medium | Keep; prefix test | Jito block engine docs | Prefix `jitodontfront` test (existing) |
| A17 | `crates/arb-constants/src/lib.rs:125` | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Raydium AMM v4 Program ID | Immutable protocol constant | Low | Keep | Raydium docs | Valid Pubkey parse test |
| A18 | `crates/arb-constants/src/lib.rs:127` | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Raydium CPMM Program ID | Immutable protocol constant | Low | Keep | Raydium docs | Valid Pubkey parse test |
| A19 | `crates/arb-constants/src/lib.rs:129` | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Raydium CLMM Program ID | Immutable protocol constant | Low | Keep | Raydium docs | Valid Pubkey parse test |
| A20 | `crates/arb-constants/src/lib.rs:131` | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | Orca Whirlpool Program ID | Immutable protocol constant | Low | Keep | Orca docs | Valid Pubkey parse test |
| A21 | `crates/arb-constants/src/lib.rs:133` | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Meteora DLMM Program ID | Immutable protocol constant | Low | Keep | Meteora docs | Valid Pubkey parse test |
| A22 | `crates/arb-constants/src/lib.rs:135` | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Pump.fun Bonding Curve Program ID | Immutable protocol constant | Medium | Keep | Pump.fun docs | Valid Pubkey parse test |
| A23 | `crates/arb-constants/src/lib.rs:137` | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | PumpSwap AMM Program ID | Immutable protocol constant | Medium | Keep | PumpSwap docs | Valid Pubkey parse test |
| A24 | `crates/arb-constants/src/lib.rs:139` | `ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw` | PumpSwap GlobalConfig PDA | PDA seed-derived | Medium | Keep; derive-at-runtime test | PumpSwap IDL | PDA derivation test |
| A25 | `crates/arb-constants/src/lib.rs:141` | `[241,154,109,4,17,177,109,188]` | PumpSwap Pool discriminator | Protocol serialization | Medium | Keep | PumpSwap IDL | Byte test |
| A26 | `crates/arb-constants/src/lib.rs:143` | `CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM` | Pump.fun Fee Recipient | Protocol constant | Medium | Keep | Pump.fun docs | Valid Pubkey parse test |
| A27 | `crates/arb-constants/src/lib.rs:145` | `[23,183,248,55,96,216,172,96]` | Pump.fun BondingCurve discriminator | Protocol serialization | Medium | Keep | Pump.fun IDL | Byte test |
| A28 | `crates/arb-constants/src/lib.rs:147` | `[102,6,61,18,1,218,235,234]` | Pump.fun buy ix discriminator | Protocol serialization | Medium | Keep | Pump.fun IDL | Byte test |
| A29 | `crates/arb-constants/src/lib.rs:149` | `[51,230,133,164,1,127,131,173]` | Pump.fun sell ix discriminator | Protocol serialization | Medium | Keep | Pump.fun IDL | Byte test |
| A30 | `crates/arb-constants/src/lib.rs:151` | `b"global"` | Pump.fun global PDA seed | Protocol seed | Medium | Keep | Pump.fun IDL | Seed byte test |
| A31 | `crates/arb-constants/src/lib.rs:153` | `b"bonding-curve"` | Pump.fun bonding curve PDA seed | Protocol seed | Medium | Keep | Pump.fun IDL | Seed byte test |
| A32 | `crates/arb-constants/src/lib.rs:155` | `b"__event_authority"` | Pump.fun event authority PDA seed | Protocol seed | Medium | Keep | Pump.fun IDL | Seed byte test |
| A33 | `crates/arb-constants/src/lib.rs:162` | `58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2` | Raydium SOL-USDC pool | Mainnet pool address | Medium | Verify on-chain ownership | Raydium mainnet | Valid Pubkey + pool state test |
| A34 | `crates/arb-constants/src/lib.rs:164` | `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz` | Raydium SOL vault | Mainnet vault | Medium | **VERIFY vs pool state** (P0 C-02) | On-chain account | Regression test vs arb-types |
| A35 | `crates/arb-constants/src/lib.rs:166` | `HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz` | Raydium USDC vault | Mainnet vault | Medium | Verify on-chain ownership | On-chain account | Regression test |
| A36 | `crates/arb-constants/src/lib.rs:173` | `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` | Orca SOL-USDC pool | Mainnet pool address | Medium | Verify | Orca mainnet | Valid Pubkey test |
| A37 | `crates/arb-constants/src/lib.rs:175` | `EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9` | Orca SOL vault | Mainnet vault | Medium | Verify | On-chain account | Regression test |
| A38 | `crates/arb-constants/src/lib.rs:177` | `2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP` | Orca USDC vault | Mainnet vault | Medium | Verify | On-chain account | Regression test |
| A39 | `crates/arb-constants/src/lib.rs:184` | `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6` | Meteora SOL-USDC pool | Mainnet DLMM pool | Medium | Verify bin_step=4 claim | Meteora mainnet | Regression test |
| A40 | `crates/arb-constants/src/lib.rs:186` | `EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o` | Meteora SOL reserve | Mainnet reserve | Medium | Verify | On-chain account | Regression test |
| A41 | `crates/arb-constants/src/lib.rs:188` | `CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz` | Meteora USDC reserve | Mainnet reserve | Medium | Verify | On-chain account | Regression test |
| A42 | `crates/arb-constants/src/lib.rs:190` | `4` (u16) | Meteora SOL-USDC bin_step bps | Protocol constant | Medium | Verify vs Meteora SDK | Meteora SDK docs | Bin-step test |
| A43 | `crates/arb-constants/src/lib.rs:198` | `70` (i64) | Meteora bins per BinArray | Protocol constant | **High** — wrong value targets wrong BinArrays | Keep at 70 (verified vs `lb_clmm::state::bin_array::BINS_PER_ARRAY`); sync with ingestion decoder | Meteora SDK `BINS_PER_ARRAY=70` | Sync test with decoder |
| A44 | `crates/arb-constants/src/lib.rs:200` | `[65,75,63,76,235,91,91,136]` | Meteora swap2 discriminator | Protocol serialization | Medium | Keep | Meteora IDL | Byte test |
| A45 | `crates/arb-constants/src/lib.rs:207` | `79RbHpxoYSB3EJnN8aGLouuRrDJVGWLzxM3r1qVKsKJ7` | Raydium CPMM SOL-USDC pool | Mainnet pool | Medium | Verify | On-chain | Regression test |
| A46 | `crates/arb-constants/src/lib.rs:209` | `DJgKsuDtTVvT1VqjC1vHTu6LHjq5Z6KRX1jaLKZJ3VwW` | Raydium CPMM SOL vault | Mainnet vault | Medium | Verify | On-chain | Regression test |
| A47 | `crates/arb-constants/src/lib.rs:211` | `HYbZV1vHH8rDvn3a2uPtR8nCnBkL5xXwK7yP1uJ9nC2L` | Raydium CPMM USDC vault | Mainnet vault | Medium | Verify | On-chain | Regression test |
| A48 | `crates/arb-constants/src/lib.rs:213` | `[143,190,90,218,196,30,51,222]` | Raydium CPMM swap_base_input discriminator | Protocol serialization | Medium | Keep | Raydium IDL | Byte test |
| A49 | `crates/arb-constants/src/lib.rs:216` | `b"vault_and_lp_mint_auth_seed"` | Raydium CPMM authority seed | Protocol seed | Medium | Keep | Raydium IDL | Seed test |
| A50 | `crates/arb-constants/src/lib.rs:218` | `b"pool_vault"` | Raydium CPMM pool vault seed | Protocol seed | Medium | Keep | Raydium IDL | Seed test |
| A51 | `crates/arb-constants/src/lib.rs:220` | `b"observation"` | Raydium CPMM observation seed | Protocol seed | Medium | Keep | Raydium IDL | Seed test |
| A52 | `crates/arb-constants/src/lib.rs:227` | `[248,198,158,145,225,117,135,200]` | Raydium CLMM swap discriminator | Protocol serialization | Medium | Keep | Raydium IDL | Byte test |
| A53 | `crates/arb-constants/src/lib.rs:229` | `b"tick_array"` | Raydium CLMM tick array seed | Protocol seed | Medium | Keep | Raydium IDL | Seed test |
| A54 | `crates/arb-constants/src/lib.rs:231` | `b"observation"` | Raydium CLMM observation seed | Protocol seed | Medium | Keep | Raydium IDL | Seed test |
| A55 | `crates/arb-constants/src/lib.rs:238` | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Kamino klend Program ID | Protocol constant | Medium | Verify | Kamino docs | Valid Pubkey test |
| A56 | `crates/arb-constants/src/lib.rs:240` | `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF` | Kamino Main Market | Protocol constant | Medium | Verify | Kamino mainnet | Regression test |
| A57 | `crates/arb-constants/src/lib.rs:242` | `9DrvZvyWh1HuAoZxvYWMvkf2XCzryCpGgHqrMjyDWpmo` | Kamino Market Authority PDA | PDA-derived | Medium | Verify derivation | Kamino IDL | PDA derivation test |
| A58 | `crates/arb-constants/src/lib.rs:244` | `d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q` | Kamino WSOL Reserve | Protocol constant | Medium | Verify | Kamino mainnet | Regression test |
| A59 | `crates/arb-constants/src/lib.rs:246` | `GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U` | Kamino WSOL Supply Vault | Protocol constant | Medium | Verify | Kamino mainnet | Regression test |
| A60 | `crates/arb-constants/src/lib.rs:248` | `3JNof8s453bwG5UqiXBLJc77NRQXezYYEBbk3fqnoKph` | Kamino WSOL Fee Vault | Protocol constant | Medium | Verify | Kamino mainnet | Regression test |
| A61 | `crates/arb-constants/src/lib.rs:250` | `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59` | Kamino USDC Reserve | Protocol constant | **High — P0 C-01** verify which address is correct | **Verify on-chain** (mint identity, vault authority, pool state). Do NOT trust audit doc. | Kamino mainnet + on-chain | Regression test vs verified_config |
| A62 | `crates/arb-constants/src/lib.rs:253` | `[135,231,52,167,7,52,212,193]` | Kamino flash_borrow discriminator | Protocol serialization | Medium | Keep | Kamino IDL | Byte test |
| A63 | `crates/arb-constants/src/lib.rs:255` | `[185,117,0,203,96,245,180,186]` | Kamino flash_repay discriminator | Protocol serialization | Medium | Keep | Kamino IDL | Byte test |
| A64 | `crates/arb-constants/src/lib.rs:257` | `b"lma"` | Lending market authority seed | Protocol seed | Medium | Keep | Kamino IDL | Seed test |
| A65 | `crates/arb-constants/src/lib.rs:268` | `10` (u64 bps) | Default hop slippage | Protocol/economic default | Medium | Keep as default; document | Protocol docs | Slippage test |
| A66 | `crates/arb-constants/src/lib.rs:275` | `260_000` | CU limit 2-hop | Compute budget limit | Medium | Keep; respect runtime max | Solana runtime docs | CU limit test |
| A67 | `crates/arb-constants/src/lib.rs:277` | `320_000` | CU limit 3-hop | Compute budget limit | Medium | Keep | Solana runtime docs | CU limit test |
| A68 | `crates/arb-constants/src/lib.rs:279` | `350_000` | Max CU limit | Compute budget limit | Medium | Keep ≤ Solana runtime cap | Solana runtime docs | Cap test |
| A69 | `crates/arb-constants/src/lib.rs:281` | `15` | CU buffer % | Operational default | Low | Move to config (operational) | N/A | Config test |
| A70 | `crates/arb-constants/src/lib.rs:283` | `1232` | Solana packet MTU | Protocol limit | Low | Keep | Solana docs | MTU test |
| A71 | `crates/arb-constants/src/lib.rs:285` | `1_000` | Base priority fee µ-lamports | Economic default | Low | Make dynamic (fee market) | Solana fee docs | Fee test |
| A72 | `crates/arb-constants/src/lib.rs:297` | `85_000_000_000` | Pump.fun graduation lamports | Protocol constant | Medium | Verify against deployed program | Pump.fun docs | Graduation test |

## B. OPERATIONAL CONFIGURATION (config.toml / arb-risk / arb-bot/config)

These are legitimate operational parameters to move into typed configuration. Each must have explicit type, unit, validation, documented meaning, safe range, env override.

| # | File:line | Existing value | Purpose | Classification | Risk | Recommended action | Verification source | Required tests |
|---|-----------|----------------|---------|----------------|------|--------------------|---------------------|----------------|
| B1 | `crates/arb-risk/src/lib.rs:230` | `25.0` | max_trade_size_usd default | Operational config | Medium | Move to typed RiskConfig with validation >0, finite | Operator policy | Config validation test |
| B2 | `crates/arb-risk/src/lib.rs:232` | `arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS` (50,000) | min_net_profit_abs | Operational config | Medium | Typed Lamports newtype; validation >0 | Strategy docs | Min profit test |
| B3 | `crates/arb-risk/src/lib.rs:233` | `8` | min_net_profit_bps | Operational config | Medium | Typed Bps; validation 0..=10_000 | Strategy docs | Bps validation test |
| B4 | `crates/arb-risk/src/lib.rs:234` | `40` | max_price_impact_bps | Operational config | Medium | Typed Bps; validation ≤10_000 | Strategy docs | Price impact test |
| B5 | `crates/arb-risk/src/lib.rs:235` | `2` | max_state_age_slots | Operational config | Medium | Typed Slots; validation >0 | Execution docs | State age test |
| B6 | `crates/arb-risk/src/lib.rs:235` | `arb_constants::DEFAULT_JITO_MAX_TIP_SHARE_BPS` (8,500) | max_tip_share_bps | Operational config | Medium | Typed Bps; validation 0..=10_000 | Jito docs | Tip share test |
| B7 | `crates/arb-risk/src/lib.rs:236` | `2_000_000` | max_tip_lamports | Operational config | Medium | Typed Lamports; validation >0 | Jito docs | Tip cap test |
| B8 | `crates/arb-risk/src/lib.rs:237` | `5.0` | max_daily_loss_usd | Operational config | High | Typed UsdMicros; validation >0 | Risk policy | Loss limit test |
| B9 | `crates/arb-risk/src/lib.rs:238` | `2.0` | max_hourly_loss_usd | Operational config | High | Typed UsdMicros; validation >0 AND ≤ daily | Risk policy | Hourly ≤ daily test |
| B10 | `crates/arb-risk/src/lib.rs:240` | `0.02` | min_landing_rate | Operational config | Medium | Typed ratio; validation 0.0..=1.0 | Landing stats | Landing rate test |
| B11 | `crates/arb-risk/src/lib.rs:241` | `2` | max_inflight_bundles | Operational config | Medium | Typed usize; bounded | Execution docs | Inflight test |
| B12 | `crates/arb-risk/src/lib.rs:242` | `0.7` | max_tip_spend_ratio | Operational config | Medium | Typed ratio; validation 0.0..=1.0 | Jito docs | Ratio test |
| B13 | `crates/arb-risk/src/lib.rs:243` | `120` | max_orders_per_hour | Operational config | Medium | Typed usize; validation >0 | Risk policy | Velocity test |
| B14 | `crates/arb-bot/src/main.rs:322` | `10` | default_max_orders_per_hour (config TOML default) | Operational config | Medium | Unify with arb-risk 120; remove contradiction | Risk policy | Default consistency test |
| B15 | `crates/arb-bot/src/main.rs:327` | `75` | cooldown_slots | Operational config | Medium | Typed Slots; validation >0 | Circuit breaker docs | Cooldown test |
| B16 | `crates/arb-bot/src/main.rs:253` | `tip_share_bps` (from TOML) | Jito tip share | Operational config | Medium | Typed Bps; validation ≤ max_tip_share_bps | Jito docs | Tip share test |
| B17 | `crates/arb-bot/src/main.rs:254` | `max_tip_lamports` (from TOML) | Max Jito tip | Operational config | Medium | Typed Lamports; validation >0 | Jito docs | Tip cap test |
| B18 | `config.toml:23` | `tip_floor_percentile = "p50"` | Tip floor percentile | Operational config | Medium | Typed enum (p50/p75/p90/p99); validation | Jito API | Percentile test |
| B19 | `config.toml:24` | `tip_share_bps = 5500` | Tip share bps (config) | Operational config | Medium | Must not exceed max_tip_share_bps (8,500) | Jito docs | Cross-field validation test |
| B20 | `config.toml:25` | `max_tip_lamports = 2_000_000` | Max tip lamports (config) | Operational config | Medium | Typed Lamports | Jito docs | Tip cap test |
| B21 | `config.toml:26` | `max_tip_share_bps = 7000` | Max tip share bps (config) | Operational config | Medium | Typed Bps 0..=10_000 | Jito docs | Tip share test |
| B22 | `config.toml:27` | `min_keep_lamports = 10_000` | Min keep lamports | Operational config | Medium | Typed Lamports; validation >0 | Jito docs | Keep test |
| B23 | `config.toml:30` | `keep_share = 0.60` | Keep share fraction | Operational config | Medium | Typed ratio 0.0..=1.0 | Jito docs | Ratio test |
| B24 | `config.toml:31` | `ladder_growth = 1.6` | Ladder growth multiplier | Operational config | Medium | Typed f64; validation >1.0 | Jito docs | Ladder test |
| B25 | `config.toml:32` | `tip_floor_max_age_ms = 5000` | Tip floor max age ms | Operational config | Medium | Typed Milliseconds; validation >0 | Tip oracle | Freshness test |
| B26 | `config.toml:33` | `tip_floor_poll_ms = 1000` | Tip floor poll ms | Operational config | Medium | Typed Milliseconds; clamped to 1 rps | Tip oracle API | Poll test |
| B27 | `config.toml:34` | `leader_gate_enabled = true` | Leader gate | Operational config | Medium | Typed bool | Jito docs | Gate test |
| B28 | `config.toml:35` | `ledger_path = "data/adaptive_ledger.json"` | Ledger path | Operational config | Low | Typed path | N/A | Path test |
| B29 | `config.toml:36` | `leader_refresh_secs = 60` | Leader refresh secs | Operational config | Medium | Typed seconds; validation >0 | Jito docs | Refresh test |
| B30 | `config.toml:41` | `timeout_ms = 400` | Jupiter timeout ms | Operational config | Medium | Typed Milliseconds; validation >0 | Jupiter API | Timeout test |
| B31 | `config.toml:44` | `keypair_path = "solana_keypair.json"` | Keypair path | Operational config | Low | Typed path; secret handling | Security docs | Path test |
| B32 | `config.toml:45` | `cold_wallet_pubkey = "2Cby3x..."` | Cold wallet pubkey | Operational config | Low | Typed Pubkey; validation | Security docs | Valid Pubkey test |
| B33 | `config.toml:46` | `min_sol_balance_lamports = 0` | Min SOL balance reserve | Operational config | High | Typed Lamports; validation ≥0 | Safety policy | Balance test |
| B34 | `config.toml:49-80` | base_tokens list | Base tokens | Operational config | Medium | Typed Vec<Pubkey>; validation non-empty | Strategy docs | Token list test |
| B35 | `config.toml:82` | `min_net_profit_abs = 50000` | Min net profit lamports | Operational config | Medium | Typed Lamports; validation >0 | Strategy docs | Min profit test |
| B36 | `config.toml:83` | `min_net_profit_bps = 5` | Min net profit bps | Operational config | Medium | Typed Bps 0..=10_000 | Strategy docs | Bps test |
| B37 | `config.toml:84` | `max_price_impact_bps = 40` | Max price impact bps | Operational config | Medium | Typed Bps 0..=10_000 | Strategy docs | Price impact test |
| B38 | `config.toml:85` | `max_state_age_slots = 2` | Max state age slots | Operational config | Medium | Typed Slots; validation >0 | Execution docs | State age test |
| B39 | `config.toml:86` | `graph_max_age_slots = 8` | Graph max age slots | Operational config | Medium | Typed Slots; ≥ max_state_age_slots | Execution docs | Graph age test |
| B40 | `config.toml:88` | `max_pools = 1000` | Max pools | Operational config | Medium | Typed usize; validation >0 | Discovery docs | Pool cap test |
| B41 | `config.toml:89` | `discovery_refresh_secs = 15` | Discovery refresh secs | Operational config | Medium | Typed seconds; validation >0 | Discovery docs | Refresh test |
| B42 | `config.toml:90` | `discovery_min_tvl_usd = 25000.0` | Min TVL USD | Operational config | Medium | Typed UsdMicros; validation ≥0 | Discovery docs | TVL test |
| B43 | `config.toml:94` | `max_consecutive_failures = 8` | Circuit breaker threshold | Operational config | High | Typed usize; validation >0 | Breaker docs | Breaker test |
| B44 | `config.toml:98` | `max_attempts_per_slot = 3` | Max attempts per slot | Operational config | Medium | Typed usize; validation >0 | Execution docs | Attempts test |
| B45 | `config.toml:100` | `max_inflight_bundles = 2` | Max inflight bundles | Operational config | Medium | Typed usize; bounded | Execution docs | Inflight test |
| B46 | `config.toml:115` | `decay_lambda_per_sec = 3.0` | Chronos decay lambda | Operational config | Medium | Typed f64; validation ≥0 | Chronos docs | Lambda test |
| B47 | `config.toml:116` | `max_age_ms = 400` | Chronos max age ms | Operational config | High | Typed Milliseconds; validation 100..=5000 | Chronos docs | Age test |
| B48 | `config.toml:117` | `min_ev_usd = 0.05` | Chronos min EV USD | Operational config | High | Typed UsdMicros; validation ≥0 | Chronos docs | EV test |
| B49 | `config.toml:118` | `failed_attempt_cost_lamports = 10000` | Failed attempt cost | Operational config | Medium | Typed Lamports; validation ≥0 | Chronos docs | Cost test |
| B50 | `config.toml:119` | `beta_risk_factor = 0.0005` | Chronos beta risk | Operational config | Medium | Typed f64; validation ≥0 | Chronos docs | Beta test |
| B51 | `config.toml:107` | `listen = "127.0.0.1:9100"` | Metrics listen | Operational config | Low | Typed SocketAddr; validation | Metrics docs | Address test |
| B52 | `config.toml:111` | `listen = "127.0.0.1:9101"` | Admin listen | Operational config | Low | Typed SocketAddr; validation | Security docs | Address test |
| B53 | `config.toml:112` | `token_env = "SENTINEL_ADMIN_KEY"` | Admin token env name | Security config | **High** | Secret provider abstraction; no plaintext in config | Security docs | Secret redaction test |
| B54 | `dispatch.rs:124` | `50_000_000` | Safety floor lamports | Safety limit | High | Typed Lamports; config-controlled; fail closed | Safety policy | Floor test |
| B55 | `dispatch.rs:163` | `250_000_000` | Sweep threshold lamports | Operational config | Medium | Typed Lamports; > safety floor | Safety policy | Sweep test |
| B56 | `dispatch.rs:167` | `10_000` | Sweep fee reserve lamports | Operational config | Low | Typed Lamports | Safety policy | Reserve test |
| B57 | `execution.rs:113` | `3` seconds | Blockhash cache TTL | Operational config | Medium | Typed Milliseconds; validation >0 | Solana docs | TTL test |

## C. DYNAMIC MARKET AND EXECUTION PARAMETERS

These must NOT remain static; they must be observed from live data with freshness checks.

| # | File:line | Existing value | Purpose | Classification | Risk | Recommended action | Verification source | Required tests |
|---|-----------|----------------|---------|----------------|------|--------------------|---------------------|----------------|
| C1 | `arb-math/src/dlmm.rs:305` | `92/100` safety factor | DLMM input scaling | **Dynamic — must be removed or justified** | High | Compare vs Meteora SDK; remove if unjustified; reject incomplete quotes | Meteora SDK quotes | Quote parity test |
| C2 | `arb-math/src/dlmm.rs` | `MAX_FEE_RATE = 100_000_000` (10%) | DLMM fee cap | Dynamic — read from chain | Medium | Read on-chain fee params; typed error if invalid | Meteora SDK | Fee decode test |
| C3 | `arb-types` | `fee_bps.unwrap_or(10_000)` | Pool fee fallback | **Dynamic — fail closed** | High | Return None/Err; never cap at 100% | On-chain pool state | Fee validation test |
| C4 | `arb-constants/src/lib.rs:285` | `1_000` µ-lamports | Base priority fee | Dynamic — fee market | Medium | Use recent fee-market observations | Solana fee market | Percentile CU test |
| C5 | `arb-engine/src/adapters.rs:77-87` | CU estimates per protocol (65k,45k,55k,50k,60k,40k,48k) | CU estimates | Dynamic — simulation-based | Medium | Use transaction simulation + historical CU | Solana runtime | CU estimate test |
| C6 | `arb-land/tip_oracle.rs:32` | `DEFAULT_MAX_AGE_MS = 5000` | Tip freshness | Dynamic | Medium | Per-source freshness policy; configurable | Tip oracle API | Freshness test |
| C7 | `arb-land/tip_oracle.rs:26-28` | `TIP_FLOOR_URL` / `TIP_WEBSOCKET_URL` | Tip floor endpoints | Dynamic | Medium | Configurable URLs; approved schemes only | Jito API | URL validation test |
| C8 | `arb-math/src/dlmm.rs` | Fee denominator `100_000_000_000` | DLMM fee denom | Dynamic — read from chain | Medium | Read on-chain parameters | Meteora SDK | Fee denom test |
| C9 | `config.toml:39` | `base_url = "https://api.jup.ag/swap/v2"` | Jupiter base URL | Dynamic source | Medium | Configurable URL; approved scheme | Jupiter API | URL test |

## D. SECURITY SECRETS

| # | File:line | Existing value | Purpose | Classification | Risk | Recommended action | Verification source | Required tests |
|---|-----------|----------------|---------|----------------|------|--------------------|---------------------|----------------|
| D1 | `config.toml:112` | `token_env = "SENTINEL_ADMIN_KEY"` | Admin bearer token env name | Secret | **High** | Secret provider abstraction; env-based local dev; no plaintext in config | Security policy | Secret redaction test |
| D2 | `config.toml:10-12` | `primary_token_env = "HELIUS_API_KEY"`, `failover_token_env = "SHYFT_API_KEY"` | RPC API key env names | Secret | High | Secret provider abstraction; no plaintext | Security policy | Secret redaction test |
| D3 | `config.toml:40` | `api_key_env = "JUPITER_API_KEY"` | Jupiter API key env name | Secret | High | Secret provider abstraction | Security policy | Secret redaction test |
| D4 | `config.toml:44` | `keypair_path = "solana_keypair.json"` | Keypair file path | Secret (key material) | **High** | Key file permission checks (0600); secret provider | Security policy | Permission check test |
| D5 | `.env` (present, modified, untracked content) | Unknown | Likely contains credentials | Secret | **High** | Ensure never committed; .env.example only | Security policy | Secret scan test |

---

## Unverified assumptions (Rule 14)

- All mainnet pool/vault addresses (A33-A61) are UNVERIFIED against on-chain state. They must be verified via mint identity, vault authority, and pool state before being treated as authoritative.
- Meteora `BINS_PER_BIN_ARRAY=70` (A43) is asserted to match `lb_clmm` but not independently re-verified in this session.
- Kamino reserve offsets (A58-A61) verified 2026-11-04 per source comment; re-verification pending if program upgraded.

---

## Session-4 refresh (2026-11-16, commit series ending 4096f14+)

Independent re-scan performed after the security refactor. Deltas against the
inventory above:

### Removed this session (were classified B/G with conflicts)
| Former value | Location | Disposition |
|---|---|---|
| `MAX_VELOCITY_ORDERS_PER_HOUR = 10` | `crates/arb-risk/src/lib.rs` | Deleted (dead + contradicted `RiskLimits::max_orders_per_hour = 120`, which remains the single authoritative value) |
| Fabricated `pool_coin_vault DQyrAcCrDXYZ...5jz` | `crates/arb-types/src/lib.rs` (dead duplicate struct) | Entire dead struct+impl deleted; `arb-constants::RAYDIUM_SOL_VAULT` is the single source of truth (on-chain verified) |

### Added this session (webhook-service operational configuration, class B)
| Key | Default | Unit | Validation | Source |
|---|---|---|---|---|
| `server.rate_limit_requests_per_minute` | 600 | requests/60 s per client | 0 = disabled; otherwise >= 10 (validated, fail-closed) | `webhook-service/src/config.rs` |
| `server.max_event_age_secs` | 0 (disabled) | seconds | u64; enabled guard rejects stale/far-future/missing timestamps | `webhook-service/src/config.rs` |
| `server.auth_token_env` | "" (unset) | env-var NAME | when set, env var MUST exist and be non-empty at load (fail-closed) | `webhook-service/src/config.rs::load_from_file` |
| `server.max_body_bytes` | 1_048_576 | bytes | usize | (added session 3/4) |
| `server.cors_allowed_origins` | [] | origin strings | empty = no CORS headers | (added session 3/4) |

### New code constants (class A/E, in-code by design)
| Value | Location | Class | Justification |
|---|---|---|---|
| Fixed rate-limit window = 60 s | `webhook-service/src/rate_limit.rs` | E (mathematical/operational unit) | "per minute" definition; configurable count around it |
| `MAX_TRACKED_KEYS = 10_000` | `webhook-service/src/rate_limit.rs` | B (memory bound) | prune threshold; bounded-memory guarantee |
| Future-skew allowance = 300 s | `webhook-service/src/lib.rs::event_age_allowed` | B (operational) | clock-skew tolerance for replay guard |
| HMAC-SHA256 + optional `sha256=` prefix | `webhook-service/src/lib.rs::verify_hmac_signature` | A (protocol: RFC 2104 / GitHub-Standard-Webhooks convention) | verified against public webhook-signature conventions |

### Independent scan results (verification of inventory completeness)
- URLs/endpoints in non-test source: Jito block-engine regions (latency_bench,
  configurable per region at runtime), Telegram API (token from env), Binance/
  Coinbase public price tickers (telegram.rs SOL/USD fallback), Kobe validators.
  All documented; none are secrets.
- 124 `Duration::from_secs/from_millis` literals in non-test crate sources —
  operational timeouts; the majority are already wired to config via arb-config;
  remainder are internal poll/tick intervals (class B, low risk, documented in
  constants_classification.md).
- 155 base58 literals outside arb-constants/tests: audited sample — canonical
  protocol addresses (Raydium AMM authority `5Q544...ge4j1`, Serum program
  `srmqP...tPX`) and real mainnet pool fixtures (class F) used as defaults in
  protocol builders; no fabricated addresses remain (guarded by
  `crates/arb-tx/tests/address_regression_guards.rs`).

**Secrets scan:** zero committed secrets (auth tokens resolved via env-var names,
fail-closed; `ServerConfig` Debug redacts `auth_token`; `Secret` type redacts
Debug/Display).
