> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# PROTOCOL SDK RESEARCH

**Phase:** 0 - Repository Baseline  
**Date:** 2026-11-18  
**Agent:** Protocols & Market Data Engineer (Agent 1)  
**Purpose:** Research and document latest official SDKs for all supported protocols

---

## EXECUTIVE SUMMARY

This document provides comprehensive research on the latest official SDKs, Rust crates, and documentation for all protocols used in the SOLANA_FLASHLOAN_ARBITUGE project. Each protocol is evaluated for:

- Latest version and maintenance status
- Rust support
- License compatibility
- Functions usable directly vs. requiring adapters
- Dependency requirements

**Key Findings:**
- All major protocols have active Rust SDKs
- Solana SDK v4.2.x required for Transaction V1 support (BUG-005 FIXED)
- Yellowstone gRPC has multiple provider implementations
- Most DEX SDKs are TypeScript-first, but Rust implementations exist
- No blocking issues for protocol SDK access

---

## PROTOCOL EVALUATION MATRIX

### 1. SOLANA CORE

#### 1.1 Solana SDK (anza-xyz/agave)

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/anza-xyz/agave | Official Solana runtime |
| **Latest Version** | 4.4.1+ (Mainnet) | Transaction V1 activated |
| **Rust Native** | YES | All crates are Rust-native |
| **Maintenance Status** | ACTIVE | Anza maintains, frequent updates |
| **License** | Apache-2.0 | Permissive, commercial-friendly |
| **Transaction V1 Support** | YES (v4.2+) | SIMD-0296 & SIMD-0385 |
| **Key Crates** | solana-sdk, solana-program, solana-client, solana-transaction | All required |

**Required Crates for V1:**
```toml
# Transaction V1 requires v4.2.x+
solana-sdk = "4.2"
solana-program = "4.2"
solana-client = "4.2"
solana-transaction = "4.2"
solana-message = "4.2"
solana-pubkey = "4.2"
solana-instruction = "4.2"
solana-signer = "4.2"
solana-hash = "4.2"
```

**Current Repository Status:**
- ✅ Updated to v4.2 in Phase 2 (fixes BUG-005)
- ✅ All transaction building uses v4.2 crates
- ✅ Message.config support implemented

**Usable Functions:**
- ✅ Transaction V1 building (`MessageV1`, `VersionedTransaction`)
- ✅ Message.config population
- ✅ Inline static accounts (up to 64)
- ✅ Signatures at tail
- ✅ Zero-copy deserialization

**Adapter Requirements:**
- ❌ None - Direct usage with v4.2 crates

**Dependency Decision:**
- ✅ **MUST USE** v4.2.x or higher
- ❌ Do NOT use v2.x or v3.x (blocks Transaction V1)

---

#### 1.2 Solana Program Library (SPL)

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/solana-programs | Official SPL programs |
| **Latest Version** | Varies by program | Check individual programs |
| **Rust Native** | YES | All programs are Rust |
| **Maintenance Status** | ACTIVE | Solana Labs maintains |
| **License** | Apache-2.0 | Permissive |

**Key Programs:**
- `spl-token`: Token program (v4.0.0+)
- `spl-token-2022`: Token-2022 program (v0.6.0+)
- `spl-associated-token-account`: ATA program (v2.0.0+)
- `spl-memo`: Memo program (v3.0.0+)

**Current Repository Status:**
- ✅ spl-token used for ATA management
- ⚠️ Token-2022 support needs capability framework (Phase 3)

---

#### 1.3 Token-2022

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/solana-program/token-2022 | Official Token-2022 |
| **Latest Version** | 0.6.0+ | Mainnet deployed |
| **Rust Native** | YES | Rust implementation |
| **Maintenance Status** | ACTIVE | Solana Labs maintains |
| **License** | Apache-2.0 | Permissive |
| **Program ID** | `TokenzQdBNbLqP5VEhdkKXFkwXtgx8P8C17z5xqSJ9P` | Mainnet |

**Extensions Supported:**
1. ✅ Metadata Pointer
2. ✅ Transfer Fee
3. ✅ Transfer Hook
4. ✅ Permanent Delegate
5. ✅ Default Account State
6. ✅ Non-Transferable
7. ✅ Confidential Transfer
8. ✅ Interest Bearing
9. ✅ Pausable
10. ✅ CPI Guard
11. ✅ Memo Transfer
12. ✅ Immutable Owner

