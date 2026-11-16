# PHASE 12 — Best-Practices Research (verified sources)

## Sources consulted (this + prior session, with application)

| Source | Verified fact | Application | Status |
|---|---|---|---|
| solana.com/docs + Helius guide | `getMultipleAccounts` batch limit 100 | `batch_fetch_accounts` batch-100 | IMPLEMENTED (`228c330`) |
| Solana docs (WS guidance) | subscribe-don't-poll for continuous state | WS arms write cache directly | IMPLEMENTED |
| Meteora DLMM program/SDK (official) | base fee `bf × bs × 10 × 10^pfp` (1e9); variable `⌈vfc×(VA×bs)²/1e11⌉`; CollectFeeMode semantics | `decode_dlmm_fee_bps`, `compute_variable_fee`, `consume_bin_liquidity` | VERIFIED + oracle |
| orca-so/whirlpools (main, whirlpool.rs) | interleaved mint/vault layout; LEN 653 (3 rewards) | decoder offsets @101/@133/@181/@213 | VERIFIED by live probe |
| raydium-io CLMM (PoolState) | offsets @73/@105/@137/@169, dec @233/@234 | decoders + `clmm_onchain_mints` | VERIFIED by live probe |
| Kamino klend IDL | flash_borrow/flash_repay discriminators + ordering + instruction sysvar | arb-tx/flash/kamino.rs | VERIFIED |
| SPL/ATA docs | ATA derivation with program id | token account math | VERIFIED |
| SIMD-0153 / packet limits | 1232B packet limit; ALT compresses keys only | transport sizing tests | RESPECTED |

## Anti-patterns rejected (with evidence)

- Stale-cache fallback on provider failure → prohibited; fail-closed instead (soak-proven).
- Sequential RPC bursts → batched (429 evidence above).
- Hardcoded fees/CU/sizes → all decoded from on-chain bytes or config (grep-verified).
- Silent orientation trust → on-chain canonicalization with loud divergence WARN.

## Uncertainties

- V1 transaction support across providers remains unsettled (no provider evidence).
- Public RPC (publicnode) acceptable for shadow/research; NOT for live execution
  (429s + public mempool exposure). Jito/private endpoint required for live.
