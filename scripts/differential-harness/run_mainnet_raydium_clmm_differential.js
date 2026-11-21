'use strict';

/*
 * run_mainnet_raydium_clmm_differential.js — official-SDK quote generation
 * over a freshly captured Mainnet Raydium CLMM snapshot (READ-ONLY).
 *
 * Mirrors run_mainnet_whirlpool_differential.js (Orca) invariants:
 *   - runs captureSnapshot() in-process, freezes the RPC cache, then drives
 *     the official @raydium-io/raydium-sdk-v2 math over the captured
 *     generation;
 *   - nothing is written unless the ENTIRE run succeeds;
 *   - every recorded number comes from the SDK, not from our own arithmetic.
 *
 * Oracle: `PoolUtils.getOutputAmountAndRemainAccounts` over the SDK's own
 * `PoolInfoLayout` / `TickArrayLayout` / `TickArrayBitmapExtensionLayout` /
 * `ClmmConfigLayout` decodes — the exact swap-simulator (`swapInternal`)
 * deriving from the deployed Raydium CLMM program semantics. Determinism is
 * pinned by `block_timestamp` = getBlockTime(captured slot), which is the
 * injectable timestamp the dynamic-fee volatility update consumes.
 *
 * Non-deterministic inputs refused at capture time (fail-closed):
 *   - dynamic-fee pools active — fee would drift mid-swap;
 *   - Token-2022 mints — outside the canonical-engine domain.
 *
 * Fee semantics: with `fee_on = 0` (FromInput) the fee is charged on the
 * input token, so the Rust engine's `fee_paid` is compared against the SDK's
 * `feeAmount` (lp + fund + protocol fee combined at quote time) directly.
 * `fee_on = 1/2` charges the fee on the OUTPUT token for one direction; the
 * SDK quotes both directions but the Rust engine refuses the fee-on-output
 * direction (fail-closed), so those cases are recorded as SDK provenance for
 * the refusal regression, not as match targets.
 *
 * Direction servability: a direction that cannot fully trade even the small
 * probe (one-sided/degenerate liquidity, or the walk leaves the captured set)
 * is skipped and recorded under `refusals` — it is never a parity target.
 */

const fs = require('fs');
const path = require('path');
const { PublicKey } = require('@solana/web3.js');
const sdk = require('@raydium-io/raydium-sdk-v2');
const { BN } = require('bn.js');
const {
  captureSnapshot,
  poolSlug,
  sdkVersion,
} = require('./capture_mainnet_raydium_clmm.js');

function defaultQuotesPath(slug) {
  return (
    process.env.QUOTES_OUT ||
    path.join(
      process.cwd(), '..', '..',
      'fixtures', 'raydium_clmm_mainnet_pools', `${slug}.quotes.json`
    )
  );
}

function bn(v) {
  return new BN(v.toString());
}

const SMALL_PROBE = 1_000_000n; // 1e6 raw units
const MAX_U64 = (1n << 64n) - 1n; // SDK inputs are u64; above this it throws MaxTokenOverflow

