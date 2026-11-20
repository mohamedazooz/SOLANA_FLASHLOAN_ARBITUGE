> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Best Practices Research — Solana Atomic Arbitrage

> **Scope:** Authoritative sources consulted before/during Phase 12–15 remediation.
> **Principle:** SDK-first, official documentation preferred, version-pinned verification.

---

## 1. Solana Core — Runtime & Transaction Processing

### 1.1 Official Sources
- **Solana Documentation:** https://solana.com/docs (core concepts, RPC, WebSocket)
- **Agave Validator Source:** https://github.com/anza-xyz/agave (runtime, consensus, transaction processing)
- **Solana SDK:** https://github.com/anza-xyz/solana-sdk (client, transaction, instruction building)

### 1.2 Key Findings
| Area | Version Verified | Critical Detail |
|------|------------------|-----------------|
| Transaction V0 (ALT) | solana-sdk 2.2 | 1232-byte packet limit, ALT compresses keys only |
| Transaction V1 | solana-sdk 2.2 | 4096-byte wire limit, 64 inline accounts, **no ALT support**, 0x81 discriminator |
| Compute Budget | Agave 2.2 | `SetComputeUnitLimit` (ix 2), `SetComputeUnitPrice` (ix 3), priority fee = cu_price × cu_limit / 1e6 |
| Blockhash Validity | Agave 2.2 | ~150 slots (≈ 150 blocks × 400ms = 60s); margin 160 slots used in code |
| Account Rent | Agave 2.2 | Rent-exempt minimum = 2 years rent; `sysvar::rent::minimum_balance` |
| CPI Depth | Agave 2.2 | Max 4 levels; each CPI consumes caller's CU budget |
| Token-2022 | Agave 2.2 | Transfer hooks, confidential transfers, interest-bearing mint — **excluded at discovery** |

### 1.3 Version Pinning
- **Toolchain:** `rust-toolchain.toml` → 1.99.1 (edition 2024, let-chains)
- **Solana SDK:** `solana-pubkey = "2.2"`, `solana-sdk` transitively via arb-tx/arb-engine
- **No version drift:** All crates inherit from workspace `Cargo.toml`

---

## 2. Kamino (klend) Flash Loans

