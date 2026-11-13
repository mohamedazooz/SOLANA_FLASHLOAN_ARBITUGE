# Phase 16 — V1 Transport Evidence

**Date:** 2026-11-13
**Scope:** versioned-transaction V1 (0x81 header, inline accounts, 4,096-byte
ceiling, SIMD-0385 ≤64 addresses) as used by the arbitrage path.

## 1. Separation of instruments

`crates/arb-tx/tests/v1_size_ladder_tests.rs` deliberately emits **two**
clearly labelled instruments into `reports/phase15_5/V1_SIZE_LADDER.json`:

1. `instrument: "real_three_pool_dlmm_route"` — a complete, signable
   Kamino→DLMM×3 route. *Constraint:* bin-array addresses are synthetic
   (`Pubkey::new_unique()`), so it is not mainnet-executable as written.
2. `instrument: "wire_capacity"` — real V1 header + real signature +
   ComputeBudget, with payloads padded to hit each byte rung. This proves
   **wire-format capacity and the compiler's size gates only**. It is not an
   arbitrage route.

Never merge these findings in a readiness claim.

## 2. Measured sizes (VERIFIED, offline replay)

Real 3-pool routes (Kamino borrow → 3× DLMM swap → repay):

| bins/leg | accounts | bytes | V1 | signature |
|---:|---:|---:|:--:|:--:|
| 1 | 38 | 1574 | ✅ | ✅ |
| 2 | 41 | 1673 | ✅ | ✅ |
| 3 | 44 | 1772 | ✅ | ✅ |
| 4 | 47 | 1871 | ✅ | ✅ |
| 5 | 50 | 1970 | ✅ | ✅ |
| 6 | 53 | 2069 | ✅ | ✅ |
| 7 | 56 | 2168 | ✅ | ✅ |
| 8 | 59 | 2267 | ✅ | ✅ |
| 9 | 62 | 2366 | ✅ | ✅ |
| 10 | 65 | 2465 | ✅ | ✅ |
| 11 | 67 | — | ❌ | — |

- 11 bins/leg → `SIMD-0385 address limit exceeded: 67 > 64`.
- Capacity rungs 1232 / 1500 / 2000 / 3000 / 3500 / 4000 all produced the exact
  target byte count and a verified signature.
- 4,097 bytes → `Transaction size exceeds Solana MTU: 4097 bytes > 4096 bytes limit`.
- The 4,096 MTU and the 64-address limit are **both enforced**; neither is
  bypassed.

## 3. No silent downgrade

- `compile_adaptive` is **V1-only** when the capability profile supports V1; it
  does not fall back to Legacy/V0.
- `test_v1_capable_profile_never_downgrades_small_payload` proves even a tiny
  payload stays V1.
- Legacy/V0-with-ALT is reachable only with an explicitly non-V1 profile.
- The V1 direct toggle (`SOLANA_TX_V1_DIRECT_ENABLED`) gates the live path; the
  old `SOLANA_TX_V1_JITO_ENABLED` key has no consumer and was removed from
  `.env.example` in PR #13.

## 4. Live simulation (CITED, not re-run)

`reports/v1_mainnet_probe/probe_report_2026-11-13.json`:

- RPC `solana-core` **4.4.0**
- WSS connected; slots advanced (468722675 → 468722679)
- V1 `simulateTransaction` **accepted**
- units consumed **62,596**
- Kamino `fee_sf` live value `115292150460684` (matches constant)
- WSOL flash liquidity **230,166.406 SOL**

This is a valid **simulate** result. It is not a broadcast and not execution.

## 5. Broadcast

**NOT VERIFIED.** No authorized signer, `ARB_LIVE` unset, no sent signature,
no on-chain status. A V1 broadcast claim is not made.

## 6. Reproduce

```bash
cargo test -p arb-tx --test v1_size_ladder_tests -- --nocapture
cargo test -p arb-tx --test adaptive_compiler_tests
cargo test -p arb-tx --test three_pool_flash_arb_tests
cargo test -p arb-tx --test v1_property_fuzz
cargo test -p arb-tx --test v1_header_differential
python scripts/probe_rpc_wss_v1_sim.py   # needs a funded-capable RPC key
```

Expected artifact: `reports/phase15_5/V1_SIZE_LADDER.json`.
