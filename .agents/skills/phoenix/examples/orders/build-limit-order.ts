import { Side, createPhoenixClient } from "@ellipsis-labs/rise";

const client = createPhoenixClient({
  apiUrl: process.env.PHOENIX_API_URL ?? "https://perp-api.phoenix.trade",
  rpcUrl: process.env.SOLANA_RPC_URL ?? "https://api.mainnet-beta.solana.com",
  ws: false,
  exchangeMetadata: { stream: false },
});

async function main() {
  const authority = process.env.PHOENIX_AUTHORITY;
  if (!authority) {
    throw new Error("Set PHOENIX_AUTHORITY to the trader authority pubkey");
  }

  const symbol = process.argv[2] ?? "SOL-PERP";
  const side = process.argv[3] === "ask" ? Side.Ask : Side.Bid;
  const priceUsd = process.argv[4] ?? "150.50";
  const baseUnits = process.argv[5] ?? "0.25";

  await client.exchange.ready();

  const orderPacket = await client.orderPackets.buildLimitOrderPacket({
    symbol,
    side,
    priceUsd,
    baseUnits,
  });

  const instruction = await client.ixs.placeLimitOrder({
    authority,
    symbol,
    orderPacket,
  });

  console.log(JSON.stringify(instruction, null, 2));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

