# HFT Path Research (2026) — established routes & adoption decisions

Status: ACTIVE research note. Evidence class labels per repo standard:
`SOURCE_VERIFIED` / `REPOSITORY_VERIFIED` / `INFERRED` / `NOT_VERIFIED`.

## 1. Data ingestion

| Path | Verdict | Evidence |
|---|---|---|
| Helius WS accountNotifications | ACTIVE — current primary | REPOSITORY_VERIFIED (arb-ingest/src/helius_ws.rs); measured 217ms Clock cadence |
| Yellowstone gRPC (`yellowstone` feature) | BUILT, feature-gated | REPOSITORY_VERIFIED (arb-ingest yellowstone feature) |
| Jito ShredStream | DEFERRED — operator infra decision | SOURCE_VERIFIED: ShredStream first in 64.5% of 2,078,707 matched txs, ~32ms avg earlier than Geyser (Dysnix benchmark, https://dysnix.com/blog/solana-mev-infrastructure, 2026; https://rpcfast.com/blog/how-to-build-low-latency-infrastructure-for-solana-ai-agents) |

Decision rationale: ShredStream requires running/co-locating a Jito-enabled
node — infrastructure spend outside this repo's code surface; the `$` cost
conflicts with the active quota-saving constraint. Revisit when a dedicated
node or provider tier is authorized.

## 2. Submission path

| Path | Verdict | Evidence |
|---|---|---|
| RPC sendTransaction (Helius) | ACTIVE | REPOSITORY_VERIFIED |
| Jito bundle submission | ACTIVE code path; V1-bundle landing rate NOT_VERIFIED | https://docs.jito.wtf/; Jito-Solana >= 4.2.2 builds v1 (solanacompass, 2026-09-15) — earlier leaders will NOT build v1 into blocks |
| SWQoS staked submission / regional failover | DEFERRED | SOURCE_VERIFIED concept (https://rpcfast.com/blog/jito-explained-bundles-tips-mev-solana, 2026); requires staked provider relationship |
| TPU-direct (leader schedule + QUIC) | DEFERRED | SOURCE_VERIFIED concept (dev.to 2026 "no acknowledgment, no retry"; unstaked QUIC streams deprioritized) — high operational complexity |

## 3. In-code latency levers (this repository, today)

| Lever | Status |
|---|---|
| 3-leg cycle enumeration (structure-aware, leg-1 reuse) | IMPLEMENTED (three_leg_meme_pipeline.rs) — differential-pinned to legacy output |
| Multi-chunk getMultipleAccounts (4-in-flight) | IMPLEMENTED (rpc_pool.rs) — DATA-01 slot contract preserved |
| Tie RPC/provider to lowest-latency endpoint per region | MEASURED (Shyft-FRA 46.3ms avg best; Nodit 791ms rejected) — see LATENCY_BENCHMARKS |
| Provider V1 tx support (base64 >= 1232B; v1 no-ALT) | IMPLEMENTED (arb-tx compiler, TXC-F-N1 canary) |

## 4. What we deliberately do NOT do

- Co-location/bare-metal/kernel tuning: out of profile for the current
  quota-saving VPS; no measurable in-code equivalent.
- Blind tip escalation: never proven net-positive; adaptive bidding requires
  verified landing history (JITO_LANDING_RESEARCH).
- Retry storms after submit: fail-closed reconciliation only.
