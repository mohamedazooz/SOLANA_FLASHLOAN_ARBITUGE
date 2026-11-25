# DEX Protocol Adapter Plan — 2026-11-24

## Purpose and current scope

This document records the official implementation surfaces and the gates required before the four currently unsupported venues can enter route discovery or execution.

**This PR does not enable any of the four venues.** It closes unsafe generic quote paths and corrects an account-field naming mistake in the DAMM v2 adapter. Exact protocol support remains a separate, evidence-driven integration phase.

## Official source map

| Venue | Official source of truth | Mainnet program ID | Integration constraints |
|---|---|---|---|
| Meteora DAMM v2 | [MeteoraAg/damm-v2](https://github.com/MeteoraAg/damm-v2), [DAMM v2 SDK](https://github.com/MeteoraAg/damm-v2-sdk), [Rust integration guide](https://docs.meteora.ag/developer-guides/damm-v2/rust-integration) | `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG` | The Rust quote API consumes a decoded Pool plus current timestamp and slot. Correct quoting depends on activation type/point, base-fee schedule, dynamic-fee state, collect-fee mode, direction, referral state, and token transfer-fee context. The Pool's `whitelisted_vault` field is not a swap config account. Exact-in/out/partial-fill instruction modes must not be conflated. |
| Meteora DBC | [MeteoraAg/dynamic-bonding-curve-sdk](https://github.com/MeteoraAg/dynamic-bonding-curve-sdk), [official Rust library guide](https://docs.meteora.ag/developer-guides/dbc/rust-integration/library) | `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN` | Quotes require VirtualPool/TransferHookPool state, PoolConfig, full piecewise curve segments, migration state and current epoch. The SDK distinguishes exact-in, exact-out and partial-fill quotes. Transfer-hook tokens require the dedicated hook-aware swap path and resolved extra accounts. |
| Meteora DLMM Pro (deferred; removed from active runtime) | [MeteoraAg/dlmm-pro-sdk](https://github.com/MeteoraAg/dlmm-pro-sdk) | `dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n` | The public SDK includes a Rust `dlmm-pro-commons` implementation surface. This is a separate program from classic Meteora DLMM. The SDK documents dynamic tick-array state and bitmap/oracle-related dependencies; the current release is beta. The public Rust surface is exact-in oriented; do not invent exact-out or partial-fill support. Pin the precise SDK commit before depending on it. |
| Raydium LaunchLab | [raydium-io/raydium-sdk-V2](https://github.com/raydium-io/raydium-sdk-V2), [LaunchLab examples](https://github.com/raydium-io/raydium-sdk-V2-demo/tree/master/src/launchpad), [Raydium IDLs](https://github.com/raydium-io/raydium-idl) | `LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj` | The public integration surface is the SDK/demo and IDL; the deployed LaunchLab program source is not published in the same manner as Meteora's programs. PoolState alone is insufficient: join GlobalConfig, PlatformConfig, real/virtual reserves, vaults, mint programs and fee shares. Creator/platform/protocol fee components and remaining-account ordering are instruction-sensitive. Pin the SDK and IDL revision together. |

## Cross-protocol rules

1. **One coherent snapshot:** collect the pool, config, vaults, mints, tick/bin/curve state, oracle/bitmap and any hook-validation accounts from a consistent context slot. Reject stale, missing, mixed-slot or mismatched-owner dependencies.
2. **Time is quote input:** DAMM v2 needs the correct slot/timestamp activation point; DBC fees can depend on the current epoch; Token-2022 transfer fees depend on epoch and maximum-fee parameters. Include the relevant slot/time/epoch in quote-cache keys and invalidate after dependent account updates.
3. **Integer arithmetic only:** preserve raw token units, exact fee numerators/denominators and protocol rounding. Never turn a protocol-specific fee into a guessed BPS constant or use generic CPMM/CLMM formulas based only on an AMM family tag.
4. **Partial fill is a different result type:** when a protocol can leave unspent input, return both actual input consumed and output produced. Do not chain the original input amount into the next hop.
5. **Token-2022 must be explicit:** decode each mint's TransferFeeConfig for the active epoch; account for maximum fee and both transfer boundaries. Transfer-hook tokens require the correct validation account and dynamically resolved extra-account metas. Until implemented, reject these mints for the venue.
6. **Execution is a separate gate:** quote support does not imply a valid instruction. Validate account order, writable/signer flags, program IDs, PDA derivations, instruction data and required remaining accounts against the pinned official IDL/SDK. Keep all new routes out of live submission until simulation parity passes.
7. **Version pinning:** pin the Rust crate/git revision and IDL or SDK version. For LaunchLab, do not float an alpha SDK version; update the SDK and IDL together and rerun golden vectors.

## Implementation sequence

### Phase A — fail-closed route guard (this PR)

- Remove generic CPMM adapters for DAMM v2 and DBC.
- Remove generic CLMM adapter for DLMM Pro.
- Keep the four venues quote-only in both the legacy route graph and the unified dispatcher.
- Disable direct DAMM v2 / DBC / DLMM Pro quotes until their full protocol snapshots are present.
- Do not advertise execution accounts where the account graph is not yet validated.
- Keep LaunchLab behind a verified fee-config gate; reject Token-2022 until epoch-aware transfer fees and transfer-hook handling are implemented.
- Add regression tests proving incomplete states cannot reach generic quote math.

### Phase B — DAMM v2 and DBC quote adapters

- Import or bridge the official Rust quote routines rather than porting formulas by hand.
- Implement owner/discriminator/layout-version checks and decode the complete fee/config state.
- Resolve current point/epoch and mint fee context; capture exact quote provenance.
- Add exact-in and exact-out parity fixtures, plus partial-fill semantics where supported.
- Keep instruction construction disabled until the IDL account list is parity-tested.

### Phase C — LaunchLab

- Pin Raydium SDK and IDL.
- Join GlobalConfig and PlatformConfig with PoolState and mint/vault accounts.
- Mirror all fee components, direction rules and remaining-account ordering.
- Compare every quote against the pinned SDK and simulate the exact locally built instruction against a same-slot snapshot.
- Keep Token-2022 paths disabled until transfer-fee and hook accounts are fully handled.

### Phase D — DLMM Pro

- Pin the exact beta SDK commit.
- Decode dynamic tick-array data safely, bitmap extensions, oracle and limit-order state.
- Use only the documented exact-in semantics until official exact-out support exists.
- Reject insufficient tick-array coverage and any path that would require unsupported partial-fill behavior.
- Add differential and simulation tests for both swap directions and tick-boundary traversal.

### Phase E — promotion gate

A venue may be considered for shadow discovery only after its state and quote tests pass. It may be considered for live execution only after instruction-account parity, same-snapshot simulation, fee/repayment-floor tests, Token-2022 policy checks, and a controlled non-economic canary all pass. No venue is promoted by this document or by this PR.

## References reviewed

- [Meteora DAMM v2 Rust SDK quote entry point](https://github.com/MeteoraAg/damm-v2/blob/main/rust-sdk/src/quote_exact_in.rs)
- [Meteora DAMM v2 program Pool state](https://github.com/MeteoraAg/damm-v2/blob/main/programs/cp-amm/src/state/pool.rs)
- [Meteora DBC SDK and release notes](https://github.com/MeteoraAg/dynamic-bonding-curve-sdk)
- [Meteora DBC curve formulas](https://docs.meteora.ag/developer-guides/dbc/overview)
- [Meteora DLMM Pro SDK](https://github.com/MeteoraAg/dlmm-pro-sdk) (research reference only; not integrated)
- [Raydium SDK V2](https://github.com/raydium-io/raydium-sdk-V2)
- [Raydium LaunchLab examples](https://github.com/raydium-io/raydium-sdk-V2-demo/tree/master/src/launchpad)
- [Raydium IDLs](https://github.com/raydium-io/raydium-idl)
