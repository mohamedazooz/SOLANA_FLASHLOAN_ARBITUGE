# Protocol Compatibility

> **Superseded (2026-11-13):** the "fail closed" status for Pump.fun / PumpSwap
> in this document is stale. Both adapters are implemented and reachable in
> `crates/arb-tx/src/protocols/swap_hop.rs`. See
> [`../quality/PHASE_16_PROTOCOL_MATRIX.md`](../quality/PHASE_16_PROTOCOL_MATRIX.md)
> for the reconciled matrix. The acceptance caveat below still applies: a
> reachable builder is not live acceptance evidence.

Status: source-level inventory only (revalidated 2026-11-08). "Quote" means a decoder/quote model exists;
"Build" means the transaction builder can emit a swap instruction; a supervised source path
does not mean production acceptance. No venue in this archive has complete differential +
fork acceptance evidence. Keep live execution disabled until each venue passes its own gate.

Official SDK terminology, versioned source references, and the limited Orca
math-parity test are recorded in [`SDK_PARITY.md`](SDK_PARITY.md). Those
references do not change the acceptance status below.

## 1. Program IDs

All IDs are centralized in `arb-constants`. Program IDs are mainnet IDs; no
devnet/testnet matrix is claimed.

| Protocol | Program ID | Pool kind |
|---|---|---|
| Raydium AMM v4 | `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8` | CPMM |
| Raydium CP-Swap | `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C` | CPMM |
| Raydium CLMM | `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK` | CLMM (Q64.64 ticks) |
| Orca Whirlpool | `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc` | CLMM (Q64.64 ticks) |
| Meteora DLMM | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` | Discrete bins |
| Pump.fun bonding curve | `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P` | Bonding curve |
| PumpSwap | `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA` | CPMM (post-graduation) |
| Kamino Klend | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` | Flash-loan lender |

Associated accounts are derived per protocol from the pool state and constants;
the TDAs and fee accounts are protocol-specific and are the highest-risk area
for account-meta drift.

## 2. Capability matrix

| Protocol | Decode | Quote | Build swap ix | Supervised execution | Evidence / caveat |
|---|---|---|---|---|---|
| Raydium v4 | Yes | Yes | Yes | Source path only; NOT ACCEPTED | Account matrix assembled from pool state; no on-chain differential run. |
| Raydium CPMM | Yes | Yes | Yes | Source path only; NOT ACCEPTED | Anchor-style accounts; no on-chain differential run. |
| Raydium CLMM | Yes | Yes | Yes | Source path only; NOT ACCEPTED | Requires tick-array coverage; missing coverage fails closed (`CLMM-01`). |
| Orca Whirlpool | Yes | Yes | Yes | Source path only; NOT ACCEPTED | Q64.64 math checked; no on-chain differential run. |
| Meteora DLMM | Yes | Yes | Yes | Source path only; NOT ACCEPTED | Bin ladder + volatility fee; unrepresentable prices return `MathOverflow` (`DLMM-01`). |
| Pump.fun | Yes | Yes | **No — fail closed** | **No** | Builder returns `UnsupportedProtocol("Pump.fun live adapter disabled…")` until the buy/sell account matrix and fee mode are verified. |
| PumpSwap | Yes | Yes | **No — fail closed** | **No** | Builder returns `UnsupportedProtocol("PumpSwap transaction adapter is not enabled; shadow-only")`. |

A source-level builder/supervisor path is not a production capability: universal guarded-quote
enforcement is still open (`GUARD-01`), and no route is accepted for live trading by this matrix.
Even a successful RPC/Jito acceptance would not establish landing or realized PnL.

## 3. Per-protocol dependencies

- **Raydium v4**: pool state, Serum/OpenBook market accounts, vaults, token
  mints, fee account.
- **Raydium CPMM**: pool state, pool authority, token vaults, observation state
  where present.
- **Raydium CLMM**: pool state, tick arrays covering the swap range, token vaults,
  observation state.
- **Orca Whirlpool**: pool state, tick arrays, token vaults, oracle account.
- **Meteora DLMM**: pool state, bin arrays covering the swap range, token vaults,
  oracle account.
- **Pump.fun**: bonding curve, associated bonding curve, global config, fee
  recipient, event authority, fee config. The account matrix is **not** trusted
  for signing in this build.
- **PumpSwap**: pool, global config, pool authority, token vaults, fee config.

`DependencyManifest`/`DependencyManifestRegistry` express required accounts and
detect duplicate/conflicting ownership. The guarded quote path uses them; the
main supervisor path uses `quote_dependencies_for_route` directly, so manifest
enforcement is not universal (`GUARD-01`).

## 4. Version and decode assumptions

- No protocol upgrade/version field is negotiated at runtime. Decoders assert
  fixed offsets and discriminator/length checks; a layout change should fail
  decode rather than silently mis-quote.
- Pump.fun and PumpSwap discriminators are checked (e.g.
  `PUMP_SWAP_POOL_DISCRIMINATOR`, `PUMP_FUN_BONDING_CURVE_DISCRIMINATOR`).
- Token-2022 transfer-fee configs are decoded and active-epoch fees are applied;
  a risky extension trailer is rejected. Token-2022 coverage is not claimed for
  every mint.
- Decimal normalization is derived from mint metadata in the ingest path. A
  missing/zero decimal is not defaulted to 9 in critical paths; it is rejected.

## 5. Open protocol work

1. Differential fixtures against official SDK/on-chain quotes for every enabled
   protocol (13.2); see [`SDK_PARITY.md`](SDK_PARITY.md) for current references.
2. Real transaction account-meta verification on a fork for each builder path
   (14.2).
3. Pump.fun/PumpSwap execution adapter verification before enabling either
   (13.3).
4. Token-2022 mint coverage matrix, including transfer-hook/confidential
   extensions (out of scope for this build).
5. Address lookup table construction and v0 message size acceptance per route.
