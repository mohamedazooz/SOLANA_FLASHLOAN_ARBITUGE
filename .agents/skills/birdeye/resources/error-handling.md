# Error Handling

## HTTP status codes

| Code | Meaning | Likely cause | Fix |
|---|---|---|---|
| `400` | Bad request | Missing required param, wrong enum value, malformed address | Check required params in `api-reference.md` |
| `401` | Unauthorized | Missing or invalid `X-API-KEY` | Verify key is in `X-API-KEY` header (not `Authorization`) |
| `403` | Forbidden | Endpoint requires higher plan tier (rarely, unusual User-Agent on legacy clients) | Upgrade plan for WebSocket (Business+) / PnL / SmartMoney (PRO+). Set any `User-Agent` defensively. |
| `404` | Not found | Token/wallet not on this chain, wrong address format | Verify `x-chain` matches the address's chain |
| `429` | Rate limited | Exceeded per-account tier limit, or Wallet API group's 30 rpm cap, or token-list-scroll 1 call / 30 s | Exponential backoff (see below) |
| `500` | Server error | Transient Birdeye issue | Retry with backoff |
| `503` | Service unavailable | Maintenance or overload | Retry after delay |

## Response body structure

Most Birdeye REST responses follow:

```json
{ "success": true, "data": { ... } }
```

On error:
```json
{ "success": false, "message": "Human-readable error", "statusCode": 400 }
```

**Check `response.success === true` before accessing `response.data` on endpoints that include it.**

**Exceptions** — a few endpoints return `data` directly without a `success` wrapper:
- `/smart-money/v1/token/list` → `{ "data": [ ... ] }` (no `success`, items array at top level)

For those, rely on the HTTP status code and presence of `data` instead.

## Exponential backoff for 429 / 5xx

```typescript
async function fetchWithRetry(
  url: string,
  options: RequestInit,
  maxRetries = 4
): Promise<Response> {
  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    const res = await fetch(url, options);

    if (res.ok) return res;

    if (res.status === 429 || res.status >= 500) {
      if (attempt === maxRetries) throw new Error(`Failed after ${maxRetries} retries: ${res.status}`);
      const delay = Math.min(1000 * Math.pow(2, attempt), 16000); // 1s → 2s → 4s → 8s → 16s cap
      await new Promise(r => setTimeout(r, delay));
      continue;
    }

    // 4xx (except 429) are not retryable
    const body = await res.json().catch(() => ({}));
    throw new Error(`API error ${res.status}: ${(body as any).message || res.statusText}`);
  }
  throw new Error('Unreachable');
}
```

## Rate limits by tier

Per-account limits apply across every endpoint. Wallet V1 group carries a separate documented 30 rpm cap (see note below).

| Tier | Per-account limit | Wallet V1 group (documented) | Token list scroll |
|---|---|---|---|
| Standard | 1 rps (60 rpm) | 30 rpm | 1 call / 30 s |
| Lite / Starter | 15 rps (900 rpm) | 30 rpm | 1 call / 30 s |
| Premium | 50 rps (1000 rpm) | 30 rpm | 1 call / 30 s |
| Business | 100 rps (1500 rpm) | 30 rpm | 1 call / 30 s |
| Enterprise | Custom | Custom | Custom |

> **Wallet API group** — the V1 wallet endpoints listed in Birdeye's rate-limiting docs (`/v1/wallet/token_list`, `/v1/wallet/token_balance`, `/v1/wallet/tx_list`, `/v1/wallet/list_supported_chain`, `/v1/wallet/simulate`, and their multichain variants) — carries a **30 rpm** cap per the official docs. Exact enforcement may differ by plan, so treat 30 rpm as a conservative floor: pace calls, handle 429 with backoff, avoid parallelizing. V2 wallet endpoints (`/wallet/v2/*`) aren't listed under that cap and appear to follow the per-account tier limit.
>
> **Token list scroll** (`/defi/v3/token/list/scroll`) is limited to **1 call per 30 seconds per account**.

```typescript
// Wallet sequencing — 30 rpm = 1 call per 2 s sustained.
async function walletFetch(url: string, headers: HeadersInit): Promise<any> {
  const res = await fetch(url, { headers });
  await new Promise(r => setTimeout(r, 2100)); // mandatory spacing for 30 rpm cap
  if (!res.ok) throw new Error(`Wallet API error ${res.status}`);
  return res.json();
}
```

## Common mistakes by symptom

| Symptom | Cause | Fix |
|---|---|---|
| `403` on every request | Usually a plan-tier restriction (PRO/Business-only endpoint) — or, rarely, a missing/bot-flagged `User-Agent` on some legacy HTTP clients | Confirm the endpoint's plan tier; set any `User-Agent` defensively |
| `success: false, message: "token not found"` | Wrong `x-chain` header | Set correct chain for the token's network |
| `400` on `/holder/v1/distribution` | Using `address=` param | Use `token_address=` param instead |
| `400` on `/defi/v3/token/meme/list` | Missing `sort_by` / `sort_type` (both marked required by official docs) or invalid `sort_by` | Pass both together. Common `sort_by` values: `liquidity`, `volume_24h_usd`, `market_cap`, `fdv`, `recent_listing_time`, `volume_24h_change_percent`, `progress_percent`, `holder`, `price_change_24h_percent`, `trade_24h_count`. See the [official docs](https://docs.birdeye.so/reference/get-defi-v3-token-meme-list) for the full enum. |
| `400` on `/trader/gainers-losers` | Passing `type=gainers` or `type=losers` | Use `yesterday`, `today`, or `1W` values instead |
| `400` on `/wallet/v2/current-net-worth` | Missing `sort_type` | `sort_type` is **required** on all `/wallet/v2/net-worth*` endpoints |
| Empty `data.items` on token search | No `keyword` with required sort params | For `/defi/v3/search`, `sort_by` and `sort_type` are required |
| Wallet endpoint returns empty or 403 | Wrong chain | Wallet APIs are Solana-only — set `x-chain: solana` |
| Swaps missing from wallet history | Using strict `mainAction === "swap"` filter | Also include `"buy"` and `"unknown"` — Raydium/Pump.fun routes show as `"unknown"` |
| Smart money 403 | Missing PRO tier | Smart money endpoints require PRO subscription |

## WebSocket errors

| Error | Cause | Fix |
|---|---|---|
| Connection refused / 403 | Wrong URL format or missing API key | Use `wss://public-api.birdeye.so/socket/{chain}?x-api-key=KEY` |
| No events received | Missing required connection setup | Add `Origin: ws://public-api.birdeye.so` header AND pass `echo-protocol` as the subprotocol argument (`new WebSocket(url, 'echo-protocol', { headers: { Origin: ... } })`) — not as a raw `Sec-WebSocket-Protocol` header |
| Subscription silently failing | Wrong message format for channel | `LARGE_TRADE_TXS`, `TOKEN_NEW_LISTING`, `NEW_PAIR` use top-level params (no `data` wrapper) |
| Connection drops | Ping timeout | Implement client-side heartbeat (see `resources/websocket.md`) |
| 403 on connect | Not on Business+ tier | WebSocket requires Business tier or higher |
