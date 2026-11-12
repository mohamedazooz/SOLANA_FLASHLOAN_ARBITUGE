# Solana Transaction V1 — Live Execution Path (2026-11-12)

**Scope:** research-backed wiring of the Solana Transaction v1 (SIMD-0385) path for
live execution, plus the fixes required to make v0/v1 pre-flight simulation work at all.

## Research findings (sources)

1. **SIMD-0385 "Transaction V1 Format" — status `Activated`.**
   Raw spec: `solana-improvement-documents/proposals/0385-transaction-v1.md`.
   - `VersionByte = 129 (0x81)`; layout: `VersionByte (u8)`, `LegacyHeader (u8,u8,u8)`,
     `TransactionConfigMask (u32 LE)`, `LifetimeSpecifier [u8;32]`, `NumInstructions (u8)`,
     `NumAddresses (u8)`, `Addresses [[u8;32]]`, `ConfigValues`, `InstructionHeaders
     [(u8,u8,u16)]`, `InstructionPayloads`, trailing `Signatures [[u8;64]]`.
   - Config mask: bits `[0,1]` = total priority fee lamports (u64 LE, **both bits must be
     set**), `[2]` = compute-unit limit (u32), `[3]` = loaded-accounts-data limit (u32),
     `[4]` = heap size (u32, 1 KiB multiple in 32..256 KiB).
   - Constraints: tx ≤ 4096 bytes, ≤ 64 accounts, ≤ 64 instructions, ≤ 12 signatures,
     no address lookup tables, no duplicate addresses, `num_readonly_signed <
     num_required_signatures`.
   - ComputeBudgetProgram instructions are **ignored for configuration** in v1 but still
     consume compute units — they should be stripped (our compiler does).
2. **RPC transaction-version numbering** — verified against the pinned upstream
   `solana-transaction-4.3.0` (`src/versioned/mod.rs`, frozen-abi comment):
   **`0 = legacy, 1 = v0, 2 = v1`.** `maxSupportedTransactionVersion` in
   `simulateTransaction` / `getTransaction` / `getBlock` defaults to legacy-only, so a
   v0/v1 transaction is rejected with `UnsupportedTransactionVersion` unless the version
   is passed explicitly.
3. **SIMD-0296 "Larger Transaction Size" — status `Review`.** Motivates the 4096-byte
   ceiling and documents that Jito bundles were the pre-SIMD workaround: mainnet bundle
   transaction sizes reach ~9216 bytes (50% ≤ 2048, 65% ≤ 6144, 100% ≤ 9216). A blanket
   1232-byte limit on Jito bundle ingress is therefore stale; the gate must be
   version-aware.

## Gaps found and closed

| # | Gap | Fix |
|---|-----|-----|
| G1 | `TransactionVersion::as_rpc_version()` mapped V0→0, V1→1 (off-by-one vs the RPC numbering 0/1/2) | `crates/arb-tx/src/adaptive.rs`: V0→`Some(1)`, V1→`Some(2)`; added `rpc_max_supported_version_from_wire()` free helper |
| G2 | `simulateTransaction` calls (pre-flight mandatory guard) omitted `maxSupportedTransactionVersion`, so every v0/v1 live pre-flight would be rejected by the RPC | `crates/arb-sim/src/lib.rs` and `crates/arb-ingest/src/rpc_pool.rs`: version number derived from the wire bytes and passed in the simulate params |
| G3 | Live V1 was structurally impossible on the Jito transport: `v1_direct_enabled` required SWQoS, and `validate_jito_bundle` hard-capped every tx at 1232 bytes | `crates/arb-bot/src/supervisor/dispatch.rs` + `run.rs`: per-transport opt-in flags; `crates/arb-tx/src/lib.rs`: version-aware bundle size gate (v1 ≤ 4096) |

### Env flags (both default `false` — fail-closed)

- `SOLANA_TX_V1_DIRECT_ENABLED` — allow v1 on the direct SWQoS/RPC path.
- `SOLANA_TX_V1_JITO_ENABLED` — allow v1 on the Jito bundle path (new).

When neither flag is set, a v1 transaction is rejected at dispatch with a transport
capability message; legacy/v0 stay at 1232 bytes. No automatic downgrade/relabeling is
performed.

## Operator checklist before enabling live V1

1. Confirm every configured RPC/sender (`SOLANA_RPC_URL`, Helius Sender, Jito block
   engines) runs a client that accepts v1 (SIMD-0385) and the 4096-byte size
   (SIMD-0296-aware transport).
2. Set the transport flag(s) you actually use; keep the other at `false`.
3. Verify with a **non-broadcast** `simulateTransaction` that returns
   `maxSupportedTransactionVersion: 2` acceptance (the pre-flight guard now sends it
   automatically) and `unitsConsumed > 0`.
4. Keep `mode=shadow` and run the full soak, then a single live canary per
   FINAL_ROUTE_INTEGRATION_STATUS.md release gates.

## Verification

- `cargo check -p arb-tx -p arb-sim -p arb-ingest -p arb-bot --all-targets` — 0 errors.
- `cargo clippy -p arb-tx -p arb-sim -p arb-ingest -p arb-bot --all-targets -- -D warnings` — 0 warnings.
- New tests: `test_rpc_max_supported_version_from_wire`,
  `test_validate_jito_bundle_is_version_aware` (arb-tx), updated
  `adaptive_compiler_tests.rs` version expectations, `arb-sim` simulate-params test,
  version-aware `fuzz_validate_jito_bundle_never_panics`.
