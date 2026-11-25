# DEX Support Matrix — Phase 3 (Mission §8)

- **Date:** 2026-11-25 · **Status:** current support snapshot; code is the source of truth.
- **Method:** source-of-truth-is-code. Every support claim below is derived from the named source file. Protocol IDs and integration constraints were checked against official Meteora and Raydium SDKs, IDLs, and documentation on 2026-11-24. Raydium CPMM quote state now preserves its 1,000,000 fee denominator, removes accrued non-LP fee counters from vault reserves, and rejects enabled creator-fee pools until the fee-side model is implemented.

## Support classification legend

Pillar coverage per protocol:
**A** Discovery · **B** Account decoding · **C** Quote (exact-in/out) · **D** Sizing · **E** Instruction building · **F** CPI/exec support (editor program) · **G** Simulation support · **H** Mainnet verification (snapshot or SDK differential).

Full support = **A+B+C+D+E+G+F+H**. Anything less is classified by its actual source path below. Generic family-level CPMM/CLMM fallbacks are prohibited; a venue without a protocol-specific, verified state model is quote-only or unsupported.

## Matrix (10 protocol variants in `DexProtocol`; 6 currently priceable, Raydium CLMM telemetry-only, 3 not enabled)

| Protocol | Program ID (mainnet) | A Discovery | B Decode | C Quote | D Sizing | E Build | F CPI/Exec | G Sim | H Mainnet verify | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| RaydiumV4 (AMM) | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | ✅ `discovery.rs` | ✅ `decode_pool_info` (offsets 336–464) | ✅ exact CP math + vault reserves | ✅ graph sizing | ✅ `swap_hop.rs:131` | ✅ `raydium_v4_boundary_tests`, `raydium_v4_v2_execution_tests` | ✅ arb-sim | ✅ SDK differential (arb-math + mainnet snapshot tests) | **SUPPORTED (full loop)** |
| RaydiumCpmm | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | live PoolState + AmmConfig | owner/mint checks; subtracts accrued protocol/fund/creator counters from raw vaults | exact 1,000,000 fee ratio; sub-bps preserved; creator-fee-enabled pools fail closed | yes | `swap_hop.rs:412` | yes | real PoolState/AmmConfig fixtures; no complete official-SDK quote differential yet | **SUPPORTED WITH GATES (legacy SPL, creator fee disabled, slot-coherent snapshot)** |
| RaydiumClmm | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | ⚠️ discovery only | ⚠️ pool/tick decode; live AmmConfig fee join not wired | ⚠️ math adapter exists; production quote hydration disabled | ❌ live sizing disabled | ⚠️ builder retained but unreachable from hydrated candidates | ⚠️ no live candidate path | ⚠️ math parity is not live-state parity | static/dynamic fee state not joined | **TELEMETRY ONLY / NOT ENABLED** |
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

- **DEX-F-2026-11-22-N1 (P1 ? safety gate applied, venues remain unsupported):** the current `UniversalQuoteDispatcher` rejects incomplete protocol states; legacy route-graph adapters are explicit `QuoteOnlyAdapter`s and execution capability rejects DAMM v2, DBC, and LaunchLab. Meteora DLMM Pro has been removed from the active enum/runtime. Direct DAMM v2 / DBC adapters refuse quotes; the DAMM parser names offset 296..328 `whitelisted_vault` (not a swap config). LaunchLab retains a fee-config verification gate and rejects Token-2022 until epoch-aware transfer-fee / transfer-hook context exists. Re-enable only after protocol-native state hydration, fees, sizing semantics, account graph, differential tests, and simulation are complete.
- **DEX-F-2026-11-25-N2 (P1):** Raydium CLMM fee metadata from the discovery catalog is not an authoritative fee snapshot. The live trade fee is stored in the separate program-owned `AmmConfig` (`trade_fee_rate`, denominator `1_000_000`); current account state can also carry dynamic-fee configuration/state. The previous hydration/WS path promoted catalog `fee_bps` without joining and subscribing to the config account. This branch now fails closed: hydration skips CLMM, WebSocket updates cannot revive it, MarketGraph excludes even stale cached CLMM states, the universal dispatcher rejects exact-in/out, and route adapters/execution capability refuse the venue. Re-enable only after owner/discriminator/index/tick-spacing validation, same-slot PoolState/AmmConfig/tick-array snapshots, config-account subscriptions, dynamic-fee upper-bound handling, and SDK differential tests. Official source: `raydium-clmm/programs/amm/src/states/config.rs` and `products/clmm/math.mdx`.
- **Token-2022 tokens:** fail-closed excluded end-to-end (mint resolution omits Token-2022; transfer-fee rejects) per `EFI`-P1-01 register entry. Unsupported by policy, not by oversight.
- **Jupiter:** v6 quote/compile client exists (`arb-tx/src/protocols/jupiter.rs`) for envelope/relay paths; **no protocol math or execution beyond assembly** — it is a transport aid, NOT a supported execution venue; no discovery arm.
- **Pump.fun graduation boundary:** bonding-curve→PumpSwap transitions covered by separate engine + boundary tests; no linear approximations anywhere in the *supported* set.

## Test evidence (this branch)

```text
cargo test -p arb-dex   → 127 passed / 0 failed
cargo test -p arb-math  → 342 passed / 0 failed
```
Boundaries covered by named suites: zero liquidity / dust / huge amounts / tick & bin boundaries / fee transitions / reversed direction / overflow (adversarial + differential + fuzz suites in `arb-math/tests`).
