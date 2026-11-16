# Phase 8 — Transaction Lifecycle and Blockhash Management Audit

## 8.1 Transaction State Machine

### 8.1.1 State Definitions

The transaction lifecycle must distinguish these states clearly:

| State | Meaning | Transition Conditions |
|---|---|---|
| **Constructed** | Transaction object created with all instructions | After `transaction_builder.build()` |
| **Signed** | Transaction has valid Ed25519 signature | After `signer.sign()` applied |
| **Simulated** | Transaction executed in simulated environment | After `simulate_transaction()` succeeds |
| **Submitted** | Transaction sent to block engine (bundle or direct) | After `bundle_submission.submit()` or `rpc_send()` |
| **Processed** | Block engine has processed the transaction | After `processed` status from RPC |
| **Confirmed** | Transaction included in a block | After `confirmed` commitment |
| **Finalized** | Transaction irreversible (cannot be rolled back) | After `finalized` commitment |
| **Failed** | Transaction execution failed (revert, insufficient funds, etc.) | After error status from block engine |
| **Expired** | Transaction exceeded `lastValidBlockHeight` | After blockhash freshness exceeded |
| **Unknown** | Status not recognized by monitoring system | Default/fallback state |

### 8.1.2 State Transitions

```
Constructed → Signed → Simulated → Submitted → Processed → Confirmed → Finalized
      ↑              ↑              ↑              ↑
   (fail)         (fail)         (fail)         (fail)
      ↓              ↓              ↓              ↓
   Expired       Expired         Expired        Failed
```

**Key invariant**: `Submitted ≠ Processed ≠ Confirmed ≠ Finalized`

- A transaction may be `Submitted` but not yet `Processed`
- A transaction may be `Processed` but not yet `Confirmed`
- A transaction may be `Confirmed` but not yet `Finalized`
- Bundle ID ≠ Landed transaction (bundle may contain multiple txes with different fates)

### 8.1.3 Blockhash Lifecycle

**Freshness requirement**: 
- Blockhash must be fetched < 1 slot ago (~400ms)
- Must use `lastValidBlockHeight` to detect expiration
- Must not use zeroed blockhashes (`[0u8; 32]`)

**Blockhash cache**:
- TTL: 3 seconds (configurable)
- Cache hit: reuse cached blockhash (saves RPC round-trip)
- Cache miss: fetch fresh blockhash from RPC
- After cache TTL expires: fetch new blockhash before submitting

**LastValidBlockHeight**:
- Every blockhash carries `lastValidBlockHeight`
- Transaction must be submitted before this slot
- Must check `current_slot > lastValidBlockHeight` → expired
- Must not submit after `lastValidBlockHeight` → transaction fails

**Stale RPC nodes**:
- If RPC returns old blockhash → detect via `lastValidBlockHeight` comparison
- If blockhash is stale → fetch fresh one; do not submit with stale hash

### 8.1.4 State Machine Implementation

The supervisor must track and transition through states:

```rust
enum TxStatus {
    Constructed,
    Signed,
    Simulated(SimulationResult),
    Submitted { bundle_id: Option<String>, submission_time: Instant },
    Processed { slot: Option<u64>, commitment: Commitment },
    Confirmed { slot: u64, confirmation_time: Instant },
    Finalized { finalized_at: Instant },
    Failed { error: TxError, failure_time: Instant },
    Expired { blockhash_time: Instant },
    Unknown,
}
```

**Transitions must be fail-closed**: Any unexpected state → `Failed` or `Expired`, never stay in ambiguous state.

## 8.2 Blockhash Management

### 8.2.1 Current Implementation

The supervisor maintains a blockhash cache:

```rust
blockhash_cache: parking_lot::Mutex<Option<([u8; 32], Instant)>>,
```

**Cache behavior**:
- Store blockhash + `Instant` when fetched
- On cache hit (within TTL): reuse cached blockhash
- On cache miss or TTL expiration: fetch fresh blockhash
- TTL: ~3 seconds (configurable; Solana blockhashes expire after ~60-90s but the cache TTL is separate)