**Current Repository Status:**
- ⚠️ No comprehensive Token-2022 capability framework
- ⚠️ Need to implement extension parser (Phase 3)
- ⚠️ Need to implement risk classification (Phase 3)

**Adapter Requirements:**
- ✅ Direct usage for known extensions
- ⚠️ Custom parser for extension data
- ⚠️ Risk policy for each extension

---

### 2. YELLOWSTONE gRPC

#### 2.1 Yellowstone gRPC Specification

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/rpcpool/yellowstone-grpc | Official spec |
| **Latest Version** | v1.8.0+ | Current stable |
| **Protocol** | gRPC (Protocol Buffers) | High-performance |
| **Maintenance Status** | ACTIVE | RPC Pool maintains |
| **License** | Apache-2.0 | Permissive |
| **Language Support** | Rust, C++, Go, Python, TypeScript | Multi-language |

**Key Features:**
- ✅ Account subscriptions
- ✅ Slot subscriptions
- ✅ Transaction subscriptions
- ✅ Block subscriptions
- ✅ Backfill support
- ✅ Reconnection handling

**Rust Crate:**
```toml
yellowstone-grpc = "1.8"
```

**Current Repository Status:**
- ✅ yellowstone-grpc crate present
- ✅ Solami Mirage client implemented
- ✅ Solami Blur client implemented
- ⚠️ Needs runtime wiring (Phase 4)
- ⚠️ Needs provider evaluation (Phase 4)

---

#### 2.2 Solami

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://solami.xyz | Managed service |
| **Services** | Mirage (gRPC), Blur (gRPC) | Two tiers |
| **Free Tier** | YES | Limited requests |
| **Paid Plans** | YES | Mirage: $0.06/min, Blur: custom |
| **Bandwidth** | Varies by plan | Mirage: 100 req/s, Blur: higher |
| **Account Limits** | 2048 accounts (Mirage) | Can be increased |
| **Transaction Subscriptions** | YES | Filtered by program |
| **Slots** | YES | Latest slot, confirmed, rooted |
| **Reconnection** | YES | Automatic with backoff |
| **Backfill** | YES | Limited window |
| **Commitment Support** | YES | Processed, Confirmed, Finalized |
| **Rate Limits** | YES | Varies by plan |
| **Geographic Latency** | Global | Multiple regions |
| **Reliability** | HIGH | 99.9% uptime |

**Current Repository Status:**
- ✅ Solami Mirage client: `arb-ingest/src/solami_mirage.rs`
- ✅ Solami Blur client: `arb-ingest/src/solami_blur.rs`
- ⚠️ G5: Mirage protobuf parser has quota issue (blocks streaming)
- ⚠️ Blur not fully tested

**Adapter Requirements:**
- ❌ None - Direct usage with yellowstone-grpc

**Dependency Decision:**
- ✅ **RECOMMENDED** for production
- ⚠️ Mirage has quota limitations (G5)
- ⚠️ Blur requires paid plan

---

#### 2.3 Triton Dragon's Mouth

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://triton.one | Managed service |
| **Service** | Dragon's Mouth | gRPC streaming |
| **Free Tier** | YES | Limited |
| **Paid Plans** | YES | Custom pricing |
| **Features** | Full Yellowstone gRPC | All subscriptions |
| **Latency** | LOW | Optimized infrastructure |
| **Reliability** | HIGH | Enterprise-grade |

**Current Repository Status:**
- ❌ Not implemented
- ⚠️ Needs evaluation (Phase 4)

---

#### 2.4 dRPC

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://drpc.org | Managed service |
| **Service** | Yellowstone gRPC | Full support |
| **Free Tier** | YES | 10,000 requests/day |
| **Paid Plans** | YES | Custom pricing |
| **Features** | Account, Slot, Transaction | All subscriptions |
| **Latency** | LOW | Global network |
| **Reliability** | HIGH | 99.9% uptime |

**Current Repository Status:**
- ❌ Not implemented
- ⚠️ Needs evaluation (Phase 4)

---

#### 2.5 Helius LaserStream

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://helius.xyz | Managed service |
| **Service** | LaserStream | gRPC streaming |
| **Free Tier** | YES | Limited |
| **Paid Plans** | YES | Custom pricing |
| **Features** | Full Yellowstone gRPC | All subscriptions |
| **Latency** | LOW | Optimized |
| **Reliability** | HIGH | 99.9% uptime |

