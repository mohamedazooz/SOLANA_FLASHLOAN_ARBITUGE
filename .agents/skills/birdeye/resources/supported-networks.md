# Supported Networks

Birdeye's multi-chain support. Set the appropriate value in the `x-chain` header on every request.

---

## Supported Chains

Authoritative list — queried from `GET /defi/networks`. Call that endpoint for the
live set; the table below is accurate as of this skill's publish date.

| Chain | `x-chain` Value | Notes |
|---|---|---|
| Solana | `solana` | **Primary chain — full feature support** |
| Ethereum | `ethereum` | |
| BNB Smart Chain | `bsc` | |
| Base | `base` | |
| Arbitrum | `arbitrum` | |
| Optimism | `optimism` | |
| Polygon | `polygon` | |
| Avalanche | `avalanche` | |
| zkSync Era | `zksync` | |
| Sui | `sui` | |
| Aptos | `aptos` | |
| Mantle | `mantle` | |
| Monad | `monad` | |
| MegaETH | `megaeth` | |
| Fogo | `fogo` | |
| HyperEVM | `hyperevm` | |

---

## Endpoint Chain Restrictions

Not all endpoints are available on all chains.

| Endpoint | Chains Supported |
|---|---|
| `GET /defi/v3/ohlcv` | Solana, Ethereum, BSC, Base, Monad |
| `GET /wallet/v2/pnl/summary` | Solana, Base |
| `GET /wallet/v2/current-net-worth` | Solana, Base |
| `GET /wallet/v2/net-worth` | Solana only |
| `GET /defi/v2/tokens/top_traders` | All chains — but PnL `sort_by` values (`total_pnl`, `unrealized_pnl`, `realized_pnl`, `volume_usd`) and `2d`–`90d` timeframes are **Solana-only** |
| `GET /defi/token_security` | All chains except Sui |
| `GET /trader/gainers-losers` | Solana only |
| `GET /defi/v3/token/meme/list` | Solana, BSC, Monad |
| `GET /defi/v3/search` — `verify_token` filter | Solana only |
| `GET /defi/v3/search` — `markets` filter | Solana only |
| `GET /defi/v3/token/txs` — AMM source filter | Solana only |
| WebSocket channels | Solana primary |

---

## Setting the Chain

```typescript
const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'base' });
// or, to read apiKey from BIRDEYE_API_KEY env var:
// const client = BirdeyeClient.create('base');
```

> The chain is set in the **`x-chain` header**, never in the URL path.
> All chains share the same base URL: `https://public-api.birdeye.so`

---

## Checking Live Supported Chains

```bash
GET /defi/networks
```
