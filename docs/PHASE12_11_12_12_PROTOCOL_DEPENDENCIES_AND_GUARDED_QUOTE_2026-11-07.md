# Phase 12.11 + 12.12 — Protocol Dependency Resolution and Guarded Quote

Date: 2026-11-07

## Phase 12.11 — Protocol dependency resolver

Added `resolve_protocol_dependencies` and protocol-specific required-role validation. It does **not** invent PDA addresses. The caller must supply addresses derived by the official SDK/IDL or decoded from a verified on-chain account. This is intentional: dynamic CLMM tick arrays and DLMM bin arrays depend on current price, direction and traversed liquidity.

Required roles:

- Common: PoolState, VaultA, VaultB, MintA, MintB.
- Raydium CPMM: FeeConfig.
- Raydium CLMM: FeeConfig, Observation, at least one TickArray.
- Orca Whirlpools: at least one TickArray; add Oracle when adaptive-fee configuration requires it. Current Whirlpool swap source marks the oracle unused for standard pools, but adaptive-fee integrations may require it.
- Meteora DLMM: at least one active BinArray; add BitmapExtension when traversal leaves the default bitmap range.
- Pump bonding curve: BondingCurve, GlobalConfig, FeeConfig, CreatorVault.
- PumpSwap: GlobalConfig, FeeConfig, CreatorVault.

The manifest roles describe state needed by quoting. Transaction-only accounts (signer, user ATAs, sysvars, program executable accounts, protocol fee recipients) belong in the instruction account builder and must be checked there separately. Token-2022 mint TLV remains part of the MintA/MintB account bytes; transfer-hook extra accounts must be represented as `TransferHook` dependencies when the mint's extension requires them.

## Phase 12.12 — Guarded route quote

Added `arb_engine::quote_route_guarded` and `revalidate_before_dispatch`. The first function obtains a complete dependency set from Yellowstone, prices the route, and re-reads the dependency set. It rejects if any version, bank, provider, generation or dependency identity changed during quote construction. The dispatch guard repeats the check immediately before sending.

The guarded quote API requires a decoder callback; the callback must decode all protocol state from the supplied snapshots, not from a secondary RPC/cache. It checks the decoded pool account slot and protocol identity. Protocol-specific binary decoding still belongs to each adapter and must be independently tested against official IDLs/SDK fixtures. This makes the binding explicit, but correctness of each decoder remains an adapter acceptance requirement. All live transaction paths must be migrated to this entry point; legacy `quote_route` remains a pure function and can still be called directly.

## Research basis

- Raydium CPMM pool state references its `amm_config`, vaults and token mints; current program docs describe config and fee accounts.
- Raydium CLMM uses pool/config, tick-array PDAs and observation account.
- Orca Whirlpool swap instruction consumes up to three tick arrays; oracle is documented as unused for standard pools and conditionally relevant to adaptive fees.
- Meteora DLMM documents BinArray, BinArrayBitmapExtension, Oracle and the 70-bin array geometry; extension is needed outside the default bitmap range.
- PumpSwap docs identify GlobalConfig and Pump Fees `fee_config`; current trade account list identifies the pool, mints/vaults and creator-vault-related accounts. Pump bonding curve and PumpSwap fee schedules are mutable and must be read live.

## Validation status

Source-level tests added for missing CLMM tick/observation roles, DLMM bin arrays, and PumpSwap creator fee dependencies. No `cargo fmt`, `cargo check`, `cargo test`, or `clippy` result is claimed: the current environment does not have the Rust toolchain installed. ZIP integrity, patch dry-run and diff whitespace checks must be run after packaging.

## Known limitations / next work

1. No PDA derivation is performed in the resolver; use official SDK helpers and test vectors for every supported program version.
2. Pump.fun/PumpSwap account sets evolve; validate against current IDL/instruction account metas before enabling execution.
3. CLMM/DLMM active-array selection must be derived from current price, swap direction and exact traversal, not a fixed static list.
4. Decode quote state from the exact snapshots and pass typed decoded views into quote functions.
5. Migrate every live quote/dispatch path; keep raw `quote_route` inaccessible to production dispatch via module visibility or executor API.
6. Add LiteSVM/mainnet-fork tests that compare generated metas against official SDK outputs.

## Primary references (reviewed 2026-11-07)

1. Raydium CPMM PoolState / `amm_config` / vault and mint fields: https://docs.rs/raydium-cp-swap/latest/src/raydium_cp_swap/states/pool.rs.html
2. Raydium PDA and CPMM/CLMM account integration notes: https://github.com/raydium-io/raydium-docs-v1/blob/main/solana-fundamentals/pdas-and-cpis.mdx
3. Raydium CLMM instruction/account reference: https://github.com/raydium-io/raydium-docs-v1/blob/main/products/clmm/instructions.mdx
4. Orca Whirlpool swap source (tick arrays and oracle behavior): https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/instructions/swap.rs
5. Meteora DLMM program accounts, BinArray, bitmap extension and PDA map: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/program/accounts.mdx
6. PumpSwap state and GlobalConfig: https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_SWAP_README.md
7. PumpSwap trade v2 account list and fee config: https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/PUMP_SWAP_TRADE_V2.md
8. Pump bonding curve mechanics and graduation: https://pump.fun/docs/bonding-curve
9. Pump fee schedules: https://pump.fun/docs/fees