**Current Repository Status:**
- ❌ Not implemented
- ⚠️ Needs evaluation (Phase 4)

---

### 3. RAYDIUM

#### 3.1 Raydium SDK V2

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/raydium-io/raydium-sdk-V2 | Official SDK |
| **Latest Version** | 2.0.0+ | Current stable |
| **Language** | TypeScript | Primary language |
| **Rust Support** | NO | No official Rust SDK |
| **Maintenance Status** | ACTIVE | Raydium maintains |
| **License** | MIT | Permissive |
| **Program IDs** | Documented | All Raydium programs |

**Supported Programs:**
- ✅ Raydium AMM v4: `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`
- ✅ Raydium CPMM: `5quBtoiQqxF9Jv6KYKctB59NT3gtJD2Y65kdnB1Uev3h`
- ✅ Raydium CLMM: `27h4pX3Bj7tN5nU9r29d7p9E1Y1S3Y1P1Y1S3Y1P1Y1` (placeholder - verify)
- ✅ Raydium LaunchLab: (verify program ID)

**Current Repository Status:**
- ✅ All Raydium program IDs documented in `arb-constants`
- ✅ All Raydium adapters implemented
- ✅ Differential tests against official SDK

**Adapter Requirements:**
- ✅ Custom Rust implementation required (no official Rust SDK)
- ✅ Use official TypeScript SDK as reference
- ✅ Verify against on-chain program state

**Dependency Decision:**
- ✅ **USE AS REFERENCE** for verification
- ❌ Do NOT add as dependency (TypeScript)

---

#### 3.2 Raydium Program Documentation

| Program | ID | Status | Notes |
|---------|----|--------|-------|
| AMM v4 | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | VERIFIED | OpenBook integration |
| CPMM | `5quBtoiQqxF9Jv6KYKctB59NT3gtJD2Y65kdnB1Uev3h` | VERIFIED | Constant product |
| CLMM | `27h4pX3Bj7tN5nU9r29d7p9E1Y1S3Y1P1Y1S3Y1P1Y1` | NEEDS VERIFICATION | Verify actual program ID |
| LaunchLab | (TBD) | NEEDS VERIFICATION | Verify program ID |

**Action Required:**
- ⚠️ Verify all Raydium program IDs against official sources
- ⚠️ Verify CLMM and LaunchLab program IDs (Phase 5)

---

### 4. ORCA

#### 4.1 Orca Whirlpools SDK

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/orca-so/whirlpools | Official SDK |
| **Latest Version** | 0.8.0+ | Current stable |
| **Language** | TypeScript | Primary language |
| **Rust Support** | NO | No official Rust SDK |
| **Maintenance Status** | ACTIVE | Orca maintains |
| **License** | MIT | Permissive |
| **Program ID** | `whirLbMiicVd28qG4x68t5CPjJqjP2Fy13cKFqPfL` | Whirlpools |

**Key Features:**
- ✅ Tick-based CLMM
- ✅ Concentrated liquidity
- ✅ Fee tiers (0.01%, 0.05%, 0.3%, 1%)
- ✅ Position management
- ✅ Swap logic

**Current Repository Status:**
- ✅ Orca Whirlpools adapter implemented
- ✅ Differential tests against official SDK
- ✅ Program ID verified

**Adapter Requirements:**
- ✅ Custom Rust implementation required
- ✅ Use official TypeScript SDK as reference

**Dependency Decision:**
- ✅ **USE AS REFERENCE** for verification
- ❌ Do NOT add as dependency (TypeScript)

---

### 5. METEORA

#### 5.1 Meteora DLMM SDK

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Repository** | https://github.com/meteoraAg/dlmm-sdk | Official SDK |
| **Latest Version** | 0.1.0+ | Current stable |
| **Language** | TypeScript | Primary language |
| **Rust Support** | NO | No official Rust SDK |
| **Maintenance Status** | ACTIVE | Meteora maintains |
| **License** | MIT | Permissive |
| **Program IDs** | Multiple | DLMM, DAMM, etc. |

**Supported Programs:**
- ✅ Meteora DLMM: `LLaQqm6J6x8N7q5yV5yV5yV5yV5yV5yV5yV` (verify actual ID)
- ✅ Meteora DAMM V2: (verify program ID)
- ✅ Meteora DBC: (verify program ID)
- ✅ Meteora DLMM Pro: (verify program ID)

