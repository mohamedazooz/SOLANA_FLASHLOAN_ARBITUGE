/**
 * Example: Fetching deep token data from Birdeye
 * Run with: npx ts-node examples/token-data/token-overview.ts
 */

import { BirdeyeClient } from '../../templates/birdeye-client';

async function run() {
    // Make sure to set export BIRDEYE_API_KEY="your-key"
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) {
        console.error("Please export BIRDEYE_API_KEY.");
        process.exit(1);
    }

    const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'solana' });

    // Solana's Wrapped SOL address
    const WSOL = 'So11111111111111111111111111111111111111112';

    console.log(`Fetching Token Overview for WSOL...`);

    try {
        const data = await client.token.getOverview(WSOL);

        // Actual /defi/token_overview response fields: marketCap, v24hUSD (not mc/volume24h)
        console.log("\n--- WSOL Metrics ---");
        console.log(`Price: $${data.price.toFixed(4)}`);
        console.log(`Market Cap: $${data.marketCap.toLocaleString()}`);
        console.log(`24h Volume: $${data.v24hUSD.toLocaleString()}`);
        console.log(`Holders: ${data.holder.toLocaleString()}`);
        console.log(`Unique Wallets 24h: ${data.uniqueWallet24h.toLocaleString()}`);

    } catch (e: any) {
        console.error("[Error]", e.message);
    }
}

run();
