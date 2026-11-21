'use strict';

/*
 * capture_mainnet_whirlpool.js — READ-ONLY Mainnet Orca Whirlpool capture.
 *
 * Mirrors capture_mainnet_snapshot.js (DLMM) invariants:
 *   - all RPC traffic is served through a caching wrapper, recorded per fetch
 *     generation, and the cache is FROZEN before any quote is computed;
 *   - nothing is signed or submitted; account reads only;
 *   - fail-closed: unexpected account layout aborts the capture instead of
 *     producing a fixture nobody can trust.
 *
 * Snapshot contents (JSON):
 *   - raw whirlpool account (base64) + decoded fields (offsets from the
 *     official whirlpools-sdk 0.22.0 IDL artifacts/whirlpool.json, account
 *     layout = 8-byte anchor discriminator + packed zero-copy struct);
 *   - token vault balances (raw SPL token account bytes, balance 64..72);
 *   - every required tick array PDA + raw bytes + decoded ticks;
 *   - whirlpool tick-math probe values (start-tick derivations) that
 *     whirlpools-core computes itself, for the Rust-side parity test.
 *
 * Environment:
 *   RPC_URL               — JSON-RPC endpoint
 *   POOL_ADDRESS          — Whirlpool to capture
 *   TICK_ARRAYS_PER_SIDE  — arrays walked per swap direction (default 3)
 *   SNAPSHOT_OUT          — output path override
 */

const fs = require('fs');
const path = require('path');
const { Connection, PublicKey } = require('@solana/web3.js');
const core = require('@orca-so/whirlpools-core');

const DEFAULT_RPC_URL = 'https://api.mainnet-beta.solana.com';
const DEFAULT_POOL_ADDRESS = '7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm';

// Whirlpool account layout (account = 8-byte anchor discriminator + struct):
// verified against orca-so/whirlpools-sdk@0.22.0 dist/artifacts/whirlpool.json
// (packed zero-copy struct; borsh and packed agree because all fields are
// fixed-size primitives).
const WHIRLPOOL_STRUCT_SIZE = 645;
const OFF = {
  whirlpools_config: 8,
  bump_seed: 40,
  tick_spacing: 41,
  fee_tier_index_seed: 43,
  fee_rate: 45,
  protocol_fee_rate: 47,
  liquidity: 49,
  sqrt_price: 65,
  tick_current_index: 81,
  protocol_fee_owed_a: 85,
  protocol_fee_owed_b: 93,
  token_mint_a: 101,
  token_vault_a: 133,
  fee_growth_global_a: 165,
  token_mint_b: 181,
  token_vault_b: 213,
  fee_growth_global_b: 245,
  reward_last_updated_timestamp: 261,
};

function sdkCoreVersion() {
  const pkg = JSON.parse(
    fs.readFileSync(require.resolve('@orca-so/whirlpools-core/package.json'), 'utf8')
  );
  return pkg.version;
}

