/**
 * Pair Overview — Birdeye /defi/v2/markets + /defi/v3/pair/overview/single
 * Finds the best liquidity pool for a token and then fetches pair stats.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/pair-data/pair-overview.ts
 */

import { BirdeyeClient } from '../../templates/birdeye-client';

async function run() {
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

    const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'solana' });
    const tokenAddress = process.env.TOKEN_ADDRESS || 'So11111111111111111111111111111111111111112';

    console.log(`Finding top market for ${tokenAddress}...`);

    try {
        // Step 1: Get all markets (pairs) for this token
        const marketsData = await client.market.getMarkets(tokenAddress);
        const markets = marketsData?.items ?? (Array.isArray(marketsData) ? marketsData : []);

        if (markets.length === 0) {
            console.log('No markets found for this token.');
            return;
        }

        // Step 2: Pick the top-liquidity pair
        const topMarket = markets.sort((a: any, b: any) => (b.liquidity || 0) - (a.liquidity || 0))[0];
        const pairAddress = topMarket.address;

        console.log(`\nTop Market:`);
        console.log(`  Pair address: ${pairAddress}`);
        console.log(`  Source:       ${topMarket.source || topMarket.name || 'Unknown AMM'}`);
        console.log(`  Liquidity:    $${(topMarket.liquidity || 0).toLocaleString()}`);

        // Step 3: Fetch detailed pair overview
        // /defi/v3/pair/overview/single returns snake_case fields:
        // base.symbol, quote.symbol, price, liquidity, volume_24h, trade_24h
        const pair = await client.market.getPairOverview(pairAddress);

        console.log(`\nPair Stats:`);
        console.log(`  Name:       ${pair.name}`);
        console.log(`  Base:       ${pair.base?.symbol} / ${pair.quote?.symbol}`);
        console.log(`  Source:     ${pair.source}`);
        console.log(`  Price:      $${pair.price?.toFixed(6)}`);
        console.log(`  Liquidity:  $${pair.liquidity?.toLocaleString()}`);
        console.log(`  Vol 24h:    $${pair.volume_24h?.toLocaleString()}`);
        console.log(`  Trade 24h:  ${pair.trade_24h?.toLocaleString()}`);

    } catch (e: any) {
        console.error('[Error]', e.message);
    }
}

run();