function buildOracleInputs(snapshot) {
  const poolData = Buffer.from(snapshot.data_base64, 'base64');
  const poolDecoded = sdk.PoolInfoLayout.decode(poolData);

  const ammData = Buffer.from(snapshot.amm_config.data_base64, 'base64');
  const ammConfig = sdk.ClmmConfigLayout.decode(ammData);

  let exBitmapDecoded = undefined;
  if (snapshot.tick_array_bitmap_extension.exists) {
    exBitmapDecoded = sdk.TickArrayBitmapExtensionLayout.decode(
      Buffer.from(snapshot.tick_array_bitmap_extension.data_base64, 'base64')
    );
  }

  const tickArrayCache = {};
  for (const a of snapshot.tick_arrays) {
    if (!a.exists) continue;
    const decoded = sdk.TickArrayLayout.decode(Buffer.from(a.data_base64, 'base64'));
    tickArrayCache[String(a.start_tick_index)] = { ...decoded, address: new PublicKey(a.pda) };
  }
  if (Object.keys(tickArrayCache).length === 0) {
    throw new Error('no captured, existing tick arrays; nothing to quote against');
  }

  const poolPk = new PublicKey(snapshot.pool_address);
  const poolInfo = {
    accInfo: poolDecoded,
    id: poolPk,
    version: 6,
    mintA: {
      address: snapshot.pair.token_mint_0,
      decimals: snapshot.pair.mint_decimals_0,
      programId: sdk.SPL_TOKEN_PROGRAM_ID_V2 ?? new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'),
      extensions: { feeConfig: undefined },
    },
    mintB: {
      address: snapshot.pair.token_mint_1,
      decimals: snapshot.pair.mint_decimals_1,
      programId: sdk.SPL_TOKEN_PROGRAM_ID_V2 ?? new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA'),
      extensions: { feeConfig: undefined },
    },
    vaultA: new PublicKey(snapshot.pair.token_vault_0),
    vaultB: new PublicKey(snapshot.pair.token_vault_1),
    ammConfig,
    observationId: poolDecoded.observationId,
    exBitmapAccount: new PublicKey(snapshot.tick_array_bitmap_extension.pda),
    creator: poolDecoded.creator,
    programId: new PublicKey(snapshot.program_id),
    tickSpacing: poolDecoded.tickSpacing,
    liquidity: poolDecoded.liquidity,
    sqrtPriceX64: poolDecoded.sqrtPriceX64,
    currentPrice: undefined,
    tickCurrent: poolDecoded.tickCurrent,
    feeGrowthGlobalX64A: poolDecoded.feeGrowthGlobalX64A,
    feeGrowthGlobalX64B: poolDecoded.feeGrowthGlobalX64B,
    protocolFeesTokenA: poolDecoded.protocolFeesTokenA,
    protocolFeesTokenB: poolDecoded.protocolFeesTokenB,
    tickArrayBitmap: poolDecoded.tickArrayBitmap,
    startTime: poolDecoded.startTime,
    exBitmapInfo: exBitmapDecoded,
    rewardInfos: poolDecoded.rewardInfos,
  };

  return { poolInfo, ammConfig, exBitmapDecoded, tickArrayCache };
}

function quoteCase(oracle, zeroForOne, amountIn, blockTimestamp) {
  const inMint = new PublicKey(zeroForOne ? oracle.poolInfo.mintA.address : oracle.poolInfo.mintB.address);
  const res = sdk.PoolUtils.getOutputAmountAndRemainAccounts(
    oracle.poolInfo,
    oracle.exBitmapDecoded,
    oracle.tickArrayCache,
    inMint,
    bn(amountIn.toString()),
    blockTimestamp,
    undefined
  );
  return {
    zero_for_one: zeroForOne,
    amount_in: amountIn.toString(),
    all_trade: res.allTrade,
    amount_out: res.expectedAmountOut.toString(),
    fee_amount: res.feeAmount.toString(),
    execution_price_x64: res.executionPrice.toString(),
    remaining_accounts: res.remainingAccounts.map((k) => k.toBase58()),
  };
}

