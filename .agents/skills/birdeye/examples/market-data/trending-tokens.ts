/**
 * Example: Find trending tokens on Solana by rank.
 *
 * sort_by valid values: 'rank' | 'volumeUSD' | 'liquidity' (for the API param).
 * sort_by and sort_type are both required.
 * Response items expose volume as `volume24hUSD` — NOT `volumeUSD`.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/market-data/trending-tokens.ts
 */

import BirdeyeClient from '../../templates/birdeye-client';

async function run() {
    const client = BirdeyeClient.create('solana');

    console.log('Fetching top trending tokens on Solana...');

    try {
        const tokens = await client.market.getTrending('rank', 'asc', 10);

        console.log(`\nTop 10 Trending Tokens (by rank):`);
        tokens.forEach((token, index: number) => {
            console.log(`${index + 1}. ${token.symbol || 'Unknown'} (${token.address})`);
            console.log(`   Price:     $${token.price}`);
            console.log(`   Volume24h: $${token.volume24hUSD?.toLocaleString()}`);
            console.log(`   Liquidity: $${token.liquidity?.toLocaleString()}`);
            console.log('---');
        });
    } catch (e: any) {
        console.error('Error:', e.message);
    }
}

run();
