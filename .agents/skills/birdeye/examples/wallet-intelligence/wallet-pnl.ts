/**
 * Wallet P&L Summary — Birdeye /wallet/v2/pnl/summary
 *
 * PRO-only endpoint. Returns 403 on Standard/Lite tier.
 * Optional param: duration = 'all' | '90d' | '30d' | '7d' | '24h'
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/wallet-intelligence/wallet-pnl.ts [wallet] [duration]
 */

import BirdeyeClient from '../../templates/birdeye-client';

async function run() {
    const client   = BirdeyeClient.create('solana');
    const wallet   = process.argv[2] || '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';
    const duration = (process.argv[3] as any) || 'all';

    console.log(`\n⏳ Fetching P&L for ${wallet} (duration: ${duration})…\n`);

    try {
        const data = await client.wallet.getPnL(wallet, duration);
        const { counts, pnl, cashflow_usd } = data.summary;

        console.log('--- P&L Summary ---');
        console.log(`Realized Profit:  $${pnl.realized_profit_usd?.toFixed(2)}`);
        console.log(`Profit %:         ${pnl.realized_profit_percent?.toFixed(2)}%`);
        console.log(`Win Rate:         ${(counts.win_rate * 100).toFixed(1)}%`);
        console.log(`Total Trades:     ${counts.total_trade}`);
        console.log(`  Buys:           ${counts.total_buy}`);
        console.log(`  Sells:          ${counts.total_sell}`);
        console.log(`Total Invested:   $${cashflow_usd.total_invested?.toFixed(2)}`);
        console.log(`Total Sold:       $${cashflow_usd.total_sold?.toFixed(2)}`);
    } catch (e: any) {
        console.error('[Error]', e.message);
        if (e.statusCode === 403) {
            console.error('→ PRO tier required. This endpoint returns 403 on Standard/Lite.');
        }
    }
}

run().catch(err => console.error('Fatal:', err));