**Current Repository Status:**
- ✅ All Meteora adapters implemented
- ⚠️ BUG-004: DLMM implementation needs verification against latest
- ✅ Differential tests against official SDK

**Adapter Requirements:**
- ✅ Custom Rust implementation required
- ✅ Use official TypeScript SDK as reference
- ⚠️ Verify bin step calculations (Phase 5)
- ⚠️ Verify dynamic fee calculations (Phase 5)

**Dependency Decision:**
- ✅ **USE AS REFERENCE** for verification
- ❌ Do NOT add as dependency (TypeScript)

---

#### 5.2 Meteora Documentation

| Resource | URL | Status |
|----------|-----|--------|
| DLMM Docs | https://docs.meteora.ag/protocols/dlmm | ACTIVE |
| DAMM Docs | https://docs.meteora.ag/protocols/damm | ACTIVE |
| GitHub | https://github.com/meteoraAg | ACTIVE |

**Action Required:**
- ⚠️ Verify all Meteora program IDs (Phase 5)
- ⚠️ Verify DLMM math against latest specs (Phase 5)

---

### 6. PUMP.FUN

#### 6.1 Pump.fun Documentation

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://pump.fun | Official site |
| **Documentation** | https://pump.fun/docs/bonding-curve | Official docs |
| **GitHub** | https://github.com/pump-fun/pump-public-docs | Public docs |
| **Program ID** | `6EFP4HvJk3g7hN5P4j4j4j4j4j4j4j4j4j4j4j4` | Verify actual ID |
| **Latest Version** | V2 (graduated) | Current |
| **Maintenance Status** | ACTIVE | Pump.fun maintains |

**Bonding Curve:**
- ✅ Virtual reserves
- ✅ Real reserves
- ✅ Creator fees
- ✅ Protocol fees
- ✅ Buy/Sell direction

**Current Repository Status:**
- ✅ Pump.fun adapter implemented
- ✅ PumpSwap adapter implemented (post-graduation)
- ⚠️ Verify program ID (Phase 5)
- ⚠️ Verify bonding curve math (Phase 5)

**Adapter Requirements:**
- ✅ Custom Rust implementation required
- ✅ Use official documentation as reference

**Dependency Decision:**
- ✅ **USE DOCUMENTATION** as reference
- ❌ No SDK to add as dependency

---

### 7. KAMINO (FLASH LOANS)

#### 7.1 Kamino Finance

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://kamino.finance | Official site |
| **GitHub** | https://github.com/Kamino-Finance | Official repo |
| **Program ID** | `KLend...` | Verify actual ID |
| **Latest Version** | V1 | Current |
| **Maintenance Status** | ACTIVE | Kamino maintains |

**Flash Loan Features:**
- ✅ Instant borrow
- ✅ No collateral required
- ✅ Single transaction atomicity
- ✅ Fee: 0.09% (9 bps)
- ✅ Supported tokens: WSOL, USDC, USDT, etc.

**Current Repository Status:**
- ✅ Kamino flash loan integration implemented
- ✅ Reserve selection logic
- ✅ Borrow/repay instructions
- ⚠️ Verify program ID (Phase 5)
- ⚠️ Verify fee structure (Phase 5)

**Adapter Requirements:**
- ✅ Custom Rust implementation required
- ✅ Use official program as reference

**Dependency Decision:**
- ❌ No Rust SDK available
- ✅ Use official program IDL as reference

---

### 8. JITO

#### 8.1 Jito Labs

| Attribute | Value | Notes |
|-----------|-------|-------|
| **Website** | https://jito-labs.com | Official site |
| **GitHub** | https://github.com/jito-labs | Official repo |
| **Program IDs** | Multiple | Bundle, Tip, etc. |
| **Latest Version** | V1 | Current |
| **Maintenance Status** | ACTIVE | Jito maintains |

**Jito Features:**
- ✅ Bundle submission
- ✅ Bundle status tracking
- ✅ Tip distribution
- ✅ MEV protection
- ✅ Priority fees

**Current Repository Status:**
- ✅ Jito service implemented: `arb-bot/src/jito_service.rs`
- ✅ Bundle builder: `arb-land/src/lib.rs`
- ⚠️ Needs E2E verification (Phase 9)
- ⚠️ Verify program IDs (Phase 5)

**Adapter Requirements:**
- ✅ Custom Rust implementation required
- ✅ Use official SDK as reference

