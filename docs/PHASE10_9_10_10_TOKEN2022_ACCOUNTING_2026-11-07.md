# Phase 10.9–10.10 — Token-2022 fail-closed parsing and denomination-safe fills

Date: 2026-11-07

## Baseline and scope

This change set is based on the actual uploaded archive `solana-arbitrage-f2-2-pump-quote-supervisor-shadow-2026-11-07(2).zip` (directory `phase9-work`). It does not assume that intermediate ZIPs described in earlier chat turns are present or applied. GitHub was not accessed or modified.

## Phase 10.9 — Token-2022 mint extension parsing

### Findings

`crates/arb-dex/src/lib.rs::token2022_has_risky_extension` previously began walking TLV data at byte 82 and advanced entries with 4-byte alignment. The Token-2022 mint extension layout preserves the 82-byte base Mint, pads the mint region to the common 165-byte account base, stores the account-type discriminator at byte 165, and begins extension TLV at byte 166. TLV entries are type/length/value records; the old offset/alignment assumptions could misinterpret bytes and fail to classify malformed data safely.

### Change

- Start mint TLV parsing at offset 166 and require account type `Mint` (1) for extended mint layouts.
- Preserve 82-byte base-only mint compatibility.
- Treat ambiguous/truncated layouts as unsupported (fail closed).
- Treat any non-empty extension as unsupported for execution until the quote engine has explicit semantics for that extension. This includes unknown/future extension IDs.
- Treat all-zero trailing padding as no further extension data.
- Add tests for base mint, extensionless extended-layout mint, wrong account type, valid extension presence, and truncated TLV.

This is deliberately a conservative gate, not a full Token-2022 decoder and not a TransferFeeConfig quote implementation.

## Phase 10.10 — denomination-safe fill accounting

### Finding

`FillRecord.amount` was typed as `Lamports`, even though the record also stores a token identifier. That permits raw token units (for example USDC's 6-decimal units) to be accidentally treated as SOL lamports by callers and reports.

### Change

- Change `FillRecord.amount` to `arb_types::TokenAmount`.
- Document `token` as the token mint address (base58), not a display symbol.
- Keep the database integer as raw atomic units; do not convert to UI decimal values in storage.
- Reject writes that exceed the signed 64-bit storage range rather than silently wrapping `u64 as i64`.
- Apply the same semantic type and range check to SQLite and PostgreSQL stores.

### Remaining accounting work

This does not yet make PnL multi-asset aware. The existing `AttemptRecord.expected_net`, `PnlDaily.realized_net`, and `outcomes.realized_net` remain Lamports/WSOL-denominated. A future migration must add explicit mint, decimals, valuation currency, price source/slot, and valuation timestamp before aggregating arbitrary token fills into a common numeraire. Never sum raw amounts from different mints.

## Online references checked

- Solana Token-2022 layout documentation: https://github.com/solana-labs/solana-program-library/blob/master/docs/src/token-2022.md
- Solana Transfer Fees documentation: https://solana.com/docs/tokens/extensions/transfer-fees
- Token-2022 transfer-fee processor: https://github.com/solana-program/token-2022/blob/main/program/src/extension/transfer_fee/processor.rs

The official documentation confirms the 82-byte Mint base and extension trailer distinction, and that TransferFeeConfig carries older/newer fee schedules, maximum fee, and basis points; fee changes become active two epochs later. The processor initializes both schedules and sets the next schedule two epochs ahead.

## Verification status

- Source-level inspection and targeted test cases added.
- No `cargo`, `rustc`, `rustfmt`, or Docker executable is installed in this execution environment; compilation, formatting, unit tests, clippy, and integration tests were not run.
- No live RPC account fixture was fetched, so no claim is made that on-chain Token-2022 parsing or quote parity has been validated against mainnet.

## Next work

1. Implement full TLV decoding with owner/program-id verification and explicit extension allowlist.
2. Decode `TransferFeeConfig` older/newer schedules, choose by epoch, and reproduce the official ceil-and-cap fee math.
3. Pass transfer fees separately for every transfer leg; account for withheld fees on destination token accounts.
4. Add mint/decimals/numeraire metadata and versioned price provenance to PnL accounting.
5. Add same-context-slot account snapshot completeness and provider provenance before enabling any affected pool for execution.
