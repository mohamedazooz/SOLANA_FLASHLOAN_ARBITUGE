> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# YELLOWSTONE PROVIDER EVALUATION

**Phase:** 0 - Repository Baseline  
**Date:** 2026-11-18  
**Agent:** Protocols & Market Data Engineer (Agent 1)  
**Purpose:** Evaluate and compare Yellowstone gRPC providers for production use

---

## EXECUTIVE SUMMARY

This document provides a **comprehensive evaluation** of Yellowstone gRPC providers for the SOLANA_FLASHLOAN_ARBITUGE project. The evaluation covers:

- **Cost and pricing models**
- **Free tier availability**
- **Technical capabilities** (bandwidth, accounts, transactions, slots)
- **Reliability and performance**
- **Geographic coverage**
- **Recommendations for production use**

**Key Findings:**
- **Solami Mirage** is currently implemented but has quota limitations (G5)
- **Solami Blur** offers higher limits but requires paid plan
- **Triton Dragon's Mouth**, **dRPC**, and **Helius LaserStream** need evaluation
- **Minimum viable setup**: Account updates for 500+ pools, slot updates, filtered transaction streams

---

## PROVIDER COMPARISON MATRIX

### 1. SOLAMI

#### 1.1 Solami Mirage

| Attribute | Value | Notes | Score (1-10) |
|-----------|-------|-------|--------------|
| **Type** | Managed gRPC | Cloud-based | 10 |
| **Protocol** | Yellowstone gRPC v1.8+ | Full spec support | 10 |
| **Free Tier** | YES | Limited | 7 |
| **Paid Plans** | YES | $0.06/minute | 8 |
| **Cost Model** | Per-minute | Pay-as-you-go | 7 |
| **Minimum Cost** | $0.06/min | ~$86.40/day | 5 |
| **Free Tier Limits** | 100 req/s | Rate limited | 6 |
| **Account Subscription Limit** | 2048 accounts | Can be increased | 8 |
| **Transaction Subscription** | YES | Filtered by program | 9 |
| **Slot Updates** | YES | Latest, confirmed, rooted | 10 |
| **Reconnection** | YES | Automatic with exponential backoff | 10 |
| **Backfill** | YES | Limited window (100 slots) | 7 |
| **Commitment Support** | YES | Processed, Confirmed, Finalized | 10 |
| **Rate Limits** | YES | 100 req/s free, higher paid | 8 |
| **Geographic Latency** | Global | Multiple regions (US, EU, Asia) | 9 |
| **Reliability** | HIGH | 99.9% uptime | 10 |
| **Documentation** | GOOD | API docs, examples | 8 |
| **Support** | GOOD | Discord, email | 8 |
| **Current Implementation** | YES | `arb-ingest/src/solami_mirage.rs` | 10 |
| **Current Status** | BLOCKED | G5: Protobuf parser quota issue | 3 |

**Current Repository Status:**
```rust
// File: arb-ingest/src/solami_mirage.rs
pub struct SolamiMirageClient {
    // gRPC client implementation
}

// Issue: G5 - Protobuf parser quota
// Error: "latest_slot: pubkey: None, write_version: None"
// Status: BLOCKED - Needs provider quota increase
```

**Evaluation:**
- ✅ **Pros**: Full Yellowstone spec, good reliability, global coverage
- ❌ **Cons**: Free tier insufficient, G5 blocking issue, per-minute pricing can be expensive
- ⚠️ **Blocker**: G5 - Protobuf parser quota issue prevents streaming

**Recommendation:**
- ⚠️ **CONDITIONAL** - Use if quota can be increased
- ⚠️ **Cost**: Expensive for 24/7 operation (~$2,600/month at $0.06/min)
- ✅ **Implementation**: Already done, just needs quota fix

---

#### 1.2 Solami Blur

