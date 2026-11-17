/**
 * Wallet Portfolio — Birdeye /wallet/v2/current-net-worth
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/wallet-intelligence/wallet-portfolio.ts [wallet]
 *
 * ⚠️ ACTUAL API FIELD NAMES (snake_case):
 *   data.total_value  → string  (NOT totalUsd)
 *   item.amount       → number  (token balance, already parsed)
 *   item.value        → string  (USD value — coerce: Number(item.value))
 *   item.price        → number  (token price)
 */

import BirdeyeClient from '../../templates/birdeye-client';

async function run() {
    const client = BirdeyeClient.create('solana');
    const wallet = process.argv[2] || '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';

    console.log(`\n⏳ Fetching portfolio for ${wallet}…\n`);

    const data = await client.wallet.getNetWorth(wallet);

    // total_value is a STRING — coerce before arithmetic
    const total = Number(data.total_value ?? 0);
    console.log(`📊 Portfolio  —  Total: $${total.toFixed(2)}\n`);

    const items = (data.items ?? [])
        .map(item => ({
            symbol:   item.symbol ?? '???',
            amount:   item.amount ?? 0,              // already a number
            valueUsd: Number(item.value ?? 0),       // value is a STRING — coerce
            price:    item.price ?? 0,
        }))
        .filter(item => item.valueUsd > 0)
        .sort((a, b) => b.valueUsd - a.valueUsd);

    if (items.length === 0) {
        console.log('  (no tokens with non-zero value)');
        return;
    }

    const col = (s: string, w: number) => String(s).slice(0, w).padEnd(w);

    console.log(
        `  ${col('Token', 12)} ${col('Balance', 16)} ${col('Value USD', 12)} Portfolio %`
    );
    console.log(`  ${'─'.repeat(57)}`);

    for (const item of items) {
        const pct = total > 0 ? ((item.valueUsd / total) * 100).toFixed(1) : '0.0';
        console.log(
            `  ${col(item.symbol, 12)} ${col(item.amount.toFixed(4), 16)} $${col(item.valueUsd.toFixed(2), 12)} ${pct}%`
        );
    }

    console.log();
}

run().catch(err => console.error('Fatal:', err));
