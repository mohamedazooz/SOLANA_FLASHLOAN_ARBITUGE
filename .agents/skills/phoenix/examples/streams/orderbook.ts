import { createPhoenixWsClient } from "@ellipsis-labs/rise";

const streams = createPhoenixWsClient({
  url: process.env.PHOENIX_WS_URL ?? "wss://perp-api.phoenix.trade/v1/ws",
  authMode: "anonymous",
});

async function main() {
  const symbol = process.argv[2] ?? "SOL";

  for await (const update of streams.orderbook(symbol)) {
    console.log(update);
    break;
  }
}

main()
  .catch((error) => {
    console.error(error);
    process.exitCode = 1;
  })
  .finally(() => {
    streams.close();
  });

