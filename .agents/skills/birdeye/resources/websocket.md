# WebSocket Reference

Birdeye WebSocket streams require **Business tier or higher**.

---

## Connection

```
wss://public-api.birdeye.so/socket/{chain}?x-api-key=YOUR_API_KEY
```

Chain goes in the **URL path** — NOT a header, NOT an additional query param.

| Chain | URL |
|---|---|
| Solana | `wss://public-api.birdeye.so/socket/solana?x-api-key=KEY` |
| Ethereum | `wss://public-api.birdeye.so/socket/ethereum?x-api-key=KEY` |
| BSC | `wss://public-api.birdeye.so/socket/bsc?x-api-key=KEY` |
| Base | `wss://public-api.birdeye.so/socket/base?x-api-key=KEY` |
| Arbitrum | `wss://public-api.birdeye.so/socket/arbitrum?x-api-key=KEY` |
| Optimism | `wss://public-api.birdeye.so/socket/optimism?x-api-key=KEY` |
| Polygon | `wss://public-api.birdeye.so/socket/polygon?x-api-key=KEY` |

### Required connection setup

- Subprotocol: `echo-protocol` — pass as the subprotocol argument to `new WebSocket(url, subprotocol, options)`, NOT as a raw `Sec-WebSocket-Protocol` header.
- `Origin: ws://public-api.birdeye.so` header is required.

```typescript
import WebSocket from 'ws';

const ws = new WebSocket(
  `wss://public-api.birdeye.so/socket/solana?x-api-key=${API_KEY}`,
  'echo-protocol',
  { headers: { Origin: 'ws://public-api.birdeye.so' } }
);
```

---

## Subscription message formats

### Channels WITH `data` wrapper (most channels)

```json
{
  "type": "SUBSCRIBE_PRICE",
  "data": {
    "address": "TOKEN_ADDRESS",
    "currency": "usd",
    "type": "1H"
  }
}
```

Other channels using the `data` wrapper: `SUBSCRIBE_TXS`, `SUBSCRIBE_BASE_QUOTE_PRICE`, `SUBSCRIBE_WALLET_TXS`, `SUBSCRIBE_TOKEN_STATS`, `SUBSCRIBE_MEME`

### Channels WITHOUT `data` wrapper (params at top level)

```json
{ "type": "SUBSCRIBE_LARGE_TRADE_TXS", "min_volume": 10000, "max_volume": 100000 }
{ "type": "SUBSCRIBE_TOKEN_NEW_LISTING", "min_liquidity": 1000 }
{ "type": "SUBSCRIBE_NEW_PAIR", "min_liquidity": 500 }
```

Channels without data wrapper: `SUBSCRIBE_LARGE_TRADE_TXS`, `SUBSCRIBE_TOKEN_NEW_LISTING`, `SUBSCRIBE_NEW_PAIR`

### Unsubscribe

Replace `SUBSCRIBE` with `UNSUBSCRIBE`. No `data` wrapper needed:

```json
{ "type": "UNSUBSCRIBE_PRICE" }
{ "type": "UNSUBSCRIBE_LARGE_TRADE_TXS" }
```

---

## All 9 channels

| Channel | Description | CU/byte | Data wrapper | Chain |
|---|---|---|---|---|
| `SUBSCRIBE_PRICE` | Token OHLCV candle updates | 0.003 | yes | all |
| `SUBSCRIBE_TXS` | Live swap/trade feed | 0.0004 | yes | all |
| `SUBSCRIBE_BASE_QUOTE_PRICE` | Pair price updates | 0.003 | yes | all |
| `SUBSCRIBE_TOKEN_NEW_LISTING` | New token listings | 0.08 | **no** | SOL only |
| `SUBSCRIBE_NEW_PAIR` | New DEX pair creation | 0.05 | **no** | SOL only |
| `SUBSCRIBE_LARGE_TRADE_TXS` | Whale trades above threshold | 0.006 | **no** | all |
| `SUBSCRIBE_WALLET_TXS` | Live wallet activity | 0.004 | yes | all |
| `SUBSCRIBE_TOKEN_STATS` | Token stat updates | 0.005 | yes | all |
| `SUBSCRIBE_MEME` | Meme token stats | varies | yes | SOL only |

### Channel capacity limits

| Channel | Max addresses per sub |
|---|---|
| `SUBSCRIBE_PRICE` | 100 |
| `SUBSCRIBE_TXS` | 100 |
| `SUBSCRIBE_TOKEN_STATS` | 100 |
| `SUBSCRIBE_WALLET_TXS` | 1 address per subscription |
| `SUBSCRIBE_BASE_QUOTE_PRICE` | 1 pair per **connection** |

---

## Heartbeat / keepalive

Birdeye does not send server-side pings — implement client-side keepalive:

```typescript
function keepAlive(ws: WebSocket, intervalMs = 30_000): NodeJS.Timeout {
  return setInterval(() => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'PING' }));
    }
  }, intervalMs);
}
```

---

## Auto-reconnect with exponential backoff

```typescript
function createWebSocket(
  apiKey: string,
  chain: string,
  onMessage: (msg: any) => void,
  retryDelay = 1000
): WebSocket {
  const url = `wss://public-api.birdeye.so/socket/${chain}?x-api-key=${apiKey}`;
  const ws = new WebSocket(url, 'echo-protocol', {
    headers: { Origin: 'ws://public-api.birdeye.so' },
  });

  ws.on('message', (raw) => {
    try { onMessage(JSON.parse(raw.toString())); } catch {}
  });

  ws.on('close', (code) => {
    const nextDelay = Math.min(retryDelay * 2, 30_000);
    console.warn(`WS closed (${code}) — reconnecting in ${retryDelay / 1000}s`);
    setTimeout(() => createWebSocket(apiKey, chain, onMessage, nextDelay), retryDelay);
  });

  ws.on('error', (err) => {
    console.error('WS error:', err.message);
    ws.close();
  });

  return ws;
}
```

---

## CU billing

WebSocket CU is charged **per byte received** (not per message). High-traffic channels on Solana (`TOKEN_NEW_LISTING`, `NEW_PAIR`) generate large volumes — use `min_liquidity` filters to reduce noise and cost.

---

## Examples

- `examples/websocket/live-price-stream.ts` — SUBSCRIBE_PRICE with candle output
- `examples/websocket/whale-alert.ts` — SUBSCRIBE_LARGE_TRADE_TXS with Telegram/Discord routing
