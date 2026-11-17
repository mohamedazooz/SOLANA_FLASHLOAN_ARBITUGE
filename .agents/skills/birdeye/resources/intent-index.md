# Intent Index

**Fast path: intent keyword → exact endpoint.**  
Find your intent below, get the endpoint, then check `api-reference.md` for full params.

---

## Price & value

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Current price of one token | `GET /defi/price` | 10 | all |
| Current prices of many tokens (batch, max 100) | `GET /defi/multi_price` | 10/t | all |
| Price + volume in one call | `GET /defi/price_volume/single` | 15 | all |
| Price + volume batch | `POST /defi/price_volume/multi` | 15/t | all |
| Price at a specific past timestamp | `GET /defi/historical_price_unix` | 10 | all |
| Price history over a time range | `GET /defi/history_price` | 60 | all |
| Price stats (change across multiple windows) | `GET /defi/v3/price/stats/single` | 20 | all |
| All-time trade summary for a token | `GET /defi/v3/all-time/trades/single` | var | all |

---

## Charts (OHLCV)

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Candle chart for a token (all pools aggregated) | `GET /defi/v3/ohlcv` | dyn | all |
| Candle chart for a specific DEX pool | `GET /defi/v3/ohlcv/pair` | dyn | all |
| Candle chart by base/quote token pair | `GET /defi/ohlcv/base_quote` | 40 | all |

**OHLCV `type` values:** `1s` `15s` `30s` (V3 only) · `1m` `3m` `5m` `15m` `30m` · `1H` `2H` `4H` `6H` `8H` `12H` · `1D` `3D` `1W` `1M`

---

## Token stats and metadata

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Everything about a token (price + volume + supply + holders) | `GET /defi/token_overview` | 30 | all |
| Token name, symbol, decimals, logo | `GET /defi/v3/token/meta-data/single` | 5 | all |
| Token name/symbol/logo for many tokens | `GET /defi/v3/token/meta-data/multiple` | 5/t | all |
| Market cap, FDV, liquidity, supply | `GET /defi/v3/token/market-data` | 15 | all |
| Market cap/FDV/liquidity for many tokens | `GET /defi/v3/token/market-data/multiple` | 15/t | all |
| Buy/sell counts, unique traders | `GET /defi/v3/token/trade-data/single` | 15 | all |
| Trade data for many tokens | `GET /defi/v3/token/trade-data/multiple` | 15/t | all |
| Exit liquidity analysis | `GET /defi/v3/token/exit-liquidity` | var | **base** |
| Pair overview (pool details) | `GET /defi/v3/pair/overview/single` | 20 | all |
| Pair overviews (batch) | `GET /defi/v3/pair/overview/multiple` | 20/p | all |

---

## Token discovery

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Search token by name, symbol, or address | `GET /defi/v3/search` | low | multi |
| Browse tokens ranked by volume/liquidity/mc | `GET /defi/v3/token/list` | 100 | multi |
| Iterate full token universe (scroll) | `GET /defi/v3/token/list/scroll` | 500 | multi |
| Trending tokens right now | `GET /defi/token_trending` | var | all |
| Recently listed tokens / new listings | `GET /defi/v2/tokens/new_listing` | 80 | all |
| Meme tokens (pump.fun, bonding curve) | `GET /defi/v3/token/meme/list` | var | SOL/BSC/Monad |
| Meme token detail (graduation status, progress) | `GET /defi/v3/token/meme/detail/single` | 30 | SOL/BSC/Monad |
| Token creation info (deployer, timestamp) | `GET /defi/token_creation_info` | 80 | **SOL** |
| All DEX pools / markets for a token | `GET /defi/v2/markets` | var | all |

> ⚠️ `/defi/v3/token/meme/list` — official docs mark both `sort_by` and `sort_type` as required; pass them together. Common `sort_by` values: `liquidity`, `volume_24h_usd`, `market_cap`, `fdv`, `recent_listing_time`, `volume_24h_change_percent`, `progress_percent`, `holder`, `price_change_24h_percent`, `trade_24h_count`. See the official docs for the full enum.

---

## Security & risk

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Token security audit (mint auth, freeze auth, lock) | `GET /defi/token_security` | 50 | all except Sui |

---

## Holders

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| List of top token holders | `GET /defi/v3/token/holder` | var | **SOL** |
| Holder distribution by balance range | `GET /holder/v1/distribution` | var | **SOL** |
| Holder data for many tokens (batch) | `POST /token/v1/holder/batch` | var/t | **SOL** |

> ⚠️ `/holder/v1/distribution` requires `token_address=` param — NOT `address=`.

