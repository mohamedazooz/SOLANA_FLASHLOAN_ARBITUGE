import { createPhoenixClient } from "@ellipsis-labs/rise";

const client = createPhoenixClient({
  apiUrl: process.env.PHOENIX_API_URL ?? "https://perp-api.phoenix.trade",
});

async function main() {
  const symbol = process.argv[2] ?? "SOL";

  const [exchange, markets, market, orderbook] = await Promise.all([
    client.api.exchange().getExchange(),
    client.api.markets().getMarkets(),
    client.api.markets().getMarket(symbol),
    client.api.orderbook().getOrderbook(symbol),
  ]);

  console.log("exchange", exchange);
  console.log(
    "markets",
    markets.map((m: { symbol?: string }) => m.symbol).filter(Boolean)
  );
  console.log("market", market);
  console.log("top of book", {
    bid: orderbook?.bids?.[0],
    ask: orderbook?.asks?.[0],
  });
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