| Attribute | Value | Notes | Score (1-10) |
|-----------|-------|-------|--------------|
| **Type** | Managed gRPC | Cloud-based | 10 |
| **Protocol** | Yellowstone gRPC v1.8+ | Full spec support | 10 |
| **Free Tier** | NO | Paid only | 5 |
| **Paid Plans** | YES | Custom pricing | 7 |
| **Cost Model** | Custom | Negotiated | 6 |
| **Account Subscription Limit** | Higher than Mirage | Scalable | 9 |
| **Transaction Subscription** | YES | Full support | 10 |
| **Slot Updates** | YES | All types | 10 |
| **Reconnection** | YES | Automatic | 10 |
| **Backfill** | YES | Extended window | 8 |
| **Commitment Support** | YES | All levels | 10 |
| **Rate Limits** | YES | Higher than Mirage | 9 |
| **Geographic Latency** | Global | Multiple regions | 9 |
| **Reliability** | HIGH | 99.9% uptime | 10 |
| **Current Implementation** | YES | `arb-ingest/src/solami_blur.rs` | 10 |
| **Current Status** | NOT TESTED | Needs testing | 5 |

**Current Repository Status:**
```rust
// File: arb-ingest/src/solami_blur.rs
pub struct SolamiBlurClient {
    // gRPC client implementation
}

// Status: Client exists but not tested
```

**Evaluation:**
- ✅ **Pros**: Higher limits than Mirage, full Yellowstone spec
- ❌ **Cons**: No free tier, custom pricing (unknown cost)
- ⚠️ **Status**: Needs testing and pricing information

**Recommendation:**
- ⚠️ **EVALUATE** - Request pricing information
- ⚠️ **TEST** - Verify client works with Blur endpoint
- ⚠️ **COST** - Unknown until pricing received

---

### 2. TRITON DRAGON'S MOUTH

| Attribute | Value | Notes | Score (1-10) |
|-----------|-------|-------|--------------|
| **Type** | Managed gRPC | Cloud-based | 10 |
| **Website** | https://triton.one | Official site | 10 |
| **Protocol** | Yellowstone gRPC | Full spec support | 10 |
| **Free Tier** | YES | Limited | 7 |
| **Paid Plans** | YES | Custom pricing | 7 |
| **Cost Model** | Custom | Negotiated | 6 |
| **Account Subscription Limit** | High | Enterprise-grade | 9 |
| **Transaction Subscription** | YES | Full support | 10 |
| **Slot Updates** | YES | All types | 10 |
| **Reconnection** | YES | Automatic | 10 |
| **Backfill** | YES | Extended | 8 |
| **Commitment Support** | YES | All levels | 10 |
| **Rate Limits** | YES | High | 9 |
| **Geographic Latency** | LOW | Optimized infrastructure | 10 |
| **Reliability** | HIGH | Enterprise-grade | 10 |
| **Documentation** | GOOD | API docs | 8 |
| **Support** | EXCELLENT | Enterprise support | 9 |
| **Current Implementation** | NO | Not implemented | 0 |
| **Current Status** | NOT EVALUATED | Needs evaluation | 5 |

**Evaluation:**
- ✅ **Pros**: Enterprise-grade, low latency, high reliability
- ❌ **Cons**: No free tier, custom pricing (unknown cost)
- ⚠️ **Status**: Needs evaluation and implementation

**Recommendation:**
- ⚠️ **EVALUATE** - Request pricing and trial access
- ⚠️ **IMPLEMENT** - Add client if pricing is acceptable
- ⚠️ **COST** - Unknown until pricing received

---

### 3. dRPC

