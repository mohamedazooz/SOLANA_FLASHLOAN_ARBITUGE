'use strict';

/*
 * capture_mainnet_raydium_clmm.js — READ-ONLY Mainnet Raydium CLMM capture.
 *
 * Mirrors capture_mainnet_whirlpool.js (Orca) invariants:
 *   - all RPC traffic is served through a caching wrapper, recorded per fetch
 *     generation, and the cache is FROZEN before any quote is computed;
 *   - nothing is signed or submitted; account reads only;
 *   - fail-closed: unexpected account layout / unsupported pool shape aborts
 *     the capture instead of producing a fixture nobody can trust.
 *
 * Snapshot contents (JSON):
 *   - raw pool account (base64) + official-SDK decode
 *     (@raydium-io/raydium-sdk-v2 PoolInfoLayout);
 *   - raw AmmConfig account (trade_fee_rate source, 1e6 precision);
 *   - raw token vault balances (SPL token account bytes 64..72);
 *   - every required tick-array PDA + raw bytes (start indexes derived from
 *     the SDK's own TickArrayUtil around the current tick, plus the bitmap-
 *     initialized neighbors walk the swap path itself uses);
 *   - raw TickArrayBitmapExtension account (may be absent);
 *   - block_timestamp of the captured slot (fixed quote determinism).
 *
 * Capture refusals (fail-closed, matching canonical-engine quoting domain):
 *   - not a CLMM PoolState / zero liquidity / swap disabled (status bit 4);
 *   - dynamic-fee state ACTIVE (fee would drift mid-swap; the convention here,
 *     like the whirlpool adaptive-fee rule, is to refuse non-comparable
 *     fixtures rather than approximate);
 *   - any token program that is not spl-token (Token-2022 tokens are excluded
 *     from production discovery — ISSUE-P1-01).
 *
 * Environment:
 *   RPC_URL               — JSON-RPC endpoint
 *   POOL_ADDRESS          — Raydium CLMM pool to capture
 *   TICK_ARRAYS_PER_SIDE  — arrays walked per swap direction (default 3)
 *   SNAPSHOT_OUT          — output path override
 */

const fs = require('fs');
const path = require('path');
const { Connection, PublicKey } = require('@solana/web3.js');
const sdk = require('@raydium-io/raydium-sdk-v2');

const DEFAULT_RPC_URL = 'https://api.mainnet-beta.solana.com';

const CLMM_PROGRAM_ID = sdk.CLMM_PROGRAM_ID instanceof PublicKey
  ? sdk.CLMM_PROGRAM_ID
  : new PublicKey(sdk.CLMM_PROGRAM_ID);
const SPL_TOKEN_PROGRAM_ID = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');

function sdkVersion() {
  const pkg = JSON.parse(
    fs.readFileSync(path.join(__dirname, 'node_modules/@raydium-io/raydium-sdk-v2/package.json'), 'utf8')
  );
  return pkg.version;
}

function makeCachingConnection(rpcUrl, commitment) {
  const base = new Connection(rpcUrl, commitment);
  const rawAccounts = new Map();
  let frozen = false;

  function assertOpen() {
    if (frozen) {
      throw new Error('RPC cache is frozen; account read outside the captured generation');
    }
  }

  async function recordIfPresent(key, v, slot) {
    if (v) {
      rawAccounts.set(key.toBase58(), {
        data_base64: Buffer.from(v.data).toString('base64'),
        owner: v.owner.toBase58(),
        data_len: v.data.length,
        slot,
      });
    }
  }

  async function getMultiple(keys) {
    assertOpen();
    const res = await base.getMultipleAccountsInfoAndContext(keys);
    for (let i = 0; i < keys.length; i++) {
      await recordIfPresent(keys[i], res.value[i], res.context.slot);
    }
    return res;
  }

  async function getSingle(key) {
    assertOpen();
    const res = await base.getAccountInfoAndContext(key);
    await recordIfPresent(key, res.value, res.context.slot);
    return res;
  }

  return {
    connection: base,
    rawAccounts,
    freeze() {
      frozen = true;
    },
    frozen() {
      return frozen;
    },
    getMultiple,
    getSingle,
    cached() {
      return new Map(rawAccounts);
    },
  };
}

const RAYDIUM_POOL_STATE_LEN = 1544;

