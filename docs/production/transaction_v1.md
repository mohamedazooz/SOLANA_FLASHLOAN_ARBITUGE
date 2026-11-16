# Solana Transaction V1 & SIMD-0385 Specification

## 1. Solana Transaction Versioning Landscape
- **Legacy**: 1,232-byte MTU, compact-u16 signature array at head, legacy 3-byte header, compact instructions.
- **Version 0 (V0)**: 1,232-byte MTU, version prefix `0x80`, Address Lookup Tables (ALTs) for account compression. Universal mainnet standard supported by all validators and Jito.
- **Version 1 (V1 / SIMD-0385)**: 4,096-byte MTU (SIMD-0296), version prefix `0x81`, native resource configuration in `TransactionConfigMask`, up to 64 inline accounts (no ALTs required), trailing Ed25519 signatures.

## 2. SIMD-0385 Wire Format Layout

```
[0]      0x81 (Version Discriminator)
[1..4]   Message Header:
           [1] num_required_signatures
           [2] num_readonly_signed_accounts
           [3] num_readonly_unsigned_accounts
[4..8]   TransactionConfigMask (32-bit little-endian)
[8..40]  Recent Blockhash (32 bytes)
[40]     num_instructions (u8)
[41]     num_addresses (u8, max 64)
[42..]   Inline Accounts Array (num_addresses * 32 bytes)
[...]    ConfigValues (ordered strictly by set bits in TransactionConfigMask)
[...]    Instruction Headers (num_instructions * 4 bytes: program_idx, num_accounts, data_len)
[...]    Instruction Account Indices & Contiguous Payloads
[...]    Trailing Ed25519 Signatures (num_required_signatures * 64 bytes)
```

## 3. TransactionConfigMask Bit Specification
- **Bit 0 & 1 (`0x03`)**: Total priority fee in lamports (8 bytes, little-endian `u64`). Replaces legacy `SetComputeUnitPrice` instruction.
- **Bit 2 (`0x04`)**: Compute unit limit (4 bytes, little-endian `u32`). Replaces legacy `SetComputeUnitLimit` instruction.
- **Bit 3 (`0x08`)**: Loaded accounts data size limit (4 bytes, little-endian `u32`).
- **Bit 4 (`0x10`)**: Requested heap size (4 bytes, little-endian `u32`).

## 4. Compute Budget Instruction Stripping & Klend Index Rewriting
In V1 transactions, the compiler automatically:
1. Strips standalone Compute Budget instructions from the message payload and encodes them into the native config mask.
2. Rewrites the Kamino Flash Repay `borrow_instruction_index`:
   - If Compute Budget instructions are stripped from indices 0 and 1, the Kamino borrow instruction shifts from index 2 to index 0.
   - The repay instruction's data payload is updated to point to `borrow_instruction_index = 0`.
   - Verified by `tests/v1_property_fuzz.rs` (`test v1_rewrites_klend_repay_borrow_index_after_compute_budget_strip`).

## 5. Dual-Engine Runtime Selection
Live execution probes `CompilerCapabilities::supports_v1`:
- If active on the connected RPC and Jito relay: compiles V1 envelope with 4,096-byte ceiling.
- If unverified or inactive: defaults seamlessly to V0 with Address Lookup Tables, ensuring zero downtime and zero landing failures due to experimental cluster features.