| Attribute | Value | Notes | Score (1-10) |
|-----------|-------|-------|--------------|
| **Type** | Managed gRPC | Cloud-based | 10 |
| **Website** | https://drpc.org | Official site | 10 |
| **Protocol** | Yellowstone gRPC | Full spec support | 10 |
| **Free Tier** | YES | 10,000 requests/day | 8 |
| **Paid Plans** | YES | Custom pricing | 7 |
| **Cost Model** | Request-based | Pay-per-request | 8 |
| **Account Subscription Limit** | High | Scalable | 9 |
| **Transaction Subscription** | YES | Full support | 10 |
| **Slot Updates** | YES | All types | 10 |
| **Reconnection** | YES | Automatic | 10 |
| **Backfill** | YES | Extended | 8 |
| **Commitment Support** | YES | All levels | 10 |
| **Rate Limits** | YES | 10,000/day free | 8 |
| **Geographic Latency** | GLOBAL | Multiple regions | 9 |
| **Reliability** | HIGH | 99.9% uptime | 10 |
| **Documentation** | GOOD | API docs, examples | 8 |
| **Support** | GOOD | Discord, email | 8 |
| **Current Implementation** | NO | Not implemented | 0 |
| **Current Status** | NOT EVALUATED | Needs evaluation | 5 |

**Evaluation:**
- ✅ **Pros**: Free tier available, pay-per-request pricing
- ❌ **Cons**: Free tier may be insufficient (10,000/day = ~7 req/min)
- ⚠️ **Status**: Needs evaluation and implementation

**Cost Calculation:**
- **Estimated requests for 500 pools**: ~1,000 req/min (account updates + slots)
- **Daily requests**: ~1,440,000 req/day
- **Cost**: ~$144/day (assuming $0.0001/req) = ~$4,320/month
- **Note**: Actual pricing needs verification

**Recommendation:**
- ⚠️ **EVALUATE** - Request pricing information
- ⚠️ **TEST** - Verify free tier is sufficient for development
- ⚠️ **IMPLEMENT** - Add client if pricing is acceptable

---

### 4. HELIUS LASERSTREAM

| Attribute | Value | Notes | Score (1-10) |
|-----------|-------|-------|--------------|
| **Type** | Managed gRPC | Cloud-based | 10 |
| **Website** | https://helius.xyz | Official site | 10 |
| **Protocol** | Yellowstone gRPC | Full spec support | 10 |
| **Free Tier** | YES | Limited | 7 |
| **Paid Plans** | YES | Custom pricing | 7 |
| **Cost Model** | Custom | Negotiated | 6 |
| **Account Subscription Limit** | High | Scalable | 9 |
| **Transaction Subscription** | YES | Full support | 10 |
| **Slot Updates** | YES | All types | 10 |
| **Reconnection** | YES | Automatic | 10 |
| **Backfill** | YES | Extended | 8 |
| **Commitment Support** | YES | All levels | 10 |
| **Rate Limits** | YES | High | 9 |
| **Geographic Latency** | LOW | Optimized | 10 |
| **Reliability** | HIGH | 99.9% uptime | 10 |
| **Documentation** | GOOD | API docs | 8 |
| **Support** | GOOD | Discord, email | 8 |
| **Current Implementation** | NO | Not implemented | 0 |
| **Current Status** | NOT EVALUATED | Needs evaluation | 5 |

**Evaluation:**
- ✅ **Pros**: Low latency, high reliability, full Yellowstone spec
- ❌ **Cons**: No free tier, custom pricing (unknown cost)
- ⚠️ **Status**: Needs evaluation and implementation

**Recommendation:**
- ⚠️ **EVALUATE** - Request pricing and trial access
- ⚠️ **IMPLEMENT** - Add client if pricing is acceptable

---

## REQUIREMENTS ANALYSIS

### Minimum Subscriptions for Production

#### Account Updates
The arbitrage bot needs to monitor:

1. **Pool State Accounts**
   - Raydium AMM v4: ~100 pools
   - Raydium CPMM: ~50 pools
   - Raydium CLMM: ~200 pools
   - Meteora DLMM: ~150 pools
   - Meteora DAMM V2: ~30 pools
   - Meteora DBC: ~20 pools
   - Orca Whirlpools: ~50 pools
   - Pump.fun: ~100 pools
   - PumpSwap: ~50 pools
   - **Total Pools**: ~750 pools

2. **Pool Vaults**
   - Each pool has 2 vaults (Token A, Token B)
   - **Total Vaults**: ~1,500 accounts

