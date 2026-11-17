/**
 * Birdeye x402 Pay-Per-Request
 *
 * Query Birdeye without an API key — pay per request in USDC on Solana.
 * No subscription, no custody of funds, no account required.
 *
 * Requirements:
 *   npm install @x402/fetch @solana/web3.js
 *
 * Run:
 *   SOLANA_PRIVATE_KEY='[1,2,3,...]' npx ts-node examples/x402/pay-per-request.ts
 *
 * The wallet must hold USDC on Solana mainnet.
 *
 * x402 flow (handled automatically by withPaymentInterceptor):
 *   1. Send request without payment → server returns 402 + PAYMENT-REQUIRED header
 *   2. Library signs USDC transaction on Solana
 *   3. Retry request with PAYMENT-SIGNATURE header → server returns 200 + data
 */

import { withPaymentInterceptor } from '@x402/fetch';
import { Keypair } from '@solana/web3.js';

// ── wallet setup ─────────────────────────────────────────────────────────────

function loadKeypair(): Keypair {
    const raw = process.env.SOLANA_PRIVATE_KEY;
    if (!raw) throw new Error('Set SOLANA_PRIVATE_KEY=[...] env var (byte array JSON)');
    return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(raw)));
}

// ── x402 fetch client ─────────────────────────────────────────────────────────

const BASE = 'https://public-api.birdeye.so/x402';  // note: /x402 prefix

function makeClient(keypair: Keypair) {
    // withPaymentInterceptor wraps fetch — automatically handles 402 → sign → retry
    return withPaymentInterceptor(globalThis.fetch, { wallet: keypair });
}

async function get(
    fetchFn: typeof fetch,
    path: string,
    params: Record<string, string> = {},
    chain = 'solana'
): Promise<any> {
    const url = new URL(`${BASE}${path}`);
    for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v);

    const res = await fetchFn(url.toString(), {
        headers: {
            'x-chain': chain,
            'Accept':  'application/json',
            // ⚠️ No X-API-KEY needed — payment is via PAYMENT-SIGNATURE header
        },
    });

    if (!res.ok) throw new Error(`HTTP ${res.status} — ${await res.text()}`);

    const json = await res.json() as any;
    if (!json.success) throw new Error(`API error: ${json.message}`);

    // PAYMENT-RESPONSE header contains the settlement receipt for audit
    const receipt = res.headers.get('PAYMENT-RESPONSE');
    if (receipt) console.log(`  ✓ payment settled — receipt: ${receipt.slice(0, 40)}…`);

    return json.data;
}

// ── examples ──────────────────────────────────────────────────────────────────

async function run() {
    const keypair  = loadKeypair();
    const fetchFn  = makeClient(keypair);
    const WSOL     = 'So11111111111111111111111111111111111111112';
    const BONK     = 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263';

    console.log('\n🔑 Wallet:', keypair.publicKey.toBase58());
    console.log('💳 Mode:   x402 pay-per-request (USDC on Solana)\n');

    // 1. Token price — /x402/defi/price
    console.log('1. SOL price:');
    const price = await get(fetchFn, '/defi/price', { address: WSOL });
    console.log(`   $${price.value} (updated ${new Date(price.updateUnixTime * 1000).toISOString()})\n`);

    // 2. Token overview — /x402/defi/token_overview
    console.log('2. BONK overview:');
    const overview = await get(fetchFn, '/defi/token_overview', { address: BONK });
    console.log(`   MC: $${(overview.marketCap / 1e6).toFixed(1)}M`);
    console.log(`   24h vol: $${(overview.v24hUSD / 1e6).toFixed(1)}M`);
    console.log(`   Holders: ${overview.holder?.toLocaleString()}\n`);

    // 3. Token security — /x402/defi/token_security
    console.log('3. BONK security:');
    const sec = await get(fetchFn, '/defi/token_security', { address: BONK });
    console.log(`   Mintable: ${Boolean(sec.isMintable)}`);
    console.log(`   Freezeable: ${Boolean(sec.freezeable)}`);
    console.log(`   Creator %: ${(sec.creatorPercentage * 100).toFixed(2)}%`);
    console.log(`   Top 10 holders: ${(sec.top10HolderPercent * 100).toFixed(1)}%\n`);

    // 4. Trending tokens — /x402/defi/token_trending
    console.log('4. Top 5 trending:');
    const trending = await get(fetchFn, '/defi/token_trending', {
        sort_by: 'rank', sort_type: 'asc', limit: '5',
    });
    const tokens = trending.tokens ?? trending;
    for (const t of tokens) {
        console.log(`   #${t.rank ?? '?'} ${t.symbol} — $${t.price?.toFixed(6) ?? 'N/A'}`);
    }

    console.log('\n✅ Done\n');
}

run().catch(err => {
    console.error('Fatal:', err.message);
    process.exit(1);
});
