# Troubleshooting Guide

Common issues when building with Birdeye's API — verified against live API responses.

---

## 1. `403 Forbidden`

**Cause**: Usually the endpoint requires a higher plan tier (WebSocket → Business+, PnL/SmartMoney → PRO+). Rarely, some older HTTP clients hit 403 without a `User-Agent`.
**Fix**: Check the endpoint's tier at [Data Accessibility by Packages](https://docs.birdeye.so/docs/data-accessibility-by-packages). As a defensive measure set any `User-Agent`.

```typescript
const headers = {
  "X-API-KEY": process.env.BIRDEYE_API_KEY,
  "x-chain": "solana",
  "User-Agent": "my-app/1.0", // defensive — any string is fine
  "Accept": "application/json"
};
```

---

## 2. Wallet history missing swaps

**Cause**: Complex DeFi routes (Raydium, Pump.fun, Jupiter aggregated) classify as `mainAction: "unknown"`. Also: the response wrapper is keyed by chain (`data.solana`), not `data.items`.
**Fix**: Read the right wrapper key and widen the `mainAction` filter.

```typescript
const data = await client.wallet.getTxHistory(wallet);
const txs = data.solana ?? [];   // NOT data.items
const swaps = txs.filter((tx) => ['swap', 'buy', 'unknown'].includes(tx.mainAction));
```

---

## 3. `429 Too Many Requests`

**Cause**: Exceeded rate limits. Especially common with wallet APIs.
**Fix**: Exponential backoff; for wallet endpoints, add mandatory 2s delay between calls.

```typescript
async function fetchWithRetry(url: string, options: RequestInit, maxRetries = 4): Promise<Response> {
  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    const res = await fetch(url, options);
    if (res.ok) return res;
    if (res.status === 429 || res.status >= 500) {
      if (attempt === maxRetries) throw new Error(`Failed after ${maxRetries} retries: ${res.status}`);
      const delay = Math.min(1000 * Math.pow(2, attempt), 16000); // 1s → 2s → 4s → 8s → 16s cap
      await new Promise(r => setTimeout(r, delay));
      continue;
    }
    const body = await res.json().catch(() => ({}));
    throw new Error(`API error ${res.status}: ${(body as any).message || res.statusText}`);
  }
  throw new Error('Unreachable');
}

// Wallet endpoints: 30 RPM = 1 call per 2 seconds
async function walletFetch(url: string, headers: HeadersInit): Promise<any> {
  const res = await fetch(url, { headers });
  await new Promise(r => setTimeout(r, 2100)); // mandatory spacing
  if (!res.ok) throw new Error(`Wallet API error ${res.status}`);
  return res.json();
}
```

