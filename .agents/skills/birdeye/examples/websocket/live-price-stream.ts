/**
 * Live Price Stream via WebSocket — Birdeye wss://public-api.birdeye.so/socket
 *
 * Subscribes to real-time 1H OHLCV candle updates for a token.
 * Press Ctrl+C to stop.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/websocket/live-price-stream.ts
 *
 * Available channel types:
 *   SUBSCRIBE_PRICE              — OHLCV candle updates per token
 *   SUBSCRIBE_TXS                — Live swap/trade feed per token
 *   SUBSCRIBE_BASE_QUOTE_PRICE   — Price for a specific trading pair
 *   SUBSCRIBE_TOKEN_NEW_LISTING  — New token listings
 *   SUBSCRIBE_NEW_PAIR           — New liquidity pair creation
 *   SUBSCRIBE_LARGE_TRADE_TXS    — Whale alert — large trades
 *   SUBSCRIBE_WALLET_TXS         — Live tx feed for a monitored wallet
 *   SUBSCRIBE_TOKEN_STATS        — Periodic token stats updates
 *   SUBSCRIBE_MEME               — Live meme token discovery feed
 */

import WebSocket from 'ws';

const WSOL = 'So11111111111111111111111111111111111111112';

async function run() {
    const apiKey = process.env.BIRDEYE_API_KEY;
    if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

    const address = process.env.TOKEN_ADDRESS || WSOL;
    const chain   = process.env.BIRDEYE_CHAIN || 'solana';

    // CRITICAL: API key goes in the WS URL query string — not as a header.
    // Birdeye requires the chain path segment and `echo-protocol` subprotocol.
    const wsUrl = `wss://public-api.birdeye.so/socket/${chain}?x-api-key=${encodeURIComponent(apiKey)}`;
    const ws = new WebSocket(wsUrl, 'echo-protocol', {
        headers: { Origin: 'ws://public-api.birdeye.so' },
    });

    ws.on('open', () => {
        console.log(`✅ Connected. Subscribing to 1H price stream for ${address}...`);
        ws.send(JSON.stringify({
            type: 'SUBSCRIBE_PRICE',
            data: {
                address,
                currency: 'usd',
                type: '1H'     // candle type: 1s | 15s | 30s | 1m | 5m | 15m | 1H | 4H | 1D etc.
            }
        }));
    });

    ws.on('message', (raw) => {
        try {
            const msg = JSON.parse(raw.toString());
            const { type, data } = msg;
            if (type === 'PRICE_DATA' && data) {
                const t = new Date(data.unixTime * 1000).toISOString();
                console.log(`[${t}] O:${data.o?.toFixed(4)} H:${data.h?.toFixed(4)} L:${data.l?.toFixed(4)} C:${data.c?.toFixed(4)} V:$${data.v?.toFixed(0)}`);
            } else if (type === 'SUBSCRIBED') {
                console.log(`Subscription confirmed: ${JSON.stringify(data)}`);
            }
        } catch { /* non-JSON frame — ignore */ }
    });

    ws.on('error', (err) => console.error('[WS Error]', err.message));
    ws.on('close', (code) => console.log(`\nConnection closed — code ${code}`));

    // Graceful shutdown on Ctrl+C. Unsubscribe has no `data` wrapper.
    process.on('SIGINT', () => {
        console.log('\nUnsubscribing...');
        ws.send(JSON.stringify({ type: 'UNSUBSCRIBE_PRICE' }));
        ws.close();
        process.exit(0);
    });
}

run();
