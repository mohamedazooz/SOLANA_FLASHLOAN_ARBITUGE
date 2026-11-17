/**
 * Price & OHLCV — Birdeye /defi/price + /defi/v3/ohlcv
 * Fetches real-time price and last 24h of 1H candles.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/price-ohlcv/price-and-ohlcv.ts
 */

import { BirdeyeClient } from '../../templates/birdeye-client';

const WSOL = 'So11111111111111111111111111111111111111112';

async function run() {
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

    const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'solana' });
    const address = process.env.TOKEN_ADDRESS || WSOL;
    const now = Math.floor(Date.now() / 1000);
    const oneDayAgo = now - 86400;

    try {
        // 1. Fetch current price
        const priceData = await client.price.getPrice(address);
        console.log(`Price: $${priceData.value?.toFixed(6)}`);
        console.log(`Updated: ${new Date(priceData.updateUnixTime * 1000).toISOString()}`);

        // 2. Fetch 24h of 1H OHLCV candles
        const ohlcv = await client.price.getOHLCV(address, '1H', oneDayAgo, now);
        const candles = ohlcv?.items ?? [];
        console.log(`\nFetched ${candles.length} hourly candles\n`);

        if (candles.length > 0) {
            const highs = candles.map((c: any) => c.h);
            const lows  = candles.map((c: any) => c.l);
            const vols  = candles.map((c: any) => c.v);
            const resistance = Math.max(...highs);
            const support    = Math.min(...lows);
            const avgVol     = vols.reduce((a: number, b: number) => a + b, 0) / vols.length;
            const lastCandle = candles[candles.length - 1];

            console.log(`24h High (resistance): $${resistance.toFixed(4)}`);
            console.log(`24h Low  (support):    $${support.toFixed(4)}`);
            console.log(`Avg hourly volume:     $${avgVol.toLocaleString(undefined, { maximumFractionDigits: 0 })}`);
            // /defi/v3/ohlcv returns unix_time (snake_case); /defi/ohlcv V1 returns unixTime.
            const tsSec = lastCandle.unix_time ?? lastCandle.unixTime ?? 0;
            console.log(`\nLast candle:`);
            console.log(`  Time:   ${tsSec ? new Date(tsSec * 1000).toISOString() : 'unknown'}`);
            console.log(`  Open:   $${lastCandle.o?.toFixed(4)}`);
            console.log(`  Close:  $${lastCandle.c?.toFixed(4)}`);
            console.log(`  Volume: $${lastCandle.v?.toLocaleString()}`);
        }

    } catch (e: any) {
        console.error('[Error]', e.message);
    }
}

run();
