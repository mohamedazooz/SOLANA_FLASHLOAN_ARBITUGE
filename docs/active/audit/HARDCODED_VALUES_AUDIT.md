# Hardcoded Values Audit

**Audit date:** 2026-11-26
**Branch:** `feat/forensic-pricing-fee-unification`
**Method:** Static grep + manual review of `crates/`. Classification key:
- **A — Protocol constant**: imposed by the on-chain program (discriminator, layout size, fixed-point denominator, sqrt-price bound). Must be documented + tested; must NOT move to ENV.
- **B — Runtime on-chain value**: must be decoded from live accounts (reserves, fee rates, active tick/bin, flash-loan fee). Must NOT be replaced by a constant.
- **C — Application configuration**: operator policy (min profit, max trade size, max state age, slippage, risk buffer, candidate cap). Must live in Config/ENV with validation.
- **D — Emergency fail-closed limit**: protection bound that must never be relaxed by missing config or data.

## 1. Protocol constants (A) — verified, keep as-is

| Value | File | Classification | Note |
|---|---|---|---|
| `METEORA_BINS_PER_BIN_ARRAY = 70` | `arb-constants/src/lib.rs` | A | Official Meteora `BINS_PER_ARRAY`. |
| `METEORA_MAX_BINS_PER_SWAP = 260` | same | A | Official DLMM changelog (reduced from 280). |
| `METEORA_BIN_ARRAY_BITMAP_SIZE = 512` | same | A | Official IDL `BIN_ARRAY_BITMAP_SIZE`. |
| `CLMM_MIN_SQRT_PRICE_X64`, `ORCA_*_MAX_SQRT_PRICE_X64`, `RAYDIUM_CLMM_MAX_*`, `METEORA_DAMM_V2_MAX_*` | same | A | Verified identical across Orca/Raydium/Meteora. |
| `SOLANA_RUNTIME_MAX_COMPUTE_UNITS = 1_400_000` | same | A | Agave runtime consensus. |
| `SOLANA_V0_PACKET_MTU_LIMIT = 1232`, `SOLANA_V1_PACKET_MTU_LIMIT = 4096` | same | A | SIMD-0296/SIMD-0385. |
| `SOLANA_BASE_FEE_LAMPORTS = 5_000` | same | A | Base signature fee. |
| `FLASH_BORROW_DISCRIMINATOR`, `FLASH_REPAY_DISCRIMINATOR` | same | A | Kamino instruction discriminators. |
| `PUMP_SWAP_*_DISCRIMINATOR`, `RAYDIUM_*_DISCRIMINATOR`, `METEORA_*_DISCRIMINATOR` | same | A | Anchor discriminators. |
| `METEORA_SWAP2_DISCRIMINATOR`, `METEORA_MAX_BIN_ARRAYS_PER_SWAP` | same | A | Derived from official bin count. |
| `MAX_INSTRUCTION_STACK_DEPTH = 5` (9 with SIMD-0268) | same | A | Agave runtime. |
| `MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES = 64 MiB` | same | A | Agave default. |
| `PUMP_FUN_GRADUATION_LAMPORTS = 85_000_000_000` | same | A | Pump.fun protocol constant (verify on upgrades). |

## 2. Runtime on-chain values (B) — must be decoded, not constant

| Value | Location | Status | Action |
|---|---|---|---|
| Kamino flash-loan fee (`flash_loan_fee_sf`) | `arb-constants` comment + `arb-tx::kamino` | Correctly NOT a constant; read from reserve. Verified WSOL reserve sf = 11_529_215_046_068. | Keep dynamic. |
| Pool fee rates (Raydium CPMM `trade_fee_rate`, Orca fee, Meteora base/dynamic fee) | adapters | Read from pool/config accounts. | Keep dynamic; fail closed if missing. |
| Pool reserves / vault balances | ingestion | Decoded from accounts. | Keep dynamic. |
| Active tick / active bin | adapters | Decoded. | Keep dynamic. |
| Token-2022 transfer fee schedule | `token2022.rs` | Epoch-aware; missing = fail closed. | Keep dynamic. |
| Dynamic priority fee | config/loader | Env-configurable with defaults. | Keep. |
| Jito tip | `DEFAULT_*_TIP_*` constants exist as *fallbacks* only | Must use live auction feed; tip only charged with actual tip instruction. | Verify no fixed tip is applied unconditionally. |

