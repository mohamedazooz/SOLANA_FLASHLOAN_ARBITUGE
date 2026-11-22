# DEX Support Matrix — Phase 3 (Mission §8)

- **Date:** 2026-11-22 · **Branch:** `phase/05-dex-quote-correctness` (PR link in register)
- **Method:** source-of-truth-is-code. every claim below is derived from the named source file, not from docs. Program IDs cross-verified against official sources (Meteora docs / Bitquery / Raydium docs) on 2026-11-22.

## Support classification legend

Pillar coverage per protocol:
**A** Discovery · **B** Account decoding · **C** Quote (exact-in/out) · **D** Sizing · **E** Instruction building · **F** CPI/exec support (editor program) · **G** Simulation support · **H** Mainnet verification (snapshot or SDK differential).

Full support = **A+B+C+D+E+G+F+H**. Anything less is quoted exactly below. "Approx" = generic constant-product fallback with fixed fee, NOT protocol math.

## Matrix (11 protocols in `DexProtocol`)

| Protocol | Program ID (mainnet) | A Discovery | B Decode | C Quote | D Sizing | E Build | F CPI/Exec | G Sim | H Mainnet verify | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| RaydiumV4 (AMM) | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | ✅ `discovery.rs` | ✅ `decode_pool_info` (offsets 336–464) | ✅ exact CP math + vault reserves | ✅ graph sizing | ✅ `swap_hop.rs:131` | ✅ `raydium_v4_boundary_tests`, `raydium_v4_v2_execution_tests` | ✅ arb-sim | ✅ SDK differential (arb-math + mainnet snapshot tests) | **SUPPORTED (full loop)** |
| RaydiumCpmm | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | ✅ `discovery.rs:472/544` + AmmConfig live fetch | ✅ vaults decode; fee from AmmConfig **fail-closed** (`decode_pool_info` returns None for fee) | ✅ `raydium_cpmm.rs` engineer | ✅ | ✅ `swap_hop.rs:412` | ✅ | ✅ | ✅ boundary tests + supervisor execution test | **SUPPORTED** |
| RaydiumClmm | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | ✅ | ✅ (state decode + tick arrays) | ✅ exact tick math (arb-math `clmm_*`) | ✅ tick-boundary sizing | ✅ `swap_hop.rs:465` (+ Memo v2) | ✅ | ✅ | ✅ `raydium_clmm_mainnet_sdk_differential`, `raydium_clmm_swap_math_parity_tests`, tick parity | **SUPPORTED** |
| OrcaWhirlpool | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | ✅ | ✅ (245+ offsets, fee bps decode) | ✅ exact tick math | ✅ | ✅ `swap_hop.rs:199` | ✅ | ✅ | ✅ `orca_sdk_parity` (arb-math), `whirlpool_mainnet_sdk_differential` | **SUPPORTED** |
| MeteoraDlmm | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | ✅ Meteora data API + gRPC decode | ✅ bins/active_id/params | ✅ exact bin ladder + dynamic fee (`dlmm_*` suites) | ✅ bin-array planning (`plan_bin_arrays`) | ✅ `swap_hop.rs:317` + `protocols/meteora_dlmm.rs` + 3/4-leg builders | ✅ (`meteora_atomic_builder`, editor CPI) | ✅ LiteSVM | ✅ `meteora_mainnet_sdk_differential`, `meteora_sdk_differential_tests`, PDA differential | **SUPPORTED** |
| PumpFun (bonding curve) | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | ✅ BankkRoll API + multi-venue pairing | ✅ `pump_state.rs` | ✅ exact bonding-curve math | ✅ | ✅ `swap_hop.rs:560` | ✅ `pump_fun_raydium.rs` | ✅ | ⚠️ boundary/engine tests; no committed Mainnet snapshot | **SUPPORTED (H = partial)** |
| PumpSwap (post-grad) | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | ✅ DexScreener mapping (`dexscreener.rs:123`) | ✅ | ✅ `pump_swap.rs` engine | ✅ | ✅ `swap_hop.rs:32` | ✅ | ✅ | ⚠️ engine tests only | **SUPPORTED (H = partial)** |
| MeteoraDammV2 | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | ❌ no discovery (`decode_pool_info` → `None`) | ⚠️ hydration builds execution accounts (`hydration.rs:723`) but no pool discovery | ⚠️ **APPROX** CP fallback, hardcoded 25bps (`engines/mod.rs:94`) | ❌ | ✅ builder exists (`swap_hop.rs:571`) | ⚠️ builder only | partial | ❌ | **NOT SUPPORTED — dormant approximation** |
| MeteoraDbc | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | ❌ no discovery | ⚠️ hydration (`hydration.rs:741`) | ⚠️ **APPROX** CP fallback 25bps | ❌ | ✅ `swap_hop.rs:733` | ⚠️ | partial | ❌ | **NOT SUPPORTED — dormant approximation** |
| MeteoraDlmmPro | `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n` | ❌ | ⚠️ | ⚠️ **APPROX** CP fallback 10bps; tick-based fee/dynamic-fee semantics absent | ❌ | ✅ `swap_hop.rs:804` | ⚠️ | ❌ | ❌ | **NOT SUPPORTED — dormant approximation** |
| RaydiumLaunchLab | `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` | ❌ | ⚠️ | ⚠️ **APPROX** CP fallback 25bps | ❌ | ✅ `swap_hop.rs:654` | ⚠️ | ❌ | ❌ | **NOT SUPPORTED — dormant approximation** |