> **Wallet API group** (V1 wallet endpoints per Birdeye's rate-limiting docs — `/v1/wallet/token_list`, `/v1/wallet/token_balance`, `/v1/wallet/tx_list`, `/v1/wallet/list_supported_chain`, `/v1/wallet/simulate`, and their multichain variants): documented **30 rpm** cap. Enforcement may differ by plan, so start with ≥ 2 s spacing and widen on 429. V2 wallet endpoints (`/wallet/v2/*`) aren't listed under that cap.
>
> **Token List Scroll** (`/defi/v3/token/list/scroll`): only **1 call per 30 seconds per account**.
>
> **Per-account tier limits**: Standard 1 rps · Lite/Starter 15 rps · Premium 50 rps (1000 rpm) · Business 100 rps (1500 rpm).

---

## 4. Wrong network — `404` or empty data

**Cause**: Birdeye defaults to Solana if `x-chain` is not set.
**Fix**: Set `x-chain` header. Do **not** change the base URL.

```typescript
headers['x-chain'] = 'base'; // or: "ethereum" | "arbitrum" | "bsc" | "solana"
```

Supported chains (per `GET /defi/networks`): `solana`, `ethereum`, `bsc`, `base`, `arbitrum`, `optimism`, `polygon`, `avalanche`, `zksync`, `sui`, `monad`, `megaeth`, `fogo`, `aptos`.

---

## 5. PRO-only endpoints returning `403`

**Cause**: Endpoint requires PRO or higher subscription.
**Fix**: Verify tier at [bds.birdeye.so](https://bds.birdeye.so). PRO-gated endpoints:
- `GET /wallet/v2/pnl/summary` — P&L summary
- `GET /wallet/v2/pnl` — P&L full detail
- `GET /wallet/v2/pnl/details` — P&L per-token breakdown
- `GET /wallet/v2/pnl/multiple` — P&L batch wallets
- `GET /smart-money/v1/token/list` — Smart money signals

---

## 6. `400` from `/holder/v1/distribution`

**Cause**: Using `address=` instead of the required `token_address=` param.
**Fix**:
```bash
# Wrong:
GET /holder/v1/distribution?address=So111...
# Correct:
GET /holder/v1/distribution?token_address=So111...
```

---

## 7. `/trader/gainers-losers` returns `"type invalid format"`

**Cause**: Using `"gainers"` or `"losers"` as the `type` value — these are NOT valid enum values.
**Fix**: Use `type=today`, `type=yesterday`, or `type=1W`. All three params (`type`, `sort_by`, `sort_type`) are required. Note: there is no `time_frame` param on this endpoint.

```typescript
// Wrong — causes 400 (invalid type value):
GET /trader/gainers-losers?type=gainers&sort_by=PnL

// Wrong — causes 400 (missing required params):
GET /trader/gainers-losers?sort_by=PnL&time_frame=24h

// Correct:
GET /trader/gainers-losers?type=today&sort_by=PnL&sort_type=desc&limit=10
```

---

## 8. Batch / "multiple" endpoints returning `403`

**Cause**: Most batch endpoints require Business tier or higher.
**Fix**: On Standard/Lite, loop single-address endpoints with rate limiting.

> Per Birdeye docs: "The LITE/STARTER package can only access the multiple price API; all other 'multiple' APIs are not accessible."

---

## 9. WebSocket disconnects or fails to authenticate

**Requires Business tier+.** Chain goes in the **URL path**, not a header. Required: `Origin` header + `echo-protocol` as the **subprotocol argument** (not a raw `Sec-WebSocket-Protocol` header).

```typescript
import WebSocket from 'ws';

const chain = 'solana'; // replace with target chain
const ws = new WebSocket(
  `wss://public-api.birdeye.so/socket/${chain}?x-api-key=${API_KEY}`,
  'echo-protocol',
  { headers: { Origin: 'ws://public-api.birdeye.so' } }
);

ws.on('open', () => {
  ws.send(JSON.stringify({
    type: 'SUBSCRIBE_PRICE',
    data: { address: 'So11111111111111111111111111111111111111112', currency: 'usd', type: '1H' }
  }));
});
```

**Note**: `SUBSCRIBE_LARGE_TRADE_TXS`, `SUBSCRIBE_TOKEN_NEW_LISTING`, `SUBSCRIBE_NEW_PAIR` use params at the **top level** (no `data` wrapper):
```json
{ "type": "SUBSCRIBE_LARGE_TRADE_TXS", "min_volume": 10000 }
```

Available channels: `SUBSCRIBE_PRICE` · `SUBSCRIBE_TXS` · `SUBSCRIBE_BASE_QUOTE_PRICE` · `SUBSCRIBE_TOKEN_NEW_LISTING` · `SUBSCRIBE_NEW_PAIR` · `SUBSCRIBE_LARGE_TRADE_TXS` · `SUBSCRIBE_WALLET_TXS` · `SUBSCRIBE_TOKEN_STATS` · `SUBSCRIBE_MEME`

---

## 10. `/defi/v3/token/meme/list` returns `400`

**Cause**: `sort_by` and `sort_type` are both marked required by the official docs — pass them together, and use a valid `sort_by` value.
**Fix**: Pass `sort_by` with `sort_type`. Common `sort_by` values: `liquidity`, `volume_24h_usd`, `market_cap`, `fdv`, `recent_listing_time`, `volume_24h_change_percent`, `progress_percent`, `holder`, `price_change_24h_percent`, `trade_24h_count`. See the [official docs](https://docs.birdeye.so/reference/get-defi-v3-token-meme-list) for the full enum. Chain support: `solana`, `bsc`, `monad`.

```bash
# Wrong — invalid sort_by value:
GET /defi/v3/token/meme/list?sort_by=volume24hUSD&sort_type=desc&limit=20

# Correct:
GET /defi/v3/token/meme/list?sort_by=liquidity&sort_type=desc&limit=20
```

---

## 11. Check remaining API credits

```typescript
const credits = await client.getCredits();
console.log(credits); // total credits, used, reset_at
```

Use proactively to avoid unexpected suspension on metered tiers.
