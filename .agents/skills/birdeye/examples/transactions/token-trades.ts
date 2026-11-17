/**
 * Token Trades — Birdeye /defi/v3/token/txs
 * Fetches recent swaps for a token and calculates buy pressure.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/transactions/token-trades.ts
 */

import { BirdeyeClient } from '../../templates/birdeye-client';

async function run() {
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

    const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'solana' });
    const address = process.env.TOKEN_ADDRESS || 'So11111111111111111111111111111111111111112';

    console.log(`Fetching recent trades for ${address}...`);

    try {
        // /defi/v3/token/txs returns snake_case: tx_type, side, volume_usd, block_unix_time
        const data = await client.trades.getTokenTrades(address, 'swap', 50);
        const txs = data?.items ?? [];

        if (txs.length === 0) {
            console.log('No recent swaps found.');
            return;
        }

        // `side` is "buy"/"sell" from the token-address perspective
        const buys  = txs.filter(tx => tx.side === 'buy');
        const sells = txs.filter(tx => tx.side === 'sell');

        const buyVolume  = buys.reduce((sum, tx)  => sum + (tx.volume_usd || 0), 0);
        const sellVolume = sells.reduce((sum, tx) => sum + (tx.volume_usd || 0), 0);
        const totalVol   = buyVolume + sellVolume;
        const buyPressure = totalVol > 0 ? (buyVolume / totalVol) * 100 : 0;

        console.log(`\nLast ${txs.length} swaps analysis:`);
        console.log(`  Buys:            ${buys.length} (Vol: $${buyVolume.toLocaleString(undefined, { maximumFractionDigits: 0 })})`);
        console.log(`  Sells:           ${sells.length} (Vol: $${sellVolume.toLocaleString(undefined, { maximumFractionDigits: 0 })})`);
        console.log(`  Buy pressure:    ${buyPressure.toFixed(1)}%`);

        if (buyPressure > 60) {
            console.log('  Signal: 🟢 Strong buying pressure');
        } else if (buyPressure < 40) {
            console.log('  Signal: 🔴 Strong selling pressure');
        } else {
            console.log('  Signal: ⚪ Balanced — no clear direction');
        }

        // Recent trades
        console.log('\nMost recent trades:');
        txs.slice(0, 5).forEach(tx => {
            const time = new Date(tx.block_unix_time * 1000).toISOString();
            console.log(`  [${time}] ${tx.side} — $${(tx.volume_usd || 0).toFixed(2)} (${tx.source})`);
        });

    } catch (e: any) {
        console.error('[Error]', e.message);
    }
}

run();