async function main() {
  const rpcUrl = process.env.RPC_URL || 'https://api.mainnet-beta.solana.com';
  const poolAddress = process.env.POOL_ADDRESS;
  if (!poolAddress) throw new Error('POOL_ADDRESS env var required');

  console.log('capturing (READ-ONLY) raydium clmm pool ' + poolAddress);
  const { snapshot } = await captureSnapshot({
    rpcUrl,
    poolAddress,
    tickArraysPerSide: Math.max(1, Number(process.env.TICK_ARRAYS_PER_SIDE || 3)),
  });
  const slug = poolSlug(snapshot.pair);
  console.log(`captured slot ${snapshot.rpc_slot} (ts ${snapshot.block_timestamp}) slug ${slug}`);

  const oracle = buildOracleInputs(snapshot);
  const blockTimestamp = snapshot.block_timestamp;
  const isFeeOnInputFor = (zeroForOne) =>
    snapshot.pair.fee_on === 0 || (snapshot.pair.fee_on === 1 ? zeroForOne : !zeroForOne);

  const cases = [];
  const refusals = [];
  for (const zeroForOne of [true, false]) {
    const dir = zeroForOne ? 'a_to_b' : 'b_to_a';
    let small;
    try {
      small = quoteCase(oracle, zeroForOne, SMALL_PROBE, blockTimestamp);
    } catch (e) {
      refusals.push({ direction: dir, reason: 'sdk_threw', detail: e.message });
      continue;
    }
    if (!small.all_trade) {
      // Fail-closed: this direction cannot fully serve even the smallest probe
      // (one-sided/degenerate liquidity or the walk leaves the captured set);
      // it is recorded as a refusal, not as a parity target.
      refusals.push({ direction: dir, reason: 'small_probe_partial', detail: `out=${small.amount_out} fee=${small.fee_amount}` });
      continue;
    }
    cases.push({ case_id: `${dir}_small`, ...small });

    // Largest fully-servable input (exact-in / deep): geometric ramp + bisection.
    let lo = SMALL_PROBE;
    let hi = SMALL_PROBE * 4n;
    if (hi > MAX_U64) hi = MAX_U64;
    // ramp until non-servable or u64 ceiling, bounded
    for (let i = 0; i < 80 && hi <= MAX_U64; i++) {
      let probe;
      try {
        probe = quoteCase(oracle, zeroForOne, hi, blockTimestamp);
      } catch {
        break;
      }
      if (!probe.all_trade) break;
      lo = hi;
      hi = hi * 4n > MAX_U64 ? MAX_U64 : hi * 4n;
      if (hi === lo) break;
    }
    while (hi - lo > 1n) {
      const mid = lo + (hi - lo) / 2n;
      let traded = false;
      try {
        traded = quoteCase(oracle, zeroForOne, mid, blockTimestamp).all_trade;
      } catch {
        traded = false;
      }
      if (traded) {
        lo = mid;
      } else {
        hi = mid;
      }
    }
    const deep = quoteCase(oracle, zeroForOne, lo, blockTimestamp);
    cases.push({
      case_id: `${zeroForOne ? 'a_to_b' : 'b_to_a'}_deep`,
      deep_probe_candidate: lo.toString(),
      ...deep,
    });

    // SDK-internal determinism gate: identical call reproduces identical bytes
    const rerun = quoteCase(oracle, zeroForOne, lo, blockTimestamp);
    if (JSON.stringify(rerun) !== JSON.stringify(deep)) {
      throw new Error('SDK oracle is non-deterministic within the frozen generation');
    }
  }

  const comparable = cases.filter(
    (c) => isFeeOnInputFor(c.zero_for_one) && c.all_trade
  );
  if (comparable.length === 0) {
    throw new Error(
      `no fully-servable fee-on-input direction for fee_on=${snapshot.pair.fee_on}; refusing a fixture without comparable cases (refusals: ${JSON.stringify(refusals)})`
    );
  }

  const quotes = {
    provenance: {
      kind: 'raydium_clmm_mainnet_sdk_quotes',
      sdk: {
        package: '@raydium-io/raydium-sdk-v2',
        version: sdkVersion(),
        oracle:
          'PoolUtils.getOutputAmountAndRemainAccounts (src/raydium/clmm/libraries/pool.ts over swapSimulator.swapInternal)',
        layouts: 'PoolInfoLayout/TickArrayLayout/TickArrayBitmapExtensionLayout/ClmmConfigLayout (SDK decodes)',
      },
      snapshot: {
        pool_address: snapshot.pool_address,
        slot: snapshot.rpc_slot,
        block_timestamp: snapshot.block_timestamp,
        sdk_version: snapshot.sdk_version,
      },
      captured_at_utc: snapshot.captured_at_utc,
      comparability_notes: [
        'dynamic-fee pools refused at capture; fee amounts derive from AmmConfig.tradeFeeRate alone',
        'fee_on=1/2 fee-on-output directions are recorded as SDK provenance only; the Rust engine refuses them fail-closed',
        'remaining_accounts is the official swap account-dependency manifest (tick arrays + bitmap extension walk order)',
      ],
    },
    pair: {
      tick_spacing: snapshot.pair.tick_spacing,
      fee_on: snapshot.pair.fee_on,
      trade_fee_rate: snapshot.amm_config.trade_fee_rate,
      mint_0: snapshot.pair.token_mint_0,
      mint_1: snapshot.pair.token_mint_1,
      decimals_0: snapshot.pair.mint_decimals_0,
      decimals_1: snapshot.pair.mint_decimals_1,
    },
    cases,
    refusals,
  };

  const out = defaultQuotesPath(slug);
  const snapOut =
    process.env.SNAPSHOT_OUT ||
    path.join(
      process.cwd(), '..', '..',
      'fixtures', 'raydium_clmm_mainnet_pools', `${slug}.snapshot.json`
    );
  fs.mkdirSync(path.dirname(out), { recursive: true });
  // Write BOTH halves of the fixture pair from the SAME captured generation
  // (mirrors the whirlpool harness), so slot provenance can never mismatch.
  fs.writeFileSync(snapOut, JSON.stringify(snapshot, null, 2) + '\n');
  fs.writeFileSync(out, JSON.stringify(quotes, null, 2) + '\n');
  console.log(
    `writes ${cases.length} SDK quote cases (fee_on=${snapshot.pair.fee_on}, ts=${snapshot.pair.tick_spacing}) -> ${out}`
  );
}

if (require.main === module) {
  main().catch((e) => {
    console.error('DIFFERENTIAL RUN FAILED:', e.message);
    process.exit(1);
  });
}
