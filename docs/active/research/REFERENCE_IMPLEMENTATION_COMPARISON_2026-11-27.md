# External Reference Comparison & Local Implementation Audit

**Audit date:** 2026-11-27
**Scope:** Solana atomic arbitrage, DEX quote/execution parity, flash-loan integration, transaction compilation, and Jito transport.
**Evidence rule:** Repository source and reproducible tests are authoritative. External README claims are treated as claims by their authors, not independent proof. This document records findings from the preceding forensic review; every code-level finding must be revalidated against the current branch and working tree before changing code.

## 1. Executive conclusion

The local project has a more complete end-to-end arbitrage architecture than the four public reference repositories reviewed: canonical economic cost-flooring, a Kamino flash-loan path, route/risk gates, transaction V0/V1 compilation, and Jito dispatch/reconciliation are represented in the local codebase. That does **not** establish production readiness. Several DEX account-resolution and quote-parity gaps remain, and no repository README or unit-test suite substitutes for same-state protocol differential tests, RPC simulation, landed transactions, and realized PnL reconciliation.

Do not copy any reference repository wholesale. Use narrow patterns and official protocol SDKs as differential oracles.

## 2. Reference repositories

### 2.1 anshuman008/solana-dex-aggregator

- URL: https://github.com/anshuman008/solana-dex-aggregator
- Useful as a reference for pool parsing, quote math, route construction, instruction building, Yellowstone ingestion, and V0/ALT handling.
- Its README describes support for Raydium AMM V4/CLMM, Meteora DAMM V1/V2 and DLMM, PumpSwap, and Orca Whirlpool, with 1–4 hop routing.
- The README's reported live-simulation success count is self-reported. It is not independent proof of production reliability.
- Its documented limitations include incomplete tick-array/bitmap-extension coverage, limited active-bin neighborhoods, no split routing, and simulation failures.
- No verified evidence in the reviewed material establishes a complete flash-loan arbitrage executor or a production MEV/Jito engine.

**Local comparison:** The local project has broader arbitrage economics, flash-loan, risk, and settlement concerns. The aggregator is a targeted reference for protocol-specific decoding/quoting/builders. The local project did not show a Meteora DAMM V1 adapter in the reviewed builder surface; it does include other venue paths such as Raydium CPMM and Pump-related adapters.

### 2.2 tfrmma/sol-mev-core

- URL: https://github.com/tfrmma/sol-mev-core
- Useful for decomposition of streaming ingestion, cycle discovery, MEV strategy boundaries, and Jito dispatch patterns.
- Its README documents material limitations: fixed 30 bps Meteora fee approximation, CLMM quoting without full tick-array traversal, and a quote-deviation rejection threshold.
- Kamino-related parsing and obligation workflows use klend-interface; their presence is not evidence of a complete flash-loan arbitrage executor.
- Treat its quote math as a design reference, not a production pricing oracle.

**Local comparison:** Consider the official klend-interface as a differential parser/oracle for the local Kamino Reserve decoder. Do not import it into the runtime dependency graph without checking Solana SDK version compatibility.

### 2.3 Cetipoo/solana-onchain-arbitrage-bot

- URL: https://github.com/Cetipoo/solana-onchain-arbitrage-bot
- Useful narrowly for vault/account-layout and route-grouping concepts.
- The README labels the project as a reference/demo rather than a fully featured production system.
- Reviewed bot code uses polling-style pool refresh; transaction code uses V0/ALT and skip_preflight patterns.
- Its custom executor/vault funding model is not equivalent to an external Kamino lending flash-loan adapter.
- The reviewed material did not establish that the on-chain program source and deployed program are independently verified as identical.

**Local comparison:** Do not transplant its polling or transport choices into the local low-latency ingestion/settlement path.

### 2.4 Al-SWAP/solana-flash-larbitrage-bot

- URL: https://github.com/Al-SWAP/solana-flash-larbitrage-bot
- Useful as an educational illustration of off-chain/on-chain flash-loan instruction sequencing.
- Its README describes the repository as an educational example.
- Example configuration thresholds and sample transactions are not evidence of sustained production profitability.

**Local comparison:** Use only to cross-check the conceptual borrow/swap/repay sequence, not as an implementation template.

## 3. Official protocol references and parity requirements

### 3.1 Kamino / klend

- Official interface crate: https://docs.rs/klend-interface/latest/klend_interface/
- Official source: https://github.com/Kamino-Finance/klend
- The local production path reviewed uses a Kamino flash-loan adapter and a manually decoded Reserve layout with discriminator/size/mint/status/liquidity validation.
- The reviewed local workspace did not have production Marginfi or SPL Token Lending adapters. Enum variants or mocks are not production integrations.
- Compare local Reserve decoding against the official interface using pinned fixtures and differential tests. Check SDK compatibility before adding the crate as a runtime dependency.
- Marginfi flash-loan guide: https://docs.marginfi.com/guides/flashloans. Its documented start/end instruction restrictions are Marginfi-specific; do not apply them to Kamino without checking Kamino's own interface.

