/**
 * Example: Fetch historical OHLCV candles
 * Run with: npx ts-node examples/market-data/ohlcv-chart.ts
 */

import { BirdeyeClient } from '../../templates/birdeye-client';

async function run() {
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) {
        console.error("Please export BIRDEYE_API_KEY.");
        process.exit(1);
    }

    const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'solana' });
    const WSOL = 'So11111111111111111111111111111111111111112';

    // Timeframes: 1s/15s/30s (V3 only), 1m, 3m, 5m, 15m, 30m, 1H, 2H, 4H, 6H, 8H, 12H, 1D, 3D, 1W, 1M
    const now = Math.floor(Date.now() / 1000);
    const thirtyDaysAgo = now - 30 * 24 * 3600;

    console.log(`Fetching 30 days of 1D OHLCV candles for WSOL...`);

    try {
        const data = await client.price.getOHLCV(WSOL, '1D', thirtyDaysAgo, now);
        const items = data?.items ?? [];
        console.log(`Successfully fetched ${items.length} candles.`);
        if (items.length) {
            console.log("Most recent candle:", items[items.length - 1]);
        }
    } catch (e: any) {
        console.error("[Error]", e.message);
    }
}

run();
