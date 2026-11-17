# Phoenix API And SDK Reference

## Endpoints

| Surface | URL |
| --- | --- |
| REST | `https://perp-api.phoenix.trade` |
| WebSocket | `wss://perp-api.phoenix.trade/v1/ws` |
| OpenAPI | `https://docs.phoenix.trade/openapi/phoenix-public-api.json` |
| LLM index | `https://docs.phoenix.trade/llms.txt` |

## TypeScript SDK

Package: `@ellipsis-labs/rise`

Main exports:

- `createPhoenixClient`
- `PhoenixHttpClient`
- `createPhoenixWsClient`
- `createPhoenixWsFacade`
- `auth`
- `get`, `post`, `put`, `patch`, `del`
- `Side`, `Direction`, `StopLossOrderKind`
- `PhoenixHttpError`, `PhoenixAuthError`

Common client surfaces:

- `client.api`: typed REST route clients
- `client.pda`: PDA/address helpers
- `client.exchange`: exchange metadata cache
- `client.orderPackets`: metadata-backed order-packet builders
- `client.ixs`: instruction builders
- `client.streams`: WebSocket adapters
- `client.rpc`: raw account read surface
- `client.auth` and `client.sessionManager`: optional auth/session surfaces

Route groups under `client.api` include:

- `candles()`
- `collateral()`
- `exchange()`
- `funding()`
- `invite()`
- `markets()`
- `notifications()`
- `orderbook()`
- `orders()`
- `splines()`
- `traders()`
- `trades()`

## Rust SDK

Crate: `phoenix-rise`, import path `phoenix_rise`

Main surfaces:

- `PhoenixHttpClient`: typed REST client
- `PhoenixWSClient`: direct typed WebSocket subscriptions
- `PhoenixClient`: higher-level HTTP bootstrap plus reconnecting live runtime
- `PhoenixTxBuilder`: local transaction and instruction builder
- `PhoenixFlightClient`: beta Flight wrapper for supported order instructions
- `Trader`, `TraderKey`: trader state containers and PDA helpers

Useful Rust modules:

- `api`: REST route clients and payloads
- `accounts`: on-chain account fetchers and decoders
- `ix`: low-level instruction builders
- `math`: price, lots, margin, and risk helpers
- `types`: API, WebSocket, and account-backed types

## REST Categories

| Category | Use |
| --- | --- |
| Auth | Wallet, service, refresh, and logout sessions |
| Exchange | Exchange config, keys, markets, market config, snapshots, candles |
| Registration | Invite and referral activation |
| Trader | Trader state, PnL, collateral, funding, orders, trades, transaction builders |

Important REST routes:

- `GET /exchange`
- `GET /exchange/keys`
- `GET /exchange/markets`
- `GET /exchange/market/{symbol}`
- `GET /v1/exchange/snapshot`
- `GET /candles`
- `GET /trader/{authority}/state`
- `GET /trader/{authority}/pnl`
- `GET /trader/{authority}/collateral-history`
- `GET /trader/{authority}/funding-history`
- `GET /trader/{authority}/order-history`
- `GET /trader/{authority}/trades-history`
- `POST /v1/ix/place-isolated-limit-order`
- `POST /v1/ix/place-isolated-market-order`
- `POST /v1/ix/place-isolated-limit-order-enhanced`
- `POST /v1/ix/place-isolated-market-order-enhanced`
- `POST /v1/ix/cancel-conditional-order`
- `POST /v1/invite/activate`
- `POST /v1/invite/activate-with-referral`

## POST Request Bodies

The canonical schema is the OpenAPI document at `https://docs.phoenix.trade/openapi/phoenix-public-api.json`. When implementing POST routes directly, validate against that schema instead of guessing field names from SDK examples.

### Transaction Builders

All transaction-builder routes accept JSON and return Solana instruction DTOs that clients must convert, compose, sign, and send. Do not treat these routes as order submission by themselves.

Non-enhanced endpoints return `ApiInstructionResponse[]`:

- `POST /v1/ix/place-isolated-limit-order`
- `POST /v1/ix/place-isolated-market-order`
- `POST /v1/ix/cancel-conditional-order`

Enhanced endpoints return an object with `instructions: ApiInstructionResponse[]` and optional `estimatedLiquidationPriceUsd`:

- `POST /v1/ix/place-isolated-limit-order-enhanced`
- `POST /v1/ix/place-isolated-market-order-enhanced`

`ApiInstructionResponse` has:

- `programId` (string)
- `keys` (array of account metadata)
- `data` (array of integer bytes)

#### Isolated Limit Order

Routes:

- `POST /v1/ix/place-isolated-limit-order`
- `POST /v1/ix/place-isolated-limit-order-enhanced`

Required body fields:

- `authority` (string): trader authority pubkey
- `symbol` (string): market symbol, for example `SOL-PERP`
- `side` (string): order side, `bid` or `ask`

Optional body fields:

- `allowCrossAndIsolatedForAsset` (boolean or null)
- `feePayer` (string or null)
- `flightBuilderAuthority` (string or null)
- `flightFeeCollectorTrader` (string or null)
- `isPostOnly` (boolean or null): maker-only order when true
- `isReduceOnly` (boolean or null)
- `numBaseLots` (integer or null)
- `pdaIndex` (integer or null)
- `positionAuthority` (string or null)
- `price` (number or null): human-readable price
- `priceInTicks` (integer or null): tick price
- `quantity` (number or null): human-readable base quantity
- `skipTransferToParent` (boolean or null)
- `slide` (boolean or null): for post-only orders, slide to best price when crossing; defaults to true
- `tpSl` (`TpSlOrderConfig` or null)
- `transferAmount` (integer)