## 3. Application configuration (C) — belongs in Config/ENV

| Value | File | Classification | Note |
|---|---|---|---|
| `DEFAULT_MIN_PROFIT_LAMPORTS = 10_000` | `arb-constants` | C | Operator policy; configurable. |
| `DEFAULT_HOP_SLIPPAGE_BPS = 10` | same | C | Operator policy; capped by `HOP_SLIPPAGE_MAX_BPS = 1_000`. |
| `MAX_KAMINO_FLASH_BORROW_LAMPORTS = 5_000_000_000` | same | C (legacy) | Legacy optimizer bound; sizing should be driven by live vault liquidity. |
| `MIN_KAMINO_FLASH_BORROW_LAMPORTS = 10_000_000` | same | C (legacy) | Same. |
| `MAX_FLASH_BORROW_POOL_RESERVE_SHARE_BPS = 1_500` | same | C | Operator policy. |
| `DEFAULT_MAX_BINS_PER_LEG = 3` | same | C | DLMM traversal cap policy. |
| `DEFAULT_JITO_TIP_SHARE_BPS = 500`, `DEFAULT_JITO_MAX_TIP_SHARE_BPS = 1_000` | same | C | Bidding policy. |
| `DEFAULT_KEEP_SHARE = 0.90`, `DEFAULT_MAX_TIP_SHARE = 0.10` | same | C (f64, display/bidding only) | Must not drive pricing. |
| `KAMINO_ZAP_FRESHNESS_TOLERANCE_SLOTS = 1_000`, `KAMINO_SUPPLY_VAULT_MAX_AGE_SLOTS = 150`, `KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS = 1_500` | same | C | Freshness policy. |
| `CU_BUFFER_PCT = 15` | same | C | CU headroom policy. |
| `DEFAULT_PRIORITY_FEE_BASE_SHARE_BPS = 200`, `DEFAULT_PRIORITY_FEE_MAX_SHARE_BPS = 1_000` | same | C | Priority-fee bidding policy. |
| `DEFAULT_RPC_MAX_CONCURRENT_REQUESTS = 1`, `DEFAULT_RPC_MIN_INTERVAL_MS = 200` | same | C | RPC pacing policy. |
| `DEFAULT_WS_COMMITMENT = "processed"`, `DEFAULT_WS_ENCODING = "base64"` | same | C | Ingestion policy. |

## 4. Emergency fail-closed limits (D)

| Value | File | Classification | Note |
|---|---|---|---|
| `HOP_SLIPPAGE_MAX_BPS = 1_000` | `arb-constants` | D | Hard cap; operator cannot raise. |
| `MAX_COMPUTE_UNIT_LIMIT = 1_400_000` | same | D | Runtime ceiling. |
| `APPLICATION_SAFETY_CU_LIMIT = 1_400_000` | same | D | Safety ceiling. |
| `KAMINO_SUPPLY_VAULT_MAX_AGE_SLOTS = 150` (also C) | same | D | Stale vault → skip opportunity. |
| `METEORA_MAX_BINS_PER_SWAP = 260` | same | D | Quote fails closed above. |

## 5. Suspicious / needs verification

