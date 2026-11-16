'use strict';

/*
 * capture_mainnet_snapshot.js — READ-ONLY Mainnet Meteora DLMM pool capture.
 *
 * Produces `fixtures/meteora_mainnet_snapshot.json` in the repository root:
 * the raw lbPair account (base64), decoded pool parameters, the derived
 * oracle / bitmap-extension PDAs, and every bin array with liquidity around
 * the active bin (both swap directions), with per-bin amounts as decimal
 * strings.
 *
 * Consistency guarantee: all RPC traffic is served through a caching wrapper.
 * Every account the SDK decodes is recorded exactly once per fetch generation,
 * and after the capture phase the cache is FROZEN — any further account read
 * that was not part of the capture fails closed instead of silently fetching
 * newer state. The quotes harness (run_mainnet_snapshot_differential.js) runs
 * in the same process against the frozen cache, so the official SDK quotes are
 * computed over byte-identical state to what the snapshot records.
 *
 * Determinism: the SDK derives its internal variable-fee reference update from
 * `Date.now()`. This harness pins `Date.now()` to the snapshot's own
 * `vParameters.lastUpdateTimestamp` (elapsed = 0, i.e. no reference update),
 * which is exactly the timestamp the Rust differential test passes to the
 * engine under test. Time-based reference decay is therefore excluded from the
 * comparison by construction on BOTH sides, not hidden by a haircut.
 *
 * Environment:
 *   RPC_URL              — JSON-RPC endpoint (default: api.mainnet-beta.solana.com)
 *   POOL_ADDRESS         — DLMM pool to capture
 *   BIN_ARRAYS_PER_SIDE  — bin arrays to walk per swap direction (default 4)
 *   SNAPSHOT_OUT         — output path override
 *
 * This script never signs, submits, or simulates a transaction. It only reads
 * accounts.
 */

const fs = require('fs');
const path = require('path');
const { Connection, PublicKey } = require('@solana/web3.js');
const DLMM = require('@meteora-ag/dlmm');
const { BN } = require('bn.js');

const DEFAULT_RPC_URL = 'https://api.mainnet-beta.solana.com';
const DEFAULT_POOL_ADDRESS = '5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6';
const BINS_PER_BIN_ARRAY = 70;
const BASIS_POINT_MAX = 10000;

function sdkVersion() {
  const pkgPath = path.join(
    path.dirname(require.resolve('@meteora-ag/dlmm')),
    '..',
    'package.json'
  );
  return JSON.parse(fs.readFileSync(pkgPath, 'utf8')).version;
}

/**
 * Wraps a Connection so every account payload the SDK observes is recorded and,
 * once frozen, served exclusively from the recorded generation. Unknown methods
 * pass through to the underlying connection (they must not carry account state).
 */
function makeCachingConnection(rpcUrl, commitment) {
  const base = new Connection(rpcUrl, commitment);
  const rawAccounts = new Map(); // base58 -> { data_base64, owner, data_len, slot }
  const callCache = new Map(); // cacheKey -> AccountInfo[] | AccountInfo|null
  let frozen = false;

  async function fetchAndRecordMultiple(keys) {
    const res = await base.getMultipleAccountsInfoAndContext(keys);
    const slot = res.context.slot;
    for (let i = 0; i < keys.length; i++) {
      const v = res.value[i];
      if (v) {
        rawAccounts.set(keys[i].toBase58(), {
          data_base64: Buffer.from(v.data).toString('base64'),
          owner: v.owner.toBase58(),
          data_len: v.data.length,
          slot,
        });
      }
    }
    return res.value;
  }

  async function fetchAndRecordSingle(key) {
    const res = await base.getAccountInfoAndContext(key);
    const slot = res.context.slot;
    if (res.value) {
      rawAccounts.set(key.toBase58(), {
        data_base64: Buffer.from(res.value.data).toString('base64'),
        owner: res.value.owner.toBase58(),
        data_len: res.value.data.length,
        slot,
      });
    }
    return res.value;
  }

  const cacheKeyFor = (keys) => keys.map((k) => k.toBase58()).join('|');

  const wrapper = new Proxy(
    {},
    {
      get(_target, prop) {
        if (prop === 'getMultipleAccountsInfo') {
          return async (keys, commitment_) => {
            const cacheKey = 'gmai:' + cacheKeyFor(keys) + '|' + String(commitment_ ?? '');
            if (frozen) {
              if (!callCache.has(cacheKey)) {
                throw new Error(
                  'FROZEN_CACHE_MISS: getMultipleAccountsInfo(' +
                    cacheKeyFor(keys) +
                    ') after snapshot freeze — refusing to mix newer chain state into the captured generation'
                );
              }
              return callCache.get(cacheKey);
            }
            const infos = await fetchAndRecordMultiple(keys);
            callCache.set(cacheKey, infos);
            return infos;
          };
        }
        if (prop === 'getAccountInfo') {
          return async (key, commitment_) => {
            const cacheKey = 'gai:' + key.toBase58() + '|' + String(commitment_ ?? '');
            if (frozen) {
              if (!callCache.has(cacheKey)) {
                throw new Error(
                  'FROZEN_CACHE_MISS: getAccountInfo(' +
                    key.toBase58() +
                    ') after snapshot freeze — refusing to mix newer chain state into the captured generation'
                );
              }
              return callCache.get(cacheKey);
            }
            const info = await fetchAndRecordSingle(key);
            callCache.set(cacheKey, info);
            return info;
          };
        }
        const v = base[prop];
        return typeof v === 'function' ? v.bind(base) : v;
      },
    }
  );

  return {
    connection: wrapper,
    freeze() {
      frozen = true;
    },
    rawAccounts,
  };
}