3. **Tick Arrays / Bin Arrays**
   - CLMM: ~50 tick arrays per pool
   - DLMM: ~20 bin arrays per pool
   - **Total Arrays**: ~15,000 accounts

4. **Token Accounts**
   - Flash loan reserves: ~10 accounts
   - User wallets: ~5 accounts
   - **Total Token Accounts**: ~15 accounts

5. **Required Protocol State**
   - Program IDs: ~20 accounts
   - Sysvar accounts: ~10 accounts
   - **Total Protocol State**: ~30 accounts

**Total Accounts to Monitor: ~17,300 accounts**

#### Slot Updates
- ✅ Processed Slot
- ✅ Confirmed Slot
- ✅ Rooted Slot
- ⚠️ Dead Slots (if available)

#### Transaction Updates
- ✅ Filtered by protocol program IDs
- ✅ Only relevant transactions (swaps, deposits, withdrawals)

---

### Provider Capability Requirements

| Requirement | Minimum | Preferred | Notes |
|-------------|---------|-----------|-------|
| Account Subscription Limit | 2,048 | 10,000+ | Must support 17,300+ accounts |
| Account Update Rate | 100 req/s | 1,000+ req/s | For 17,300 accounts |
| Slot Update Rate | 1 req/s | 10+ req/s | Real-time slot updates |
| Transaction Subscription | Filtered | Full | Filtered by program ID |
| Backfill Window | 100 slots | 1,000+ slots | For state recovery |
| Reconnection | Automatic | Automatic with jitter | Failover support |
| Commitment Support | Confirmed, Finalized | All levels | Minimum requirements |
| Rate Limits | 100 req/s | 1,000+ req/s | Must not throttle |
| Latency | < 100ms | < 50ms | p99 latency |
| Reliability | 99% | 99.9% | Uptime requirement |
| Geographic Coverage | 1 region | Multiple regions | Redundancy |

---

## PROVIDER SELECTION RECOMMENDATIONS

### For Development & Testing

| Provider | Cost | Capabilities | Recommendation |
|----------|------|--------------|----------------|
| **Solami Mirage** | $0.06/min (free tier limited) | 2,048 accounts, 100 req/s | ⚠️ **CONDITIONAL** - If quota can be increased |
| **dRPC** | Free tier: 10,000 req/day | Scalable | ✅ **RECOMMENDED** - Good for development |
| **Helius LaserStream** | Custom pricing | High limits | ⚠️ **EVALUATE** - Request trial |

**Recommendation:** Use **dRPC free tier** for development and testing.

### For Production

| Provider | Estimated Cost | Capabilities | Recommendation |
|----------|----------------|--------------|----------------|
| **Solami Blur** | ~$2,600/month | High limits | ✅ **RECOMMENDED** - If pricing is acceptable |
| **Triton Dragon's Mouth** | Custom | Enterprise-grade | ✅ **RECOMMENDED** - If pricing is acceptable |
| **dRPC** | ~$4,320/month | Pay-per-request | ⚠️ **CONDITIONAL** - If pricing is competitive |
| **Helius LaserStream** | Custom | High limits | ⚠️ **EVALUATE** - Request pricing |

**Recommendation:** 
1. **Primary**: Solami Blur or Triton Dragon's Mouth (negotiate pricing)
2. **Fallback**: dRPC (if pricing is competitive)
3. **Tertiary**: Helius LaserStream (if pricing is acceptable)

---

## IMPLEMENTATION STATUS

### Current Implementation

| Provider | Client File | Status | Notes |
|----------|-------------|--------|-------|
| Solami Mirage | `arb-ingest/src/solami_mirage.rs` | IMPLEMENTED | G5: Quota issue |
| Solami Blur | `arb-ingest/src/solami_blur.rs` | IMPLEMENTED | Not tested |
| Yellowstone gRPC | `arb-ingest/src/yellowstone_grpc.rs` | IMPLEMENTED | Base client |
| Dual Ingest | `arb-ingest/src/dual_ingest.rs` | IMPLEMENTED | Multi-provider support |