1. **`DEFAULT_KEEP_SHARE: f64 = 0.90` and `DEFAULT_MAX_TIP_SHARE: f64 = 0.10`** are floating-point policy constants. They must remain display/bidding-only and never enter authoritative profit math. Verified the bellman_ford boundary documents this; keep the boundary.
2. **`MAX_KAMINO_FLASH_BORROW_LAMPORTS = 5_000_000_000` (50 SOL) and `MIN_KAMINO_FLASH_BORROW_LAMPORTS = 10_000_000` (0.01 SOL)** are legacy optimizer bounds. Sizing is now bounded by live supply-vault liquidity; these must not override the live cap.
3. **`METEORA_SOL_USDC_BIN_STEP = 4`** is a per-pool value, not a protocol constant. It is used as a default for a specific pool; must be read from the pool account in production. Confirm it is not used as a global default for other pools.
4. **`PUMP_FUN_GRADUATION_LAMPORTS = 85_000_000_000`** is a protocol constant but may change on program upgrades; keep verification on upgrades.
5. **`DEFAULT_MIN_PROFIT_LAMPORTS = 10_000`** is small by design (per task: small targets allowed). Must not be silently raised to hide missing opportunities.
6. **`ARB_EXECUTOR_PROGRAM_ID = "ArbExec111…"`** is a placeholder, not a deployed program. Must not be enabled (EXEC-F-2026-11-22-N1).
7. **`DEFAULT_MAINNET_ALT_KEY = "ALT111…"`** is a placeholder ALT. V0 compilation must verify warmup against the real on-chain ALT.
8. **`DEFAULT_FLASH_LOAN_FEE_BPS = 30` and `DEFAULT_RAYDIUM_FEE_BPS = 25` in `crates/arb-math/src/pump_fun_raydium.rs`** — fixed bps fee defaults that violate the no-fixed-fee rule. **Verified test-only:** grep across `crates/` shows zero production callers; the constants are consumed exclusively by that file's `#[cfg(test)]` module. Quarantine verified; if the pump_fun_raydium path is ever productized, these MUST be replaced by reserve/config-decoded values before any live wiring.
9. **Legacy `PumpFunRaydiumArbitrageBuilder` (`crates/arb-tx/src/protocols/pump_fun_raydium.rs`)** — carries hardcoded Jito tip / priority-fee defaults. **Verified zero production callers** (no imports from `arb-bot`, `arb-land`, or `supervisor/`). Dead code from the safety standpoint; must not be wired into the canonical builder chain (`arb-tx/src/primitives/builder.rs`) without adopting the unified fee ledger and policy-driven tip/fee inputs.
10. **Referral fee (Kamino)** — verified structurally zero in production: all flash builders pass `referrer_account`/`referrer_token_state` = `None` and upstream klend (`686913ee`) puts the referral share INSIDE the single flash fee (`protocol + referral == fee`). Differential proof: `crates/arb-math/tests/kamino_klend_differential.rs`.

## 5.1 Evidence-gated opportunity stages (related hardening)

Stage promotion is no longer inferable from free-form status strings alone.
`arb-types/src/opportunity_stage.rs` now requires explicit evidence for every
stage above `DISCOVERED` (quote completeness, freshness, same-slot
consistency, positive integer net profit, builder success proof, simulation
SUCCESS outcome, and dedicated proof structures for the shadow/mainnet
tiers). This closes the class of bugs where a stale quote or a failed
simulation could be reported at a profitable-looking stage.

## 6. Hardcoded fee assumptions found & resolved (historical)

- Raydium CLMM discovery previously carried `fee_bps: 25` placeholder; operative fee now from catalog `feeRate` with fail-closed exclusion of `fee_on`/`dynamic_fee_info` pools (DEX-F-2026-11-22-N2).
- Orca no longer receives a guessed 5-bps default (FEE-F-2026-11-25-N2).
- PumpSwap generic quote no longer fabricates an 80/20 fee split (FEE-F-2026-11-25-N1).
- Kamino fee is never a constant (documented in `arb-constants`).

## 7. Conclusion

No hardcoded fee value in the current tree silently acts as a zero-cost shortcut for a live pricing path. The remaining risk is **configuration drift** (operator setting `MIN_NET_PROFIT` too low, or `MAX_TRADE_SIZE` too high) and **stale on-chain data**, both of which are gated by fail-closed freshness checks. The largest open correctness gap remains Raydium CLMM (AmmConfig not live-joined) and Token-2022 (not wired end-to-end).
