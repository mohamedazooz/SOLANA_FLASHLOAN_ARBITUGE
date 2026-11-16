# Protocol Constants

**Repository:** Solana Arbitrage Bot  
**Source:** `crates/arb-constants/src/lib.rs`  
**Status:** Phase 1 — centralized, immutable, verifiable

Protocol invariants live in `arb-constants` as strongly typed constants. They
**must not** be moved into ordinary operator configuration.

---

## Categories

| Category | Examples | Location |
|----------|----------|----------|
| Token programs & system IDs | SPL Token, Token-2022, ATA, ALT, sysvars, System, Compute Budget | `arb-constants:14-39` |
| Token mints | WSOL, USDC, Token-2022 native SOL | `arb-constants:46-50` |
| Jito tip accounts | 8 mainnet tip accounts + `JITODONTFRONT` marker | `arb-constants:57-118` |
| DEX program IDs | Raydium v4/CPMM/CLMM, Orca, Meteora DLMM, Pump.fun, PumpSwap | `arb-constants:124-137` |
| PDA seeds & discriminators | Pump.fun/global, bonding-curve, event authority; Anchor 8-byte discriminators | `arb-constants:138-155` |
| Pool & vault addresses | Raydium/Orca/Meteora/Kamino SOL-USDC pools & vaults | `arb-constants:162-250` |
| Kamino | klend program, main market, reserves, flash borrow/repay discriminators | `arb-constants:238-263` |
| Compute budget | 2-hop/3-hop CU limits, max CU, buffer %, MTU, base priority fee | `arb-constants:274-285` |
| Pump.fun | Graduation threshold | `arb-constants:297` |

## Verification Requirements

Each constant must:
- Have one authoritative source (no duplication).
- Record the upstream repository, SDK version, and commit where verifiable.
- Have a test validating its value (Pubkey parse, byte equality, seed match).
- Have cluster/network compatibility checks.
- Use versioned decoders for changing account layouts.
- Have fixtures for known account layouts.
- Detect unsupported protocol versions and fail closed.

## Unverified Addresses (must be checked on-chain before mainnet use)

| Constant | Value | Status |
|----------|-------|--------|
| `RAYDIUM_SOL_USDC_POOL` | `58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2` | UNVERIFIED |
| `RAYDIUM_SOL_VAULT` | `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz` | UNVERIFIED (P0 C-02) |
| `RAYDIUM_USDC_VAULT` | `HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz` | UNVERIFIED |
| `KAMINO_USDC_RESERVE` | `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59` | UNVERIFIED (P0 C-01) |

Verification: mint identity, vault authority, pool state, correct mainnet
cluster. Do not choose an address based solely on the audit document.

## Regression Tests

`crates/arb-types/src/lib.rs::address_conflict_regression_tests` locks the
canonical addresses so historical typo values (from the audit document only)
cannot be reintroduced:
- C-01: Kamino USDC reserve ≠ typo `...Ug6xK25`
- C-02: Raydium pool_coin_vault ≠ typo `...Ww5jz`
