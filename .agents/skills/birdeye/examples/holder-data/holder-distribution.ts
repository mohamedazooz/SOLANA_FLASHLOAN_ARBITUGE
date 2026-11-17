/**
 * Holder Distribution — Birdeye /holder/v1/distribution
 * Analyzes token holder concentration.
 *
 * IMPORTANT: param name is `token_address` (not `address`) — confirmed via live API.
 * Response: data.summary.wallet_count, data.holders[].wallet, data.holders[].percent_of_supply
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/holder-data/holder-distribution.ts
 */

import { BirdeyeClient } from '../../templates/birdeye-client';

async function run() {
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

    const client = BirdeyeClient.createWithConfig({ apiKey, chain: 'solana' });
    const tokenAddress = process.env.TOKEN_ADDRESS || 'So11111111111111111111111111111111111111112';

    console.log(`Fetching holder distribution for ${tokenAddress}...`);

    try {
        const data = await client.holder.getDistribution(tokenAddress);

        const { summary, holders } = data;
        console.log('\n--- Holder Distribution ---');
        console.log(`Total Holders in Top Group: ${summary.wallet_count}`);
        console.log(`Total Holdings:             ${parseFloat(summary.total_holding).toLocaleString()}`);
        console.log(`% of Supply (top group):    ${summary.percent_of_supply?.toFixed(4)}%`);

        console.log('\nTop holders:');
        (holders || []).slice(0, 5).forEach((h: any, i: number) => {
            console.log(`  ${i + 1}. ${h.wallet} — ${h.percent_of_supply?.toFixed(4)}%`);
        });

        if (summary.percent_of_supply > 20) {
            console.warn('\n⚠️  HIGH CONCENTRATION: Top holders control >20% of supply.');
        } else {
            console.log('\n✅ Distribution looks healthy.');
        }
    } catch (e: any) {
        console.error('[Error]', e.message);
    }
}

run();
