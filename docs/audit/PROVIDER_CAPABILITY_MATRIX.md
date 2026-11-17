# Provider Capability Matrix — Live Evidence

Date of verification: **2026-11-17** (live probes from this session; public endpoints + operator keys from local `.env`, never logged).

Method: direct WebSocket/HTTP JSON-RPC probes (python `websockets` + `urllib`), one subscription per method, immediate unsubscribe, all URLs redacted in logs.

## Live probe results (2026-11-17)

| Provider | Transport | Connect | slotSubscribe | accountSubscribe | logsSubscribe | programSubscribe | blockSubscribe | getMultipleAccounts (HTTP) |
|---|---|---|---|---|---|---|---|---|
| Public `api.mainnet-beta.solana.com` | HTTP + WS JSON-RPC | OK | ✅ ack + `slotNotification` | ✅ ack + `accountNotification` | ✅ ack (config struct required) | ✅ ack | ❌ `-32602` (unsupported/needs filter variant) | ✅ 200, 2 values |
| Helius (operator key) | HTTP + WS JSON-RPC | ❌ HTTP 429 (WS), 429 (HTTP) — persists after 60s cooldown | n/a | n/a | n/a | n/a | n/a | ❌ 429 |
| Shyft `rpc.fra.shyft.to` | HTTP + WS JSON-RPC | HTTP ✅ / WS ❌ HTTP 429 | n/a | n/a | n/a | n/a | n/a | ✅ 200, 2 values |
| Shyft `rpc.shyft.to` | HTTP JSON-RPC | OK | n/a | n/a | n/a | n/a | n/a | ✅ 200 + `getSlot` ✅ |
| Solami Mirage `ws.solami.dev` | WS proprietary (decoded stream) | Connect OK → closed **4002** "no bandwidth and balance under the minimum" | n/a | n/a | n/a | n/a | n/a | n/a |

## Interpretation

- **Public endpoint** is the only fully working free data plane right now: real slot + account notifications observed live. `logsSubscribe` requires the config-object form `{"commitment":"confirmed"}` (string form is rejected with `-32602`); `blockSubscribe` is not usable on the public node.
- **Helius**: persistent HTTP 429 on both transports even after a 65s cooldown — consistent with exhausted plan credits/CU for the period (matches `rpc_pool` quota-exhaustion classification: 1800s cooldown). Treated as **BLOCKED** until the plan resets.
- **Shyft**: HTTP plane healthy; WS plane rejects with HTTP 429 on this plan — matches the `ProviderCapabilities::standard_json_rpc_ws` declaration path but the *endpoint* is currently unusable for streaming (runtime correctly rotates; no crash).
- **Solami Mirage**: reconfirms gap **G5** — free plan closes the stream with WS close code 4002 ("no bandwidth"). Mirage stays disabled fail-closed; no fabricated Yellowstone frames are produced.

## Runtime enforcement (code evidence)

- Per-provider capability declarations + rejection of unsupported methods: `crates/arb-ingest/src/capability.rs` (`ProviderCapabilityRegistry::validate_subscription`, `validate_execution_prerequisites`; unit tests).
- Fail-closed startup gate wired into the supervisor: `crates/arb-bot/src/supervisor/run.rs:492-507` (refuses to start without a live account-streaming provider).
- Per-endpoint rate limits from live measured benchmarks: `crates/arb-ingest/src/rpc_pool.rs` (`classify_provider`: Alchemy 20rps/burst 25, RPCFast 25/30 batch 5, Shyft 8/10, Solami `/Solana` 2/3, Solami `/sol` 3/4, Helius 12/15).
- 429 / quota-exhaustion / Retry-After handling with cooldown + rotation: `crates/arb-ingest/src/rpc_pool.rs:868-905` (quota-exhausted → 1800s cooldown).
- All-providers-fail is fail-closed and bounded: `crates/arb-ingest/src/rpc_pool.rs` `call_filtered_idx` bails with `All N matching RPC endpoints failed…`; regression test `all_endpoints_failing_fails_closed_without_storm` (added 2026-11-17) proves one bounded error, cooldown entry, and no retry storm.
- Reconnect jitter: `crates/arb-ingest/src/helius_ws.rs` reconnect cooldown now carries a ±25% jitter overlay (thundering-herd desync); `arb_ingest::IngestClient::backoff_delay` already applies exponential backoff + jitter on the ingest reconnect path.

## Known limitations

- Concurrent-subscription and WS-connection limits are *declared* per provider (`RateLimitPolicy`) and observed live (Solami plan: "Concurrent websocket subscription limit reached: 1"), but are not yet probed exhaustively per plan tier.
- Per-method quota accounting exists (`method_metrics` per endpoint) but per-method *provider-side* limits are not separately verified.
- Monthly CU/RU limits: Helius 429-after-cooldown behavior is consistent with credit exhaustion but cannot be distinguished from burst limiting without provider dashboard access.

## Adapter inventory — hot-path consumption & retention reasons (2026-11-17)

| Adapter / module | In supervisor hot path? | Status | Retention reason |
|---|---|---|---|
| `helius_ws` (multi-endpoint WS JSON-RPC) | ✅ YES — `run.rs:485-507` drives the whole pipeline | ACTIVE | Primary live account/slot stream |
| `rpc_pool` (HTTP JSON-RPC, per-endpoint limits) | ✅ YES — bootstrap, hydration, dispatch support | ACTIVE | Isolated from hot-path market state (bootstrap + submission only) |
| `registry` / `discovery` / `PoolDiscovery` | ✅ YES — `run.rs:422-465` | ACTIVE | Pool discovery + decode |
| `provider_supervisor` | ✅ YES — `run.rs:845-854` | ACTIVE | Circuit breaker / health |
| `capability` | ✅ YES — fail-closed startup gate | ACTIVE | Capability enforcement |
| `slot_integrity`, `quote_readiness`, `dependency_manifest`, `rpc_reconcile` | ✅ YES | ACTIVE | State integrity + pre-dispatch guards |
| `solami_mirage` / `solami_blur` / `solami_stream` | ❌ NO — registered in capability registry only | DISABLED (fail-closed) | Free plan closes stream (WS 4002 "no bandwidth", G5). Kept: complete client code ready when plan allows; capability registry prevents mislabeling as JSON-RPC |
| `yellowstone_grpc` | ❌ NO — `yellowstone-grpc` cargo feature, off by default | FEATURE-GATED | No free gRPC provider available; code exists and is gated, never mislabeled as WS |
| `bitquery`, `solami_stream` | ❌ NO — `extra-ingest` feature | FEATURE-GATED | Optional enrichment feeds; excluded from default build |
| `dexscreener`, `meteora_data_api`, `gmgn`, `oxylabs`, `pumpdev_stream`, `pumpfun_discovery` | ❌ Not in supervisor loop | RETAINED (documented) | Used by auxiliary bins/tests/benches (e.g. `live_scanner` uses `gmgn`; `oxylabs_mock` test); candidate-discovery enrichment only, never market-state source of truth |

Authentication handling per active adapter: keys are read exclusively from environment (`HeliusWsClient::from_env`, `RpcPool::collect_rpc_urls`); URLs are redacted in all logs (`redacted_hosts`, `test_redact_url_and_hosts_hide_credentials`); no key is ever embedded in source or telemetry labels (only `..xxxx` suffix hints).
