# Solana V1-only release policy

This release profile is **V1-only**. It must not compile, simulate, or dispatch a new Legacy or V0 transaction.

## Wire and account rules
- V1 discriminator: `0x81`.
- Maximum serialized transaction size: 4,096 bytes.
- Address Lookup Tables are not available in V1. Every account is inline.
- Enforce the currently active 64-address limit; do not assume the proposed 96-address expansion is active.
- Maximum top-level instruction count: 64.
- Require explicit `compute_unit_limit` and `loaded_accounts_data_size_limit` in every V1 message.
- Strip ComputeBudget instructions from V1 messages; V1 resource settings belong in the message config.
- RPC `maxSupportedTransactionVersion` is integer `1` for V1 (`0` for V0).

## Meteora DLMM bins
Meteora DLMM BinArray accounts are ordinary inline V1 account addresses. The route builder must derive and include every BinArray PDA required by the exact swap traversal, in the correct order, alongside pool, bitmap-extension, vault, mint, token-program, oracle, and program accounts required by the deployed DLMM program version. Missing arrays or exceeding the 64-address budget must reject the route; there is no ALT compression or stale-array fallback. The number of bin arrays is determined from the actual swap direction, active bin, amount, and bin traversal—not a fixed guessed count.

## Resource sizing
The current builder requires explicit non-zero CU and loaded-account data limits and enforces runtime ceilings. Production sizing should run an RPC simulation with conservative maximum resource settings, then use `unitsConsumed` and `loadedAccountsDataSize` with measured headroom. The packaged Shadow profile does not broadcast transactions.

## Transport
This package only treats V1 as valid. It does not assume Jito bundle ingress supports the V1 wire format. Simulation is through RPC with `maxSupportedTransactionVersion: 1`; any future live sender must be independently tested against the exact RPC/TPU/SWQoS provider and must remain disabled in Shadow.

## References
- Solana V1 transaction format: https://solana.com/docs/core/transactions/versioned-transactions
- Solana mainnet V1 activation and SDK compatibility: https://solana.com/upgrades/larger-transaction-sizes
