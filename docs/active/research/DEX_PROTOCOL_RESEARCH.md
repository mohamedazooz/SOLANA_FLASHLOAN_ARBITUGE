# DEX Protocol Research

**S3 verification date**: 2026-11-22. Deep per-protocol verification lives in register Sessions 1–2 and in [`../audit/` + QUOTE_PARITY_REPORT](); this doc records the canonical source-of-truth set and the open risk areas.

## Verified facts

| Protocol | Program ID / discriminator | Source |
|---|---|---|
| Meteora DLMM | `lb_clmm` = `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`; `swap2` discriminator `[65, 75, 63, 76, 235, 91, 91, 136]` | [solroute pkg (mirror)](https://pkg.go.dev/github.com/solana-zh/solroute/pkg/pool/meteora) (S3), matches repo `METEORA_SWAP2_DISCRIMINATOR` (register S2) |
| DLMM structure | bins (70 per BinArray), dynamic (volatility-accumulator) fees, `swapForY` direction, slippage in bps, token-program detection + transfer-hook extra-account-metas on load | [DLMM TS SDK reference](https://docs.meteora.ag/developer-guides/dlmm/typescript-sdk/reference) (S3) |
| DLMM risk class | wrong bitmap/extension handling → on-chain errors 6036/3008; bitmap-extension PDA needed when crossing beyond the internal 16-word bitmap | [Iron_crab PR summarization](https://github.com/Exploratorsclub/Iron_crab/pull/480) (S3, secondary) — treat as risk note, verify against official source before acting |
| Raydium CLMM | post-2026-09-30 `PoolState` = 1,544 B; offsets verified live: liquidity u128 @237, sqrt_price @253, tick_current @269, status @389, fee_on @390, dynamic_fee @1096..1176; arrays start on 60·spacing multiples; fee gross-up `fee = ceil(in_max×rate/(1e6−rate))` | register Session 1, verified against official SDK 0.2.85-alpha (inherited) |
| Orca Whirlpool | single-sequence tick-array view; keep fixture inside active tick boundary (PR #97) | register (inherited) |
| Kamino | `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD` (mainnet; `SLend…` = staging feature) | [klend architecture docs](https://www.mintlify.com/kamino-finance/klend/concepts/architecture) (S3) |

## Existing implementation (support classification)

| Protocol | Decode | Quote | Build | CPI | Notes |
|---|---|---|---|---|---|
| Meteora DLMM | ✅ | ✅ (SDK-parity fixture-tested) | ✅ swap2 | ✅ test-only harness | fee-mode gate (collect_fee_mode=1, PR #121) enforced |
| Raydium CLMM | ✅ (1544B offsets) | ✅ (JS SDK parity across 6 live pools) | ✅ | n/a | empty-span + wide-step parity fixed (PR #118 line) |
| Raydium CPMM | ✅ | ✅ | ✅ | n/a | 2-pool atomic tests |
| Raydium AMM v4 | ✅ | ✅ basic | ✅ | n/a | oldest curve; quote parity breadth thin |
| Orca Whirlpool | ✅ | ✅ | ✅ | n/a | tick-array fixture corrected |
| Pump.fun / PumpSwap | ✅ | ✅ (PumpSwap only; PumpFun fails closed) | ⚠️ (`swap_hop` CLMM & PumpFun gaps) | n/a | See `DEX_SUPPORT_MATRIX.md`; PumpSwap `PUMP_CASHBACK_README` requires cashback accumulator ATA remaining accounts when `is_cashback_coin = true`; Pump.fun `create_v2` deploys Token-2022 mints |

## 2026-11-26 Protocol Verification Addendum

- **PumpSwap & Pump.fun (`pump-fun/pump-public-docs`, `PUMP_CASHBACK_README.md`)**:
  - Pump.fun `create_v2` creates **Token-2022** mints (`TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`) and supports `mayhemMode` / `cashback` flags. Bonding-curve `sell` on cashback coins requires the writable `user_volume_accumulator` PDA at `remaining_accounts[0]`.
  - PumpSwap AMM (`buy`/`sell` on cashback coins) requires the quote WSOL ATA of `user_volume_accumulator` (`remaining_accounts[0]`, and `user_volume_accumulator` PDA at `[1]` on sells).
  - PumpSwap pool decode must also handle negative `virtual_quote_reserves` (`i128`) and preserve decoded `base_token_program` / `quote_token_program` into execution (`PUMPSWAP-01`).
- **Raydium CLMM (`raydium-io/raydium-clmm`, `tick_math.rs`, `swap.rs`)**:
  - `MIN_SQRT_PRICE_X64 = 4295048016`, `MAX_SQRT_PRICE_X64 = 79226673521066979257578248091`. `sqrt_price_limit_x64` is a Q64.64 square-root price bound (or `0` for unbounded), checked strictly against current sqrt price and `MIN/MAX`, never a raw `amount_out / amount_in` ratio (`SWAP-01`).
- **Meteora DLMM (`MeteoraAg/dlmm-sdk`)**:
  - `swap2` encodes Token-2022 Transfer Hook extra-account metas via `RemainingAccountsInfo` (`getExtraAccountMetasForTransferHook`); mints with Transfer Hooks must remain rejected fail-closed unless hook metas are explicitly resolved (`TOKEN-003`).

Status sources: `COMPONENT_STATUS.md`, `CURRENT_STATUS.md` (matrix), `QUOTE_PARITY_REPORT.md`.

## Gaps / recommendations (REC)

1. AMM v4 parity breadth is the weakest quote surface (basic constant-product only) — schedule a JS-SDK differential like the CLMM/DLMM rounds before expanding v4 exposure.
2. Token-2022 (transfer fees, transfer-hook extra account metas) remains fail-closed-excluded from discovery (ISSUE-P1-01); the DLMM SDK's auto transfer-hook metas handling shows the eventual implementation cost. Keep excluded until epoch-aware mint data is wired end-to-end.
3. Jupiter integration is out of the execution path by design (route discovery/aggregation only); do not add jup CPI without an explicit decision record.

## Tests required

- Differential: protocol quote vs official SDK on captured live snapshots (exists for DLMM/CLMM; add AMM v4 + PumpSwap sweep).
- Boundary: bin/tick-array boundary crossing, zero-liquidity bins, extreme amounts, reversed direction (partially covered by the parity matrices; keep as CI gates).
