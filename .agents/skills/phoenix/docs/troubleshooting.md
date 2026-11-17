# Phoenix Troubleshooting

## Wrong Phoenix SDK

**Symptom:** Code imports `@jup-ag/phoenix-sdk` or `@ellipsis-labs/phoenix-sdk` while trying to use Phoenix perpetuals.

**Fix:** Use `@ellipsis-labs/rise` for Phoenix perpetual futures. The older Phoenix SDKs target the legacy spot orderbook program.

## WebSocket Connects But No Data Arrives

**Cause:** Wrong endpoint, unsupported channel, mismatched symbol, or missing auth for a private stream.

**Fix:**

1. Use `wss://perp-api.phoenix.trade/v1/ws`.
2. Check the channel name against `resources/api-reference.md`.
3. Try a public channel such as `orderbook` with `SOL`.
4. For trader-specific streams, pass `authority` and `traderPdaIndex`.

## Stale Exchange Metadata

**Cause:** The app built order packets using old market metadata.

**Fix:** Call `await client.exchange.ready()` before instruction building. For long-lived processes, configure `exchangeMetadata: { stream: true }` and resync if exchange sequence numbers skip.

## Stop-Loss Trigger Price Looks Wrong

**Cause:** Some conditional-order helpers expect tick values rather than USD strings.

**Fix:** Convert from market metadata first, or follow the official conditional-order examples in the Rise repository.

## HTTP 429

**Cause:** API rate limiting.

**Fix:** Add retry budgets, exponential backoff, and caching. Do not retry live order submission blindly; confirm whether the previous request produced instructions or a submitted transaction before retrying.

## Floating Point Precision Problems

**Cause:** Protocol values are often integer-scaled and may exceed safe JavaScript integer precision.

**Fix:** Keep wire values as strings or bigint. Use SDK conversion and packet builders instead of hand-written floating point conversions.

