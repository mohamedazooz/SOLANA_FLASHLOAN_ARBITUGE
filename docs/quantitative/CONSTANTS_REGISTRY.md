# QUANTITATIVE REGISTRY — CONSTANTS REGISTRY

This registry records every protocol and system constant used across the Sentinel codebase.

---

## 1. Protocol Program IDs & Sysvars

| Constant Name | Current Value | Unit | Protocol or Config | Configurable? | Safe to Hardcode? | Code Locations |
|---|---|---|---|---|---|---|
| `KAMINO_KLEND_PROGRAM_ID` | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-tx` |
| `RAYDIUM_V4_PROGRAM_ID` | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `RAYDIUM_CPMM_PROGRAM_ID`| `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `RAYDIUM_CLMM_PROGRAM_ID`| `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `ORCA_WHIRLPOOL_PROGRAM_ID`| `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `METEORA_DLMM_PROGRAM_ID`| `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `METEORA_DAMM_PROGRAM_ID`| `Eo7WjKq67rjJQSZxS6z3YkapzY3eMj6Xy8X5EQVn5UaB` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `PUMP_FUN_PROGRAM_ID` | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `PUMP_SWAP_PROGRAM_ID` | `pmswP5B8eBfW2sEskkbhJkU6cAgqF5U4d2L3vD7GjQW` | Pubkey | Protocol | No | Yes (Mainnet deployed) | `arb-constants`, `arb-dex`, `arb-tx` |
| `SYSVAR_INSTRUCTIONS_ID` | `Sysvar1nstructions1111111111111111111111111` | Pubkey | Solana Core | No | Yes (Native Sysvar) | `arb-constants`, `arb-tx` |
| `TOKEN_PROGRAM_ID` | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | Pubkey | Solana Core | No | Yes (SPL Token) | `arb-constants`, `arb-tx` |
| `TOKEN_2022_PROGRAM_ID` | `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` | Pubkey | Solana Core | No | Yes (Token-2022) | `arb-constants`, `arb-tx` |

---

## 2. Key Protocol Constants & Scaling Factors

| Constant Name | Current Value | Unit | Protocol / Source | Changes Over Time? | Safe to Hardcode? | Notes / Usage |
|---|---|---|---|---|---|---|
| `KAMINO_FLASH_LOAN_FEE_SF`| `11_529_215_046_068` | Q68.60 integer | Kamino Main Market | Rare governance vote | Yes (verified on-chain) | Computes 0.1 bps fee (`arb-math`, `arb-tx`) |
| `METEORA_BINS_PER_BIN_ARRAY` | `70` | Integer | Meteora DLMM IDL | Fixed protocol spec | Yes | Bin array division (`arb-constants`, `arb-dex`) |
| `TICK_ARRAY_SIZE` | `88` | Integer | Orca Whirlpool IDL | Fixed protocol spec | Yes | Whirlpool tick array length (`arb-dex`) |
| `MIN_JITO_TIP_LAMPORTS` | `10_000` | Lamports (0.00001 SOL) | Jito Block Engine | Floor may rise | Yes (Floor minimum) | Minimum bundle tip (`arb-land`) |
| `PUMP_GRADUATION_TARGET_LAMPORTS`| `85_000_000_000` | Lamports (85 SOL) | Pump.fun Curve | Curve invariant | Yes | Completion threshold (`arb-math`) |
| `POOL_OPENING_BASE_TOKENS` | `206_900_000_000_000` | Atomic (206.9M, 6 dec)| Pump.fun Migration | Protocol invariant | Yes | Initial pool tokens (`arb-math`) |
| `POOL_OPENING_QUOTE_LAMPORTS` | `84_990_000_000` | Lamports (~84.99 SOL) | Pump.fun Migration | Protocol invariant | Yes | Initial pool SOL (`arb-math`) |
| `MAX_TRANSACTION_BYTES` | `1232` | Bytes | Solana MTU | Protocol limit | Yes (Legacy/V0) | Wire packet limit (`arb-tx`) |
| `MAX_V1_TRANSACTION_BYTES` | `4096` | Bytes | SIMD-0385 | Mainnet feature | Yes (V1 only) | Extended packet limit (`arb-tx`) |

---

## 3. Project Configuration Defaults (Configurable via `.env` / `config.toml`)

| Configuration Name | Default Value | Unit | Config Source | Modifiable at Runtime? | Safe Range |
|---|---|---|---|---|---|
| `DEFAULT_MIN_PROFIT_LAMPORTS` | `500_000` | Lamports (0.0005 SOL) | `config.toml` | Yes (`/api/control/config`) | `100_000` to `50_000_000` |
| `MAX_BORROW_LAMPORTS` | `100_000_000_000` | Lamports (100 SOL) | `config.toml` | Yes | `1_000_000_000` to `500_000_000_000` |
| `MAX_SLIPPAGE_BPS` | `50` | Basis Points (0.5%) | `config.toml` | Yes | `10` to `300` |
| `MAX_PRICE_IMPACT_BPS` | `150` | Basis Points (1.5%) | `config.toml` | Yes | `20` to `500` |
| `CU_LIMIT_DEFAULT` | `300_000` | Compute Units | `arb-tx` | Yes | `100_000` to `1_400_000` |
| `MAX_TIP_SHARE_BPS` | `5000` | Basis Points (50%) | `arb-land` | Yes | `1000` to `8000` |
