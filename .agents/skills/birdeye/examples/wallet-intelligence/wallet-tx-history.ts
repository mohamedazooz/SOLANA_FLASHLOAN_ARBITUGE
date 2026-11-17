/**
 * Wallet Transaction History — Birdeye /v1/wallet/tx_list
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/wallet-intelligence/wallet-tx-history.ts [wallet]
 *
 * ⚠️ ACTUAL FIELD SHAPES on /v1/wallet/tx_list (differ from /defi/v3/token/txs):
 *   tx.blockTime  → ISO string "2026-04-13T06:10:38+00:00"  (NOT unix number)
 *   tx.from       → plain wallet address string              (NOT an object)
 *   tx.to         → plain wallet address string              (NOT an object)
 *   token info    → tx.balanceChange[].symbol / .amount
 *
 * Correct time parse: new Date(tx.blockTime).getTime()
 * WRONG:              tx.blockTime * 1000  → NaN (blockTime is a string)
 */

import BirdeyeClient, { WalletTransaction } from '../../templates/birdeye-client';

function timeAgo(isoString: string): string {
    // blockTime is ISO string — parse with Date constructor, not numeric ops
    const ms   = Date.now() - new Date(isoString).getTime();
    const secs = Math.floor(ms / 1000);
    if (secs < 60)   return `${secs}s ago`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
    return `${Math.floor(secs / 86400)}d ago`;
}

async function run() {
    const client = BirdeyeClient.create('solana');
    const wallet = process.argv[2] || '9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM';

    console.log(`\n⏳ Fetching last 20 transactions for ${wallet}…\n`);

    // Response wrapper is keyed by chain: { solana: [...] }, NOT { items: [...] }
    const data = await client.wallet.getTxHistory(wallet, 20);
    const txs: WalletTransaction[] = data.solana ?? [];

    if (txs.length === 0) {
        console.log('No transactions found.');
        return;
    }

    for (const tx of txs) {
        // blockTime is an ISO string — use new Date(), not * 1000
        const when   = timeAgo(tx.blockTime);
        const action = tx.mainAction || 'unknown';

        // from/to are plain address strings, NOT objects with .symbol
        // Token info is in balanceChange[]
        const received = (tx.balanceChange ?? [])
            .filter(b => b.amount > 0)
            .map(b => `+${b.amount.toFixed(4)} ${b.symbol}`)
            .join(', ');

        const sent = (tx.balanceChange ?? [])
            .filter(b => b.amount < 0)
            .map(b => `${b.amount.toFixed(4)} ${b.symbol}`)
            .join(', ');

        console.log(`[${when}] ${action.padEnd(10)} | ${sent || '—'} → ${received || '—'}`);
        console.log(`  tx: ${tx.txHash}`);
    }
}

run().catch(err => console.error('Fatal:', err));
