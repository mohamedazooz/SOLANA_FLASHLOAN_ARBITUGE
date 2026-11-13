# V1 Mainnet Read-Only Probe — 2026-11-13

Live, read-only evidence for the Solana Transaction V1 (SIMD-0385) path and the
Kamino flash-loan spine, executed against `api.mainnet-beta.solana.com`
(solana-core 4.4.0, feature-set 2681577653). No transaction was signed or
broadcast; `ARB_LIVE=0` throughout.

## Method

1. `cargo run -q -p arb-tx --example v1_mainnet_sim_envelope` builds two
   UNSIGNED V1 simulation envelopes (zero-filled signature slots):
   - `baseline`: zero-instruction V1 envelope (pure transport probe).
   - `kamino_flash_roundtrip`: idempotent WSOL ATA create → SOL transfer →
     `SyncNative` → `flash_borrow_reserve_liquidity` (1 SOL, main-market WSOL
     reserve) → `flash_repay_reserve_liquidity` (principal only, index 3).
2. `python3 scripts/probe_rpc_wss_v1_sim.py` runs the RPC/WSS/state probes and
   submits both envelopes to `simulateTransaction` with
   `maxSupportedTransactionVersion: 1`, `sigVerify: false`,
   `replaceRecentBlockhash: true`, base64 encoding.

## Results (probe_report_2026-11-13.json)

| Check | Result |
| --- | --- |
| RPC reachable | solana-core 4.4.0, slot 468,721,487+ |
| Kamino WSOL reserve discriminator | matches `KAMINO_RESERVE_DISCRIMINATOR` |
| Live `flash_loan_fee_sf` (offset 8+4896) | 11,529,215,046,068 — matches constant |
| WSOL supply-vault flash liquidity | ~230,166 SOL available |
| Meteora SOL/USDC LbPair owner | DLMM program, 904-byte account |
| WSS `slotSubscribe` | connected, 5 slot notifications |
| Baseline V1 `simulateTransaction` | **err: null (success)** |
| Kamino flash roundtrip V1 sim | **err: null (success), ~62.6k CU** |

The flash roundtrip proves on live mainnet state: the V1 wire format is
accepted and executed by the current runtime; the klend borrow/repay
discriminators, account matrix, and `borrow_instruction_index` pairing are
correct; and the repay instruction's `amount` argument must be the borrow
PRINCIPAL (see below).

## Critical defect found and fixed by this probe

Simulating the pre-fix envelopes reproduced klend's
`InvalidFlashRepay` (6033 / 0x1791) twice:

1. `Liquidity amount for flash repay doesn't match borrow` — four builders
   (`dlmm_flash_arb.rs`, `dlmm_3leg_flash_arb.rs`, `dlmm_4leg_flash_arb.rs`,
   `cross_dex_flash_arb.rs`) passed `borrow + fee` as the repay instruction's
   `amount`. klend's `flash_borrow_check_matching_repay` (flash_ixs.rs)
   requires `repay_liquidity_amount == borrow_liquidity_amount` exactly; the
   origination fee is debited on top by the repay handler. Fixed: all four
   builders now pass `params.borrow_amount`.
2. `Borrow instruction index N ... doesn't match current index` — the V1
   compiler strips ComputeBudget no-ops from the message, shifting every
   executed instruction position, so a build-time `borrow_instruction_index`
   goes stale. Fixed: `compile_v1_message` now rewrites klend flash-repay
   indices to executed positions (fail-closed on dangling indices), and
   `v1_executed_instruction_index` exposes the mapping for reported indices.

## Reproduce

```bash
cargo run -q -p arb-tx --example v1_mainnet_sim_envelope > /tmp/v1_envelopes.json
python3 scripts/probe_rpc_wss_v1_sim.py /tmp/v1_envelopes.json
```

Optional: `V1_SIM_PAYER=<funded system account pubkey>` selects a fee payer
that exists on mainnet so the roundtrip executes past fee validation (the fee
payer is never charged in a simulation).