function bnToDecimalString(v) {
  if (v === undefined || v === null) return '0';
  if (typeof v === 'string') return v;
  if (BN.isBN(v)) return v.toString(10);
  return String(v);
}

function toNum(v) {
  if (v === undefined || v === null) return 0;
  if (BN.isBN(v)) return v.toNumber();
  if (typeof v === 'number') return v;
  const n = Number(v);
  if (!Number.isSafeInteger(n)) {
    throw new Error('value is not a safe integer: ' + String(v));
  }
  return n;
}

function toBase58(v) {
  if (v instanceof PublicKey) return v.toBase58();
  return String(v);
}

function serializeBinArray(ba, slot) {
  const index = toNum(ba.account.index);
  const bins = [];
  for (let i = 0; i < BINS_PER_BIN_ARRAY; i++) {
    const binId = index * BINS_PER_BIN_ARRAY + i;
    const bin = DLMM.getBinFromBinArray(binId, ba.account);
    bins.push({
      binId,
      amountX: bnToDecimalString(bin.amountX),
      amountY: bnToDecimalString(bin.amountY),
      price: bin.price ? bin.price.toString(10) : "0",
      openOrderAmount: bnToDecimalString(bin.openOrderAmount),
      processedOrderRemainingAmount: bnToDecimalString(bin.processedOrderRemainingAmount),
      limitOrderAskSide: Boolean(bin.limitOrderAskSide),
    });
  }
  return {
    index,
    pda: ba.publicKey.toBase58(),
    exists: true,
    slot,
    bins,
  };
}

/**
 * Captures the pool snapshot. Returns { snapshot, pool, caching, binArrays,
 * activeBin, lastUpdateTimestampSec }.
 */
