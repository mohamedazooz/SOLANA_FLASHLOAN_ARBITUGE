# Release Notes — F2.1 PumpSwap State Pipeline

- Added Yellowstone subscriptions for PumpSwap and Pump Fees program-owned accounts.
- Added dynamic tracked-account dependencies for PumpSwap pools, vaults, mints, GlobalConfig and FeeConfig.
- Added strict same-slot Pool/vault joining and SPL/Token-2022 account validation.
- Added bounded FeeConfig decoding and canonical/stable/exotic fee schedule selection.
- Added GlobalConfig disable-buy/disable-sell gating, current creator-fee configurable byte, and maximum creator-fee cap validation.
- Added PumpSwap promotion into the shared Shadow market-state cache.
- Added fee-bucket-aware real quote liquidity, signed virtual-reserve handling, and BPS-to-PPM quote normalization.
- Added PumpSwap exact-in fee rounding and creator-fee configurable gate.
- Updated the Pump.fun BondingCurve decoder to the current 83-byte layout, including creator, mayhem and cashback flags; truncated/legacy data without mode flags is rejected.
- Removed Pump.fun's static 125 BPS and 6-decimal assumptions; quotes require explicit fee BPS, mint decimals and mode flags. Mayhem/cashback are rejected. Pump.fun live discovery remains disabled pending the separate Pump Global + FeeConfig state join (FeeConfig PDA seed uses the Pump program ID, not PumpSwap program ID).
- Added Yellowstone-backed Kamino reserve fee and WSOL mint inputs; removed the HTTP JSON-RPC reserve-fee reader.
- Removed mock Multi-DEX catalog, synthetic cross-venue arbitrage profit, and fabricated latency telemetry from MCP tools. Pump.fun MCP quotes now require caller-supplied raw account state and are explicitly marked unverified/non-executable.
- Archived the unregistered legacy single-executor source and disabled Cargo auto-discovery of unreviewed binaries.
- Split of Supervisor and other >1,000-line source files remains outstanding; see `docs/RELEASE_READINESS_AND_REFACTOR.md`.
- Live signing remains disabled; this release is not approved for Mainnet live execution.

Build/test status: not run because cargo, rustc, rustfmt and Docker are unavailable in the packaging environment.
