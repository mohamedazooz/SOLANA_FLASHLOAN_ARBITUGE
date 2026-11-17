# Pagination Guide

Birdeye uses four pagination patterns depending on the endpoint.

---

## Pattern 1: offset / limit (most common)

Used by: token list, holder list, trade history, balance changes, smart money list, etc.

**Request params**: `offset` (default: 0), `limit` (endpoint-specific max)  
**Response signals**: `data.total` (total count) or `data.hasNext` (boolean)

```typescript
async function paginate<T>(
  fetchPage: (offset: number, limit: number) => Promise<{ items: T[]; total?: number; hasNext?: boolean }>,
  limit = 50
): Promise<T[]> {
  const all: T[] = [];
  let offset = 0;

  while (true) {
    const page = await fetchPage(offset, limit);
    all.push(...page.items);

    const done = page.hasNext === false
      || (page.total !== undefined && all.length >= page.total)
      || page.items.length < limit;
    if (done) break;
    offset += limit;
  }

  return all;
}
```

### Per-endpoint max limits

| Endpoint | Max `limit` | Notes |
|---|---|---|
| `/defi/v3/token/list` | 50 | Default: 20 |
| `/defi/v3/token/txs` | 50 | — |
| `/defi/v3/token/holder` | 100 | — |
| `/defi/v3/search` | 20 | Hard cap |
| `/smart-money/v1/token/list` | 100 | — |
| `/defi/v2/tokens/new_listing` | — | No documented cap |
| `/wallet/v2/current-net-worth` | 100 | Token list in portfolio |
| `/v1/wallet/tx_list` | 100 | Pagination: `before` (tx hash cursor, Solana only) |

---

## Pattern 2: scroll_id (full dataset iteration)

Used by: `/defi/v3/token/list/scroll` **only**

⚠️ Costs 500 CU per call — only use when you need the full token universe.

```typescript
async function scrollAllTokens(apiKey: string): Promise<any[]> {
  const headers = { 'X-API-KEY': apiKey, 'x-chain': 'solana', 'accept': 'application/json' };
  const all: any[] = [];
  let scrollId: string | undefined;

  while (true) {
    // Valid sort_by for /defi/v3/token/list/scroll: liquidity | fdv | market_cap | holder
    const params = new URLSearchParams({ sort_by: 'liquidity', sort_type: 'desc', limit: '50' });
    if (scrollId) params.set('scroll_id', scrollId);

    const res = await fetch(`https://public-api.birdeye.so/defi/v3/token/list/scroll?${params}`, { headers });
    const json = await res.json() as any;
    const tokens = json.data?.tokens ?? [];
    all.push(...tokens);

    if (!json.data?.hasNext || tokens.length === 0) break;
    scrollId = json.data.scroll_id;
  }

  return all;
}
```

---

## Pattern 3: time-based pagination

Used by: `/defi/v3/token/txs`, `/defi/txs/token/seek_by_time`, `/trader/txs/seek_by_time`

Use `before_time` / `after_time` (Unix seconds) to window queries.
(Note: `/v1/wallet/tx_list` is NOT time-based — it uses a `before` tx-hash cursor, shown separately below.)

```typescript
async function fetchAllTrades(
  apiKey: string,
  tokenAddress: string,
  startTime: number,
  endTime: number
): Promise<any[]> {
  const headers = { 'X-API-KEY': apiKey, 'x-chain': 'solana', 'accept': 'application/json' };
  const all: any[] = [];
  let before = endTime;

  while (true) {
    const url = `https://public-api.birdeye.so/defi/v3/token/txs` +
      `?address=${tokenAddress}&before_time=${before}&after_time=${startTime}&limit=50`;
    const res = await fetch(url, { headers });
    const json = await res.json() as any;
    const items = json.data?.items ?? [];
    all.push(...items);

    if (!json.data?.hasNext || items.length === 0) break;
    // /defi/v3/token/txs returns snake_case: use `block_unix_time`, not `blockUnixTime`.
    before = items[items.length - 1].block_unix_time - 1;
  }

  return all;
}
```

---

## Pattern 4: tx-hash cursor (`/v1/wallet/tx_list`)

`/v1/wallet/tx_list` paginates with a `before` param — the transaction hash of
the last item from the previous page. Solana-only per official docs.

```typescript
async function fetchAllWalletTxs(apiKey: string, wallet: string): Promise<any[]> {
  const headers = { 'X-API-KEY': apiKey, 'x-chain': 'solana', 'accept': 'application/json' };
  const all: any[] = [];
  let before: string | undefined;

  while (true) {
    const params = new URLSearchParams({ wallet, limit: '100' });
    if (before) params.set('before', before);
    const res = await fetch(`https://public-api.birdeye.so/v1/wallet/tx_list?${params}`, { headers });
    const json = await res.json() as any;
    const txs = json.data?.solana ?? [];   // response is chain-keyed
    all.push(...txs);
    if (txs.length < 100) break;
    before = txs[txs.length - 1].txHash;
  }

  return all;
}
```

---

## Pattern 5: cursor (rare)

Some newer/beta endpoints may use cursor-based pagination. Check response for a `cursor` field and pass it back in the next request's `cursor` param.

---

## Summary

| Pattern | Endpoints | Key params |
|---|---|---|
| offset/limit | Most list endpoints | `offset`, `limit` → check `hasNext` or `total` |
| scroll_id | `/defi/v3/token/list/scroll` | `scroll_id` from response (500 CU/call) |
| time-based | `/defi/v3/token/txs`, `/defi/txs/*/seek_by_time`, `/trader/txs/seek_by_time` | `before_time`, `after_time`, `limit` |
| tx-hash cursor | `/v1/wallet/tx_list` | `before` (prev page's last `txHash`) — Solana only |
| cursor | Some beta endpoints | `cursor` from response |
