/**
 * Trader Gainers — Birdeye /trader/gainers-losers
 * Returns top performing traders on-chain by P&L.
 *
 * Required params: type, sort_by, sort_type (all three — omitting any causes 400)
 *   type: 'today' | 'yesterday' | '1W'
 *   sort_by: 'PnL' (only valid value)
 *   sort_type: 'desc' | 'asc'
 *
 * WARNING: type='gainers' or type='losers' causes 400 "type invalid format".
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/trader-intelligence/gainers-losers.ts
 */

import BirdeyeClient from '../../templates/birdeye-client';

async function run() {
    const client = BirdeyeClient.create('solana');

    console.log('Fetching top gainers (today, by P&L)...');

    try {
        const data = await client.trader.getGainers('today', 'PnL', 'desc', 10);
        const traders = data?.items ?? [];

        console.log(`\nTop ${traders.length} Gainers:\n`);
        traders.forEach((t: any, i: number) => {
            console.log(`${i + 1}. ${t.address}`);
            console.log(`   PnL:    $${t.pnl?.toFixed(2)}`);
            console.log(`   Volume: $${t.volume?.toFixed(2)}`);
            console.log(`   Trades: ${t.trade_count}`);
        });
    } catch (e: any) {
        console.error('[Error]', e.message);
    }
}

run();
