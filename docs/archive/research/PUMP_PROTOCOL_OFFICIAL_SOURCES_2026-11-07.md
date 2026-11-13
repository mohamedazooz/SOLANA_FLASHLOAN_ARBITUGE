# Pump protocol source register — 2026-11-07

Only official Pump documentation is used for protocol layout/fee assumptions in this phase.

1. Pump bonding curve: https://pump.fun/docs/bonding-curve
   - Virtual-reserve constant-product curve; graduation is automatic and irreversible; standard bonding-curve fee is 1.25%.
2. Pump fee schedule: https://pump.fun/docs/fees
   - Last updated 17 October 2026; SOL and USDC canonical PumpSwap tier schedules; custom-pair and holder-reward rates; fees collected in quote asset.
3. PumpSwap program README: https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_SWAP_README.md
   - PumpSwap program id, GlobalConfig, Pool account fields, effective quote reserve formula, pool PDA seeds and buy/sell instruction semantics.
4. Pump sweep fees: https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/SWEEP_FEES.md
   - Pump V3 and PumpSwap V2 retain protocol/creator fees on curve/pool and expose sweep instructions; trade fee accounting and payout events must not be double-counted.
5. Pump fee program: https://github.com/pump-fun/pump-public-docs/blob/main/docs/FEE_PROGRAM_README.md
   - Canonical PumpSwap pools are detected from `pool.creator` vs the Pump pool authority PDA, not `coin_creator`; market cap and tier calculation are on-chain fee-config driven.
6. Pump bonding curve V3: https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/TRADE_V3.md
   - V3 fee-retention behavior and account-matrix differences versus earlier instructions.

## Implementation caveat

The docs are mutable. The runtime must decode the active on-chain fee configuration and must not rely on these schedule values as a permanent constant. The code in this phase includes tier calculation helpers but does not yet join the fee-config PDA and vault updates into Yellowstone state.