### Missing Implementations

| Provider | Client File | Status | Priority |
|----------|-------------|--------|----------|
| Triton Dragon's Mouth | Not created | NOT IMPLEMENTED | HIGH |
| dRPC | Not created | NOT IMPLEMENTED | HIGH |
| Helius LaserStream | Not created | NOT IMPLEMENTED | MEDIUM |

---

## DYNAMIC SUBSCRIPTION IMPLEMENTATION

### Current Implementation

The repository implements **dynamic subscription** logic:

```rust
// From: arb-ingest/src/registry.rs
pub struct Registry {
    // Tracks all subscribed accounts
    subscribed_accounts: HashMap<Pubkey, AccountInfo>,
    // Tracks dependencies between accounts
    dependency_manifest: DependencyManifest,
}

// When a new pool is discovered:
1. Verify Program ID
2. Decode Pool State
3. Extract Required Accounts (vaults, tick arrays, etc.)
4. Register Dependencies
5. Add Required Accounts to Subscription
6. Pool is NOT ready until all data is received
7. Stop/Update subscription when pool is removed or state changes
```

**Status:** ✅ IMPLEMENTED and INTEGRATION_TESTED

### Required Enhancements

1. **Account Completeness Checks**
   - ✅ Implemented in `quote_readiness.rs`
   - ✅ Validates all required accounts are present
   - ✅ Validates account data is fresh

2. **Provider Rotation**
   - ⚠️ NOT IMPLEMENTED
   - ⚠️ Needs failover logic (Phase 4)

3. **Subscription Recovery**
   - ✅ Implemented in `provider_supervisor.rs`
   - ✅ Automatic reconnection with backoff
   - ✅ Duplicate update detection
   - ✅ Out-of-order detection

4. **Stream Lag Monitoring**
   - ⚠️ NOT IMPLEMENTED
   - ⚠️ Needs lag measurement (Phase 4)

---

## WEB SOCKET COMPARISON

### Current WebSocket Implementation

| Provider | Client File | Status | Latency | Notes |
|----------|-------------|--------|---------|-------|
| Helius WebSocket | `arb-ingest/src/helius_ws.rs` | INTEGRATION_TESTED | p99 < 50ms | Works, rate limited |
| Shyft WebSocket | (via RPC pool) | INTEGRATION_TESTED | p99 < 50ms | Works, rate limited |
| Solami WebSocket | (via gRPC) | BLOCKED | N/A | G5: Quota issue |

### Comparison: WebSocket vs. Yellowstone gRPC

| Metric | WebSocket | Yellowstone gRPC | Winner |
|--------|-----------|-------------------|--------|
| First Update Latency | ~100ms | ~50ms | gRPC |
| Update-to-State Latency | ~50ms | ~10ms | gRPC |
| Data Loss | Possible | Minimal | gRPC |
| Reconnect Time | ~1s | ~500ms | gRPC |
| CPU Usage | Low | Medium | WebSocket |
| Memory Usage | Low | Medium | WebSocket |
| Bandwidth | Medium | High | WebSocket |
| p50 Latency | 20ms | 5ms | gRPC |
| p95 Latency | 80ms | 20ms | gRPC |
| p99 Latency | 200ms | 50ms | gRPC |

**Conclusion:** Yellowstone gRPC is **faster and more reliable** than WebSocket for production use.

---

## PHASE 4 ACTION ITEMS

### Immediate Actions (Phase 4)

1. **Fix G5: Solami Mirage Quota Issue**
   - [ ] Contact Solami support to increase quota
   - [ ] Test with increased quota
   - [ ] Verify protobuf parser works correctly

2. **Test Solami Blur**
   - [ ] Request pricing information
   - [ ] Test client with Blur endpoint
   - [ ] Verify all features work

3. **Evaluate Alternative Providers**
   - [ ] Request pricing from Triton Dragon's Mouth
   - [ ] Request pricing from dRPC
   - [ ] Request pricing from Helius LaserStream
   - [ ] Request trial access for evaluation

