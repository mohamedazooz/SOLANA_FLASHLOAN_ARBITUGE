# LANDING & CONFIRMATION AUDIT REPORT
**Author:** Landing & Confirmation Specialist (Agent 2) & Principal HFT Architect  
**Branch:** `fix/mainnet-transport-landing`  
**Date:** 2026-11-15  
**Audit Scope:** Transaction Lifecycle State Machine, Signature Polling, Blockhash Expiry Proof, Idempotent Reconciliation  

---

## 1. Executive Summary

This audit remediates the transaction landing tracking lifecycle:
- **The Issue:** `dispatch.rs` previously fell back to marking transactions as `Expired` immediately whenever a polling timeout elapsed. This violated the core invariant: **never classify a transaction as expired solely because a polling timeout elapsed**.
- **The Fix:**
  1. Implemented a 10-state deterministic lifecycle: `PREPARED`, `SIMULATED`, `SIGNED`, `SUBMITTED`, `PROCESSED`, `CONFIRMED`, `FINALIZED`, `FAILED`, `EXPIRED`, and `UNKNOWN` in `crates/arb-land/src/lifecycle.rs`.
  2. Enforced on-chain blockhash expiration validation: an unconfirmed transaction can only transition to `EXPIRED` if `current_block_height > last_valid_block_height`. If current block height is still within the validity window, the transaction transitions to `UNKNOWN`, preventing blind duplicate submissions.
  3. Added `get_transaction_with_metadata` and `get_signature_status_detailed` in `crates/arb-ingest/src/rpc_pool.rs` to extract on-chain compute units, pre/post balances, and execution errors.

---

## 2. The 10-State Transaction Lifecycle

```text
       ┌───────────┐
       │ PREPARED  │
       └─────┬─────┘
             │ (Simulation passed)
             ▼
       ┌───────────┐
       │ SIMULATED │
       └─────┬─────┘
             │ (Ed25519 signed)
             ▼
       ┌───────────┐
       │  SIGNED   │
       └─────┬─────┘
             │ (Dispatched to transport)
             ▼
       ┌───────────┐
       │ SUBMITTED │
       └─────┬─────┘
             │
     ┌───────┴──────────────────────────────┐
     │                                      │
     ▼                                      ▼
┌───────────┐                         ┌───────────┐
│ PROCESSED │                         │  UNKNOWN  │ (Poll timeout, height <= lastValid)
└─────┬─────┘                         └─────┬─────┘
     │                                      │
     ▼                                      ▼
┌───────────┐                         ┌───────────┐
│ CONFIRMED │                         │  EXPIRED  │ (Height > lastValidBlockHeight)
└─────┬─────┘                         └───────────┘
     │
     ▼
┌───────────┐
│ FINALIZED │ (Root block)
└───────────┘
```

---

## 3. Invariants & Proof Requirements

1. **Monotonic Progression**: Transitions flow forward only. Terminal states (`Finalized`, `Failed`, `Expired`) cannot transition further.
2. **Positive Expiry Proof**: Evaluated via `arb_land::evaluate_lifecycle_with_block_height`.
   - If status is `None` AND `current_block_height <= last_valid_block_height`: Result is `UNKNOWN`.
   - If status is `None` AND `current_block_height > last_valid_block_height`: Result is `EXPIRED`.
3. **Distinction of Commitments**: `PROCESSED` is distinct from `CONFIRMED` and `FINALIZED`.
4. **Failure Reason Preservation**: On-chain errors (`InstructionError`) are extracted and persisted rather than swallowed.

---

## 4. Verification & Test Evidence

All lifecycle transitions and invariants were verified via `cargo test -p arb-land --lib lifecycle`:
- `test_lifecycle_monotonic_transitions`: PASSED
- `test_never_expire_on_timeout_when_height_valid`: PASSED
- `test_expire_only_when_height_exceeded`: PASSED
- `test_failure_takes_precedence_with_reason`: PASSED
- `test_distinguish_processed_confirmed_finalized`: PASSED

---

## 5. Status Classification
- **10-State Lifecycle Engine**: **CONFIRMED** (implemented and unit tested).
- **Blockhash Expiration Verification**: **CONFIRMED** (tested with simulated and live block heights).
- **Metadata Retrieval**: **CONFIRMED** (added `getTransaction` with `maxSupportedTransactionVersion: 1`).