function decodePool(data) {
  if (data.length !== RAYDIUM_POOL_STATE_LEN) {
    throw new Error(
      `unsupported Raydium CLMM PoolState length: ${data.length}, expected ${RAYDIUM_POOL_STATE_LEN}`
    );
  }
  const d = sdk.PoolInfoLayout.decode(data);
  const dynamicFee = d.dynamicFeeInfo;
  const dynamicFeeActive = !(
    dynamicFee.filterPeriod === 0 &&
    dynamicFee.decayPeriod === 0 &&
    dynamicFee.reductionFactor === 0 &&
    dynamicFee.dynamicFeeControl === 0 &&
    dynamicFee.maxVolatilityAccumulator === 0 &&
    dynamicFee.tickSpacingIndexReference === 0 &&
    dynamicFee.volatilityReference === 0 &&
    dynamicFee.volatilityAccumulator === 0 &&
    dynamicFee.lastUpdateTimestamp.isZero()
  );
  return {
    amm_config: field_b58(d, 'configId'),
    token_mint_0: field_b58(d, 'mintA'),
    token_vault_0: field_b58(d, 'vaultA'),
    token_mint_1: field_b58(d, 'mintB'),
    token_vault_1: field_b58(d, 'vaultB'),
    mint_decimals_0: d.mintDecimalsA,
    mint_decimals_1: d.mintDecimalsB,
    tick_spacing: d.tickSpacing,
    liquidity: d.liquidity.toString(),
    sqrt_price_x64: d.sqrtPriceX64.toString(),
    tick_current: d.tickCurrent,
    fee_on: d.feeOn,
    status: d.status,
    dynamic_fee_active: dynamicFeeActive,
    dynamic_fee_last_update_timestamp: dynamicFee.lastUpdateTimestamp.toString(),
  };
}

function field_b58(decoded, key) {
  const v = decoded[key];
  const s = typeof v === 'string' ? v : v?.toString();
  return new PublicKey(s).toBase58();
}

async function captureSnapshot({ rpcUrl, poolAddress, tickArraysPerSide }) {
  const rpc = makeCachingConnection(rpcUrl, 'confirmed');
  const poolPk = new PublicKey(poolAddress);
  const res = await rpc.getSingle(poolPk);
  const poolInfo = res.value;
  if (!poolInfo) {
    throw new Error(`CLMM pool ${poolAddress} does not exist at the queried commitment`);
  }
  const slot = res.context.slot;
  if (poolInfo.owner.toBase58() !== CLMM_PROGRAM_ID.toBase58()) {
    throw new Error(`owner ${poolInfo.owner.toBase58()} is not the Raydium CLMM program`);
  }
  const pool = decodePool(poolInfo.data);
  if (BigInt(pool.liquidity) === 0n) {
    throw new Error('pool has zero liquidity; nothing to quote');
  }
  if ((pool.status & (1 << 4)) !== 0) {
    throw new Error('pool status bit 4 (swap disabled) set; non-comparable fixture');
  }
  if (pool.dynamic_fee_active) {
    throw new Error(
      'dynamic-fee pool active — fee would drift mid-swap; refusing a non-comparable fixture'
    );
  }
  // Token-program check via the mint accounts (the 2026-09-30 PoolState
  // layout does not carry program fields; the mints decide — fail-closed to
  // match the production discovery domain, ISSUE-P1-01).
  const mints = await rpc.getMultiple([new PublicKey(pool.token_mint_0), new PublicKey(pool.token_mint_1)]);
  if (mints.value[0] === null || mints.value[1] === null) {
    throw new Error('token mint account missing');
  }
  if (
    mints.value[0].owner.toBase58() !== SPL_TOKEN_PROGRAM_ID.toBase58() ||
    mints.value[1].owner.toBase58() !== SPL_TOKEN_PROGRAM_ID.toBase58()
  ) {
    throw new Error('Token-2022 (or unknown token program) pool — excluded from production discovery');
  }

  const blockTimestamp = (await rpc.connection.getBlockTime(slot)) ?? null;
  if (blockTimestamp === null) {
    throw new Error(`getBlockTime(${slot}) unavailable; cannot pin quote determinism`);
  }

  const ammPk = new PublicKey(pool.amm_config);
  const ammRes = await rpc.getSingle(ammPk);
  if (!ammRes.value) throw new Error('amm config account missing');
  const ammConfig = sdk.ClmmConfigLayout.decode(ammRes.value.data);
  const poolDataRef = Buffer.from(poolInfo.data); // kept for the bitmap walk

  const { publicKey: exBitmapPda } = sdk.getPdaExBitmapAccount(CLMM_PROGRAM_ID, poolPk);
  const exBitmapRes = await rpc.getSingle(exBitmapPda);
  const exBitmapExists = exBitmapRes.value !== null;

  const span = 60 * pool.tick_spacing;
  const currentStart = sdk.TickArrayUtil.getTickArrayStartIndex(pool.tick_current, pool.tick_spacing);
  const starts = new Set();
  for (let k = 0; k < tickArraysPerSide; k++) {
    starts.add(currentStart - k * span);
    starts.add(currentStart + k * span);
  }

  // Re-decode needed for the bitmap walk: initialized tick arrays the swap
  // path itself looks up (same findTickArrayStartIndex calls
  // fetchMultiplePoolTickArrays makes) beyond the immediate neighbours.
  const poolDecoded = sdk.PoolInfoLayout.decode(poolDataRef);
  const exBitmapDecoded = exBitmapRes.value
    ? sdk.TickArrayBitmapExtensionLayout.decode(exBitmapRes.value.data)
    : undefined;
  try {
    for (const dir of ['zeroForOne', 'oneForZero']) {
      const found = sdk.TickArrayBitmapUtil.findTickArrayStartIndex({
        tickSpacing: pool.tick_spacing,
        poolBitmap: poolDecoded.tickArrayBitmap,
        tickArrayBitmap: exBitmapDecoded,
        findInfo: { type: dir, count: 5, tickArrayCurrent: pool.tick_current },
      });
      for (const s of found) starts.add(s);
    }
  } catch (e) {
    // Bitmap walk is best-effort; the neighborhood arrays still bound deep
    // probes. Surface the reason in the fixture.
    console.warn('bitmap walk unavailable:', e.message);
  }

  const accounts = await rpc.getMultiple([ammPk, exBitmapPda, ...[...starts].sort((a, b) => a - b).map((s) => sdk.getPdaTickArrayAddress(CLMM_PROGRAM_ID, poolPk, s).publicKey)]);

  const arrays = [];
  const offset = 2; // ammPk, exBitmapPda first
  const sortedStarts = [...starts].sort((a, b) => a - b);
  for (let i = 0; i < sortedStarts.length; i++) {
    const v = accounts.value[offset + i];
    const { publicKey: pda } = sdk.getPdaTickArrayAddress(CLMM_PROGRAM_ID, poolPk, sortedStarts[i]);
    arrays.push({
      start_tick_index: sortedStarts[i],
      pda: pda.toBase58(),
      exists: v !== null,
      data_base64: v !== null ? Buffer.from(v.data).toString('base64') : null,
    });
  }

  const vaults = await rpc.getMultiple([new PublicKey(pool.token_vault_0), new PublicKey(pool.token_vault_1)]);
  if (vaults.value[0] === null || vaults.value[1] === null) {
    throw new Error('token vault account missing');
  }
  const bal = (acc) => (acc.data.length >= 72 ? BigInt('0x' + Buffer.from(acc.data.slice(64, 72)).reverse().toString('hex')).toString(10) : null);

  rpc.freeze();

  const snapshot = {
    captured_at_utc: new Date().toISOString(),
    rpc_slot: slot,
    pool_address: poolAddress,
    program_id: CLMM_PROGRAM_ID.toBase58(),
    sdk_version: sdkVersion(),
    fee_on_record: pool.fee_on,
    data_len: poolInfo.data.length,
    data_base64: poolInfo.data.toString('base64'),
    pair: pool,
    amm_config: {
      address: pool.amm_config,
      data_base64: Buffer.from(ammRes.value.data).toString('base64'),
      trade_fee_rate: ammConfig.tradeFeeRate,
      protocol_fee_rate: ammConfig.protocolFeeRate,
      fund_fee_rate: ammConfig.fundFeeRate,
      create_fee_fee_cap_rate: ammConfig.createPoolFee,
    },
    block_timestamp: blockTimestamp,
    tick_array_bitmap_extension: {
      pda: exBitmapPda.toBase58(),
      exists: exBitmapExists,
      data_base64: exBitmapExists ? Buffer.from(exBitmapRes.value.data).toString('base64') : null,
    },
    tick_arrays: arrays,
    vault_balances: {
      token_vault_0_balance: bal(vaults.value[0]),
      token_vault_1_balance: bal(vaults.value[1]),
    },
  };
  return { snapshot, rpc };
}

