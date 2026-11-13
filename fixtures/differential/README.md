# Differential quote fixtures (Phase 12.13 / 13.2)

Golden values in this directory are produced OFFLINE by the official protocol
SDKs on identical on-chain state, then committed here. The harness in
`crates/arb-dex/tests/differential_fixtures.rs` decodes the same accounts and
asserts the local engine matches. No golden value is ever fabricated in Rust.

## Fixture schema

```json
{
  "protocol": "meteora_dlmm",
  "pool_address": "<32-byte hex, no 0x>",
  "pool_account_hex": "<full base64-decoded pool account bytes, hex>",
  "dependency_accounts": [
    { "pubkey": "<hex>", "data_hex": "<bin array / tick array / config bytes, hex>" }
  ],
  "direction": "a_to_b",
  "amount_in": 1000000,
  "expected_amount_out": 995123,
  "tolerance_bps": 0,
  "state_slot": 123456789,
  "sdk_reference": { "name": "@meteora-ag/dlmm", "version": "1.9.14" },
  "notes": "how and when this golden was generated"
}
```

Rules: `tolerance_bps` must be justified per protocol (default 0 = exact).
Record `state_slot` and the SDK version for every fixture. One direction and
one amount per file; generate both directions and several amounts (small,
boundary, large) per pool.

## Generation procedure per protocol

Fetch the live accounts once with a mainnet RPC (`getAccountInfo` with
base64), decode to raw bytes, and run the official SDK quote on those exact
bytes. Do not let the SDK re-fetch state mid-generation.

- Meteora DLMM: `@meteora-ag/dlmm@1.9.14` (IDL 0.8.5). `DLMM.create` on the
  pair, `getBinArrayForSwap(swapYtoX)`, then `swapQuote(amountIn, swapYtoX,
  binArrays)`. Save the pair account and every returned bin array as
  dependency accounts. Mind the changelog: exact-in quotes expose
  `consumedInAmount` semantics and recent releases changed fee attribution and
  limit-order handling inside bin decoding.
- Raydium AMM v4: pinned `raydium-amm` program source (commit 91739d8);
  compute the swap_base_input result with the pool's native fee
  numerator/denominator and effective reserves.
- Raydium CPMM: official CPMM SDK/docs; join pool + pool config; include the
  config account as a dependency. Note the status bitmask (bit 2 disables swaps).
- Raydium CLMM: official CLMM docs/SDK; include every tick array the swap
  range touches.
- Orca Whirlpools: `orca_whirlpools_core` swap math on the whirlpool + tick
  arrays; resolve the pinned-commit (2.1.1) vs crates.io (2.0.1) version
  discrepancy before generating.
- Pump.fun: `pump-sdk@4.1.0` / `pump-rust-client@0.4.1` curve math on the
  bonding curve account (WSOL-quoted curves only for now).
- PumpSwap / DAMM v2 / LaunchLab: harness decoder support must land first
  (HARNESS_GAP); generate fixtures only after the dispatcher accepts them.

## Running

```bash
cargo test -p arb-dex --test differential_fixtures
# or point at an external fixture tree:
ARB_DIFFERENTIAL_FIXTURES_DIR=/path/to/fixtures cargo test -p arb-dex --test differential_fixtures
```
