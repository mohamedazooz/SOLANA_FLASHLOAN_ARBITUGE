# Unified Execution Spine ? Reverse-Engineering Audit

**Date:** 2026-11-27
**Branch:** fix/unified-arbitrage-spine-20261127
**Scope:** full live path from discovery through signed dispatch, chain confirmation, and PnL settlement. No live transaction was signed or broadcast during this audit.

## Executive result

The project already had a mostly unified live path: generic graph candidates and specialized DLMM candidates converge through MarketGraph::evaluate_directed_route; all accepted candidates enter Supervisor::process_opportunities; same-DEX and cross-DEX transaction assembly converge on TxBuilder::assemble_flash_arbitrage_with_converted_costs; and one supervisor dispatch path selects Jito or direct/SWQoS transport.

The most material defects were at the end of the pipeline, not the quote graph:

1. Dispatch treated TransactionLifecycle::Processed as LandStatus::Landed, even though recovery logic recognized Processed as provisional. That could book a fill and PnL before confirmation.
2. Jito's Landed bundle response could be returned as Confirmed without a matching Confirmed/Finalized Solana signature status.
3. Post-landing PnL parsing was hardwired to WSOL. USDC-anchored routes therefore did not measure their own base-mint balance delta. The dispatch notification also recalculated borrow size as size_in / 1e9, wrong for six-decimal assets.
4. An ambiguous dispatch intent could be marked unresolved_after_poll and removed from the pending set; reconciliation ran only at startup.
5. PnL aggregation counted outcome rows rather than unique attempts, although an attempt can have sent followed by a terminal state. Failed transaction fees were charged to the risk gate but not included in actual PnL aggregation.

## Remediations applied

### A. Confirmation and terminal state

- Added one lifecycle-to-land-status mapping in dispatch. Only Confirmed and Finalized become LandStatus::Landed; Processed, Pending, and Unknown remain Sent.
- The extended Jito verifier cross-checks Jito's landed bundle result with getSignatureStatuses through transaction RPC. Confirmed and Finalized remain distinct. Processed continues polling and never resolves as success.
- The older verifier was hardened too: Jito bundle status without confirmed/finalized signature proof is not returned as landed.
- Added regression tests for Processed -> Sent and signature status proof mapping.

### B. Base-mint-aware realized PnL

- Added parse_landed_asset_balance_changes to parse payer-owned token accounts for the actual route base mint, with strict owner/index/amount checks and duplicate-index rejection.
- Kept the WSOL parser as a compatibility adapter; new dispatch settlement reads the first hop's base mint.
- Non-WSOL actual settlement requires both attributable base-token delta and verified payer lamport delta. The token delta is converted to lamports using the same verified rational base-mint conversion used by opportunity economics, then native payer movement (fees, priority fee, tip, rent movement) is added.
- Missing or malformed evidence remains estimate provenance; it is never treated as zero.
- Dispatch notification now uses the already converted SOL-equivalent borrow amount from canonical economics rather than dividing arbitrary base raw units by 1e9.
- Added USDC and incomplete-evidence parser tests.

### C. Durable recovery and accounting

- unresolved_after_poll remains pending rather than being resolved as terminal.
- Added a background reconciler outside the strategy hot path. It polls at a 10-second cadence and handles at most eight pending intents per sweep to respect free/demo RPC budgets.
- Recovered landed attempts are recorded with estimate provenance (crash-recovery records do not yet persist base-mint/rate metadata); recovered failed transactions use actual meta.fee when available.
- Terminal outcome writes are idempotent for SQLite, PostgreSQL, and the in-memory store; an estimated retry cannot overwrite already recorded actual evidence.
- PnL attempt/success counts count distinct attempt IDs, not status-transition rows.
- Failed on-chain transactions persist the actual negative network fee when metadata supplies it; PnL aggregation includes actual failed fees as losses.
- Added regression coverage for terminal idempotency, unique attempt counts, and actual failed-fee aggregation.

## Verified live path

### Discovery and quote authority

- Supervisor::strategy builds the market graph from registry/snapshots and scans closed cycles with per-anchor caps.
- Specialized DLMM cross-pool and three-leg routines supply candidate seeds only; they are re-evaluated through evaluate_directed_route, which validates mint closure, hop continuity, pool uniqueness, and graph membership.
- Route dedup preserves direction and hop order.
- Supervisor::process_opportunities revalidates freshness/account dependencies, rebuilds VerifiedQuoteInputs, performs canonical bounded sizing/economic gate, and recomputes net using selected tip and CU price before assembly.

### Build and send authority

- assemble_native_ixs_for_anchor enforces flash loans as mandatory.
- Cross-DEX adapter validates protocol/mint-chain invariants, then delegates to the same cost-aware TxBuilder path as same-DEX routes.
- Builder enforces borrow/repay index and principal parity, computes Kamino fee separately, clamps later hop inputs to previous guaranteed minimum outputs, and enforces final output floor: principal + flash fee + base/priority fees + tip + retained minimum profit.
- Atomic integrity is verified before V1 compilation/signing.
- Durable dispatch intent is persisted before transport send. Ambiguous transport errors do not trigger blind re-signing/resubmission.

## Remaining limitations / not claimed

1. Crash-recovery records currently lack persisted base_mint, payer, and verified rational conversion. If the process crashes before the live landing tracker records balance evidence, recovery can prove status but only records estimated expected net for a landed trade. A follow-up schema migration should persist these settlement fields so recovery can reconstruct actual base-mint PnL too.
2. A non-WSOL base-token delta is exact in base raw units; its SOL-equivalent normalization uses the verified quote-time rational rate. It is not a separately observed execution-time USD oracle mark.
3. No live RPC, mainnet simulation, Jito ingress, transaction landing, or realized PnL evidence was generated by this static/local test pass.
4. Protocol support remains fail-closed where adapters or account hydration are incomplete. No new DEX support was added.
5. Optional split-routing remains a separate assembly branch and was not promoted into the standard execution path.
6. The 2-vCPU/4-GB and free/demo RPC deployment envelope remains the design target; periodic reconciliation is deliberately bounded and low frequency.

## Validation

Pre-change focused baseline completed:
- cargo test -p arb-sim -p arb-land -p arb-bot --lib --locked ? passed (114 + 66 + 53 tests).

Post-remediation validation was started:
- cargo fmt --all -- --check
- cargo test -p arb-sim -p arb-land -p arb-store -p arb-bot --lib --locked
- cargo clippy -p arb-sim -p arb-land -p arb-store -p arb-bot --all-targets --locked -- -D warnings
- git diff --check

Final pass/fail results must be taken from process output; this report is not a claim that the running validation has passed.