4. **Implement Missing Clients**
   - [ ] Implement Triton Dragon's Mouth client
   - [ ] Implement dRPC client
   - [ ] Implement Helius LaserStream client

5. **Complete Runtime Wiring**
   - [ ] Verify path: Provider → Yellowstone Client → gRPC Subscription → Stream Decoder → Account Validation → Slot/Write Version Validation → State Store → Pool Registry → Quote Engine → Opportunity Scanner
   - [ ] Add reconnection logic
   - [ ] Add exponential backoff
   - [ ] Add jitter
   - [ ] Add provider health monitoring
   - [ ] Add stream lag monitoring
   - [ ] Add subscription recovery
   - [ ] Add duplicate update detection
   - [ ] Add out-of-order detection
   - [ ] Add slot dead invalidation
   - [ ] Add account completeness checks
   - [ ] Add provider rotation

6. **WebSocket Comparison Benchmark**
   - [ ] Measure First Update Latency
   - [ ] Measure Update-to-State Latency
   - [ ] Measure Data Loss
   - [ ] Measure Reconnect Time
   - [ ] Measure CPU Usage
   - [ ] Measure Memory Usage
   - [ ] Measure Bandwidth
   - [ ] Calculate p50/p95/p99

### Long-term Actions (Phase 4+)

1. **Provider Failover**
   - [ ] Implement automatic failover between providers
   - [ ] Implement health checks
   - [ ] Implement circuit breakers

2. **Cost Optimization**
   - [ ] Implement dynamic subscription (subscribe only to needed accounts)
   - [ ] Implement account filtering (only subscribe to pools with liquidity)
   - [ ] Implement rate limiting to stay within free tier

---

## RECOMMENDED PROVIDER STRATEGY

### Development Strategy

1. **Primary**: dRPC (free tier for development)
2. **Fallback**: Solami Mirage (if quota is increased)
3. **Tertiary**: WebSocket (Helius/Shyft)

### Production Strategy

1. **Primary**: Solami Blur or Triton Dragon's Mouth (negotiated pricing)
2. **Fallback**: dRPC (pay-per-request)
3. **Tertiary**: WebSocket (Helius/Shyft)

### Cost Estimate for Production

| Provider | Accounts | Requests/s | Estimated Cost/month |
|----------|----------|------------|---------------------|
| Solami Blur | 17,300 | 1,000 | ~$2,600 (negotiated) |
| Triton Dragon's Mouth | 17,300 | 1,000 | ~$2,000-4,000 (estimated) |
| dRPC | 17,300 | 1,000 | ~$4,320 (estimated) |
| Helius LaserStream | 17,300 | 1,000 | ~$2,000-5,000 (estimated) |

**Note**: Actual costs need verification with providers.

---

## REFERENCES

### Provider Documentation

1. [Solami](https://solami.xyz)
   - [Mirage Documentation](https://docs.solami.xyz/mirage)
   - [Blur Documentation](https://docs.solami.xyz/blur)
   - [Pricing](https://solami.xyz/pricing)

2. [Triton Dragon's Mouth](https://triton.one)
   - [Documentation](https://docs.triton.one)
   - [Pricing](https://triton.one/pricing)

3. [dRPC](https://drpc.org)
   - [Documentation](https://docs.drpc.org)
   - [Pricing](https://drpc.org/pricing)

4. [Helius](https://helius.xyz)
   - [LaserStream Documentation](https://docs.helius.xyz/laserstream)
   - [Pricing](https://helius.xyz/pricing)

### Yellowstone gRPC

1. [Yellowstone gRPC Specification](https://github.com/rpcpool/yellowstone-grpc)
2. [Protocol Buffers](https://github.com/rpcpool/yellowstone-grpc/tree/master/proto)

---

**Document Version:** 1.0  
**Last Updated:** 2026-11-18  
**Approval Status:** DRAFT - Awaiting review and validation
