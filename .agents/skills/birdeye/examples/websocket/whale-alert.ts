/**
 * Whale Alert via WebSocket — Birdeye SUBSCRIBE_LARGE_TRADE_TXS
 *
 * Streams large trades (whale activity) in real time across all tokens.
 * Filters by USD volume threshold. Requires Business tier+.
 *
 * Run: BIRDEYE_API_KEY=xxx npx ts-node examples/websocket/whale-alert.ts
 *
 * Note: SUBSCRIBE_LARGE_TRADE_TXS uses TOP-LEVEL params (no "data" wrapper).
 */

import WebSocket from 'ws';

const MIN_VOLUME_USD = 10_000;  // alert on trades > $10k
const MAX_VOLUME_USD = 10_000_000; // optional upper cap
const CHAIN = 'solana';

interface WhaleTradeEvent {
  tokenAddress: string;
  owner: string;
  side: 'buy' | 'sell';
  volumeUSD: number;
  tokenAmount: number;
  symbol?: string;
  blockUnixTime: number;
  txHash: string;
}

async function run() {
  const apiKey = process.env.BIRDEYE_API_KEY;
  if (!apiKey) { console.error('Set BIRDEYE_API_KEY env var.'); process.exit(1); }

  const wsUrl = `wss://public-api.birdeye.so/socket/${CHAIN}?x-api-key=${apiKey}`;

  function connect(): WebSocket {
    // Pass 'echo-protocol' as the subprotocol argument (not as a raw header).
    const ws = new WebSocket(wsUrl, 'echo-protocol', {
      headers: { Origin: 'ws://public-api.birdeye.so' },
    });

    ws.on('open', () => {
      console.log(`Connected. Watching for trades > $${MIN_VOLUME_USD.toLocaleString()}...`);

      // LARGE_TRADE_TXS: params at TOP LEVEL — no "data" wrapper
      ws.send(JSON.stringify({
        type: 'SUBSCRIBE_LARGE_TRADE_TXS',
        min_volume: MIN_VOLUME_USD,
        max_volume: MAX_VOLUME_USD,
      }));
    });

    ws.on('message', (raw) => {
      try {
        const msg = JSON.parse(raw.toString());

        if (msg.type === 'TXS_LARGE_TRADE_DATA' && msg.data) {
          const trade: WhaleTradeEvent = msg.data;
          const time = new Date(trade.blockUnixTime * 1000).toISOString();
          const side = trade.side === 'buy' ? '🟢 BUY ' : '🔴 SELL';
          console.log(
            `[${time}] ${side} $${trade.volumeUSD.toLocaleString(undefined, { maximumFractionDigits: 0 })} ` +
            `| ${trade.symbol || trade.tokenAddress.slice(0, 8)} ` +
            `| wallet: ${trade.owner.slice(0, 8)}... ` +
            `| tx: ${trade.txHash.slice(0, 12)}...`
          );
        } else if (msg.type === 'SUBSCRIBED') {
          console.log('Subscription confirmed:', JSON.stringify(msg.data));
        }
      } catch { /* non-JSON frame */ }
    });

    ws.on('close', (code) => {
      console.warn(`Connection closed (code ${code}) — reconnecting in 3s...`);
      setTimeout(connect, 3000);
    });

    ws.on('error', (err) => {
      console.error('WS error:', err.message);
      ws.close();
    });

    return ws;
  }

  const ws = connect();

  // Graceful shutdown
  process.on('SIGINT', () => {
    console.log('\nUnsubscribing...');
    ws.send(JSON.stringify({ type: 'UNSUBSCRIBE_LARGE_TRADE_TXS' }));
    ws.close();
    process.exit(0);
  });
}

run();