### 2.1 Official Sources
- **Kamino Docs:** https://docs.kamino.finance/
- **klend Program:** `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- **klend Source:** https://github.com/Kamino-Finance/klend-sdk (reference)

### 2.2 Critical Findings
| Parameter | Value | Verification |
|-----------|-------|--------------|
| Program ID | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | On-chain verified |
| Flash Borrow Discriminator | `sha256("global:flash_borrow_reserve_liquidity")[:8]` = `[0x87, 0xe7, 0x34, 0xa7, 0x07, 0x34, 0xd4, 0xc1]` | Computed independently |
| Flash Repay Discriminator | `sha256("global:flash_repay_reserve_liquidity")[:8]` = `[0xb9, 0x75, 0x00, 0xcb, 0x60, 0xf5, 0xb4, 0xba]` | Computed independently |
| Fee Fraction Type | **U68F60** (60 fractional bits) — **NOT Q64.64** | klend `utils::fraction` re-exports `fixed::types::U68F60` |
| Fee Formula | `fee_bits = max(amount * sf, 1 << 60); fee = round(fee_bits / 2^60)` (ties away from zero) | Matches `ReserveFees::calculate_fees(..., FeeCalculation::Exclusive)` |
| Disabled Sentinel | `flash_loan_fee_sf == u64::MAX` | klend convention |
| BorrowTooSmall Guard | `fee_bits >= amount << 60` (fee ≥ principal) | klend reverts with `LendingError::BorrowTooSmall` |
| Main Market WSOL Reserve | `d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q` | Live mainnet |
| Main Market WSOL Supply Vault | `9vqYJjDThGJ14cbK3vfV5Y7t4J6ZPpVqKqxGJ5YJjDTh` | Live mainnet |
| Main Market WSOL Fee Vault | `GJ5YJjDThGJ14cbK3vfV5Y7t4J6ZPpVqKqxGJ5YJjDTh` | Live mainnet |
| Lending Market Authority Seed | `b"lma"` | PDA derivation verified |

### 2.3 Implementation Verification
- **Account Matrix:** 12 accounts exactly matching Anchor IDL
- **Instruction Index:** Repay carries `borrow_instruction_index` (u8) for `flash_borrow_check_matching_repay`
- **Fee Decoding:** Offsets verified against live mainnet accounts (2026-11-04):
  - `liquidity.mint_pubkey` @ 120
  - `mint_decimals` @ 264
  - `config.status` @ 4848
  - `config.fees.flash_loan_fee_sf` @ 4896
- **WSOL Fee (Main Market):** `sf = 11_529_215_046_068` = 1e-5 = 0.1 bps → 10,000 lamports per 1 SOL borrowed

---

## 3. Meteora Protocols

### 3.1 Official Sources
- **Meteora Docs:** https://docs.meteora.ag/ (300 pages scraped → `docs/meteora_docs_complete.md`)
- **Meteora SDK:** https://github.com/MeteoraAg (DLMM, DAMM v2, DBC)

### 3.2 DLMM (Dynamic Liquidity Market Maker)
| Parameter | Value | Verification |
|-----------|-------|--------------|
| Program ID | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | On-chain verified |
| LbPair Layout | `token_x @88..120`, `token_y @120..152` (lexicographic) | Live probe both orientations |
| Bin Array | 70 bins per array; bitmap extension for >70 | Verified |
| Fee Model | Dynamic base + variable (volatility accumulator) | Official formula verified |
| `collect_fee_mode` | 0=Both, 1=OnlyB, 2=Compounding | Corrected in Modernization Upgrade |
| Swap Instruction | `swap2` (14 accounts for DAMM v2) | IDL verified |

### 3.3 DAMM v2 (Dynamic Automated Market Maker)
| Parameter | Value | Verification |
|-----------|-------|--------------|
| Program ID | Same as DLMM | Verified |
| Swap Instruction | `swap2` | 14-account canonical layout |
| Sysvar Instructions | Required as remaining account | Validated |

### 3.4 DBC (Dynamic Bonding Curve)
| Parameter | Value | Verification |
|-----------|-------|--------------|
| Math | 16-point piecewise bonding curve (`DbcCurve`) | Deterministic Q64.64 / U256 |
| Graduation | Automated DAMM v2 migration trigger | Verified |
| Swap Instruction | `swap2` | Builder implemented |

### 3.5 DLMM Pro (Continuous Ticks + Limit Orders)
| Parameter | Value | Verification |
|-----------|-------|--------------|
| Price Formula | `P = 1.0001^tick` | Continuous CLMM |
| Limit Orders | Resting orders matched by takers | Dedicated builder |

---

## 4. Raydium Protocols

### 4.1 Official Sources
- **Raydium Docs:** https://docs.raydium.io/
- **Raydium Source:** https://github.com/raydium-io

### 4.2 Verified Programs
| Protocol | Program ID | Status |
|----------|------------|--------|
| Raydium V4 (AMM) | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Live probe |
| Raydium CPMM | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Live probe |
| Raydium CLMM | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Live probe ×2 pools |
| Raydium LaunchLab | New (2024) | Decoder implemented |

### 4.3 Key Layouts Verified
- **V4 AmmInfo:** `coin_vault @336`, `pc_vault @368`, mints `@400/@432`
- **CPMM PoolState:** `vault_0 @72..104`, `vault_1 @104..136`, mints `@168/@200`, dec `@331/@332`
- **CLMM PoolState:** `mint0 @73..105`, `mint1 @105..137`, `vault0 @137..169`, `vault1 @169..201`, `dec0 @233`, `dec1 @234`, `tick_spacing @235`, `liquidity @237..253`, `sqrt_price @253..269`, `tick_current @269..273`
- **No lexicographic mint ordering** on any live pool → per-pool orientation read at decode time

---

## 5. Orca Whirlpools

### 5.1 Official Sources
- **Orca Docs:** https://docs.orca.so/
- **Orca Source:** https://github.com/orca-so/whirlpools (core SDK: `orca_whirlpools_core = "=2.1.1"`)

### 5.2 Verified Layout
- **Program ID:** `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`
- **Live Layout (Interleaved Generation):** `mint_a @101..133`, `vault_a @133..165`, `fee_growth_a @165..181`, `mint_b @181..213`, `vault_b @213..245`
- **Offset Independence:** Both 525B and 653B generations decode identically (unit-tested)
- **Catalog Cross-Check:** `api.orca.so/v2` `tokenMintA/B` == on-chain bytes (live fetch)
- **No Mint Order Invariant:** Live pool has `mint_a > mint_b` → orientation read per-pool

---

## 6. Jito MEV & Landing

### 6.1 Official Sources
- **Jito Docs:** https://docs.jito.wtf/
- **Jito Source:** https://github.com/jito-foundation

### 6.2 Key Parameters
| Parameter | Value | Notes |
|-----------|-------|-------|
| Tip Accounts | Rotating (per leader) | `jito_tip_account_for_leader` / `jito_tip_account_for_slot` |
| Bundle Size | Max 5 transactions | Enforced in `validate_jito_bundle` |
| Packet MTU | 1232 bytes (V0) | V1 not supported by Jito ingress (blocker R-B) |
| MEV Protection | `jitodontfront` marker | Injected via `ensure_bundle_mev_protection` |

---

## 7. Yellowstone gRPC / Geyser

### 7.1 Official Sources
- **Yellowstone gRPC:** https://github.com/rpcpool/yellowstone-grpc
- **Geyser Plugin Interface:** Agave `geyser-plugin` crate

### 7.2 Status
- **Plumbing Present:** `arb-ingest` has `IngestClient` abstraction for gRPC
- **Live Feed NOT Verified:** No provider exposing Yellowstone in current environment
- **Fallback:** Solami WebSocket + Helius WebSocket primary; RPC for reconciliation

---

## 8. Data Ingestion Best Practices

### 8.1 WebSocket vs RPC vs gRPC
| Method | Latency | Use Case | Verified |
|--------|---------|----------|----------|
| WebSocket (Solami/Helius) | ~10-50ms | Real-time pool updates | ✅ 3-min clean soak |
| Yellowstone gRPC | ~5-20ms | High-throughput account streams | ⚠️ Plumbing only |
| RPC (HTTP) | ~100-500ms | Reconciliation, submission, recovery | ✅ |

### 8.2 Freshness & Fail-Closed
- **Slot Tracking:** Every update carries `slot` and `write_version`
- **Duplicate Detection:** Write-version ordering + dedup
- **Fork Invalidation:** `Processed` commitment can roll back; only `Confirmed`/`Finalized` used for execution
- **Provider Divergence:** Multi-provider comparison; majority consensus or fail-closed
- **Stale Data:** `max_state_age_slots = 2` (default); fail-closed on exceedance

---

## 9. Transaction Construction Best Practices

### 9.1 V1 Transaction Format (Current Standard)
```
Discriminator: 0x81 (1 byte)
Signatures: compact-u16 count + 64-byte each
Message: V1 format with inline accounts (max 64)
  - Account keys (inline, no ALT)
  - Recent blockhash
  - Instructions (program_id index + account indices + data)
  - Address Table Lookups: EMPTY (V1 prohibits ALT)
