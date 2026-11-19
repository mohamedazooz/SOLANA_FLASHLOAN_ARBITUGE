# End-to-End Execution Path Map

## Complete Execution Pipeline: Signal to Landed Arbitrage

```mermaid
sequenceDiagram
    autonumber
    participant Yellowstone as Yellowstone gRPC / RPC
    participant Supervisor as Provider Supervisor
    participant Decoders as Decoders (DLMM/Raydium/Orca)
    participant Registry as Pool Registry & Ingest
    participant Engine as Engine (Discovery & Sizing)
    participant Math as arb-math (Kamino + Swap Quotes)
    participant Risk as Risk Gate (20 Checks)
    participant TxBuilder as Tx Compiler (V0 ALT / V1)
    participant Simulator as Preflight Simulation
    participant Land as Jito Land Subsystem
    participant Settlement as Realized Settlement & Recovery

    Yellowstone->>Supervisor: Account / Slot Streaming Update
    Supervisor->>Decoders: Raw bytes + slot + bank ID
    Decoders->>Registry: Decoded PoolState (reserve depth, active bin, tick)
    Registry->>Engine: Trigger scan on updated pairs
    Engine->>Math: Compute exact quotes & Klend fee for route candidates
    Math-->>Engine: Canonical Net Profit Quote
    Engine->>Risk: Submit OpportunityIntent for pre-authorization
    Risk-->>Engine: Authorization Status (PASS / CIRCUIT_BREAKER)
    Engine->>TxBuilder: Build Flashloan Arbitrage Instruction Sequence
    TxBuilder->>Simulator: Preflight Simulation (simulateTransaction / LiteSVM)
    Simulator-->>TxBuilder: Simulation Result (Consumed CU, balance delta)
    TxBuilder->>Land: Hand off Signed V0/V1 Transaction Bundle
    Land->>Jito: Submit Bundle with Dynamic Tip Floor
    Jito-->>Land: Bundle UUID & Inflight Status
    Land->>Settlement: Signature confirmation loop & recovery tracking
    Settlement-->>Risk: Realized Balance Delta & PnL Feedback
```

## Detailed Path Invariants

### 1. Discovery Path
- **Entry**: `arb-bot/src/supervisor/strategy.rs` -> `arb-engine/src/pipeline.rs`.
- **Mint Continuity**: Hop 1 output == Hop 2 input; Hop 2 output == Hop 3 input; Hop 3 output == Hop 1 input.
- **Cycle Detection**: No duplicate pool instances in a single cycle.

### 2. Sizing & Economics Path
- **Entry**: `arb-engine/src/sizing.rs` & `arb-math/src/economic.rs`.
- **Constraint**: Borrow size strictly bounded by supply vault liquidity in live reserve account.
- **Kamino Repayment Formula**: `repayment = principal + fee`, where `fee_bits = max(amount * sf, 2^60)` pre-rounding.
- **Net Profit Model**: `Gross Output - Borrow Principal - Flash Fee - DEX Fees - Jito Tip - Priority Fee - Base Lamports - Token Transfer Fees`. Authoritative decisions strictly prohibit floating point arithmetic.

### 3. Risk Authorization Path
- **Entry**: `arb-risk/src/policy.rs` & `arb-risk/src/circuit_breaker.rs`.
- **20-Point Check**:
  1. Kill Switch status (durable).
  2. Global execution authorization (`ARB_LIVE=1` required, default 0).
  3. Circuit breaker state (consecutive loss ceiling).
  4. Hot wallet health (minimum operational balance reserved).
  5. Available capital & trade size ceiling.
  6. Route freshness (maximum allowable slot age).
  7. State coherence across hop banks.
  8. Price impact ceiling.
  9. Expected net profit floor (`MIN_NET_PROFIT_USD`).
  10. Maximum Jito tip budget (max 30% of profit, hard ceiling).
  11. Total execution cost limit.
  12. Daily loss limit.
  13. Hourly loss limit.
  14. Inflight bundle capacity.
  15. Provider health & slot lag.
  16. Minimum landing probability threshold.
  17. Transaction validity & wire size.
  18. Duplicate execution prevention.
  19. Account collision check.
  20. Final dispatch readiness gate.

### 4. Transaction Assembly Path
- **Entry**: `arb-tx/src/compiler.rs` & `arb-tx/src/flash/kamino.rs`.
- **Instruction Ordering**:
  1. Compute Budget (SetComputeUnitLimit + SetComputeUnitPrice) [V0 mode only; stripped in V1].
  2. Kamino Flash Borrow (`KlendInstruction::FlashBorrowReserveLiquidity`).
  3. Hop 1 Swap (Meteora DLMM / Raydium / Orca).
  4. Hop 2 Swap.
  5. [Optional] Hop 3 Swap.
  6. Kamino Flash Repay (`KlendInstruction::FlashRepayReserveLiquidity`, referencing borrow index).
  7. Jito Tip Transfer (to dynamic tip account selected by slot).
- **Atomicity Guard**: Zero bundle-splitting. All swaps and repayment enclosed in a single transaction.

### 5. Settlement & Recovery Path
- **Entry**: `arb-bot/src/supervisor/recovery.rs`.
- **Principle**: Ambiguous dispatch ≠ failure. On timeout, check signature status; only mark expired if current slot exceeds `last_valid_block_height + 160`. Never blindly re-sign.
