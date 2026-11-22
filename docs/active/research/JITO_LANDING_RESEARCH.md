# Jito Landing Research

**S3 verification date**: 2026-11-22. Cross-checked against official docs + independent secondary sources; details also recorded in register Sessions 1–2.

## Verified facts

| Item | Finding | Source (S3 unless noted) |
|---|---|---|
| Tip accounts | 8 canonical accounts; retrieve via `getTipAccounts`; randomize selection to reduce contention | [jito low-latency send docs](https://docs.jito.wtf/lowlatencytxnsend/), [Alchemy sendBundle](https://www.alchemy.com/docs/chains/solana/solana-jito-bundles-and-transactions/solana-jito-bundles-and-transactions/send-bundle) |
| Min tip | 1,000 lamports, block-engine enforced; below-auction tips are accepted but do not land | [Alchemy](https://www.alchemy.com/docs/chains/solana/solana-jito-bundles-and-transactions/solana-jito-bundles-and-transactions/send-bundle), [devrels block engine article](https://devrels.xyz/articles/jito-block-engine-demystified) |
| Bundles | ≤ 5 signed txs, executed sequentially & all-or-nothing in one block | [QuickNode guide](https://www.quicknode.com/guides/solana-development/transactions/jito-bundles), [mev-protos](https://github.com/jito-labs/mev-protos/blob/master/json_rpc/http.md) |
| Acceptance ≠ landing | `sendBundle` returns a `bundle_id` on receipt (even with success if bundle not expiring before next Jito leader); poll `getInflightBundleStatuses` (≤5 ids, 5-min lookback) → Failed/Pending/Landed/Invalid, then `getBundleStatuses` (slot + confirmation status) | [mev-protos http.md](https://github.com/jito-labs/mev-protos/blob/master/json_rpc/http.md) (S3) |
| Tip placement | Tip must be in the **last transaction** of the bundle; any SOL transfer to a tip account (top-level or CPI); tip instruction must not go through ALT resolution | [mev-protos](https://github.com/jito-labs/mev-protos/blob/master/json_rpc/http.md) (S3) |
| Auction | Parallel auctions at ~50 ms ticks; bundle priority by tip / CU efficiency; write-locked account overlap shares an auction | [devrels](https://devrels.xyz/articles/jito-block-engine-demystified) (S3) |
| Regions | global, amsterdam, dublin, frankfurt, london, ny, slc, singapore | register Session 1 (inherited) + devrels (S3) |
| `sendTransaction` path | Proxies to validator, `skip_preflight=true` always → the bot must self-simulate | register Session 1 (inherited) |
| Rate limits | default 1 request/s/IP/region; 429 on exceed | register Session 1 (inherited) |
| Jito-Solana stake share | ~88% of stake sorts bundles by tip | [devrels priority fees 2026](https://devrels.xyz/articles/solana-priority-fees-2026) (S3) |

## Existing implementation

- `crates/arb-land/` Jito transports (`jito-client 0.3.5`, `jito-sdk-rust 0.3.2`), tip floor default 10,000 lamports (stricter than the 1,000-lamport engine minimum — safe direction).
- On-chain conditional tip: `programs/arb-executor` `execute_three_leg_with_tip` — tip paid only after the three-leg body succeeds; canonical tip-account validation + WSOL guard + retained-edge floor (register Session 2). Bundle all-or-nothing semantics make this the safe cost posture.
- Landing reconciliation treats acceptance ≠ landed; signature + bundle id reconciliation in `arb-bot/src/supervisor/recovery.rs` (ISSUE-P1-02).

## Gaps / recommendations (REC, not facts)

1. **Regional endpoint selection is static/assumed** — adaptive region choice by leader schedule requires `getRegionBundles`-style measurement we do not have. Action: record region + tip + landing outcome per attempt in `arb-store`; only then allow region preference (never hardcode).
2. **Adaptive bidding** must use verified historical landing data only; higher tip reduces net profit monotonically and can make a marginal opportunity negative. Action: enforce `tip ≤ ceil(edge_share × net_edge)` (already implemented as share caps 70/85 bps defaults) — keep as config, not constant.
3. V1 bundle support: bundles carry versioned txs; verify upstream jito-client accepts 0x81 wire bytes before shipping V1-over-bundle (tracked in TEST_MATRIX; blocked without networked RPC evidence).

## Tests required

- Bundle-status state machine: `sendBundle` accepted + never landed + blockhash expiry ⇒ intent expired, no resubmit-with-new-signature (exists in recovery tests — keep).
- Tip economic invariant: losing execution pays no tip (LiteSVM test of `execute_three_leg_with_tip` path).
- Rate-limit handling: 429 → ≥1 s backoff, single automatic retry, then surface.