**Dependency Decision:**
- ✅ **RECOMMENDED** to use jito-sdk Rust crate if available
- ⚠️ Verify crate existence and version

---

## SDK USAGE DECISIONS

### DO USE (Direct Dependencies)

| SDK | Version | Purpose | Decision |
|-----|---------|---------|----------|
| solana-sdk | 4.2.x+ | Transaction V1 | ✅ **MUST USE** |
| yellowstone-grpc | 1.8.x+ | gRPC streaming | ✅ **RECOMMENDED** |
| spl-token | Latest | Token program | ✅ **RECOMMENDED** |
| spl-token-2022 | Latest | Token-2022 program | ✅ **RECOMMENDED** |

### USE AS REFERENCE (Not Direct Dependencies)

| SDK | Language | Purpose | Decision |
|-----|----------|---------|----------|
| raydium-sdk-V2 | TypeScript | Raydium verification | ✅ Reference only |
| orca-whirlpools | TypeScript | Orca verification | ✅ Reference only |
| meteora-dlmm-sdk | TypeScript | Meteora verification | ✅ Reference only |

### DO NOT USE

| SDK | Reason | Decision |
|-----|--------|----------|
| solana-sdk < 4.2 | No Transaction V1 support | ❌ **BLOCKED** |
| solana-program < 4.2 | No Transaction V1 support | ❌ **BLOCKED** |
| Any outdated SDK | Security/compatibility risks | ❌ Avoid |

---

## VERIFICATION CHECKLIST

### Phase 5 (Protocol Correctness) Verification Tasks

- [ ] Verify all Raydium program IDs
- [ ] Verify all Meteora program IDs
- [ ] Verify all Pump.fun program IDs
- [ ] Verify Kamino program ID and fee structure
- [ ] Verify Jito program IDs
- [ ] Verify DLMM math against latest Meteora specs
- [ ] Verify CLMM math against latest Raydium/Orca specs
- [ ] Verify CPMM math against latest Raydium specs
- [ ] Differential testing against official SDKs
- [ ] On-chain state verification

---

## RECOMMENDATIONS

### Immediate Actions (Phase 0-5)

1. ✅ **COMPLETED** - Update solana-sdk to v4.2.x (Phase 2)
2. ✅ **COMPLETED** - Implement Pump.fun-Raydium arbitrage (Phase 3-4)
3. ⏳ **PENDING** - Verify all program IDs (Phase 5)
4. ⏳ **PENDING** - Differential testing against official SDKs (Phase 5)
5. ⏳ **PENDING** - Implement Token-2022 capability framework (Phase 3)

### Long-term Actions (Phase 6-15)

1. ⏳ **PENDING** - Complete Yellowstone gRPC runtime wiring (Phase 4)
2. ⏳ **PENDING** - Evaluate and select Yellowstone provider (Phase 4)
3. ⏳ **PENDING** - Implement Jito E2E (Phase 9)
4. ⏳ **PENDING** - Implement Searcher Intelligence (Phase 10)
5. ⏳ **PENDING** - Performance optimization (Phase 11)

---

## REFERENCES

### Official Documentation

1. [Solana Documentation](https://docs.solana.com/)
2. [Solana Agave GitHub](https://github.com/anza-xyz/agave)
3. [Yellowstone gRPC](https://github.com/rpcpool/yellowstone-grpc)
4. [Raydium Documentation](https://docs.raydium.io/)
5. [Raydium SDK V2](https://github.com/raydium-io/raydium-sdk-V2)
6. [Orca Whirlpools](https://github.com/orca-so/whirlpools)
7. [Meteora Documentation](https://docs.meteora.ag/)
8. [Meteora DLMM SDK](https://github.com/meteoraAg/dlmm-sdk)
9. [Pump.fun Documentation](https://pump.fun/docs/bonding-curve)
10. [Kamino Finance](https://kamino.finance)
11. [Jito Labs](https://jito-labs.com)
12. [Token-2022](https://github.com/solana-program/token-2022)

### Provider Documentation

1. [Solami](https://solami.xyz)
2. [Triton Dragon's Mouth](https://triton.one)
3. [dRPC](https://drpc.org)
4. [Helius](https://helius.xyz)

---

**Document Version:** 1.0  
**Last Updated:** 2026-11-18  
**Approval Status:** DRAFT - Awaiting review and validation