**RTT tracking**: 
```rust
rtt: parking_lot::Mutex::new((RttTracker::new(300.0), None)),
```
- Tracks round-trip time to RPC
- EWMA smoothing with 300ms baseline

### 8.2.2 Blockhash Freshness Checks

Before submitting a transaction, verify:

1. **Blockhash not zero**: `blockhash != [0u8; 32]`
2. **Within lastValidBlockHeight**: `current_slot <= blockhash.lastValidBlockHeight`
3. **Recent enough**: `current_slot - blockhash_slot <= max_age_slots` (configurable, default 2)
4. **Not cached too long**: If using cached blockhash, verify `instant.elapsed() < cache_ttl`

**Fail-closed behavior**: If any check fails → fetch new blockhash → if fetch fails → fail closed (do not submit).

### 8.2.3 Blockhash Rotation

- **Rotate before TTL expiration**: Rotate blockhash after ~45 seconds (15s before actual expiry)
- **Rotate on slot change**: When new slot starts, fetch fresh blockhash
- **Never reuse expired blockhash**: Would cause transaction simulation/signing to fail or transaction to be rejected

### 8.2.4 RPC Credential Failure

- If RPC endpoint fails to fetch blockhash → switch to fallback provider
- If both primary and fallback fail → fail closed (no trading until RPC recover)
- Never guess or zero-fill blockhash when RPC fails

## 8.3 Transaction Validity Checks

### 8.3.1 Preflight Checks (Before Signing)

1. **Mode gate**: `mode == Live` gated by `ARB_LIVE` env var; shadow/simulate modes skip live execution
2. **Safety floor**: Balance ≥ 50,000,000 lamports (0.05 SOL) else skip execution
3. **Self-sweep guard**: Cold wallet ≠ signer (skip self-sweep)
4. **MTU check**: tx_bytes ≤ 1232 bytes else skip
5. **Preflight simulation**: LiteSVM local + RPC simulation; RPC error → fail closed
6. **Final stream readiness**: `validate_execution_readiness` (connected, generation, slot, age)
7. **Kill switch**: Durable kill switch marker → all execution blocked
8. **Circuit breaker**: Open circuit → all execution blocked; cooldown before resume
9. **Risk gate**: All risk checks must pass (trade size, price impact, profit, loss limits)
10. **Duplicate execution**: Check if same transaction already submitted and unconfirmed

### 8.3.2 Preflight Simulation

- **LiteSVM local simulation**: Preferred; deterministic, no RPC dependency
- **RPC simulateTransaction**: Fallback if LiteSVM unavailable
- **RPC error → fail closed**: If RPC simulation fails, do not submit
- **Simulation result**: Must be reproducible across runs with identical input

**Gap**: The codebase mentions LiteSVM simulation but may not have full integration. The `arb-sim` crate exists but integration with the supervisor may be partial.

### 8.3.2 Blockhash Freshness Gates

- **max_state_age_slots** (default 2): State age must be within 2 slots
- **Blockhash age**: Blockhash must be < 1 slot old (~400ms)
- **LastValidBlockHeight**: Current slot must be ≤ lastValidBlockHeight
- **Generation changes**: After Yellowstone generation change, re-derive account filters

**Fail-closed**: Any gate fails → do not submit transaction; reauthorize on next cycle.

## 8.4 Retry and Resend Logic

### 8.4.1 Bounded Retry

- **Max retry attempts**: 3 attempts for simulation, 5 attempts for bundle submission
- **Exponential backoff**: Between retries (e.g., 1s, 2s, 4s)
- **Idempotency**: Do not create new signed transaction while previous may still be valid

### 8.4.2 Resend Logic

- **Check original transaction status**: Before resending, query `getInflightBundleStatuses` or `getSignatureStatuses`
- **If original landed**: Do not resend; profit already realized (or loss already taken)
- **If original failed**: May resend with same or adjusted parameters (within risk limits)
- **If original expired**: Fetch new blockhash; resubmit with fresh hash
- **If unknown**: May retry once with fresh blockhash; if still unknown → give up