Program IDs above were re-verified 2026-11-22 against: Meteora official docs (`docs.meteora.ag`, DLMM Pro `dPro1D5…` confirmed), Bitquery protocol documentation (DBC, DAMM v2, DLMM), well-known Raydium/Orca/Pump addresses, and previously recorded research sessions (`docs/research/EXTERNAL_BEST_PRACTICES.md`).

## Key findings

- **DEX-F-2026-11-22-N1 (P3):** DAMM v2 / DBC / DLMM Pro / LaunchLab quote through a **generic constant product approximation with hard-coded fees** (`crates/arb-dex/src/engines/mod.rs:94–118`). They are fail-closed for *discovery* (`decode_pool_info` arm `_ => None`, `decoders.rs:274`), so no live edge can currently reference them; but any future manual pool registration would silently accept the approximations. Required before any enablement: exact per-protocol math + dynamic fee model + discovery + differential tests.
- **DEX-F-2026-11-22-N2 (P3):** RaydiumClmm `decode_pool_info` (discovery fallback) hard-codes `fee_bps: 25` (`decoders.rs:262`). The operative fee is NOT this constant: discovery supplies `feeRate` from the Raydium catalog (`discovery.rs:517`), and `decode_raydium_clmm_state` converts the protocol's millionths-based fee unit with explicit fail-closed exclusion of `fee_on` and `dynamic_fee_info` pools (`decoders.rs:692–701`). The residual risk is only a theoretical path where a CLMM pool reaches the graph without hydration; register as telemetry, not a live correctness defect.
- **Token-2022 tokens:** fail-closed excluded end-to-end (mint resolution omits Token-2022; transfer-fee rejects) per `EFI`-P1-01 register entry. Unsupported by policy, not by oversight.
- **Jupiter:** v6 quote/compile client exists (`arb-tx/src/protocols/jupiter.rs`) for envelope/relay paths; **no protocol math or execution beyond assembly** — it is a transport aid, NOT a supported execution venue; no discovery arm.
- **Pump.fun graduation boundary:** bonding-curve→PumpSwap transitions covered by separate engine + boundary tests; no linear approximations anywhere in the *supported* set.

## Test evidence (this branch)

```text
cargo test -p arb-dex   → 127 passed / 0 failed
cargo test -p arb-math  → 342 passed / 0 failed
```
Boundaries covered by named suites: zero liquidity / dust / huge amounts / tick & bin boundaries / fee transitions / reversed direction / overflow (adversarial + differential + fuzz suites in `arb-math/tests`).