---

## Smart money

| Intent | Endpoint | CU | Tier |
|---|---|---|---|
| Tokens being bought/sold by smart money | `GET /smart-money/v1/token/list` | var | PRO+ |
| Smart money accumulation (sort net_flow desc) | `GET /smart-money/v1/token/list?sort_by=net_flow&sort_type=desc` | var | PRO+ |
| Smart money distribution (sort net_flow asc) | `GET /smart-money/v1/token/list?sort_by=net_flow&sort_type=asc` | var | PRO+ |

---

## Transactions & trades

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Recent trades for a token | `GET /defi/v3/token/txs` | 20 | all |
| Trades for a token filtered by time range | `GET /defi/v3/token/txs` + `before_time`/`after_time` | 20 | all |
| Trades above a USD volume threshold | `GET /defi/v3/token/txs-by-volume` | dyn | **SOL** |
| Trades for a specific DEX pool | `GET /defi/txs/pair` | 10 | all |
| All trades on chain (firehose) | `GET /defi/v3/txs` | 25 | all |
| All trades by a specific wallet | `GET /trader/txs/seek_by_time` | var | all |
| Mint events for a token | `GET /defi/v3/token/mint-burn-txs?type=mint` | var | **SOL** |
| Burn events for a token | `GET /defi/v3/token/mint-burn-txs?type=burn` | var | **SOL** |

---

## Wallet & portfolio

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Current portfolio value (net worth) | `GET /wallet/v2/current-net-worth` | 60 | **SOL** |
| Token-by-token portfolio breakdown | `GET /wallet/v2/net-worth-details` | var | **SOL** |
| Historical net worth chart | `GET /wallet/v2/net-worth` | 60 | **SOL** |
| Batch portfolio values for multiple wallets | `POST /wallet/v2/net-worth-summary/multiple` | var | **SOL** |
| Current token holdings | `GET /v1/wallet/token_list` | 100 | **SOL** |

---

## PnL (Profit & Loss) — PRO tier only

| Intent | Endpoint | CU |
|---|---|---|
| Overall PnL summary (win rate, total) | `GET /wallet/v2/pnl/summary` | var |
| PnL breakdown per token | `GET /wallet/v2/pnl` | var |
| Per-trade entry/exit PnL detail | `POST /wallet/v2/pnl/details` | var |
| Compare PnL across multiple wallets | `GET /wallet/v2/pnl/multiple` | var |
| Best traders for a specific token | `GET /defi/v2/tokens/top_traders` | 30 |
| Top performers / gainers | `GET /trader/gainers-losers` | var |

> ⚠️ `/trader/gainers-losers` — do NOT pass `type=gainers`/`type=losers`. Use `yesterday`, `today`, `1W`.

---

## Wallet history & transfers

| Intent | Endpoint | CU | Chain |
|---|---|---|---|
| Full transaction history for a wallet | `GET /v1/wallet/tx_list` | 150 | **SOL** |
| Balance changes (per-token in/out) | `GET /wallet/v2/balance-change` | 20 | **SOL** |
| Token transfer history | `POST /token/v1/transfer` | var | **SOL** |
| First wallet funding transaction | `POST /wallet/v2/tx/first-funded` | var | **SOL** |

---

## Realtime streams (WebSocket) — Business tier+

| Intent | Channel | CU/byte |
|---|---|---|
| Live price / OHLCV updates | `SUBSCRIBE_PRICE` | 0.003 |
| Live trades for a token | `SUBSCRIBE_TXS` | 0.0004 |
| Live trades for a wallet | `SUBSCRIBE_WALLET_TXS` | 0.004 |
| New token listings in real time | `SUBSCRIBE_TOKEN_NEW_LISTING` | 0.08 |
| New DEX pairs in real time | `SUBSCRIBE_NEW_PAIR` | 0.05 |
| Large trade / whale alerts | `SUBSCRIBE_LARGE_TRADE_TXS` | 0.006 |
| Live token stats | `SUBSCRIBE_TOKEN_STATS` | 0.005 |
| Live meme token stats | `SUBSCRIBE_MEME` | var |
| Live price for base/quote pair | `SUBSCRIBE_BASE_QUOTE_PRICE` | 0.003 |

See `resources/websocket.md` for connection setup and subscription formats.

---

## Utilities

| Intent | Endpoint |
|---|---|
| Check API credit usage | `GET /utils/v1/credits` |
| List supported blockchain networks | `GET /defi/networks` |
| Latest block number | `GET /defi/v3/txs/latest-block` |
| Supported chains for wallet API | `GET /v1/wallet/list_supported_chain` |
