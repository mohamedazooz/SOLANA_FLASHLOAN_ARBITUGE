'use strict';

/*
 * run_mainnet_whirlpool_differential.js — official-SDK quote generation over a
 * freshly captured Mainnet Whirlpool snapshot (READ-ONLY).
 *
 * Mirrors run_mainnet_snapshot_differential.js (DLMM) invariants:
 *   - runs captureSnapshot() in-process, freezes the RPC cache, then drives
 *     the official @orca-so/whirlpools-core math over the captured generation;
 *   - nothing is written unless the ENTIRE run succeeds;
 *   - every recorded number comes from the SDK, not from our own arithmetic.
 *
 * Oracle: `swapQuoteByInputToken` / `swapQuoteByOutputToken` from
 * @orca-so/whirlpools-core — the same Rust math library the deployed Whirlpool
 * program logic derives from, compiled to WASM and pinned to the harness by
 * the package.json version. Oracle account is null (classic whirlpools carry
 * no adaptive-fee oracle; the capture refuses pools whose layout says
 * otherwise).
 *
 * Timestamp: `reward_last_updated_timestamp` from the captured pool (only used
 * by adaptive-fee paths, which cannot trigger with oracle=null; passed for
 * determinism).
 *
 * Fee semantics: Whirlpool charges the full trade fee on the input token
 * (LP share + protocol share combined at quote time), so the Rust engine's
 * `fee_paid` (gross trading fee) is compared against the SDK's `tradeFee`
 * directly. The protocol split (`protocolFeeRate`) accrues later on chain and
 * is NOT part of the swap math — asserted by the SDK's own behavior.
 *
 * Coverage per direction (spec a_to_b / b_to_a):
 *   - exact-in  / small — quote stays inside the current tick ARRAY
 *   - exact-in  / deep  — largest input fully servable by the captured
 *     arrays (crosses many ticks; possibly several arrays)
 *   - exact-out small/deep — `swapQuoteByOutputToken` (recorded for parity
 *     of the direction math; the production Rust engine is exact-in only, so
 *     these are fixture-side provenance and are asserted only on SDK-internal
 *     consistency: relaying the returned `tokenIn` through exact-in must
 *     reproduce `desiredOut` exactly).
 *
 * Comparison policy with the Rust engine: exact-in cases are compared
 * field-for-field (amount_in_consumed == tokenIn, amount_out_received ==
 * tokenEstOut, fee_paid == tradeFee). Exact-out SDK cases are recorded but
 * the Rust test asserts ONLY that the SDK's own relay consistency holds
 * (checked here, in-process, before anything is written).
 */

const fs = require('fs');
const path = require('path');
const { PublicKey } = require('@solana/web3.js');
const core = require('@orca-so/whirlpools-core');
const {
  captureSnapshot,
  defaultSnapshotPath,
  poolSlug,
  sdkCoreVersion,
} = require('./capture_mainnet_whirlpool.js');

function defaultQuotesPath(slug) {
  return (
    process.env.QUOTES_OUT ||
    path.join(
      process.cwd(), '..', '..',
      'fixtures', 'whirlpool_mainnet_pools', `${slug}.quotes.json`
    )
  );
}

const SLIPPAGE_BPS = 100; // only affects tokenMinOut, never compared
const MAX_ITERATIONS = 48;

function whirlpoolFacade(pair) {
  // Real seed bytes from the captured account. whirlpools-core derives
  // feeTierIndex = seed[0] + seed[1] * 256 and applies the adaptive-fee path
  // when it differs from tickSpacing; classic pools carry seed == tickSpacing
  // LE (enforced fail-closed by the capture).
  const seed = Buffer.from(pair.feeTierIndexSeedHex, 'hex');
  if (seed.length !== 2) {
    throw new Error(`invalid fee_tier_index_seed length ${seed.length}`);
  }
  return {
    feeTierIndexSeed: new Uint8Array([seed[0], seed[1]]),
    tickSpacing: pair.tickSpacing,
    feeRate: pair.feeRate,
    protocolFeeRate: pair.protocolFeeRate,
    liquidity: BigInt(pair.liquidity),
    sqrtPrice: BigInt(pair.sqrtPrice),
    tickCurrentIndex: pair.tickCurrentIndex,
    feeGrowthGlobalA: 0n,
    feeGrowthGlobalB: 0n,
    rewardLastUpdatedTimestamp: BigInt(pair.rewardLastUpdatedTimestamp),
    rewardInfos: [
      { emissionsPerSecondX64: 0n, growthGlobalX64: 0n },
      { emissionsPerSecondX64: 0n, growthGlobalX64: 0n },
      { emissionsPerSecondX64: 0n, growthGlobalX64: 0n },
    ],
  };
}