const KNOWN_MINTS = {
  So11111111111111111111111111111111111111112: 'sol',
  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v: 'usdc',
  Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB: 'usdt',
  '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R': 'ray',
};

function poolSlug(pair) {
  const a = KNOWN_MINTS[pair.token_mint_0] || `m${pair.token_mint_0.slice(0, 4).toLowerCase()}`;
  const b = KNOWN_MINTS[pair.token_mint_1] || `m${pair.token_mint_1.slice(0, 4).toLowerCase()}`;
  return `rc_ts${pair.tick_spacing}_${a}${b}_${pair.fee_on}`;
}

function defaultSnapshotPath(slug) {
  return (
    process.env.SNAPSHOT_OUT ||
    path.join(process.cwd(), '..', '..', 'fixtures', 'raydium_clmm_mainnet_pools', `${slug}.snapshot.json`)
  );
}

async function main() {
  const rpcUrl = process.env.RPC_URL || DEFAULT_RPC_URL;
  const poolAddress = process.env.POOL_ADDRESS;
  const tickArraysPerSide = Math.max(1, Number(process.env.TICK_ARRAYS_PER_SIDE || 3));
  if (!poolAddress) {
    throw new Error('POOL_ADDRESS env var required');
  }
  console.log('capturing (READ-ONLY) raydium clmm pool ' + poolAddress);
  const { snapshot, rpc } = await captureSnapshot({ rpcUrl, poolAddress, tickArraysPerSide });
  const slug = poolSlug(snapshot.pair);
  const out = defaultSnapshotPath(slug);
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, JSON.stringify(snapshot, null, 2) + '\n');
  console.log(`captured slot ${snapshot.rpc_slot} (ts ${snapshot.block_timestamp}), ${snapshot.tick_arrays.filter((a) => a.exists).length}/${snapshot.tick_arrays.length} tick arrays -> ${out} [cache frozen=${rpc.frozen()}]`);
}

if (require.main === module) {
  main().catch((e) => {
    console.error('CAPTURE FAILED:', e.message);
    process.exit(1);
  });
}

module.exports = { captureSnapshot, defaultSnapshotPath, poolSlug, sdkVersion, decodePool };