Example:

```json
{
  "authority": "AUTHORITY_PUBKEY",
  "symbol": "SOL-PERP",
  "side": "bid",
  "quantity": 0.25,
  "price": 150.5,
  "isPostOnly": true,
  "slide": true
}
```

#### Isolated Market Order

Routes:

- `POST /v1/ix/place-isolated-market-order`
- `POST /v1/ix/place-isolated-market-order-enhanced`

Required body fields:

- `authority` (string): trader authority pubkey
- `symbol` (string): market symbol, for example `SOL-PERP`
- `side` (string): order side, `bid` or `ask`

Optional body fields:

- `allowCrossAndIsolatedForAsset` (boolean or null)
- `feePayer` (string or null)
- `flightBuilderAuthority` (string or null)
- `flightFeeCollectorTrader` (string or null)
- `isReduceOnly` (boolean or null)
- `maxPriceInTicks` (integer or null)
- `numBaseLots` (integer or null)
- `pdaIndex` (integer or null)
- `positionAuthority` (string or null)
- `quantity` (number or null): human-readable base quantity
- `skipTransferToParent` (boolean or null)
- `tpSl` (`TpSlOrderConfig` or null)
- `transferAmount` (integer)

Example:

```json
{
  "authority": "AUTHORITY_PUBKEY",
  "symbol": "SOL-PERP",
  "side": "bid",
  "quantity": 0.25,
  "isReduceOnly": false
}
```

#### TP/SL Config

`tpSl` is optional on isolated limit and market orders. Fields are:

- `quantity` (number or null)
- `numBaseLots` (integer or null)
- `orderKind` (string or null)
- `stopLossTriggerPrice` (number or null)
- `stopLossTriggerPriceInTicks` (integer or null)
- `stopLossExecutionPrice` (number or null)
- `stopLossExecutionPriceInTicks` (integer or null)
- `takeProfitTriggerPrice` (number or null)
- `takeProfitTriggerPriceInTicks` (integer or null)
- `takeProfitExecutionPrice` (number or null)
- `takeProfitExecutionPriceInTicks` (integer or null)

Prefer tick fields when the calling context already has market metadata and exact tick conversion. Prefer SDK helpers when converting from human-readable USD prices.

#### Cancel Conditional Order

Route: `POST /v1/ix/cancel-conditional-order`

Required body fields:

- `authority` (string)
- `traderPdaIndex` (integer)
- `symbol` (string)
- `conditionalOrderIndex` (integer)
- `executionDirection` (string)

Optional body fields:

- `isIsolated` (boolean)
- `positionAuthority` (string or null)
- `traderSubaccountIndex` (integer or null)

Example:

```json
{
  "authority": "AUTHORITY_PUBKEY",
  "traderPdaIndex": 0,
  "symbol": "SOL-PERP",
  "conditionalOrderIndex": 0,
  "executionDirection": "lessThan",
  "isIsolated": true
}
```

### Invite And Referral Activation

`POST /v1/invite/activate` body:

- `authority` (string, required)
- `code` (string, required)

`POST /v1/invite/activate-with-referral` body:

- `authority` (string, required)
- `referral_code` (string, required)

Both return `ActivateInviteResponse` with `trader_pda`.

### Auth

`POST /v1/auth/login/service/challenge` body:

- `client_id` (string, required)
- `key_id` (string or null)

Returns `nonce`, `message`, `expires_at`, and `key_id`.

`POST /v1/auth/login/wallet` body:

- `wallet_pubkey` (string, required)
- `signature` (string, required)
- `nonce_id` (string, required)

`POST /v1/auth/login/service` body:

- `client_id` (string, required)
- `nonce` (string, required)
- `timestamp` (string, required)
- `signature` (string, required)
- `key_id` (string or null)

`POST /v1/auth/refresh` body:

- `refresh_token` (string, required)

Wallet login, service login, and refresh return `AuthResponse` with `token_type`, `access_token`, `expires_in`, `refresh_token`, `refresh_expires_in`, and `pop_key`.

`POST /v1/auth/logout` has no JSON body and returns `204` when the session is revoked.

## WebSocket Channels

Subscribe with:

```json
{
  "type": "subscribe",
  "subscription": {
    "channel": "orderbook",
    "symbol": "SOL"
  }
}
```

Unsubscribe with the same `subscription` object and `"type": "unsubscribe"`.

| Channel | Subscription fields | Response type |
| --- | --- | --- |
| `allMids` | none | all mid prices |
| `exchange` | optional `encoding` | exchange snapshot and deltas |
| `fundingRate` | `symbol` | funding rate update |
| `orderbook` | `symbol`, optional `bypassExecutionBand` | L2 book update |
| `traderState` | `authority`, `traderPdaIndex` | trader snapshot and deltas |
| `market` | `symbol` | market stats update |
| `trades` | `symbol` | trades update |
| `candles` | `symbol`, `timeframe` | candle update |

Supported candle timeframes: `1s`, `5s`, `1m`, `5m`, `15m`, `30m`, `1h`, `4h`, `1d`.

## Common Environment Variables

```bash
PHOENIX_API_URL=https://perp-api.phoenix.trade
PHOENIX_WS_URL=wss://perp-api.phoenix.trade/v1/ws
SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
NEXT_PUBLIC_SOLANA_RPC_URL=https://api.mainnet-beta.solana.com
```
