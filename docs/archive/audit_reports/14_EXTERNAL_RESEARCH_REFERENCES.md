# 14 — External Research References

Compiled 2026-11-11. Every claim below was retrieved from a primary source during this audit; where
a fetch failed it is marked **UNVERIFIED** rather than guessed. solana.com direct pages were
rate-limited (HTTP 429); claims were verified against the official solana-foundation content mirror
(`github.com/solana-foundation/solana-com/apps/docs/content/...` — same text, version-controlled) or
`docs.anza.xyz`. Source type legend: **SC** = official source code; **OD** = official docs;
**ODM** = official docs (mirror); **SIMD** = official Solana improvement document.

## 1. Solana wire formats, 1232-byte limit, ALTs

| Claim | Source | Type | Authoritative | Supports |
|---|---|---|---|---|
| `PACKET_DATA_SIZE = 1232` (1280−40−8); legacy/v0 max 1232 B | `github.com/anza-xyz/solana-sdk/blob/master/packet/src/lib.rs` | SC | yes | §08 §2; live gate `dispatch.rs:170` |
| Version prefix `0x80` (v0), high bit clear = legacy; `V1 = 0x81` | `solana-sdk/.../message/src/versions/mod.rs`, `.../versions/v1/mod.rs` | SC | yes | §08 |
| `MAX_LEGACY_AND_V0_TRANSACTION_SIZE = 1232`; v1 = 4096; `MAX_V1_SIGNATURES = 12`; v1 max 64 inline accounts | `agave/.../runtime-transaction/src/transaction_limits.rs` | SC | yes | §08 |
| "v0 = legacy message bytes + 0x80 + `address_table_lookups` tail" | `docs.anza.xyz/proposals/versioned-transactions`; solana-com mirror `core/transactions/versioned-transactions.mdx` | OD / ODM | yes | §08 §2 |
| v1 is **active on mainnet, devnet, testnet**; needs `maxSupportedTransactionVersion: 1`; base58 capped at 1232 | solana-com mirror `versioned-transactions.mdx` | ODM | yes | §08 §1 (V1 is real on chain; the repo has no submission path) |
| SIMD-0385 status **Activated**; no ALTs in v1; `NumInstructions > 64` fails; config in `TransactionConfigMask`; ComputeBudget ixs are no-ops | `solana-foundation/solana-improvement-documents/blob/master/proposals/0385-transaction-v1.md` | SIMD | yes | §08 §3 |
| Agave feature `enable_tx_v1` (`txv1aq4p…`) | `agave/.../feature-set/src/lib.rs`; Agave CHANGELOG "The `solana` CLI now creates version 1 transactions" | SC | yes | §08 §1 |
| `DEFAULT_TX_ACCOUNT_LOCKS = 64` (legacy/v0/v1); v1 → 96 behind SIMD-0596 (Draft, `8ZVMrzG6pPBREHMbHNyrErKJsXA7b1vg9e1iycSooCeY`); **128 obsolete** | `agave/.../accounts-db/src/account_locks.rs`; `runtime/src/bank.rs`; SIMD-0596 | SC/SIMD | yes | §06 (lock budget); §08 |
| ALT: 256 addresses/table; `LOOKUP_TABLE_META_SIZE = 56`; owned by ALT program; same-slot extensions invisible; ~512-slot SlotHashes cool-down; **deactivation permanent (no reactivate)**; ALT-loaded accounts cannot sign | `solana-sdk/.../address-lookup-table-interface/src/state.rs`; `solana-program/address-lookup-table` program source | SC | yes | §07 §3 |
| Runtime loads ≤ **64 accounts per transaction** regardless of ALT size; v0 remaining addresses appended after static keys (writable lookups first) | solana-com mirror `cookbook/transactions/lookup-tables.mdx`; `agave/.../accounts.rs load_lookup_table_addresses_into` | ODM/SC | yes | §07 §6 |
| Signers cannot be ALT-loaded | solana-com mirror; `compile_v0_message` privilege routing (code) | ODM/code | yes | §07 §1 |
| Compute budget: 200k CU/non-builtin ix, 3k builtin (SIMD-0170), 1.4M/tx cap, 64 MiB loaded data, priority fee = ceil(price×limit/1e6); no "20,000 CU" constant exists | solana-com mirror `fees/compute-budget.mdx`, `fees/fee-structure.mdx`; `getrecentprioritizationfees.mdx` (150-block cache, ≤128 addresses) | ODM | yes | 00; §11 |

## 2. Jito

| Claim | Source | Type | Supports |
|---|---|---|---|
| Bundles: **max 5 txs**, sequential, all-or-nothing, tip ≥ **1,000 lamports**, tip = any SOL transfer to one of **8 tip accounts** (ADaUMid…, DfXygS…, DttWaM…, ADuUkR…, Cw8CFy…, 96gYZG…, 3AVi9T…, hFqU5x…); do **not** ALT-load tip accounts; pick at random | docs.jito.wtf/lowlatencytxnsend | OD | §11 §1-2 |
| `bundleOnly=true` = single-tx bundle, revert protection | same + /bundles/ | OD | §11 §1 |
| `getBundleStatuses` / `getInflightBundleStatuses` (5-min lookback, max 5 ids, max recent-300-slots) | same | OD | §11 §4 |
| Tip floor via **REST** `bundles.jito.wtf/api/v1/bundles/tip_floor` and **WS** `tip_stream` (not JSON-RPC `getTipFloor` — that naming is UNVERIFIED) | same | OD | §11 §3 (the repo polls the right URLs, clamp ≥1 s for 1 rps/IP) |
| Bundles land only if every tx simulates and all execute | same | OD | §11 §1, §00 |
| UUID auth via `x-jito-auth` header or `?uuid=` | same | OD | §11 §1 (repo uses none — tip-auth only) |
| jito-solana repo exists (contents not re-verified this session) | github.com/jito-foundation/jito-solana | OD repo | — |

