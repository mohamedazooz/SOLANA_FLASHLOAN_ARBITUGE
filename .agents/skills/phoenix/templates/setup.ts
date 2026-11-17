import { createPhoenixClient } from "@ellipsis-labs/rise";

export const PHOENIX_API_URL =
  process.env.PHOENIX_API_URL ?? "https://perp-api.phoenix.trade";

export const PHOENIX_WS_URL =
  process.env.PHOENIX_WS_URL ?? "wss://perp-api.phoenix.trade/v1/ws";

export const SOLANA_RPC_URL =
  process.env.SOLANA_RPC_URL ??
  process.env.NEXT_PUBLIC_SOLANA_RPC_URL ??
  "https://api.mainnet-beta.solana.com";

export function createPhoenixPerpsClient() {
  return createPhoenixClient({
    apiUrl: PHOENIX_API_URL,
    rpcUrl: SOLANA_RPC_URL,
    pdaCache: { maxEntries: 1024 },
    exchangeMetadata: { stream: true },
    ws: { connectMode: "lazy" },
  });
}

