# PHASE 5 — Protocol Verification

## Meteora DLMM — VERIFIED (live probe + official formula)

- Program: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`.
- LbPair layout: `token_x @88..120`, `token_y @120..152` (probe-verified live, both
  orientations), bins/bin arrays hydrated separately (fail-closed on missing active bin).
- token_x = lexicographically smaller mint (program invariant) — used as canonical base.
- Fees: dynamic base + variable verified against official formula (see E6 in
  MATHEMATICAL_VERIFICATION.md).
- Oracle: single-bin consumption 6/6 vectors match (E4/E5).

## Orca Whirlpools — VERIFIED (live probe, slot 468876781)

- Program: `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`.
- **Live layout is the interleaved generation** (probe pool `Czfq3x…44zE`, data_len 653):
  `mint_a @101..133`, `vault_a @133..165`, `fee_growth_a @165..181`, `mint_b @181..213`,
  `vault_b @213..245`. Offsets are independent of NUM_REWARDS → both 525B and 653B
  generations decode identically (unit-tested).
- Catalog (api.orca.so v2) `tokenMintA/B` == on-chain bytes (live fetch cross-check).
- **No lexicographic mint-order invariant on live pools** (live pool has mint_a > mint_b) —
  orientation must be read per-pool (now enforced via `clmm_onchain_mints`).

## Raydium CLMM — VERIFIED (live probe ×2 pools)

- Program: `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`. PoolState LEN 1544.
- `mint0 @73..105`, `mint1 @105..137`, `vault0 @137..169`, `vault1 @169..201`,
  `dec0 @233`, `dec1 @234`, `tick_spacing @235`, `liquidity @237..253`,
  `sqrt_price @253..269`, `tick_current @269..273` — all probe-verified.
- No `mint0 < mint1` enforcement → per-pool orientation read (same fix as Orca).

## Raydium V4 / CPMM — VERIFIED structurally

- V4 `AmmInfo`: `coin_vault @336`, `pc_vault @368`, mints `@400/@432` (live-verified offsets).
- CPMM: `vault_0 @72..104`, `vault_1 @104..136`, mints `@168/@200`, dec `@331/@332`.
- Fee decoders: `decode_raydium_fee_bps`, `decode_raydium_cpmm_fee_bps`.

## Kamino flash loan — VERIFIED (earlier session, re-checked)

- Program: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`.
- `flash_borrow_reserve_liquidity` (first) / `flash_repay_reserve_liquidity` (last)
  with `Sysvar1nstructions…` introspection enforced in the compiler (arb-tx/flash).
- Discriminators verified against official IDL; fee from reserve config bytes, not hardcoded.

## Transaction transport

- V0 + ALT: primary path; measured serialization tests exist (arb-tx).
- V1 (4096B): synthetic capacity tests + header differential present
  (`v1_header_differential.rs`); **live V1 submission NOT VERIFIED** (no provider
  commitment to V1 on current infra; documented blocker).
- Packet limit: 1232 bytes for V0 — respected; ALT compresses account keys only
  (correct understanding verified in transport code).

## Token program / ATA / Token-2022

- Token-2022 pools excluded at discovery (Orca arm skips `token2022: true`) —
  conservative, correct until transfer-fee modeling lands.
- ATA derivation via SPL `get_associated_token_address_with_program_id`.

## NOT VERIFIED / BLOCKED

| Item | Reason |
|---|---|
| Live V1 broadcast | No RPC provider evidence of V1 support in target infra |
| Yellowstone gRPC live feed | No provider exposing it in current env (plumbing only) |
| Pump.fun curves | Provider 403 (P2-2) |
