# Meteora Two-Pool Arbitrage on Solana Transaction v1 (SIMD-0385)

Date: 2026-11-12
Status: implemented, unit + live-simulation verified, broadcast NOT enabled

## What this is

A second wire-format for the existing Meteora DLMM two-pool atomic arbitrage route.

The route itself is unchanged: two `swap2` calls on two **distinct** Meteora DLMM pools in one
atomic transaction, `Quote -> Base` on hop 1 and `Base -> Quote` on hop 2, with hop 2's input
pinned to hop 1's guaranteed minimum output and a final minimum-output floor.

What changes is the envelope:

| | legacy path | v1 path |
|---|---|---|
| Builder | `build_atomic_two_swap_with_traversal` | `build_atomic_two_swap_v1` |
| Wire discriminator | none (compact-u16 signature count) | `0x81` at byte 0 |
| Packet limit | 1,232 bytes | 4,096 bytes |
| Addresses | message account keys | inlined, 64 max, **no ALT** |
| Compute unit limit | `ComputeBudgetProgram` instruction | signed message config field |
| Priority fee | micro-lamports per CU instruction | total lamports in the message config |
| Loaded-accounts data cap | implicit | signed message config field |
| Instruction budget | 1,232 bytes is the binding limit | 64 instructions |
| Jito bundle ingress | supported | **not** supported (still 1,232 bytes) |

## Why v1 for a two-pool Meteora route

1. **Bin arrays are the cost driver.** A DLMM swap carries one account per bin array it may
   touch, and each extra array costs roughly 33 bytes of wire payload (32-byte key + one
   account index). On this route shape, measured: 2 arrays total = 911 bytes legacy /
   850 bytes v1; 6 arrays = 1,010 bytes legacy (1,232-byte cap leaves room for only about
   six more); 7 arrays = 1,015 bytes v1 with 24 distinct addresses. The legacy packet, not
   the route, is the binding constraint.
2. **Compute budget becomes tamper-proof.** In v1 the CU limit and the total priority fee are
   signed header fields. A wrapping program cannot strip or rewrite a `ComputeBudget`
   instruction because there is none.
3. **No ALT dependency.** Addresses are inlined, so a stale or missing lookup table can never
   turn a valid route into an unloadable transaction.

## API

```rust
use arb_tx::{MeteoraDlmmAtomicArbitrageBuilder, MeteoraTwoPoolV1Config};

let tx = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_v1(
    &payer,                  // KeypairSigner
    &pool_a, &pool_b,        // &PoolState, must be distinct
    swap_a_first,            // direction
    amount_in,
    expected_hop1_out,
    min_final_out,           // hard floor; never relaxed
    slippage_bps,
    MeteoraTwoPoolV1Config {
        compute_unit_limit: 260_000,              // 1..=1_400_000
        priority_fee_lamports: 0,                 // total lamports
        loaded_accounts_data_size_limit: 262_144, // 256 KiB
    },
    recent_blockhash,
    hop1_bins_traversed,     // quote.bins_crossed
    hop2_bins_traversed,
)?;

assert_eq!(tx.version, arb_tx::TransactionVersion::V1);
assert_eq!(tx.wire_bytes[0], 0x81);
```

`MeteoraAtomicArbitrageTx` now carries a `version: TransactionVersion` field, so a caller can
never mistake one envelope for the other. The legacy constructors report `Legacy`.

Both paths assemble their instructions through one private helper
(`assemble_two_swap_plan`), so the two envelopes cannot drift into different routes.

### ATA creation is now token-program aware

The shared plan creates the intermediate and input associated token accounts with
`build_create_ata_idempotent_for_program`, naming the mint's own token program. The previous
code used the hardcoded SPL Token variant, which would have produced an unrunnable ATA
instruction for a Token-2022 mint. A third ATA creation is added only when the route's output
mint differs from its input mint (i.e. the cycle does not close), which is the only case where
the output account is not already covered.

## Fail-closed behaviour

`build_atomic_two_swap_v1` returns an error rather than a broken transaction when:

* the two pools are not distinct (`TxError::InvalidInput`);
* `amount_in`, `expected_hop1_out` or `min_final_out` is zero (`TxError::ZeroBorrowAmount`);
* `compute_unit_limit` is `0` or above `1_400_000`, or
  `loaded_accounts_data_size_limit` is `0` (`TxError::InvalidInput`) — checked before signing,
  because v1 with a zero CU limit is guaranteed to fail at runtime;
* the route needs more than 64 distinct addresses (`TxError::InvalidInput`, and the message
  names the required count);
* the compiler falls back to another format, or the produced wire bytes do not parse back as
  v1 (`TxError::Serialization`) — the builder asserts the envelope instead of trusting it;
* the payload exceeds 4,096 bytes (`TxError::TransactionTooLarge`).

## Verification

* `crates/arb-tx/tests/meteora_atomic_builder_tests.rs` — 11 tests, all passing:
  `0x81` discriminator, `<= 4,096` bytes, no `ComputeBudgetProgram` instruction on the v1
  path, legacy = v1 route + 2 compute-budget instructions, final min-out floor bound into the
  hop-2 instruction data, invalid native config rejected, identical pools rejected, deep
  traversal inside the 64-address budget, over-budget route rejected by name.
  Measured on the fixture route (`--nocapture`): 2 arrays, 4 instructions —
  legacy 911 bytes vs v1 850 bytes; deep traversal, 7 arrays — v1 1,015 bytes,
  24 distinct addresses (40 of the 64 available).
* `crates/arb-sim/tests/meteora_mainnet_simulation_test.rs` compiles the live mainnet route in
  both envelopes from real pool state, alongside the existing non-broadcast simulation.
* Bin-array expectations in the builder tests are derived from `meteora_bin_array_indices`
  rather than a stale arithmetic shortcut, so a boundary fix cannot leave a test asserting the
  wrong array count.

## Enabling the route

* Library: call `build_atomic_two_swap_v1` directly. The library does not self-gate on an
  environment variable — the transport decision belongs to the caller.
* Simulation pipeline: `MeteoraFreshPipelineConfig::two_pool_v1` (default `false`) opts the
  fresh-state pipeline into v1. A v1 build failure falls back to the legacy packet with a
  warning, because v1 support is a transport property, not a route property.
* Live supervisor: unchanged. The supervisor's v1 use remains gated by
  `SOLANA_TX_V1_DIRECT_ENABLED` and is restricted to direct SWQoS/RPC routes; Jito bundle
  ingress stays on legacy/V0 at 1,232 bytes.

## Known limits

* v1 is only accepted by senders/relays that implement SIMD-0385. It is **not** Jito-ingress
  compatible.
* The workspace still does not pin an official SIMD-0385 implementation; the wire layout is
  validated differentially against the bundled expectations, not against a third-party parser.
* Nothing in this change broadcasts a transaction. The live test is non-broadcast simulation.
* The builder is a route-level primitive. Wiring `two_pool_v1` into the live supervisor's
  Meteora path (which today uses the Kamino flash-loan route) is separate work.