async function captureSnapshot(opts) {
  const rpcUrl = opts.rpcUrl;
  const poolAddress = opts.poolAddress;
  const binArraysPerSide = opts.binArraysPerSide;
  const capturedAtUtc = new Date().toISOString();

  const caching = makeCachingConnection(rpcUrl, 'confirmed');
  const poolPubkey = new PublicKey(poolAddress);
  const pool = await DLMM.create(caching.connection, poolPubkey);

  const programId = pool.program.programId.toBase58();
  if (programId !== DLMM.LBCLMM_PROGRAM_IDS['mainnet-beta']) {
    throw new Error(
      'unexpected DLMM program id ' + programId + ' — refusing to capture'
    );
  }

  // Walk bin arrays with liquidity in both swap directions. swapForY=true
  // consumes bins BELOW the active bin; swapForY=false consumes bins ABOVE.
  const arraysDown = await pool.getBinArrayForSwap(true, binArraysPerSide);
  const arraysUp = await pool.getBinArrayForSwap(false, binArraysPerSide);

  const seen = new Map();
  for (const ba of [...arraysDown, ...arraysUp]) {
    seen.set(ba.publicKey.toBase58(), ba);
  }

  const activeBin = await pool.getActiveBin();

  // Freeze BEFORE any quote is computed: from here the SDK cannot observe any
  // state outside the captured generation.
  caching.freeze();

  const lb = pool.lbPair;
  const pairRaw = caching.rawAccounts.get(poolAddress);
  if (!pairRaw) {
    throw new Error('lbPair raw account was not recorded by the caching layer');
  }

  const lastUpdateTimestampSec = toNum(lb.vParameters.lastUpdateTimestamp);
  const rpcSlot = pairRaw.slot;

  const [oraclePda] = DLMM.deriveOracle(poolPubkey, pool.program.programId);
  const [bitmapPda] = DLMM.deriveBinArrayBitmapExtension(
    poolPubkey,
    pool.program.programId
  );

  const binArrays = [...seen.values()]
    .map((ba) =>
      serializeBinArray(
        ba,
        caching.rawAccounts.get(ba.publicKey.toBase58())?.slot ?? rpcSlot
      )
    )
    .sort((a, b) => a.index - b.index);

  const snapshot = {
    captured_at_utc: capturedAtUtc,
    rpc_slot: rpcSlot,
    pool_address: poolAddress,
    program_id: programId,
    owner: pairRaw.owner,
    data_len: pairRaw.data_len,
    data_base64: pairRaw.data_base64,
    pair: {
      baseFactor: toNum(lb.parameters.baseFactor),
      filterPeriod: toNum(lb.parameters.filterPeriod),
      decayPeriod: toNum(lb.parameters.decayPeriod),
      reductionFactor: toNum(lb.parameters.reductionFactor),
      variableFeeControl: toNum(lb.parameters.variableFeeControl),
      maxVolatilityAccumulator: toNum(lb.parameters.maxVolatilityAccumulator),
      minBinId: toNum(lb.parameters.minBinId),
      maxBinId: toNum(lb.parameters.maxBinId),
      protocolShare: toNum(lb.parameters.protocolShare),
      baseFeePowerFactor: toNum(lb.parameters.baseFeePowerFactor),
      functionType: toNum(lb.parameters.functionType),
      collectFeeMode: toNum(lb.parameters.collectFeeMode),
      volatilityAccumulator: toNum(lb.vParameters.volatilityAccumulator),
      volatilityReference: toNum(lb.vParameters.volatilityReference),
      indexReference: toNum(lb.vParameters.indexReference),
      lastUpdateTimestamp: lastUpdateTimestampSec,
      activeId: toNum(lb.activeId),
      binStep: toNum(lb.binStep),
      status: toNum(lb.status),
      tokenXMint: toBase58(lb.tokenXMint),
      tokenYMint: toBase58(lb.tokenYMint),
      reserveX: toBase58(lb.reserveX),
      reserveY: toBase58(lb.reserveY),
      oracle: toBase58(lb.oracle),
      lastUpdatedAt: toNum(lb.lastUpdatedAt),
    },
    oracle_pda: oraclePda.toBase58(),
    bitmap_extension_pda: bitmapPda.toBase58(),
    bin_arrays: binArrays,
  };

  if (snapshot.owner !== programId) {
    throw new Error(
      'lbPair account owner ' + snapshot.owner + ' != DLMM program ' + programId
    );
  }

  return {
    snapshot,
    pool,
    caching,
    binArrays: [...seen.values()],
    activeBin,
    lastUpdateTimestampSec,
  };
}

function defaultSnapshotPath() {
  return process.env.SNAPSHOT_OUT
    || path.join(__dirname, '..', '..', 'fixtures', 'meteora_mainnet_snapshot.json');
}

async function main() {
  const rpcUrl = process.env.RPC_URL || DEFAULT_RPC_URL;
  const poolAddress = process.env.POOL_ADDRESS || DEFAULT_POOL_ADDRESS;
  const binArraysPerSide = Math.max(
    1,
    Number(process.env.BIN_ARRAYS_PER_SIDE || 8)
  );

  console.log('capturing snapshot (READ-ONLY)');
  console.log('  rpc:      ' + rpcUrl);
  console.log('  pool:     ' + poolAddress);
  console.log('  arrays/side: ' + binArraysPerSide);

  const { snapshot } = await captureSnapshot({
    rpcUrl,
    poolAddress,
    binArraysPerSide,
  });

  const out = defaultSnapshotPath();
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, JSON.stringify(snapshot, null, 2) + '\n');

  const live = snapshot.bin_arrays.filter((a) => a.exists);
  const binIds = live.flatMap((a) => a.bins.map((b) => b.binId));
  console.log('wrote ' + out);
  console.log(
    '  slot=' + snapshot.rpc_slot +
      ' active_id=' + snapshot.pair.activeId +
      ' bin_step=' + snapshot.pair.binStep +
      ' arrays=' + live.length +
      ' bins=' + binIds.length +
      ' range=[' + Math.min(...binIds) + ',' + Math.max(...binIds) + ']'
  );
}

if (require.main === module) {
  main().catch((e) => {
    console.error('ERR:', e.message);
    process.exit(1);
  });
}

module.exports = {
  captureSnapshot,
  makeCachingConnection,
  defaultSnapshotPath,
  sdkVersion,
  BASIS_POINT_MAX,
};