## 3. Meteora DLMM (`lb_clmm` `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`)

| Claim | Source | Type | Supports |
|---|---|---|---|
| `MAX_BIN_PER_ARRAY = 70`; `BIN_ARRAY_BITMAP_SIZE = 512` (default −512…511); `BinArray` PDA `["bin_array", lb_pair, i64 idx_le]`; `BinArrayBitmapExtension` PDA `["bitmap", lb_pair]` **mandatory outside default range** (`BitmapExtensionAccountIsNotProvided` otherwise); `Oracle` PDA `["oracle", lb_pair]`; bin price P=(1+step/1e4)^i; fee denom 1e9; MAX_FEE_RATE=1e8; `bin_id` signed i32 | docs.meteora.ag developer-guides/dlmm/program/accounts; IDL raw (idl.json) | OD/SC | §06 §1-3 |
| Outputs floor / inputs ceil / protocol fee floor ("protocol keeps dust"); variable fee `ceil(vfc·(va·bs)²/1e11)`; volatility decay `reduction_factor/10_000` (BASIS_POINT_MAX) | docs.meteora.ag core-products/dlmm/formulas | OD | §02 §1; §06 §6-7 |
| **`swap_for_y` is an event field; direction derives from account ordering** (no swap_for_y byte in `swap`/`swap2` args) | same IDL/docs | OD/SC | §05 §2 |
| HOST_FEE_BPS = 2000 (20% of protocol fee); MAX_PROTOCOL_SHARE = 2500; MAX_BIN_STEP = 400; EXTENSION_BINARRAY_BITMAP_SIZE = 12 | IDL constants | OD | §06 §7, §13 §3 |

## 4. Token-2022

| Claim | Source | Type | Supports |
|---|---|---|---|
| Transfer fee: `fee = ceil(amount × bps / 10_000)` capped by `maximum_fee`; ceil both directions; withheld per recipient account; harvest via WithdrawWithheld* ixs; effectiveness gated by `epoch`; schedule changes per epoch | `spl-token-2022-interface/src/extension/transfer_fee/*`; spl.solana.com/token-2022 | SC/OD | §03 §4, §02 §2.1 |

## 5. Kamino flash loans

| Claim | Source | Type | Supports |
|---|---|---|---|
| Borrow/repay same top-level tx; repay = borrow + `flash_loan_fee_sf`; **U68F60 fixed point** (`Fraction = fixed::types::U68F60`); **min 1 lamport**; CPI forbidden; one borrow per tx; repay mandatory; `borrow_instruction_index` ties pair | docs.kamino.com …/flash-loans; `handler_flash_borrow_reserve_liquidity.rs`, `handler_flash_repay_reserve_liquidity.rs`, `flash_ixs.rs`, `lending_operations.rs`, `state/reserve.rs`, `utils/fraction.rs` | OD/SC | §10 §1-2 |
| SOL/BTC-market USDC reserve fee ≈ 0.001% | docs.kamino.com | OD | order-of-magnitude check ok (code measured sf ≈ 6.25e-7) |

## 6. Raydium / Whirlpool discriminator ground truth (local SHA-256 verified)

| Item | Value | Source |
|---|---|---|
| CPMM `swap_base_input` | `[143,190,90,218,196,30,51,222]` | raydium-idl/raydium_cpmm/raydium_cp_swap.json (OD, IDL) |
| CPMM `swap_base_output` | `[55,217,98,86,163,74,180,173]` | same |
| CLMM `swap` | `[248,198,158,145,225,117,135,200]` | raydium-idl/raydium_clmm/raydium_clmm.json |
| CLMM `swap_v2` | `[43,4,237,11,26,201,30,98]` | same |
| AMM v4 tags (no IDL): SwapBaseIn=9, SwapBaseOut=11, V2=16, V2-out=17; OpenBook removed 2026-07; classic SPL only | docs.raydium.io/products/amm-v4 | OD |
| Whirlpool `swap` sighash | identical bytes `[248,198,158,145,225,117,135,200]` = sha256("global:swap")[..8] — shared with CLMM/DLMM | whirlpools source + local hash |
| `global:swap` shared across programs ⇒ dispatch on **program id + discriminator**, never discriminator alone | computed locally | all adapters |

## 7. Explicitly UNVERIFIED / conflicting

1. SIMD-0385 `feature:` line blank in the SIMD file though status = Activated; mainnet activation
   slot not published — treated as active (docs+agave+CHANGELOG agree).
2. SIMD-0596 (v1 96 locks): **Draft**; mainnet activation NOT confirmed — assume 64.
3. "MAX_TX_ACCOUNT_LOCKS = 128" — obsolete; current 64/96.
4. Jito JSON-RPC `getTipFloor` naming — UNVERIFIED (REST/WS are the documented transports).
5. Whirlpool program id not verified on-chain this session.
6. Raydium v4 2026-07/09 upgrades (OpenBook removal, `WithdrawExcessLamports` tag 18) per docs; account decode must tolerate reserved-padding fields.
7. GitHub API rate-limited mid-audit; critical files fetched HTTP 200 via raw; "list everything" queries degraded — content integrity unaffected.

## 8. Verification recipes (re-runnable)

```
# sighash check
printf 'global:swap' | sha256sum | cut -c1-16    # → f8c69e91e17587c8
# ALT activation on a cluster
solana feature status | grep -E 'txv1|account_lock'
# tip floor shape
curl -s https://bundles.jito.wtf/api/v1/bundles/tip_floor | jq .
# Kamino reserve decode (klend SDK)
sdk.reserve.state.config.fees.flashLoanFeeSf         # U68F60
```