function tickArrayFacades(arrays) {
  return arrays
    .filter((a) => a.exists)
    .map((a) => ({
      startTickIndex: a.start_tick_index,
      ticks: a.ticks.map((t) => ({
        initialized: t.initialized,
        liquidityNet: BigInt(t.liquidity_net),
        liquidityGross: BigInt(t.liquidity_gross),
        feeGrowthOutsideA: 0n,
        feeGrowthOutsideB: 0n,
        rewardGrowthsOutside: [0n, 0n, 0n],
      })),
    }));
}

function quoteExactIn(facade, arrays, amountIn, specifiedTokenA, timestamp) {
  const q = core.swapQuoteByInputToken(
    BigInt(amountIn),
    specifiedTokenA,
    SLIPPAGE_BPS,
    facade,
    null,
    arrays,
    timestamp
  );
  return { tokenIn: q.tokenIn, tokenEstOut: q.tokenEstOut, tradeFee: q.tradeFee };
}

function quoteExactOut(facade, arrays, desiredOut, specifiedTokenA, timestamp) {
  // VERIFIED (rust-sdk/core/src/quote/swap.rs, orca-so/whirlpools main):
  // for swapQuoteByInputToken the flag means "input token is A", but for
  // swapQuoteByOutputToken it means "OUTPUT token is A" — the flag is the
  // output side, hence inverted here.
  const q = core.swapQuoteByOutputToken(
    BigInt(desiredOut),
    !specifiedTokenA,
    SLIPPAGE_BPS,
    facade,
    null,
    arrays,
    timestamp
  );
  return { tokenIn: q.tokenEstIn, tokenOut: q.tokenOut, tradeFee: q.tradeFee };
}

function isInsufficient(err) {
  // Some wasm-boundary errors carry no `.message` at all — fall back to the
  // stringified error object rather than classifying nothing.
  const m = String(err && err.message) !== 'undefined' && err && err.message !== undefined
    ? String(err.message)
    : String(err);
  return (
    m.includes('InsufficientLiquidity') ||
    m.includes('insufficient liquidity') ||
    m.includes('INVALID_TICK_ARRAY_SEQUENCE') ||
    m.includes('Invalid tick array sequence') ||
    m.includes('InvalidTickArraySequence') ||
    m.includes('TickArraySequenceInvalid') ||
    m.includes('liquidity exhausted') ||
    m.includes('AmountExceedsMaxU64') ||
    m.includes('AMOUNT_EXCEEDS_MAX_U64')
  );
}

function tryQuote(fn) {
  try {
    return { ok: true, value: fn() };
  } catch (e) {
    if (isInsufficient(e)) return { ok: false };
    throw e;
  }
}

/** Largest exact-in input fully servable by the captured arrays. */
function maxFeasibleInput(facade, arrays, specifiedTokenA, timestamp) {
  let lo = 0n;
  let hi = 1n;
  while (true) {
    const r = tryQuote(() => quoteExactIn(facade, arrays, hi, specifiedTokenA, timestamp));
    if (!r.ok) break;
    lo = hi;
    hi <<= 8n;
    if (hi > (1n << 70n)) throw new Error('deep probe failed to exceed array coverage');
  }
  if (lo === 0n) throw new Error('arrays cannot serve even the smallest exact-in probe');
  while ((hi - lo) > 1n) {
    const mid = (lo + hi) >> 1n;
    const r = tryQuote(() => quoteExactIn(facade, arrays, mid, specifiedTokenA, timestamp));
    if (r.ok) lo = mid;
    else hi = mid;
  }
  return lo;
}

