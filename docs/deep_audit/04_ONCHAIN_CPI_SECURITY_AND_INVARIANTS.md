# On-Chain CPI Security & Invariant Verification Report
**Module Focus:** `programs/arb-executor`, `crates/arb-risk`, `crates/arb-bot/src/supervisor/dispatch.rs`  
**Auditor:** Security Architecture Lead / Supervisor  

---

## 1. On-Chain Smart Contract Audit (`programs/arb-executor`)

### Program Structure
- **Program ID:** `ArbExec1111111111111111111111111111111111111`
- **Entrypoint:** `process_instruction`
- **Discriminator:** 8-byte Anchor hash `[173, 221, 98, 44, 189, 21, 102, 184]` (`execute_three_leg`)

### Security Invariants Grounded in Code (`instructions/execute_three_leg.rs`)

#### Invariant 1: Guaranteed Sequential Leg Coverage
```rust
if args.leg1_min_amount_out < args.leg2_amount_in {
    return Err(ExecutorError::Leg2InputExceedsLeg1Output.into());
}
if args.leg2_min_amount_out < args.leg3_amount_in {
    return Err(ExecutorError::Leg3InputExceedsLeg2Output.into());
}
if args.leg3_min_amount_out < args.min_final_wsol_delta {
    return Err(ExecutorError::FinalWsolDeltaBelowMinimum.into());
}
```
**Impact:** It is mathematically impossible for the contract to execute if any leg's minimum guaranteed output does not fully cover the subsequent leg's required input.

#### Invariant 2: Intermediate Inventory Isolation (Anti-Subsidization)
A common vulnerability in flawed arbitrage bots is that a bad trade drains existing tokens in the bot's wallet to "succeed" on-chain.
In `execute_three_leg.rs`:
```rust
let meme_a_post2 = read_token_account_balance(user_meme_a_account)?;
if meme_a_post2 < meme_a_initial {
    return Err(ExecutorError::IntermediateInventorySubsidized.into());
}
```
And in `verify_sequential_balance_transitions`:
```rust
if meme_b_final < meme_b_initial {
    return Err(ExecutorError::IntermediateInventorySubsidized);
}
```
**Impact:** If intermediate token A or B's balance after the swap is less than its initial balance, the transaction immediately reverts with `IntermediateInventorySubsidized`. Zero wallet subsidization is permitted.

#### Invariant 3: Net Profit & Flash Loan Repayment Floor
```rust
let required_final_wsol = wsol_baseline
    .checked_add(args.min_final_wsol_delta)
    .ok_or(ExecutorError::CalculationOverflow)?;
if wsol_final < required_final_wsol {
    return Err(ExecutorError::FinalWsolDeltaBelowMinimum);
}
```
**Impact:** The final WSOL balance of the trader MUST equal or exceed the pre-borrow baseline plus the required repayment delta. If the market moved unfavorably, the entire transaction reverts atomically.

---

## 2. Protocol & Account Verification

- **Distinct Pools:** `validate_distinct_pools(leg1, leg2, leg3)` ensures no duplicated pools in the cycle.
- **Authority Validation:** Validates that `user` is a signer and owns all intermediate token accounts (`validate_user_token_account`).
- **Program ID Pinning:** `validate_protocol_programs` verifies that the `dlmm_program`, `memo_program`, and `event_authority` match Meteora's canonical mainnet addresses, preventing malicious program substitution.

---

## 3. Off-Chain Risk Gates & Kill-Switch (`crates/arb-risk`)

### CircuitBreaker
- Configured with `max_consecutive_failures` (default: 8) and `cooldown_slots` (default: 10).
- If consecutive bundle submissions fail due to block reorganization or sudden volatility, the circuit breaker opens (`BreakerState::Open`), halting trading until conditions stabilize.

### Persistent Kill-Switch
- Persisted to a durable marker file on disk.
- Can be activated instantly via web API (`/pause`) or Telegram bot command.
- **Strict Invariant:** An autonomous agent or LLM can **only tighten** risk limits; it can **never widen** limits or disable the kill-switch while `ARB_LIVE=1`.
