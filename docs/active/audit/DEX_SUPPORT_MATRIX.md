# DEX Support Matrix — Phase 3 (Mission §8)

- **Date:** 2026-11-24 · **Status:** current support snapshot; code is the source of truth.
- **Method:** source-of-truth-is-code. Every support claim below is derived from the named source file. Protocol IDs and integration constraints were checked against official Meteora and Raydium SDKs, IDLs, and documentation on 2026-11-24.

## Support classification legend

Pillar coverage per protocol:
**A** Discovery · **B** Account decoding · **C** Quote (exact-in/out) · **D** Sizing · **E** Instruction building · **F** CPI/exec support (editor program) · **G** Simulation support · **H** Mainnet verification (snapshot or SDK differential).

Full support = **A+B+C+D+E+G+F+H**. Anything less is classified by its actual source path below. Generic family-level CPMM/CLMM fallbacks are prohibited; a venue without a protocol-specific, verified state model is quote-only or unsupported.

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
| MeteoraDammV2 | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | ⚠️ candidate via Meteora Data API; no complete pool/config discovery join | ⚠️ partial Pool prefix; `whitelisted_vault` is not a swap config; dynamic-fee/current-point/mint context missing | ❌ quote disabled; boolean fee flag is not a verified fee snapshot | ❌ | ⚠️ builder exists but execution accounts withheld until official account graph is verified | ❌ live path gated | ❌ full parity | **QUOTE-ONLY / NOT ENABLED** |
| MeteoraDbc | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | ❌ no production discovery | ⚠️ partial hydration; VirtualPool/PoolConfig and complete curve segments not joined | ❌ direct adapter now rejects; scalar fee and partial curve are insufficient | ❌ | ⚠️ builder exists, but execution accounts withheld pending official account-graph validation | ❌ | ❌ | **QUOTE-ONLY / NOT ENABLED** |
| MeteoraDlmmPro (retired) | N/A | N/A | N/A | N/A | N/A | N/A | N/A | N/A | **REMOVED FROM ACTIVE RUNTIME / NOT SUPPORTED** |
| RaydiumLaunchLab | `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` | ❌ | ⚠️ partial PoolState parser | ⚠️ exact-in curve math exists but requires verified GlobalConfig/PlatformConfig fee join; exact-out not wired | ❌ | ⚠️ builder exists; requires verified config, vaults, and non-Token-2022 mint context | ❌ | ❌ | **QUOTE-ONLY / NOT ENABLED** |

Program IDs above were re-verified 2026-11-22 against: Meteora official docs (`docs.meteora.ag`, DLMM Pro `dPro1D5…` confirmed), Bitquery protocol documentation (DBC, DAMM v2, DLMM), well-known Raydium/Orca/Pump addresses, and previously recorded research sessions (`docs/research/EXTERNAL_BEST_PRACTICES.md`).

## Key findings

- **DEX-F-2026-11-22-N1 (P1 — safety gate applied, venues remain unsupported):** the current `UniversalQuoteDispatcher` already rejects these incomplete protocol states, but the separate legacy route-graph path in `crates/arb-engine/src/adapters.rs` previously routed DAMM v2 and DBC through generic CPMM math and DLMM Pro through generic CLMM math. The fallback adapters are now explicit `QuoteOnlyAdapter`s and `execution_capability` rejects all four venues. Direct DAMM v2 / DBC / DLMM Pro adapters also refuse quotes; the DAMM parser now names offset 296..328 as `whitelisted_vault` (the official Pool field), not `config`, and does not expose an execution account until the swap account graph is verified. LaunchLab retains a fee-config verification gate and rejects Token-2022 until epoch-aware transfer-fee / transfer-hook context exists. Re-enable only after protocol-native state hydration, fees, sizing semantics, account graph, differential tests, and simulation are complete.
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
