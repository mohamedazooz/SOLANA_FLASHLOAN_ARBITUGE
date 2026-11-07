# Phase 10.11–10.12 — Token-2022 transfer-fee hardening

Date: 2026-11-07
Status: source changes applied; Rust compilation/tests not run in this environment.

## What changed

1. Added a bounded decoder for the Token-2022 `TransferFeeConfig` TLV extension (extension type 1) in `arb-dex`.
2. Decoded the 108-byte POD value into the two `TransferFeeSchedule` values (`epoch`, `maximum_fee`, `basis_points`).
3. Added epoch selection: use `newer` when `current_epoch >= newer.epoch`, otherwise `older`.
4. Added the on-chain fee arithmetic: `ceil(amount * basis_points / 10_000)`, capped by `maximum_fee`, with checked `u128` arithmetic.
5. Removed the generic BPS-only transfer-fee approximation from the route quote path. A pool marked as risky or carrying a non-zero legacy `transfer_fee_bps` is now rejected rather than quoted with an incomplete fee model.
6. Replaced the former test that asserted the approximation was applied with a fail-closed test.

## Important limitations

- This is a decoder and isolated fee calculator, not full Token-2022 execution support.
- The caller must supply a trusted Clock epoch from a coherent bank/slot context. Do not use wall-clock time to select the fee schedule.
- The decoder must only be called after verifying the mint account owner is the canonical Token-2022 program and the account is a mint. This helper parses bytes; it does not authenticate account ownership.
- The route engine currently rejects risky/fee-bearing pools. The decoded schedule is not yet wired into every protocol adapter or each transfer boundary. Keep those assets out of live execution until that integration is complete.
- Transfer hooks, permanent delegates, non-transferable, pausable/default-frozen and other extensions need separate policy/decoder coverage. Unknown extensions remain unsupported.
- `None` from the decoder is ambiguous by design (no extension vs malformed/unsupported); callers must distinguish via the full mint validation layer before treating it as zero fee.

## Source basis

- Solana Token Extensions — Transfer Fees: https://solana.com/docs/tokens/extensions/transfer-fees
- Token-2022 processor source: https://github.com/solana-program/token-2022/blob/main/program/src/extension/transfer_fee/processor.rs

The processor source confirms that `SetTransferFee` rolls the active schedule and sets the next schedule to `current_epoch + 2`; the selected schedule is epoch-dependent. Fee calculation is rounded up and capped by the maximum fee in the Token-2022 implementation.

## Tests added (not executed)

- rounding up and maximum-fee cap
- old/new schedule boundary
- TLV TransferFeeConfig decoding and field offsets
- quote rejection when only the legacy BPS approximation is present

## Verification still required

Run from workspace root:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Do not enable live Token-2022 routing based on this phase alone.