/** Largest exact-out desired output fully servable by the captured arrays. */
function maxFeasibleOutput(facade, arrays, specifiedTokenA, timestamp) {
  let lo = 0n;
  let hi = 1n;
  while (true) {
    const r = tryQuote(() => quoteExactOut(facade, arrays, hi, specifiedTokenA, timestamp));
    if (!r.ok) break;
    lo = hi;
    hi <<= 8n;
  }
  if (lo === 0n) throw new Error('arrays cannot serve even the smallest exact-out probe');
  while ((hi - lo) > 1n) {
    const mid = (lo + hi) >> 1n;
    const r = tryQuote(() => quoteExactOut(facade, arrays, mid, specifiedTokenA, timestamp));
    if (r.ok) lo = mid;
    else hi = mid;
  }
  return lo;
}

async function main() {
  const rpcUrl = process.env.RPC_URL || 'https://api.mainnet-beta.solana.com';
  const poolAddress = process.env.POOL_ADDRESS || '7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm';
  const arraysPerSide = Math.max(1, Number(process.env.TICK_ARRAYS_PER_SIDE || 3));

  console.log('capturing + quoting (READ-ONLY) whirlpool ' + poolAddress);
  const captured = await captureSnapshot({
    rpcUrl,
    poolAddress,
    tickArraysPerSide: arraysPerSide,
  });
  const { snapshot: pool } = captured;
  const facade = whirlpoolFacade(pool.pair);
  const arrays = tickArrayFacades(pool.tick_arrays);
  const currentArrays = arrays.filter(
    (a) =>
      a.startTickIndex <= pool.pair.tickCurrentIndex &&
      pool.pair.tickCurrentIndex < a.startTickIndex + 88 * pool.pair.tickSpacing
  );
  if (currentArrays.length !== 1) {
    throw new Error(`current tick array resolution ambiguous (${currentArrays.length})`);
  }
  const timestamp = BigInt(pool.pair.rewardLastUpdatedTimestamp);

  const cases = {};
  for (const dir of ['a_to_b', 'b_to_a']) {
    const specifiedTokenA = dir === 'a_to_b';

    let small = 1000n;
    // small: largest input resting inside the current array (full-arrays quote
    // equals current-array-only quote byte-for-byte).
    {
      let lo = 1n;
      let hi = null;
      // grow until crossing detected, then bisect the boundary
      while (true) {
        const full = tryQuote(() => quoteExactIn(facade, arrays, small, specifiedTokenA, timestamp));
        const inner = tryQuote(() =>
          quoteExactIn(facade, currentArrays, small, specifiedTokenA, timestamp)
        );
        if (!full.ok || !inner.ok) {
          hi = small;
          break;
        }
        if (
          full.value.tokenEstOut === inner.value.tokenEstOut &&
          full.value.tradeFee === inner.value.tradeFee
        ) {
          lo = small;
          small <<= 4n;
          continue;
        }
        hi = small;
        break;
      }
      while ((hi - lo) > 1n) {
        const mid = (lo + hi) >> 1n;
        const full = tryQuote(() => quoteExactIn(facade, arrays, mid, specifiedTokenA, timestamp));
        const inner = tryQuote(() =>
          quoteExactIn(facade, currentArrays, mid, specifiedTokenA, timestamp)
        );
        const same =
          full.ok && inner.ok &&
          full.value.tokenEstOut === inner.value.tokenEstOut &&
          full.value.tradeFee === inner.value.tradeFee;
        if (same) lo = mid;
        else hi = mid;
      }
      small = lo;
    }

    const deepIn = maxFeasibleInput(facade, arrays, specifiedTokenA, timestamp);
    const deepOut = maxFeasibleOutput(facade, arrays, specifiedTokenA, timestamp);

    let smallOut = null;
    {
      // small exact-out: 1/1000 of deep out, then verify servo: relaying the
      // returned tokenIn through exact-in reproduces tokenOut exactly.
      let candidate = deepOut / 1000n;
      for (let i = 0; i < MAX_ITERATIONS; i++) {
        const q = tryQuote(() => quoteExactOut(facade, arrays, candidate, specifiedTokenA, timestamp));
        if (q.ok) {
          const back = tryQuote(() =>
            quoteExactIn(facade, arrays, q.value.tokenIn, specifiedTokenA, timestamp)
          );
          if (back.ok && back.value.tokenEstOut === q.value.tokenOut) {
            smallOut = candidate;
            break;
          }
        }
        candidate /= 2n;
        if (candidate <= 1n) throw new Error('small exact-out calibration failed');
      }
    }

    const inSmallQ = quoteExactIn(facade, arrays, small, specifiedTokenA, timestamp);
    const inDeepQ = quoteExactIn(facade, arrays, deepIn, specifiedTokenA, timestamp);
    const outSmallQ = quoteExactOut(facade, arrays, smallOut, specifiedTokenA, timestamp);
    const outDeepQ = quoteExactOut(facade, arrays, deepOut, specifiedTokenA, timestamp);
    for (const q of [outSmallQ, outDeepQ]) {
      // SDK-internal consistency for every recorded exact-out case.
      const back = quoteExactIn(facade, arrays, q.tokenIn, specifiedTokenA, timestamp);
      if (back.tokenEstOut !== q.tokenOut) {
        throw new Error(
          `exact-out relay mismatch: in=${q.tokenIn} out=${q.tokenOut} back=${back.tokenEstOut}`
        );
      }
    }

    cases[dir] = {
      exact_in_small: {
        class: 'in-current-array',
        input: inSmallQ.tokenIn.toString(10),
        amount_in: inSmallQ.tokenIn.toString(10),
        amount_out: inSmallQ.tokenEstOut.toString(10),
        trade_fee: inSmallQ.tradeFee.toString(10),
      },
      exact_in_deep: {
        class: 'max-feasible',
        input: deepIn.toString(10),
        amount_in: inDeepQ.tokenIn.toString(10),
        amount_out: inDeepQ.tokenEstOut.toString(10),
        trade_fee: inDeepQ.tradeFee.toString(10),
      },
      exact_out_small: {
        class: 'in-current-array-provenance',
        desired_out: smallOut.toString(10),
        amount_in: outSmallQ.tokenIn.toString(10),
        amount_out: outSmallQ.tokenOut.toString(10),
        trade_fee: outSmallQ.tradeFee.toString(10),
      },
      exact_out_deep: {
        class: 'max-feasible-provenance',
        desired_out: deepOut.toString(10),
        amount_in: outDeepQ.tokenIn.toString(10),
        amount_out: outDeepQ.tokenOut.toString(10),
        trade_fee: outDeepQ.tradeFee.toString(10),
      },
    };
  }

  const fixture = {
    captured_at_utc: pool.captured_at_utc,
    rpc_slot: pool.rpc_slot,
    pool_address: pool.pool_address,
    program_id: pool.program_id,
    sdk_core_version: sdkCoreVersion(),
    tick_spacing: pool.pair.tickSpacing,
    fee_tier_index_seed: pool.pair.feeTierIndexSeedHex,
    fee_rate: pool.pair.feeRate,
    protocol_fee_rate: pool.pair.protocolFeeRate,
    liquidity: pool.pair.liquidity,
    sqrt_price: pool.pair.sqrtPrice,
    tick_current_index: pool.pair.tickCurrentIndex,
    token_mint_a: pool.pair.tokenMintA,
    token_mint_b: pool.pair.tokenMintB,
    reward_last_updated_timestamp: pool.pair.rewardLastUpdatedTimestamp,
    captured_arrays: pool.tick_arrays.filter((a) => a.exists).length,
    cases,
    tick_math_probes: pool.tick_math_probes,
  };

  const snapPath = defaultSnapshotPath(poolSlug(pool.pair));
  const quotesPath = defaultQuotesPath(poolSlug(pool.pair));
  fs.mkdirSync(path.dirname(snapPath), { recursive: true });
  fs.writeFileSync(snapPath, JSON.stringify(pool, null, 2) + '\n');
  fs.writeFileSync(quotesPath, JSON.stringify(fixture, null, 2) + '\n');

  console.log('wrote ' + snapPath);
  console.log('wrote ' + quotesPath);
  for (const [dir, byKind] of Object.entries(cases)) {
    for (const [kind, c] of Object.entries(byKind)) {
      console.log(
        `  ${dir} ${kind} (${c.class}): in=${c.amount_in} out=${c.amount_out} fee=${c.trade_fee}`
      );
    }
  }
}

if (require.main === module) {
  main().catch((e) => {
    console.error('ERR:', e.message);
    process.exit(1);
  });
}

module.exports = { main };
