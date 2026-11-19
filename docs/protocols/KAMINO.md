# PROTOCOL SPECIFICATION — KAMINO FLASH LOANS (`klend`)

## 1. Protocol Identity & Addresses

- **Program ID:** `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
- **Main Lending Market:** `7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF`
- **Lending Market Authority PDA:** `seeds = [b"lma", lending_market.as_ref()]` -> `9DrvZvyWh1HuAoZxvYWMvkf2XCzryCpGgHqrMjyDWpmo`
- **WSOL Reserve Account:** `d4A2AQFBACydagHKrGDVUghAaMLiYCqcmx7MtEPFFG2`
- **WSOL Supply Vault:** `GafNuEbGPGHrxco9mEHhWqZPk7ECU3mVoAwpibEseQCq`
- **WSOL Fee Vault:** `4ZhvytBvjB7zK4T1fG3YwKj9V3gKj4d9WkKj4d9WkKj4`
- **USDC Reserve Account:** `D6q6wuQSrifJKZYBVWdnDnAcNn5NRSpEzkD3GwnmaJa2`
- **Sysvar Instructions Account:** `Sysvar1nstructions1111111111111111111111111`

## 2. Flash Loan Instructions & Discriminators

1. **Flash Borrow (`flash_borrow_reserve_liquidity`):**
   - Discriminator: `0xb90ed74e2dc66cac` (`[185, 14, 215, 78, 45, 198, 108, 172]`)
   - Arguments: `liquidity_amount: u64`
2. **Flash Repay (`flash_repay_reserve_liquidity`):**
   - Discriminator: `0xb97249e6ed676917` (`[185, 114, 73, 230, 237, 103, 105, 23]`)
   - Arguments: `liquidity_amount: u64`, `borrow_instruction_index: u8`

## 3. Account Matrix (12 Accounts)

Both borrow and repay require the following exact account sequence:

| Index | Account Role | Writable | Signer | Description |
|---|---|---|---|---|
| 0 | `user_transfer_authority` | No | Yes | Trader wallet authority |
| 1 | `lending_market_authority` | No | No | Derived PDA (`b"lma"`) |
| 2 | `lending_market` | No | No | Kamino Main Market |
| 3 | `reserve` | Yes | No | Reserve state account |
| 4 | `reserve_liquidity_mint` | No | No | Asset mint (e.g. WSOL or USDC) |
| 5 | `reserve_supply_vault` | Yes | No | Lending reserve token vault |
| 6 | `user_liquidity_ata` | Yes | No | User's ATA receiving / repaying liquidity |
| 7 | `reserve_fee_vault` | Yes | No | Protocol fee accumulation vault |
| 8 | `referrer_token_state` | Yes | No | Optional referrer token state |
| 9 | `referrer_account` | Yes | No | Optional referrer destination account |
| 10 | `sysvar_instructions` | No | No | Solana Instructions Sysvar for introspection |
| 11 | `token_program` | No | No | SPL Token Program (`TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`) |

## 4. Atomic Introspection & Repayment Guarantee

Kamino Lending enforces atomicity via the Solana `sysvar_instructions` introspection sysvar:
1. When `flash_borrow_reserve_liquidity` executes at instruction index $i$, it records the borrow event in runtime memory.
2. The runtime asserts that `flash_repay_reserve_liquidity` exists at instruction index $j > i$ in the **same transaction**.
3. `borrow_instruction_index` passed to repay must match the exact index $i$.
4. If repayment is not executed, or if the repaid amount is less than $\text{principal} + \text{fee}$, the entire transaction aborts and reverts on-chain.

## 5. Flash Loan Fee Mathematics

Kamino reserves define an on-chain fee scaling factor $sf$ in Q68.60 fixed-point format:
$$\text{fee} = \max\left(1, \left\lfloor \frac{\text{amount} \times sf}{2^{60}} \right\rfloor\right)$$
For the WSOL Main Market Reserve:
$$sf = 11,529,215,046,068 \implies \text{fee\_rate} = \frac{11529215046068}{2^{60}} \approx 0.00001 = 0.1\text{ bps (0.001\%)}$$
For a 10 SOL borrow ($10^{10}$ lamports):
$$\text{fee} = \frac{10^{10} \times 11,529,215,046,068}{1,152,921,504,606,846,976} = 100,000\text{ lamports (0.0001 SOL)}$$
Checked 64-bit integer bit-shift arithmetic (`(amount * sf) >> 60`) guarantees exact consensus matching.