### 8.4.3 Duplicate Execution Prevention

- **Track submitted bundle IDs**: Maintain set of in-flight bundle IDs
- **If bundle ID already in flight**: Do not create new signed transaction
- **If bundle ID terminal**: May resubmit with new transaction if strategy requires
- **Same transaction hash**: Do not resubmit identical transaction; varies tip or blockhash if resubmitting

## 8.5 Summary of Phase 8 Findings

### Correctness
- Supervisor tracks blockhash cache with TTL
- RTT tracked for adaptive CU pricing
- Previous tips tracked for escalation capping
- State age checked via `max_state_age_slots`
- Kill switch durable (survives restart)
- Circuit breaker with configurable failures/cooldown
- Safety floor: 0.05 SOL balance check
- MTU check: tx_bytes ≤ 1232 bytes

### Issues Identified

1. **No complete state machine**: The supervisor tracks many state elements but does not have a formal, documented state machine with all transitions explicitly defined and tested.

2. **Blockhash cache TTL not enforced in code**: The cache exists but the TTL expiration check may not be actively enforced before every submission.

3. **Preflight simulation integration**: LiteSVM simulation mentioned but integration path with supervisor may be partial; RPC fallback "unavailable until provider-native gRPC/WebSocket simulation path is implemented" (supervisor.rs line 169-170).

4. **No duplicate execution prevention**: No explicit tracking to prevent submitting the same transaction twice unless strategy accounts for it.

5. **Blockhash rotation not documented**: No guidance on when to rotate blockhash (before TTL expiration, on slot change).

6. **RPC simulation fallback**: Code states "HTTP JSON-RPC is disabled by architecture. Simulation remains unavailable until a provider-native gRPC/WebSocket simulation path is implemented." This is a significant gap — without simulation, preflight validation cannot happen.

7. **Generation change handling**: After Yellowstone generation change, account filters must be re-derived; no explicit gate in the supervisor for this.

8. **RPC credential failure**: If both primary and fallback RPC fail, code fails closed but the user experience may not be clear.

### Required Actions

1. **Document formal transaction state machine** with all states and transitions
2. **Enforce blockhash cache TTL** before every submission
3. **Implement LiteSVM or RPC simulation integration** (the critical gap — without simulation, preflight validation cannot happen)
4. **Add duplicate execution prevention** via inflight bundle ID tracking
5. **Document blockhash rotation policy** (when to fetch new blockhash)
6. **Add generation change gate**: Re-derive account filters after Yellowstone generation change
7. **Improve RPC failure handling**: Clear error messages and graceful degradation
8. **Test all state transitions** with integration tests

### Phase 8 Hardcoded Value Classification

| Value | Classification | justification |
|---|---|---|
| `BASE_FEE_LAMPORTS = 5_000` | Operational configuration | Base per-sig fee; configurable via runtime |
| `PRIORITY_FEE_MAX_SHARE_BPS = 2_000` | Risk gate configuration | Max priority fee as proportion of edge |
| `blockhash_cache TTL ~3s` | Operational configuration | Empirical; not hardcoded constant but observed behavior |
| `RttTracker baseline 300.0ms` | Operational configuration | Round-trip time baseline for CU pricing |
| `SlotClock` | Protocol-aware | Tracks slot monotonicity |
| `RttTracker(300.0)` | Performance baseline | EWMA tracker initialization |
| Health monitor checks | Operational | Disk, gRPC, Telegram health |
| `previous_tips` map key `(u64, u32)` | Operational | (slot, rung) → tip lamports |

## 8.5 Next Phase Prerequisites

Phase 9 (Risk Engine) depends on:
- Formal state machine documented
- Blockhash freshness gates implemented
- Simulation pipeline operational
- Duplicate execution prevention active

---
*Note: Phase 8 identifies critical gaps in transaction lifecycle management. The absence of a formal state machine and the simulation unavailability are blocking production readiness.*