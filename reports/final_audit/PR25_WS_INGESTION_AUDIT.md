# PR25 — WS Ingestion Audit: Blind-Endpoint Strand

Branch: `vibe/ws-capability-failover-3adf0b` · Commit: `ea7f1a1`
Evidence date: 2026-11-13 (live mainnet, read-only probes)

## Symptom (live VPS run, commit 07159ab)

- `Missing fresh SOL/USD reference or invalid USD size cap; skip graph sizing`
  — **451 occurrences in ~2 minutes** of runtime. Graph sizing permanently skipped.
- `WS: subscription rejection warning (non-fatal) for id=Some(100): Method 'accountSubscribe' not found`
- `Advancing slot from RPC slot stream fallback` — 475 occurrences (slotSubscribe
  intentionally omitted to reserve the connection for pool accounts).
- Strategy cycles ran but produced 0 opportunities; every DLMM cross-pool scan
  rejected `EDGE_BELOW_HURDLE` on stale state.

## Endpoint probe matrix (independent, from sandbox + VPS)

| Endpoint | WS handshake | `accountSubscribe` | Verdict |
|---|---|---|---|
| `wss://solana-mainnet.g.alchemy.com/v2/<old key>` | OK | **REJECTED `-32601 Method 'accountSubscribe' not found`** | Cannot stream accounts |
| `wss://solana-mainnet.g.alchemy.com/v2/<new key>` (HTTP-valid: `getSlot` OK) | OK | **REJECTED `-32601`** | Platform limitation, not a key issue |
| `wss://mainnet.helius-rpc.com/?api-key=<key>` (+ `/v1/` path variant) | **HTTP 401** | — | Key not authorized for WS (valid for HTTP: earlier 429s prove auth) |
| `wss://ws.solami.dev/ws/sol?api_key=<key>` | **HTTP 400** | — | Unusable |
| `wss://solana-rpc.publicnode.com` | OK | **OK** (subscription id granted) | ✅ Only working endpoint |

## Root cause (code)

`crates/arb-ingest/src/helius_ws.rs`:

1. `build_ws_urls` prioritizes Alchemy first in the multi-sub sort, so the worker
   always connects to Alchemy first.
2. `handle_message` classified ANY error containing `limit` / `not found` /
   `unsupported` as `account_sub_limited` (soft): pending subscriptions were
   cleared, the connection stayed "healthy", and the stream loop never failed
   over. An endpoint that cannot implement `accountSubscribe` AT ALL was treated
   identically to a transient quota limit.

Result: the bot idled on a blind connection forever. Config alone cannot fix
this — adding a working endpoint behind Alchemy in the list never gets reached
because the Alchemy connection never fails.

## Fix (commit ea7f1a1)

- Capability rejections (`-32601`, message contains `not found` / `unsupported`)
  raise a new `endpoint_unsupported` flag.
- The stream loop checks the flag after each message: records a connect backoff
  for the endpoint (`record_connect_backoff`) and breaks to fail over.
- `limit` rejections keep the existing non-fatal `account_sub_limited` path.
- Flags reset on every successful connect.
- Regression tests: `handle_message_flags_unsupported_method_as_capability_failure`
  (capability vs limit orthogonality) and extended
  `handle_message_treats_subscription_errors_as_non_fatal`.

## Config companion fix (VPS, applied)

- `SOLANA_WS_1_URL=wss://solana-rpc.publicnode.com` added to `.env`
  (highest-priority WS slot; verified supporting `accountSubscribe`).
- Alchemy HTTP key rotated to the user-supplied valid key (HTTP RPC only).

## Operational risks

- **Single working WS endpoint (PublicNode)** — single point of failure for
  account streaming. Recommended: repair the Helius key's WS authorization or
  add a second provider (Shyft / QuickNode / Triton).
- PublicNode is a free public endpoint: unknown subscription ceilings; the
  worker's backoff/failover handles rejections, but capacity is unverified.
- `arb-executor.service` + `arb-monitor.service` on the VPS belong to an
  unrelated project (`~/arbitrum_full_engine`) with a missing binary — they
  crash-loop (`status=203/EXEC`) every 3s. Not our bot; recommend
  `systemctl disable --now` for both (user decision).
