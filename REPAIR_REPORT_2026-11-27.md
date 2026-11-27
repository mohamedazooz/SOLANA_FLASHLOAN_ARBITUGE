# Solana Flashloan Arbitrage — Repair Report

**Date:** 2026-11-27
**Input:** `SOLANA_FLASHLOAN_ARBITUGE-main_5.zip` (uploaded project snapshot)
**Working branch:** Local repair copy only; the ZIP did not contain `.git` metadata.

## Executive summary

This repair pass focused on correctness and fail-closed behavior in the on-chain executor and the PumpSwap transaction builder. It fixes several high-impact contract/validation defects, aligns the PumpSwap V2 CPI with Pump's published interface, and disables a legacy PumpSwap V1 builder whose account list was incomplete.

This is **not a production certification**. The environment did not contain `cargo`, `rustc`, `rustup`, or `rustfmt`; Rust compilation, unit tests, Clippy, formatting, and Solana program-test execution could not be run. No mainnet profit claim or live-trading readiness is asserted.

## Changes applied

### 1. `arb-executor` routing and instruction validation

- Fixed dispatcher precedence so 8-byte `execute_route` and `profit_guard` discriminators are checked before the overlapping legacy 1-byte tags. The old ordering could consume the first byte as a legacy tag and make the full-width route unreachable.
- Made `execute_route` payload decoding canonical: exact payload sizes, zeroed reserved bytes, nonzero input/output floors, zeroed unused third-hop descriptor for 2-hop routes, and strict account-slice exhaustion.
- Added an explicit connected-token-flow invariant: hop 1 spends the settlement account; each hop's output account is exactly the next hop's input account; the final hop returns to the same settlement account measured for profit. This prevents disconnected swaps from being combined with a separate balance check.
- Verified the user-controlled token accounts and signer for each supported hop. Added protocol-specific account counts, direction flags, and account indexes for Meteora DLMM, Raydium CPMM, Orca Whirlpool, and PumpSwap V2.
- Kept Raydium CLMM fail-closed because the current adapter does not validate the full same-slot dynamic-fee, bitmap-extension, and tick-array dependency set. Orca's writable adaptive-fee oracle path is also rejected while that fee model is not represented by the quote path.

### 2. SPL Token and Token-2022 account safety

- Balance readers now accept only accounts owned by SPL Token or Token-2022.
- Require at least the 165-byte base token-account layout and `AccountState::Initialized` at byte offset 108; uninitialized and frozen accounts are rejected. Token-2022 accounts with extensions remain eligible when the base layout is valid.
- User token-account validation continues to verify mint and token-account authority, and the PumpSwap CPI additionally verifies the account's token program against the owning program of its mint.

### 3. PumpSwap program and V2 CPI

- Fixed an incorrect, locally hardcoded PumpSwap program ID in `arb-executor`. The executor now derives its address constant from the shared `arb-constants::PUMP_SWAP_PROGRAM_ID`, which is the published Pump AMM address `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`.
- The Pump.fun bonding-curve program ID is no longer accepted as a PumpSwap AMM program.
- Replaced the old generic `global:swap` CPI payload with the V2 exact-input instructions: `buy_exact_quote_in_v2` and `sell_v2`, using their distinct Anchor discriminators and little-endian `u64` arguments.
- The V2 CPI requires the exact 17-account order, checks fixed program/config/event/fee accounts, verifies the Pool and GlobalConfig account discriminators, checks token-program ownership and user token-account mints, enforces the signer/writable matrix, and rejects extra accounts.
- The off-chain V2 builder currently permits WSOL and USDC quote mints only; arbitrary quote mints fail closed until the buyback-fee recipient ATA is resolved and any required account creation is explicitly rent-accounted.
- Added unit-test coverage for V2 instruction data encoding and for rejecting the stale PumpSwap program ID.

### 4. PumpSwap legacy V1 — fail closed

- The old off-chain V1 builder emitted a shortened account list and omitted accounts from the published V1 interface. It is now disabled with an explicit sanitization error rather than generating a malformed transaction.
- The route builder therefore rejects V1 legacy/Cashback pools until a complete current V1 account builder is implemented and tested. Pump documents that V2 is not supported for Cashback pools; those pools are intentionally not enabled by this patch.

### 5. Three-leg route and profit guard

- Corrected mint-chain validation so a route may legitimately cross between SPL Token and Token-2022 mints; each account is still checked against its own mint's token program.
- Added nonzero amount/floor checks and exact account-count validation to `execute_three_leg`.
- Corrected `execute_three_leg_with_tip` to pass only the canonical body-account slice to the shared three-leg processor, keeping the Jito tip account outside that body and rejecting trailing accounts.
- Tightened `profit_guard` to exactly one settlement token account and exactly eight bytes of minimum-balance data; test fixtures now mark SPL token accounts initialized.

### 6. Legacy CLMM placeholders

- Replaced fake `(u128::MAX, u128::MAX)` CLMM reserve values with zero capacity in two dormant legacy Pump/Raydium modules.
- Removed the placeholder quote that passed an empty tick list; that CLMM quote now returns an explicit unsupported error.
- These two `pump_raydium_*` modules are not declared by the current `arb-engine` module tree, so the changes are defensive cleanup in dormant legacy source, not evidence that the active runtime now supports Raydium CLMM.

### 7. Test cleanup