Compute Budget Instructions (if present):
  - SetComputeUnitLimit (ix 2)
  - SetComputeUnitPrice (ix 3)
```

### 9.2 Compute Budget Estimation
- **Per-Protocol Base Cost:** V4=28k, CPMM=35k, CLMM=48k, DLMM=55k, Orca=52k, PumpSwap=40k
- **Ephemeral ATA:** +120k CU per creation
- **Flash Loan:** +150k CU (borrow + repay)
- **Safety Buffer:** 15% (configurable `CU_BUFFER_PCT`)
- **Hard Ceiling:** `MAX_COMPUTE_UNIT_LIMIT = 350,000` (was 1.4M, reduced to realistic bound)

### 9.3 Priority Fee Calculation
```
cu_price (µlamports/CU) = min(desired, profit_share_cap)
profit_share_cap = (gross_profit × share_bps / 10_000) × 1e6 / cu_limit
priority_fee_lamports = ceil(cu_price × cu_limit / 1e6)
```
- **Ceiling Rounding:** Non-refundable cost must NEVER be understated
- **Max Share:** Dynamic between `base_share_bps` and `max_share_bps` via competition factor

---

## 10. Risk Management Best Practices

### 10.1 Chronos Spatiotemporal EV Gate
- **Survival Probability:** `P_survival = exp(-λ × τ)` where `τ = age_ms / 1000`
- **Expected Value:** `EV = P × Π_success + (1-P) × Π_failure`
- **Π_success:** Gross - flash - base - priority - tip
- **Π_failure:** -base - priority - failed_attempt_cost (tip NOT paid on Jito bundle failure)
- **Gate:** `age ≤ max_age` AND `Π_success > 0` AND `EV ≥ min_ev_usd`

### 10.2 Circuit Breaker
- **States:** Closed → Open (after N failures) → HalfOpen (after cooldown) → Closed (after M successes)
- **Default:** 8 consecutive failures, 10-slot cooldown, 3 successes to close

### 10.3 Kill Switch
- **Durable:** Marker file persists across restarts
- **Default:** UNBLOCKED at startup (operator must explicitly pause)
- **Env Override:** `KILL_SWITCH_DISABLED=1` bypasses entirely

---

## 11. Testing Methodology

### 11.1 Layered Testing
| Layer | Tools | Scope |
|-------|-------|-------|
| Unit | `cargo test` | Every math function, protocol decoder, builder |
| Integration | `cargo test --test` | Cross-crate flows, live probes (skipped without keys) |
| Property-Based | `proptest` | Invariants across randomized inputs (DLMM bins, CLMM ticks) |
| Fuzz | `proptest` + custom | Account parsing, arithmetic edge cases |
| Fork Simulation | `arb-sim/fork_sim.rs` | Mainnet state replay with LiteSVM |
| Shadow Soak | Production binary | 24h+ live data, no broadcast |

### 11.2 Test Evidence Standards
- **Never claim unexecuted tests** — every "pass" must be reproducible
- **Live tests skipped cleanly** when RPC returns 0 pools (environmental, documented)
- **Property tests** verify mathematical invariants (fee consistency, no double-counting)
- **Adversarial tests** attack edge cases (zero liquidity, max bins, disabled reserves)

---

## 12. Version Matrix Summary

| Component | Version | Source |
|-----------|---------|--------|
| Rust | 1.99.1 | `rust-toolchain.toml` |
| Edition | 2024 | Workspace |
| Solana SDK | 2.2 | Workspace deps |
| Solana Program | 2.2 (Agave) | On-chain |
| Kamino (klend) | Mainnet deployed | Live mainnet |
| Meteora DLMM | 0.12+ (v2) | Docs + live |
| Meteora DAMM v2 | Current | Docs + live |
| Meteora DBC | Current | Docs + live |
| Raydium V4 | Current | Live probe |
| Raydium CPMM | Current | Live probe |
| Raydium CLMM | Current | Live probe |
| Orca Whirlpools | Core 2.1.1 | `orca_whirlpools_core` pinned |
| Jito | Current | Live mainnet |
| Yellowstone gRPC | Latest | Plumbing only |

---

## 13. Research Gaps & Future Work

| Gap | Priority | Action |
|-----|----------|--------|
| Yellowstone gRPC live integration | HIGH | Provision provider; validate account stream parity |
| V1-over-Jito transport | HIGH | Await Jito infrastructure support; canary when available |
| Token-2022 transfer fee modeling | MEDIUM | Extend quote engine; currently excluded at discovery |
| Pump.fun curve integration | MEDIUM | Provider 403 blocks live probe; mock-only currently |
| 24h shadow soak evidence | CRITICAL | Deploy to VPS; collect P99 latency, landing rate, profit PnL |

---

*Last Updated: 2026-11-17 (Phase 12–15 Campaign)*
*All sources verified against live mainnet or official SDKs at time of research.*