function makeCachingConnection(rpcUrl, commitment) {
  const base = new Connection(rpcUrl, commitment);
  const rawAccounts = new Map();
  let frozen = false;

  function assertOpen() {
    if (frozen) {
      throw new Error(
        'RPC cache is frozen; account read outside the captured generation'
      );
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
    getMultiple,
    getSingle,
    cached() {
      return new Map(rawAccounts);
    },
  };
}

function readU16(d, off) {
  return d.readUInt16LE(off);
}
function readU64(d, off) {
  return BigInt('0x' + Buffer.from(d.slice(off, off + 8)).reverse().toString('hex')).toString(10);
}
function readU128(d, off) {
  return BigInt('0x' + Buffer.from(d.slice(off, off + 16)).reverse().toString('hex')).toString(10);
}
function readI128(d, off) {
  let v = BigInt('0x' + Buffer.from(d.slice(off, off + 16)).reverse().toString('hex'));
  if (v >> 127n) {
    v -= 1n << 128n; // two's complement
  }
  return v.toString(10);
}
function readI32(d, off) {
  return d.readInt32LE(off);
}

function decodeWhirlpool(data) {
  if (data.length !== 8 + WHIRLPOOL_STRUCT_SIZE) {
    throw new Error(
      `unsupported whirlpool account layout: len=${data.length}, expected ${8 + WHIRLPOOL_STRUCT_SIZE} ` +
        '(classic pre-adaptive-fee whirlpool). Refusing to guess offsets.'
    );
  }
  return {
    tick_spacing: readU16(data, OFF.tick_spacing),
    fee_tier_index_seed_hex: Buffer.from(
      data.slice(OFF.fee_tier_index_seed, OFF.fee_tier_index_seed + 2)
    ).toString('hex'),
    fee_rate: readU16(data, OFF.fee_rate),
    protocol_fee_rate: readU16(data, OFF.protocol_fee_rate),
    liquidity: readU128(data, OFF.liquidity),
    sqrt_price: readU128(data, OFF.sqrt_price),
    tick_current_index: readI32(data, OFF.tick_current_index),
    token_mint_a: new PublicKey(data.slice(OFF.token_mint_a, OFF.token_mint_a + 32)).toBase58(),
    token_vault_a: new PublicKey(data.slice(OFF.token_vault_a, OFF.token_vault_a + 32)).toBase58(),
    token_mint_b: new PublicKey(data.slice(OFF.token_mint_b, OFF.token_mint_b + 32)).toBase58(),
    token_vault_b: new PublicKey(data.slice(OFF.token_vault_b, OFF.token_vault_b + 32)).toBase58(),
    reward_last_updated_timestamp: readU64(data, OFF.reward_last_updated_timestamp),
  };
}

function decodeTickArray(data) {
  if (data.length < 12) {
    throw new Error('tick array account too short');
  }
  const startTickIndex = readI32(data, 8);
  const ticks = [];
  for (let i = 0; i < 88; i++) {
    const off = 12 + i * 113;
    const initialized = data[off] === 1;
    const liquidityNet =
      initialized ? readI128(data, off + 1) : '0';
    const liquidityGross =
      initialized ? BigInt('0x' + Buffer.from(data.slice(off + 17, off + 33)).reverse().toString('hex')).toString(10) : '0';
    ticks.push({ initialized, liquidity_net: liquidityNet, liquidity_gross: liquidityGross });
  }
  return { start_tick_index: startTickIndex, ticks };
}

async function captureSnapshot({ rpcUrl, poolAddress, tickArraysPerSide }) {
  const rpc = makeCachingConnection(rpcUrl, 'confirmed');
  const poolPk = new PublicKey(poolAddress);
  const res = await rpc.getSingle(poolPk);
  const poolInfo = res.value;
  if (!poolInfo) {
    throw new Error(`whirlpool ${poolAddress} does not exist at the queried commitment`);
  }
  const slot = res.context.slot;
  if (poolInfo.data.length !== 8 + WHIRLPOOL_STRUCT_SIZE) {
    throw new Error(`whirlpool account length ${poolInfo.data.length} is not the classic layout`);
  }
  const pool = decodeWhirlpool(poolInfo.data);
  if (pool.liquidity === '0') {
    throw new Error('whirlpool has zero liquidity; nothing to quote');
  }
  // Official adaptive-fee rule (orca-so/whirlpools ts-sdk/whirlpool/src/swap.ts):
  // feeTierIndex = seed[0] + seed[1] * 256; the pool is adaptive-fee iff
  // tickSpacing != feeTierIndex. The canonical engine implements classic
  // Whirlpool math only — fail closed on adaptive pools instead of producing
  // a fixture nobody can compare against.
  const seedBuf = Buffer.from(pool.fee_tier_index_seed_hex, 'hex');
  const feeTierIndex = seedBuf[0] + seedBuf[1] * 256;
  if (pool.tick_spacing !== feeTierIndex) {
    throw new Error(
      `adaptive-fee whirlpool (tick_spacing=${pool.tick_spacing}, fee_tier_index=${feeTierIndex}) ` +
        'not supported by the canonical engine — refusing to capture a non-comparable fixture'
    );
  }
  const ts = pool.tick_spacing;

  // Official whirlpools-core tick-array start derivation (offset 0 semantics).
  const currentStart = core.getTickArrayStartTickIndex(pool.tick_current_index, ts);
  const step = 88 * ts;
  const starts = new Set();
  // a_to_b walks decreasing ticks; b_to_a increasing.
  for (let k = 0; k < tickArraysPerSide; k++) {
    starts.add(currentStart - k * step);
    starts.add(currentStart + k * step);
  }

  const PROGRAM_ID = poolInfo.owner;
  const arrays = [];
  for (const start of [...starts].sort((a, b) => a - b)) {
    const [pda] = PublicKey.findProgramAddressSync(
      [Buffer.from('tick_array'), poolPk.toBuffer(), Buffer.from(start.toString())],
      PROGRAM_ID
    );
    arrays.push({ start_tick_index: start, pda });
  }

  const accounts = await rpc.getMultiple(arrays.map((a) => new PublicKey(a.pda)));
  for (let i = 0; i < arrays.length; i++) {
    const v = accounts.value[i];
    arrays[i].exists = v !== null;
    arrays[i].slot = v !== null ? accounts.context.slot : null;
    arrays[i].data_base64 = v !== null ? Buffer.from(v.data).toString('base64') : null;
    arrays[i].ticks = v !== null ? decodeTickArray(v.data).ticks : [];
  }

  // Vault balances (probe-size provenance only, not compared by the test).
  const vaults = await rpc.getMultiple([new PublicKey(pool.token_vault_a), new PublicKey(pool.token_vault_b)]);
  if (vaults.value[0] === null || vaults.value[1] === null) {
    throw new Error('token vault account missing');
  }
  const bal = (acc) => acc.data.length >= 72 ? readU64(acc.data, 64) : null;
  const vault_balances = {
    token_vault_a_balance: bal(vaults.value[0]),
    token_vault_b_balance: bal(vaults.value[1]),
  };

  rpc.freeze();

  const snapshot = {
    captured_at_utc: new Date().toISOString(),
    rpc_slot: slot,
    pool_address: poolAddress,
    program_id: PROGRAM_ID.toBase58(),
    sdk_core_version: sdkCoreVersion(),
    owner: poolInfo.owner.toBase58(),
    data_len: poolInfo.data.length,
    data_base64: poolInfo.data.toString('base64'),
    pair: {
      tickSpacing: pool.tick_spacing,
      feeTierIndexSeedHex: pool.fee_tier_index_seed_hex,
      feeRate: pool.fee_rate,
      protocolFeeRate: pool.protocol_fee_rate,
      liquidity: pool.liquidity,
      sqrtPrice: pool.sqrt_price,
      tickCurrentIndex: pool.tick_current_index,
      tokenMintA: pool.token_mint_a,
      tokenVaultA: pool.token_vault_a,
      tokenMintB: pool.token_mint_b,
      tokenVaultB: pool.token_vault_b,
      rewardLastUpdatedTimestamp: pool.reward_last_updated_timestamp,
    },
    ...vault_balances,
    tick_arrays: arrays,
    tick_math_probes: (() => {
      const out = [];
      const probeTicks = new Set([pool.tick_current_index, pool.tick_current_index - 1, pool.tick_current_index + 1]);
      for (const a of arrays) probeTicks.add(a.start_tick_index);
      for (const t of [...probeTicks].sort((a, b) => a - b)) {
        out.push({
          tick_index: t,
          sdk_start_tick: core.getTickArrayStartTickIndex(t, ts),
          sdk_next_initializable: core.getNextInitializableTickIndex(t, ts),
          sdk_prev_initializable: core.getPrevInitializableTickIndex(t, ts),
        });
      }
      return out;
    })(),
  };
  return { snapshot, rpc };
}

const KNOWN_MINTS = {
  So11111111111111111111111111111111111111112: 'sol',
  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v: 'usdc',
  Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB: 'usdt',
};

function poolSlug(pair) {
  const a = KNOWN_MINTS[pair.tokenMintA] || `m${pair.tokenMintA.slice(0, 4).toLowerCase()}`;
  const b = KNOWN_MINTS[pair.tokenMintB] || `m${pair.tokenMintB.slice(0, 4).toLowerCase()}`;
  return `wp_ts${pair.tickSpacing}_${a}${b}`;
}

function defaultSnapshotPath(slug) {
  return (
    process.env.SNAPSHOT_OUT ||
    path.join(
      process.cwd(), '..', '..',
      'fixtures', 'whirlpool_mainnet_pools', `${slug}.snapshot.json`
    )
  );
}

async function main() {
  const rpcUrl = process.env.RPC_URL || DEFAULT_RPC_URL;
  const poolAddress = process.env.POOL_ADDRESS || DEFAULT_POOL_ADDRESS;
  const tickArraysPerSide = Math.max(1, Number(process.env.TICK_ARRAYS_PER_SIDE || 3));

  console.log('capturing (READ-ONLY) whirlpool ' + poolAddress);
  console.log('  rpc:      ' + rpcUrl);
  console.log('  arrays/side: ' + tickArraysPerSide);
  const { snapshot } = await captureSnapshot({ rpcUrl, poolAddress, tickArraysPerSide });
  const out = defaultSnapshotPath(poolSlug(snapshot.pair));
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, JSON.stringify(snapshot, null, 2) + '\n');
  console.log('wrote ' + out);
  console.log(
    '  slot=' + snapshot.rpc_slot +
      ' tick_spacing=' + snapshot.pair.tickSpacing +
      ' fee_rate=' + snapshot.pair.feeRate +
      ' arrays=' + snapshot.tick_arrays.filter((a) => a.exists).length + '/' + snapshot.tick_arrays.length
  );
}

if (require.main === module) {
  main().catch((e) => {
    console.error('ERR:', e.message);
    process.exit(1);
  });
}

module.exports = { captureSnapshot, defaultSnapshotPath, poolSlug, sdkCoreVersion, decodeWhirlpool, decodeTickArray };
