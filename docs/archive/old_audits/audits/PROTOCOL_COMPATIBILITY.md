> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# AUDIT — PROTOCOL COMPATIBILITY & SDK INTEGRATION REPORT

**Auditor:** Protocol & SDK Research Engineer  
**Date:** 2026-11-19  
**Status:** FULLY AUDITED & VERIFIED  

---

## 1. Protocol Compatibility Matrix

| Protocol | Program ID | Verified SDK / Source | Account Layout Status | Discriminator Status | Instruction Builder Status |
|---|---|---|---|---|---|
| **Kamino Lending** | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Official Anchor IDL | Verified (12 accounts) | Verified (`0xb90ed...` / `0xb9724...`) | Native direct builder (`arb-tx`) |
| **Raydium CPMM** | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | Raydium CP-Swap SDK | Verified (13 accounts) | Verified (`swap_base_input`) | Native direct builder (`arb-tx`) |
| **Raydium CLMM** | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | Raydium CLMM SDK | Verified (Tick arrays) | Verified (`swap` / `swap_v2`) | Native direct builder (`arb-tx`) |
| **Raydium V4** | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | Raydium SDK V1 | Verified (OpenBook) | Opcode `9` / Opcode `16` | Native direct builder (`arb-tx`) |
| **Orca Whirlpool** | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | `orca_whirlpools_core v2.1.1`| Verified (11 accounts) | Verified (`global:swap`) | Native SDK integration (`arb-tx`) |
| **Meteora DLMM** | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Meteora DLMM SDK | Verified (70-bin arrays) | Verified (`Swap2`) | Native direct builder (`arb-tx`) |
| **Pump.fun** | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Official Program IDL | Verified (83 bytes) | Verified (`0x17b7f...`) | Native direct builder (`arb-tx`) |
| **PumpSwap** | `pmswP5B8eBfW2sEskkbhJkU6cAgqF5U4d2L3vD7GjQW` | PumpSwap IDL | Verified (245+ bytes) | Verified (`0x4b786...`) | Native direct builder (`arb-tx`) |

---

## 2. Protocol Details & Invariant Verifications

### 2.1 Kamino Flash Loans
- **Borrow Discriminator:** `[185, 14, 215, 78, 45, 198, 108, 172]`
- **Repay Discriminator:** `[185, 114, 73, 230, 237, 103, 105, 23]`
- **Atomicity Guard:** Tested against `sysvar_instructions` introspection. Repay instruction index strictly checked against borrow index.

### 2.2 Raydium CPMM
- **Authority PDA:** Derived with seed `b"vault_and_lp_mint_auth_seed"`.
- **Observation Account:** Required by on-chain program to update TWAP price accumulators.
- **Token-2022 Support:** Passes correct Token-2022 program IDs when trading Token-2022 mints.

### 2.3 Meteora DLMM
- **Swap2 Instruction:** Replaces legacy `swap` with canonical `Swap2`.
- **Euclidean Bin Array Mapping:** Verified floor division for negative bin IDs prevents out-of-bounds bin lookups.
- **Active Bin Verification:** Confirmed that active bin id matches pool state.

### 2.4 Orca Whirlpool
- **Core Crate Usage:** Uses official audited `orca_whirlpools_core` algorithms for tick array walking and math.
- **Directional Ordering:** Tick arrays sorted ascending for B-to-A, descending for A-to-B.
