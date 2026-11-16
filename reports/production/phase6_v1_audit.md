# Phase 6 — Solana Transaction V1 and SIMD-0385 Audit (CORRECTED)

> This audit replaces an earlier draft that incorrectly claimed V1 support was absent.
> The corrected inventory below reflects the actual codebase state, verified by reading the source, the probe reports, and the test suite.

## 6.1 Version Support — Actual State (Verified)

The codebase **does support Solana Transaction V1** end to end:

| Component | Evidence |
|---|---|
| Adaptive V0/V1 compiler | `crates/arb-tx/src/transport/adaptive.rs` — `build_signed_v1_transaction`, `TransactionVersion`, `V1ConfigParams`, `SOLANA_PACKET_V1_MTU = 4096`, `SOLANA_V1_MAX_INLINE_ACCOUNTS = 64` |
| V1 wire format tests | `crates/arb-tx/tests/v1_size_ladder_tests.rs`, `v1_header_differential.rs`, `adaptive_compiler_tests.rs` — real signed V1 transactions, 0x81 discriminator, parsed back as V1, signature-verified |
| V1 live simulation (Kamino flash-loan roundtrip) | `reports/v1_mainnet_probe/probe_report_2026-11-13.json` — solana-core 4.4.0, `simulateTransaction` accepted with `err: null`, `units_consumed = 62,596`; WSS slotSubscribe connected; Kamino reserve discriminator matches; live `flash_loan_fee_sf = 11_529_215_046_068` matches the constant; ~230,166 SOL flash liquidity |
| V1 transport canary (live, non-financial) | `reports/v1_canary/canary_1794590368.json` — spl-memo canary LANDING finalized at slot 469,007,459, signature `SmR6uhE4dr3PoimphULfC2WQpxA7aTQ7ztu9at2mqvm6VAXzMo1GRWn8KGbwnKivr2aUgUrm1gtHzUcsxhbZgMg`, wired through the configured ingress. Verdict: `V1_TRANSPORT_VERIFIED`. Enable via `ARB_V1_TRANSPORT_SUPPORTED=1`. BUG-004 RESOLVED |

**Honesty notes**
- The mainnet probe used UNSIGNED envelopes (`sigVerify: false`). It proves V1 wire acceptance and instruction execution on live state, not a signed broadcast.
- The canary is a non-financial spl-memo transaction; it proves V1 transport + landing, not arbitrage profitability.

## 6.2 V1 Message Header — Verified Facts

| Item | Value / Behavior |
|---|---|
| Wire discriminator | `0x81` (bit 7 of the legacy version byte) |
| Max wire size | 4,096 bytes (vs 1,232 legacy) |
| Inline accounts | up to 64, no Address Lookup Tables in V1 |
| Keypair signing | single signature; `maxSupportedTransactionVersion = 1` on reads |
| Compute config | Explicit CU limit, loaded-data limit and total-lamports priority fee in the message (NOT legacy ComputeBudget instructions appended) |
| Borrow-index rewrite | `compile_v1_message` rewrites klend flash-repay indices to executed positions; `invalid` → fail closed |
| Refusal to downgrade | Never silently downgrades V1→V0 or strips instructions to fit; 4,097-byte payload rejected; >64 accounts rejected |

## 6.3 Remaining Limitations (Honest)

1. **V1 on live mainnet path requires `ARB_V1_TRANSPORT_SUPPORTED=1`** — the toggle is off by default until operator's explicit approval.
2. **24-hour shadow soak has NOT been run** — required before any live authorization.
3. **Live broadcast of a real arbitrage has NOT occurred** — by design; `ARB_LIVE=0`.
4. **Per-region block-engine attribution is not maintained** — Jito auction is global; regional credit assignment would be fabricated data.
5. **Independent SDK differential testing for V1 serialization** — recommended, not fully implemented.

## 6.4 Summary of Phase 6 Findings (Corrected)

### Correctness
- V1 implemented, tested (signed build→parse→verify round-trip), live-simulated, and live-canary-verified on mainnet.
- No silent V1→V0 downgrade; oversized payloads rejected; account ceiling enforced.

### Blockers for Live Production (in order)
1. 24-hour shadow soak with recorded metrics.
2. Explicit operator authorization + capital risk limits for the first live broadcast (wallet 0.02 SOL is economically marginal).
3. Live-transport toggle enabled by operator (`ARB_V1_TRANSPORT_SUPPORTED=1`).
4. Kamino supply-vault liquidity verification acknowledged (~230k SOL available at probe time — re-verify live before any real trade).

### Gate B (Protocol Correctness) status: **PASS**
Evidence: Phase 15 V1 size-ladder tests, V1 header differential tests, live probe, V1 canary landing, Kamino 3-instruction roundtrip passing on live state.