### 3.2 Orca Whirlpools

- Official Rust core: https://github.com/orca-so/whirlpools
- Official docs: https://docs.orca.so/
- The reviewed local workspace pins orca_whirlpools_core. Verify quote parity with exact same pool/tick-array/oracle/time inputs.
- The current decoder rejects adaptive-fee pools from the classic fee path when the on-chain fee_tier_index_seed differs from tick_spacing. Adaptive-fee support is not implemented; it requires authoritative oracle-aware fee state/math and complete account hydration.

### 3.3 Meteora DLMM

- Official SDK: https://github.com/MeteoraAg/dlmm-sdk
- Official Rust SDK/docs: https://docs.rs/meteora-dlmm/
- The reviewed local path includes exact-in quoting and bin-array/bitmap-extension traversal with conservative Token-2022 handling. Exact-out and some advanced extension/limit-order behavior were not fully at parity.
- Add differential vectors against the official SDK for both directions, including bitmap extension, active-bin boundaries, volatility/variable fees, Token-2022 transfer fees, and remaining accounts.

### 3.4 Meteora DAMM V2

- Official SDK: https://github.com/MeteoraAg/cp-amm-sdk
- Official crate: https://docs.rs/cp-amm/
- The reviewed local code has custom exact-in DAMM V2 math and fee-state handling, but its production route graph deliberately keeps DAMM V2 quote-only until execution parity is proven.
- Preserve this gate. Do not enable execution merely because a quote or instruction builder exists.
- Differential tests must cover currentPoint/time-dependent fees, partial fills/swap2 semantics, token extensions, account metas, and min-output behavior.

### 3.5 Raydium

- CLMM source: https://github.com/raydium-io/raydium-clmm
- CPMM source: https://github.com/raydium-io/raydium-cp-swap
- The reviewed local CPMM path models Trading Fee and Creator Fee separately and has AmmConfig-loading gates. Validate against official SDK/source golden vectors, including creator-fee direction and transfer-fee effects.
- The local CLMM builder now represents the fixed first tick-array account, optional bitmap extension, and variable additional arrays; it rejects empty/default/duplicate keys. Hydration still does not fetch and owner/discriminator/length-validate AmmConfig, tick arrays, or the optional bitmap extension at one coherent slot, and the live fee state is not joined. The route remains unpriceable and unexecutable. Builder structure is not runtime account-resolution proof.
- Keep CLMM unpriceable/unexecutable while its authoritative AmmConfig/dynamic-fee state is missing or stale.

### 3.6 Solana transaction format and Jito

- SIMD-0385: https://github.com/solana-foundation/solana-improvement-documents
- Jito docs: https://docs.jito.wtf/lowlatencytxnsend/
- Jito issue tracker (V1 ingress compatibility evidence must be rechecked before use): https://github.com/jito-labs/searcher-examples/issues
- The reviewed local code has V1 compilation and V0/ALT fallback paths, plus an explicit Jito V1 opt-in gate. Preserve the gate until a real end-to-end Jito ingress canary proves V1 acceptance.
- Do not silently downgrade, strip instructions, or split an atomic flash-loan transaction to fit transport limits.
- Re-verify current network activation, provider support, Jito ingress limits, and transaction size at the time of deployment; protocol and infrastructure behavior can change.

## 4. Local code findings from the preceding review — revalidate before patching

The following findings were observed in the prior forensic pass and are **not automatically asserted to remain true on the current branch**:

| Area | Prior observation | Required action |
|---|---|---|
| Canonical execution | Supervisor path used TxBuilder::assemble_flash_arbitrage with Kamino borrow, top-level DEX instructions, final min-output, repay, and Jito dispatch | Trace current call graph and preserve the canonical path |
| Economic floor | Canonical builder derived final output floor from principal, flash fee, base fee, priority fee, tip, and minimum net profit | Add regression coverage for every cost and avoid double-counting swap fees |
| Kamino decoder | Existing Reserve offsets are checked against a pinned 8,624-byte Mainnet Reserve + supply-vault fixture; discriminator, mint, status, decimals, fee SF, vaults, liquidity, and truncation are tested. No klend-interface dependency was added because its reviewed BSL-1.1 license is outside deny.toml. | Keep the pinned Mainnet regression fixture; pursue a legally compatible official-schema differential oracle separately. Fixture coverage is not an SDK differential or proof of a live borrow/repay |
| Raydium CLMM | Builder now accepts the required first tick array, optional bitmap extension, and variable additional arrays, with duplicate/default validation. Hydration still supplies no verified arrays/bitmap and does not join live AmmConfig/dynamic fees; CLMM remains unpriceable/unexecutable. | Fetch and validate all required dependencies and exact fee state from a coherent slot; add official quote differential and simulation before changing the gate |
| PumpSwap V2 | The official V2 interface specifies exactly 17 fixed accounts and no inferred remaining accounts. The builder now derives the user-volume accumulator and buyback recipient ATA and separates V1 legacy. Only WSOL/USDC quote mints are currently accepted; others fail closed pending ATA existence/rent handling. | Preserve the official fixed matrix; keep mode-specific discriminator/argument/account-meta tests and add coherent-state RPC simulation before execution claims |
| Orca adaptive fees | Decoder now rejects adaptive-fee pools from the classic path when fee_tier_index_seed differs from tick_spacing; adaptive fee math remains unsupported. | Keep the fail-closed rejection until oracle-aware state/math and full account dependencies are implemented and differentially tested |
| Meteora DAMM V2 | Quote path existed while production execution was deliberately gated | Preserve quote-only gate until SDK parity and transaction simulation pass |
| Bellman-Ford economics | Legacy path appeared to zero network costs and convert flash-fee errors to zero | Determine whether it is reachable; unify with canonical economics or deprecate safely |
| Legacy builders | Some legacy/test builders appeared weaker than canonical economic checks | Keep unreachable from production; add compile-time/API boundaries where appropriate |
| On-chain executor | Program source existed but no verified deployment/runtime use was established | Do not claim its guard is active in live execution |

## 5. Required remediation order

### P1 — correctness and execution parity

1. Revalidate the current Kamino Reserve decoder and add differential tests against the official interface.
2. Complete Raydium CLMM runtime hydration: bitmap extension, all required tick arrays, AmmConfig and dynamic-fee state, owner/discriminator/pool validation, and coherent slot checks. The builder shape alone does not complete this gate.
3. Preserve PumpSwap V2 fixed 17-account layout with no inferred remaining accounts; finish non-WSOL/USDC buyback ATA existence/rent resolution or keep those quote mints rejected. Keep V1 cashback/legacy routing separate.
4. Implement or explicitly reject Orca adaptive-fee pools based on authoritative pool state.
5. Add Meteora DAMM V2 differential quote + instruction/account parity while preserving the quote-only production gate.
6. Add same-state RPC simulation fixtures/matrix for every route marked executable.
7. Keep Jito V1 disabled until a documented ingress canary proves support.

### P2 — architecture and completeness

1. Unify or retire legacy economics and sizing paths without changing the canonical live execution path.
2. Add exact-out support only where the official SDK and account-resolution semantics can be matched.
3. Account for ATA rent/locked capital where it affects deployable capital or realized economics; do not subtract refundable rent as a per-trade fee.
4. Add secondary lender adapters only after independent protocol-specific design, tests, and operator approval.

## 6. Acceptance criteria

A venue is executable only when all applicable layers pass:

1. Authoritative account discovery and owner/discriminator validation.
2. Complete, same-slot account dependency resolution.
3. Protocol-exact quote math and explicit fee/rounding semantics.
4. Correct instruction discriminator, account metas, writable/signer flags, and remaining accounts.
5. Route risk gate and canonical all-cost net-profit floor.
6. Transaction compilation within the selected transport's verified limits.
7. RPC simulation success against coherent state.
8. Landing and settlement reconciliation evidence before any production claim.

Unit tests alone establish only unit-tested behavior. A successful quote does not establish an executable route. A built transaction does not establish simulation, landing, or profit.

## 7. Agent instructions

Read and obey docs/agents/PROTOCOL_PARITY_RULES.md before touching any protocol adapter, quote math, transaction builder, flash-loan integration, or route eligibility gate.

## 8. Research source index

- https://github.com/anshuman008/solana-dex-aggregator
- https://github.com/tfrmma/sol-mev-core
- https://github.com/Cetipoo/solana-onchain-arbitrage-bot
- https://github.com/Al-SWAP/solana-flash-larbitrage-bot
- https://docs.rs/klend-interface/latest/klend_interface/
- https://docs.marginfi.com/guides/flashloans
- https://github.com/orca-so/whirlpools
- https://github.com/MeteoraAg/dlmm-sdk
- https://github.com/MeteoraAg/cp-amm-sdk
- https://github.com/raydium-io/raydium-clmm
- https://github.com/raydium-io/raydium-cp-swap
- https://docs.jito.wtf/lowlatencytxnsend/
- https://github.com/solana-foundation/solana-improvement-documents