- Removed the orphan `programs/arb-executor/tests.rs` file at the crate root; it was not Cargo's integration-test directory and contained `unimplemented!()` test skeletons.
- Added regression cases around malformed route payloads, full-width dispatcher routing, initialized/frozen token accounts, mixed SPL/Token-2022 mint paths, strict profit-guard data, PumpSwap V2 serialization, and the disabled V1 path.

## Validation performed

| Check | Result | Notes |
|---|---|---|
| `npm --prefix web run test:security` | **PASS — 3/3** | Login submission contract, short-lived WebSocket tickets, canonical emergency-control API |
| `python3 scripts/audit_codebase.py` | **Ran — exit 0** | Reported 15 production WebSocket endpoint literals, 8 localhost/port defaults, and 4 production pubkeys outside the central constants crate |
| PumpSwap Anchor discriminator checks | **PASS — 5/5** | SHA-256 prefixes matched `buy_exact_quote_in_v2`, `sell_v2`, Pool, GlobalConfig, and FeeConfig account discriminators |
| Changed Rust delimiter sanity check | **PASS** | Braces/parentheses/brackets balance in modified Rust files; this is not a parser or compiler check |
| Rust format / compile / tests / Clippy | **NOT RUN** | `cargo`, `rustc`, `rustup`, and `rustfmt` are not installed in the execution environment |
| Web production build | **BLOCKED** | `web/node_modules` is absent; build previously failed resolving React and UI dependencies |
| Mainnet/devnet transaction simulation | **NOT RUN** | No RPC-backed integration environment or deployed executor binary was provided |

## Remaining release blockers and risks

1. **Rust toolchain verification is mandatory.** Run the commands below with the repository's pinned toolchain before accepting or deploying these changes.
2. **The executor is not verified as deployed.** The project comments identify `ArbExec1111111111111111111111111111111111111` as a vanity placeholder, and the off-chain builder says not to include its instruction in production until the program is deployed and its binary identity verified. This patch does not deploy a Solana program.
3. **No live trade or profitability proof was performed.** There is no 24-hour shadow soak, no verified flash-borrow/repay round trip, and no mainnet differential quote test in this repair pass. Keep live trading disabled and dry-run/shadow mode enabled.
4. **Raydium CLMM remains unsupported in the generic executor.** The correct config, tick-array bitmap extension, tick arrays, and dynamic-fee state must be resolved and validated against a consistent slot before enabling it.
5. **Orca adaptive-fee pools remain unsupported in the generic route path** until the writable oracle and adaptive-fee quote calculation are modeled together.
6. **PumpSwap V1/Cashback remains unsupported** until a complete V1 account matrix is implemented. PumpSwap V2 is the only PumpSwap CPI path enabled by this patch, and the current off-chain V2 builder is limited to WSOL/USDC quote mints pending arbitrary buyback-ATA handling.
7. **Token-2022 extensions require protocol-level validation.** Not every extension (for example transfer-fee or transfer-hook behavior) is proven compatible with every DEX adapter. Reject unsupported extension combinations rather than assuming base-layout compatibility is sufficient.
8. **Configuration debt remains.** The static audit still reports 15 production WebSocket endpoint literals, 8 local host/port defaults, and 4 production pubkeys outside the central constants crate. These should be moved to validated configuration/constants in a separate pass.
9. **No complete secret scanner was available.** The archive scan did not find a real `.env`, `.pem`, `.key`, private-key-named, or wallet-JSON file; this does not prove that no secret-like value exists in source or history.

## Recommended verification commands

Run with the toolchain pinned by `rust-toolchain.toml`:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm ci --prefix web
npm --prefix web run build
npm --prefix web run test:security
```

Then deploy only to a local validator / LiteSVM / Surfpool test environment and simulate:

- Kamino flash borrow → supported swaps → profit guard → flash repay in one atomic transaction;
- failure on every leg, min-output breach, disconnected user token accounts, frozen/uninitialized token accounts, wrong program IDs, and trailing accounts;
- PumpSwap V2 buy and sell against official SDK-generated fixtures;
- mixed SPL Token / Token-2022 routes and unsupported Token-2022 extension combinations;
- transaction compute budget, account locks, priority fees, Jito tip bounds, and replay/freshness constraints.

Do not enable mainnet live mode until those checks pass and the executor deployment ID/binary is independently verified.

## Open-source references consulted

- [Pump official public docs — PumpSwap V2 trades](https://github.com/pump-fun/pump-public-docs/blob/main/docs/instructions/PUMP_SWAP_TRADE_V2.md)
- [Pump official public docs — PumpSwap program and legacy instructions](https://github.com/pump-fun/pump-public-docs/blob/main/docs/PUMP_SWAP_README.md)
- [Pump official public docs — PumpSwap IDL](https://github.com/pump-fun/pump-public-docs/blob/main/idl/pump_amm.ts)
- [SPL Token account layout and unpack behavior](https://docs.rs/spl-token/latest/spl_token/state/struct.Account.html)
- [Solana token programs and Token-2022 account model](https://solana.com/developers/docs/tokens)
- [Raydium CLMM docs](https://docs.raydium.io/)
- [Meteora DLMM docs](https://docs.meteora.ag/)

---

**Delivery status:** repaired local project snapshot only. The remote `main` branch was not modified, and no remote branch or pull request was created from this ZIP because it had no Git metadata and the GitHub integration previously rejected branch creation with HTTP 403.
