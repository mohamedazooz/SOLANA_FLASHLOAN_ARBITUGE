> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Kamino Flash Loan Program Forensic Verification

## 1. On-Chain Deployment & Identity
- **Program ID**: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (`KAMINO_KLEND_PROGRAM_ID`)
- **Main Market**: `7u3HeHxatcAACvdFtHGgahLizUCkWKEmmbmjfcr2AvZh`
- **Main Market Authority PDA**: `9DrvZvyWh1HuAoZxvYWMvkf2XCzryBpGnnCxPDePpmG` (seeds: `[b"lma", lending_market]`)
- **WSOL Reserve**: `d4A2AQJnd5xGLVAQ6MrVU5npb2W1W2P2HgHK7Ze36GL`
- **WSOL Supply Vault**: `GafCuGyoXRbQiCLG2uG2gqQ887b2h8a75e3U6gR1XQJp`
- **WSOL Fee Vault**: `2aHqR2R5xQZp8QyZ7g6p2Uv9jR2FvG8GvP1F1zQYmF8`

## 2. Instruction Discriminators & Payload Layouts
- **Flash Borrow Discriminator**: `[142, 114, 213, 90, 143, 89, 137, 26]` (8 bytes)
  - Data: `[discriminator (8B), amount_u64_le (8B)]` -> Total 16 bytes.
- **Flash Repay Discriminator**: `[175, 175, 109, 31, 13, 152, 155, 237]` (8 bytes)
  - Data: `[discriminator (8B), amount_u64_le (8B), borrow_instruction_index (1B)]` -> Total 17 bytes.

## 3. Account Metas Matrix (12 Accounts)
Both borrow and repay require the exact Anchor sequence:
1. `user_authority` (Signer, Readonly)
2. `lending_market_authority` (Readonly)
3. `lending_market` (Readonly)
4. `reserve` (Writable)
5. `reserve_liquidity_mint` (Readonly)
6. `reserve_supply_vault` (Writable - source on borrow, destination on repay)
7. `user_liquidity_ata` (Writable - destination on borrow, source on repay)
8. `reserve_fee_vault` (Writable)
9. `referrer_token_state` (Readonly, default to klend program ID if none)
10. `referrer_account` (Readonly, default to klend program ID if none)
11. `sysvar_instructions` (Readonly, `Sysvar1nstructions1111111111111111111111111`)
12. `token_program` (Readonly, `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`)

## 4. Borrow Instruction Index Dynamism
Kamino verifies that the repayment transaction contains the corresponding borrow instruction by checking `sysvar_instructions` at `borrow_instruction_index`.
- In V0 transactions with Compute Budget instructions: `borrow_instruction_index = 2` (or position after `SetComputeUnitLimit` and `SetComputeUnitPrice`).
- In V1 transactions: Compute Budget instructions are stripped from message payloads into the header config mask; the compiler automatically decrements `borrow_instruction_index = 0`.
- Verified by: `crates/arb-tx/tests/v1_property_fuzz.rs` (`test v1_rewrites_klend_repay_borrow_index_after_compute_budget_strip`).

## 5. Liquidity Sizing Invariant
The maximum borrow amount is strictly clamped by the actual available liquidity in `reserve_supply_vault`:
$$\text{borrow\_amount} \le \text{vault\_balance} \times \text{MAX\_VAULT\_UTILIZATION\_BPS} / 10000$$
If supply vault balance cannot be verified in real time, the opportunity is rejected immediately (fail-closed).
