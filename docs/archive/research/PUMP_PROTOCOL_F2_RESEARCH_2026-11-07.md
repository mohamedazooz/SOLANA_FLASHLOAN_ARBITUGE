# Pump Protocol F2 Research Notes — 2026-11-07

## Primary sources consulted

- Pump official public docs: `docs/PUMP_SWAP_README.md` — PumpSwap program ID, GlobalConfig PDA, disable flags, effective quote reserve, and instruction semantics.
- Pump official public docs: `docs/FEE_PROGRAM_README.md` — canonical pool definition (`pool.creator`, not `coin_creator`), pool market-cap formula, tier selection, and non-canonical flat fees.
- Pump official IDL: `idl/pump_amm.ts` — PumpSwap program ID, GlobalConfig and Pool discriminators, FeeConfig PDA seed contract.
- Pump Rust client docs.rs: [AMM quote source](https://docs.rs/pump-rust-client/latest/src/pump_rust_client/math/amm.rs.html) and [fee math source](https://docs.rs/pump-rust-client/latest/src/pump_rust_client/math/fees.rs.html) — exact-in fee rounding, quote reserve, real quote reserve, fee-bucket semantics, tier selection, creator-fee gate, and disable buy/sell bits.
- Pump Rust client docs.rs: [PDA source](https://docs.rs/pump-rust-client/latest/src/pump_rust_client/pda.rs.html) — GlobalConfig, FeeConfig and pool-authority PDA seeds.
- Pump official release notes — PumpSwap V2, fee retention, virtual quote reserve adjustments, holder rewards, and synthetic migration.

## Implementation consequences

- Canonical PumpSwap pool classification uses `pool.creator` against the Pump pool-authority PDA.
- Effective quote reserve is raw quote-vault amount plus signed `virtual_quote_reserves`.
- Spendable quote liquidity excludes un-swept protocol and creator fee buckets.
- PumpSwap FeeConfig PDA uses `fee_config` + PumpSwap program ID as seeds under the Pump Fees program.
- GlobalConfig is a separate PumpSwap-owned PDA; disable flag bits 3 and 4 gate buy and sell. In the current 949-byte Borsh layout, `creator_fee_configurable` is byte 940 and `max_configurable_creator_fee_bps` occupies bytes 941..949; the legacy 940-byte layout has no creator-fee override gate.
- Canonical-pool market cap is `quote_reserve * base_mint_supply / base_reserve`.
- FeeConfig has standard tiers, stable tiers, and optional exotic flat fees; non-canonical pools use flat fees.
- On-chain fee components are whole BPS and are normalized to PPM only at the adapter boundary (`1 BPS = 100 PPM`).
- Exact-in buy fee math floors the net, ceils each component fee on the unshaved net, then shaves any rounding overrun before swapping `net - 1`.
- Sell math ceils each fee component and checks real quote liquidity against `raw_quote - lp_fee`.
- The effective pricing reserve is exactly `quote_vault_balance + virtual_quote_reserves`, even when the virtual reserve is negative (a negative resulting reserve is rejected).
- The first fee tier is the fallback below its threshold.
- The creator-fee override is applied only when `GlobalConfig.creator_fee_configurable` is true and the pool's stored creator fee is non-zero.

## Limitations

Pump IDLs and SDKs evolve. The custom decoder is bounded and rejects unknown non-zero FeeConfig extensions. Before live execution, compare the decoder and quote implementation against the current official `pump-rust-client` and official SDK outputs for every supported pool class and quote mint.

## Pump.fun is a separate fee/state contract

- Current BondingCurve account layout is 83 bytes including Anchor discriminator, five u64 reserve/supply fields, `complete`, `creator`, `is_mayhem_mode`, and `is_cashback_coin`. The decoder now requires the current complete layout and rejects legacy/truncated data that lacks mode flags.
- The Pump.fun bonding-curve FeeConfig PDA uses seeds `[b"fee_config", pump_fun_program_id.as_ref()]` under the Pump Fees program. The generated IDL names the second 32-byte seed `FEE_CONFIG_EXTRA_SEED`; its bytes are exactly the Pump program ID bytes. PumpSwap uses its own PumpSwap program ID as the second seed, so the two FeeConfig PDAs differ. See the generated Pump.fun SDK constants: https://docs.rs/pumpfun-bondingcurve-sdk/latest/src/pumpfun_bondingcurve_sdk/constants.rs.html and the official Pump IDL: https://github.com/pump-fun/pump-public-docs/blob/main/idl/pump.ts
- The Pump.fun SDK's `resolve_fees` uses the curve's `real_sol_reserves` as its market-cap proxy, selects the last matching fee tier (or flat fees), and conditionally includes cashback based on both curve and Global flags. See https://docs.rs/pumpfun-bondingcurve-sdk/latest/src/pumpfun_bondingcurve_sdk/amm.rs.html
- The same SDK calculates buy fees from SOL input and sell fees from gross SOL output using integer BPS floor arithmetic. This adapter is not enabled for live discovery until its curve/global/fee decoders are implemented and differentially validated.
