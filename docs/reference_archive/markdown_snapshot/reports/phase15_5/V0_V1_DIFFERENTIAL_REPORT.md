# V0/V1 Differential Report

## Summary

This report documents the differential validation between V0 (Jito-compatible) and V1 (Direct SWQoS) transaction compilation paths.

## Test Results

### V0 Path
- Wire size: ~1321 bytes (with top-level programs as static)
- Format: V0 with Static Master ALT
- Transport: Jito Bundle + SWQoS
- MEV Protection: jitodontfront marker present

### V1 Path
- Wire size: Variable (depends on account count)
- Format: V1 (SIMD-0296) with inline accounts
- Transport: Direct RPC/TPU via SWQoS
- MEV Protection: None (no Jito bundle)

## Semantic Equivalence

| Component | V0 | V1 | Match |
|-----------|-----|-----|-------|
| Fee Payer | payer | payer | ✅ |
| Intermediate ATA (Meme A) | meme_a_ata | meme_a_ata | ✅ |
| Intermediate ATA (Meme B) | meme_b_ata | meme_b_ata | ✅ |
| Profit Destination | payer | payer | ✅ |
| Swap Amounts | Preserved | Preserved | ✅ |
| Slippage Constraints | Preserved | Preserved | ✅ |
| Flash Loan Repayment | Preserved | Preserved | ✅ |

## Key Differences

1. **Jito Tip Handling**: V0 includes Jito tip instruction; V1 strips it and rolls into CU price
2. **Account Storage**: V0 uses ALT compression; V1 inlines all accounts
3. **MEV Protection**: V0 has jitodontfront marker; V1 has none
4. **Transport**: V0 uses Jito bundle; V1 uses direct SWQoS

## V1 Fee Conversion

```
extra_micro_lamports = jito_tip * 1e6 / cu_limit
v1_cu_price = base_cu_price + extra_micro_lamports
```

This converts Jito tip to priority fee for V1 Direct SWQoS path.

## Verdict

**PASS** — V0 and V1 paths produce semantically equivalent transactions with correct fee conversion.