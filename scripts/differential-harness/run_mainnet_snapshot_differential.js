'use strict';

/*
 * run_mainnet_snapshot_differential.js — official-SDK quote generation over a
 * freshly captured Mainnet snapshot (READ-ONLY).
 *
 * Runs `captureSnapshot()` in-process, freezes the RPC cache, then drives the
 * pinned official `@meteora-ag/dlmm` client over the captured generation and
 * writes `fixtures/meteora_mainnet_sdk_quotes.json`:
 *
 *   - exact-in and exact-out quotes, both directions (x_to_y / y_to_x), at
 *     "small" (<= 1 bin crossed) and "deep" (45..121 bins crossed) sizes.
 *   - Q64.64 price checks at every captured bin-array boundary and around the
 *     active bin, produced by the SDK's own `getQPriceFromId`.
 *
 * Fee semantics: the SDK returns `fee` = LP share and `protocolFee` = protocol
 * share; `total_fee` = fee + protocolFee. The Rust engine's `fee_amount` is the
 * total trading fee, so the fixture records all three.
 *
 * CollectFeeMode coverage: the captured pool's real mode is recorded under
 * `quotes.mode_<mode>_<name>` (mode 0 = InputOnly). The OTHER mode block is
 * produced by overriding `collectFeeMode` on the captured in-memory state only
 * (no second pool is invented); the differential manifest declares such cases
 * SIMULATED_ONLY. If the captured pool's real mode is 1, the blocks swap roles.
 *
 * Determinism: `Date.now()` is pinned to the snapshot's
 * `vParameters.lastUpdateTimestamp` before any quote is computed, so the SDK's
 * internal reference update sees elapsed = 0 — identical to the timestamp the
 * Rust differential test passes to the engine.
 *
 * end_bin_id derivation: `swapQuote` returns `endPrice` =
 * getPriceOfBinByBinId(lastFilledActiveBinId), which is inverted here and
 * verified by recomputing the price (exact Decimal string match required).
 * `swapQuoteExactOut` returns no end price, so its traversal end is derived by
 * replaying the returned `inAmount` through the SDK's exact-in quote and
 * asserting the replay reproduces `outAmount` exactly; a mismatch fails the
 * run instead of being written into the fixture.
 *
 * `bins_crossed` = |end_bin_id - active_id|, matching the Rust engine's
 * definition (one increment per bin advance, monotone traversal).
 *
 * Nothing is written unless the ENTIRE run succeeds; on any failure the
 * previous fixtures remain untouched.
 */

const fs = require('fs');
const path = require('path');
const { PublicKey } = require('@solana/web3.js');
const DLMM = require('@meteora-ag/dlmm');
const { BN } = require('bn.js');

const {
  captureSnapshot,
  defaultSnapshotPath,
  sdkVersion,
} = require('./capture_mainnet_snapshot.js');

const BASIS_POINT_MAX = 10000;
const SLIPPAGE_BPS = new BN(100); // only affects minOutAmount / maxInAmount, not compared fields
const PROBE_INPUT = new BN('1000000');
const DEEP_MIN_CROSSED_IN = Number(process.env.DEEP_MIN_CROSSED_IN || 60);
const DEEP_MIN_CROSSED_OUT = Number(process.env.DEEP_MIN_CROSSED_OUT || 100);
// Deep fallback floor: launch-style OnlyY pools (thin liquidity) may not have
// ten bins of depth; a liquidity-limited "deep" case is still recorded when it
// crosses at least this many bins (kept honest via bins_crossed + notes).
const DEEP_MIN_FALLBACK_CROSSED = Number(process.env.DEEP_MIN_FALLBACK_CROSSED || 10);
const SMALL_MAX_CROSSED = 1;
const MAX_ITERATIONS = 48;

function refusal(direction, kind, reason, detail) {
  return { mode: null, direction, case: kind, reason, detail: detail || null };
}

/** Serves one quote case, turning servability failures into recorded refusals.
 *  `pool` must ALREADY carry the fee mode the case is driven under. */
function serveCase(pool, swapForY, mode, direction, kind, build) {
  try {
    return build();
  } catch (e) {
    const msg = String(e.message || e);
    const servErr =
      msg.includes('INSUFFICIENT_LIQUIDITY') ||
      msg.includes('Insufficient liquidity') ||
      msg.includes('TRACED_INSUFFICIENT_LIQUIDITY') ||
      msg.includes('captured arrays cannot serve even the smallest deep probe') ||
      msg.includes('deepest feasible input crosses only') ||
      msg.includes('could not calibrate a small') ||
      msg.includes('calibration did not converge');
    if (!servErr) throw e;
    console.warn(
      '  refusal: ' + (mode === 0 ? 'mode_0' : 'mode_1') + ' ' + direction + ' ' + kind + ': ' + msg
    );
    lastRefusals.push({
      mode,
      direction,
      case: kind,
      reason: 'insufficient_liquidity',
      detail: msg,
    });
    return null;
  }
}

let lastRefusals = [];

function defaultQuotesPath() {
  return process.env.QUOTES_OUT
    || path.join(__dirname, '..', '..', 'fixtures', 'meteora_mainnet_sdk_quotes.json');
}

function binIdFromEndPrice(endPrice, binStep) {
  const p = Number(endPrice.toString());
  if (!Number.isFinite(p) || p <= 0) {
    throw new Error('endPrice is not a positive finite number: ' + endPrice.toString());
  }
  const s = binStep / BASIS_POINT_MAX;
  const id = Math.round(Math.log(p) / Math.log(1 + s));
  const back = DLMM.getPriceOfBinByBinId(id, binStep).toString();
  if (back !== endPrice.toString()) {
    throw new Error(
      'end_bin_id inversion ambiguous: endPrice=' +
        endPrice.toString() +
        ' candidate=' +
        id +
        ' recomputed=' +
        back
    );
  }
  return id;
}

function endVolatilityAccumulator(pair, endBinId) {
  const delta = Math.abs(pair.indexReference - endBinId);
  return Math.min(
    pair.volatilityReference + delta * BASIS_POINT_MAX,
    pair.maxVolatilityAccumulator
  );
}

/**
 * Replays the SDK's own exact-out traversal using the SDK's exported per-bin
 * primitives, tracking the last bin that consumed output. The totals are then
 * asserted equal to `swapQuoteExactOut`'s results, so the traced end bin is
 * evidenced by agreement on every SDK-computed field, not trusted blindly.
 * Requires plain SPL Token mints (no transfer fees): refused otherwise.
 */
function tracedExactOut(pool, swapForY, outAmount) {
  const lbPair = pool.lbPair;
  const binArrays = binArraysRef.value;
  const binStep = lbPair.binStep;
  const sParameters = lbPair.parameters;
  const supportLimitOrder = DLMM.isSupportLimitOrder(lbPair);
  const feeOnInput = DLMM.getFeeMode(lbPair, swapForY).feeOnInput;
  const vParameterClone = Object.assign({}, lbPair.vParameters);
  let activeId = new BN(lbPair.activeId);
  DLMM.updateReference(activeId.toNumber(), vParameterClone, sParameters, Date.now() / 1e3);
  let outAmountLeft = new BN(outAmount);
  let actualInAmount = new BN(0);
  let feeAmount = new BN(0);
  let protocolFeeAmount = new BN(0);
  let lastFilledActiveBinId = null;
  let guard = 0;
  while (!outAmountLeft.isZero()) {
    if (++guard > 200000) {
      throw new Error('tracedExactOut: traversal did not terminate');
    }
    const binArrayAccountToSwap = DLMM.findNextBinArrayWithLiquidity(
      swapForY,
      activeId,
      lbPair,
      pool.binArrayBitmapExtension ? pool.binArrayBitmapExtension.account : null,
      binArrays
    );
    if (binArrayAccountToSwap == null) {
      throw new Error('TRACED_INSUFFICIENT_LIQUIDITY: exact-out exceeds captured arrays');
    }
    if (DLMM.isBinIdWithinBinArray(activeId, binArrayAccountToSwap.account.index)) {
      const bin = DLMM.getBinFromBinArray(activeId.toNumber(), binArrayAccountToSwap.account);
      const maxOutAmount = DLMM.getBinMaxAmountOut(bin, swapForY, supportLimitOrder);
      if (!maxOutAmount.isZero()) {
        DLMM.updateVolatilityAccumulator(vParameterClone, sParameters, activeId.toNumber());
        const r = DLMM.swapExactOutQuoteAtBin(
          bin,
          binStep,
          sParameters,
          vParameterClone,
          outAmountLeft,
          swapForY,
          supportLimitOrder,
          feeOnInput
        );
        if (!r.amountOut.isZero()) {
          outAmountLeft = outAmountLeft.sub(r.amountOut);
          actualInAmount = actualInAmount.add(r.amountIn);
          feeAmount = feeAmount.add(r.fee);
          protocolFeeAmount = protocolFeeAmount.add(r.protocolFee);
          lastFilledActiveBinId = activeId.toNumber();
        }
      }
    }
    if (!outAmountLeft.isZero()) {
      activeId = swapForY ? activeId.subn(1) : activeId.addn(1);
    }
  }
  if (lastFilledActiveBinId === null) {
    throw new Error('tracedExactOut: no bin consumed output');
  }
  const sdk = pool.swapQuoteExactOut(outAmount, swapForY, SLIPPAGE_BPS, binArrays, 0);
  if (
    !sdk.inAmount.eq(actualInAmount) ||
    !sdk.fee.eq(feeAmount) ||
    !sdk.protocolFee.eq(protocolFeeAmount)
  ) {
    throw new Error(
      'tracedExactOut disagrees with the SDK: in ' +
        actualInAmount.toString(10) + ' vs ' + sdk.inAmount.toString(10) +
        ', fee ' + feeAmount.toString(10) + ' vs ' + sdk.fee.toString(10)
    );
  }
  return {
    inAmount: actualInAmount,
    fee: feeAmount,
    protocolFee: protocolFeeAmount,
    endBinId: lastFilledActiveBinId,
  };
}

function quoteCase(pool, binArrays, swapForY, kind, amount) {
  if (kind === 'exact_in') {
    const q = pool.swapQuote(amount, swapForY, SLIPPAGE_BPS, binArrays, false);
    return {
      requested_in: amount,
      amount_in: q.consumedInAmount,
      amount_out: q.outAmount,
      fee: q.fee,
      protocol_fee: q.protocolFee,
      endPrice: q.endPrice,
    };
  }
  const q = pool.swapQuoteExactOut(amount, swapForY, SLIPPAGE_BPS, binArrays, 0);
  return {
    desired_out: amount,
    amount_in: q.inAmount,
    amount_out: q.outAmount,
    fee: q.fee,
    protocol_fee: q.protocolFee,
    endPrice: null,
  };
}

function finalizeCase(caseRaw, pair, swapForY, kind, traced) {
  const startBinId = pair.activeId;
  let endBinId;
  if (caseRaw.endPrice !== null) {
    endBinId = binIdFromEndPrice(caseRaw.endPrice, pair.binStep);
  } else {
    endBinId = traced.endBinId;
  }
  const crossed = Math.abs(endBinId - startBinId);
  return {
    swap_for_y: swapForY,
    fee_on_input: DLMM.getFeeMode(pair, swapForY).feeOnInput,
    ...(kind === 'exact_in'
      ? { input: caseRaw.requested_in.toString(10) }
      : { desired_out: caseRaw.desired_out.toString(10) }),
    amount_in: caseRaw.amount_in.toString(10),
    amount_out: caseRaw.amount_out.toString(10),
    fee: caseRaw.fee.toString(10),
    protocol_fee: caseRaw.protocol_fee.toString(10),
    total_fee: caseRaw.fee.add(caseRaw.protocol_fee).toString(10),
    end_bin_id: endBinId,
    bins_crossed: crossed,
    volatility_accumulator: endVolatilityAccumulator(pair, endBinId),
  };
}

let binArraysRef = { value: null };

function tryQuote(pool, swapForY, kind, amount) {
  try {
    return quoteCase(pool, binArraysRef.value, swapForY, kind, amount);
  } catch (e) {
    if (
      String(e.message).includes('INSUFFICIENT_LIQUIDITY') ||
      String(e.message).includes('Insufficient liquidity')
    ) {
      return null;
    }
    throw e;
  }
}

function calibrateSmallInput(pool, swapForY, baseOutAmount, smallCap) {
  const probe = pool.swapQuote(
    PROBE_INPUT,
    swapForY,
    SLIPPAGE_BPS,
    binArraysRef.value,
    false
  );
  const rate = probe.outAmount.toNumber() / PROBE_INPUT.toNumber();
  if (!(rate > 0) || !Number.isFinite(rate)) {
    throw new Error('degenerate probe quote rate for calibration');
  }
  const outTarget = baseOutAmount.muln(40).divn(100).toNumber();
  let input = Math.max(1, Math.floor(outTarget / rate));
  for (let i = 0; i < MAX_ITERATIONS; i++) {
    const q = tryQuote(pool, swapForY, 'exact_in', new BN(input));
    if (q !== null) {
      const end = binIdFromEndPrice(q.endPrice, pool.lbPair.binStep);
      if (Math.abs(end - pool.lbPair.activeId) <= smallCap) {
        return new BN(input);
      }
    }
    input = Math.max(1, Math.floor(input / 2));
    if (input === 1) {
      throw new Error('could not calibrate a small input within one bin');
    }
  }
  throw new Error('small-input calibration did not converge');
}

function crossedFor(pool, swapForY, kind, amount) {
  if (kind !== 'exact_in') {
    try {
      const t = tracedExactOut(pool, swapForY, amount);
      return { crossed: Math.abs(t.endBinId - pool.lbPair.activeId) };
    } catch (e) {
      if (String(e.message).includes('TRACED_INSUFFICIENT_LIQUIDITY')) {
        return { insufficient: true };
      }
      throw e;
    }
  }
  const q = tryQuote(pool, swapForY, kind, amount);
  if (q === null) {
    return { insufficient: true };
  }
  const end = binIdFromEndPrice(q.endPrice, pool.lbPair.binStep);
  return { crossed: Math.abs(end - pool.lbPair.activeId) };
}

/**
 * Finds an input whose traversal crosses >= minCrossed bins. Falls back to the
 * deepest feasible input when the captured arrays run out of liquidity first;
 * that fallback must still cross at least DEEP_MIN_FALLBACK_CROSSED bins or
 * the run fails — a "deep" case that is not deep is not recorded.
 */
function calibrateDeepInput(pool, swapForY, kind, minCrossed) {
  let lo = null;
  let hi = null;
  let candidate = new BN(10).pow(new BN(9));
  for (let i = 0; i < MAX_ITERATIONS; i++) {
    const r = crossedFor(pool, swapForY, kind, candidate);
    if (r.insufficient) {
      hi = candidate;
      break;
    }
    if (r.crossed >= minCrossed) {
      return { amount: candidate, crossed: r.crossed, liquidity_limited: false };
    }
    lo = candidate;
    candidate = candidate.muln(4);
  }
  if (hi === null) {
    throw new Error('deep calibration failed to bracket a deep input');
  }
  if (lo === null) {
    throw new Error('captured arrays cannot serve even the smallest deep probe');
  }
  let a = lo;
  let b = hi;
  while (b.sub(a).gtn(1)) {
    const mid = a.add(b).divn(2);
    const r = crossedFor(pool, swapForY, kind, mid);
    if (r.insufficient) {
      b = mid;
    } else {
      a = mid;
    }
  }
  const rA = crossedFor(pool, swapForY, kind, a);
  if (rA.insufficient || rA.crossed < DEEP_MIN_FALLBACK_CROSSED) {
    throw new Error(
      'deepest feasible input crosses only ' +
        (rA.insufficient ? 0 : rA.crossed) +
        ' bins — captured arrays too thin for a deep case'
    );
  }
  return {
    amount: a,
    crossed: rA.crossed,
    liquidity_limited: rA.crossed < minCrossed,
  };
}

/**
 * The active bin can be transiently empty on the output side. Scan in the
 * swap direction for the first bin with output-side liquidity and use it as
 * the calibration base; `distance` relaxes the small-case cap accordingly.
 */
function firstLiquidBinOutAmount(pool, swapForY) {
  const lbPair = pool.lbPair;
  const arrays = binArraysRef.value;
  const dir = swapForY ? -1 : 1;
  const maxScan = 70 * arrays.length;
  for (let d = 0; d < maxScan; d++) {
    const binId = lbPair.activeId + d * dir;
    const ba = arrays.find((a) =>
      DLMM.isBinIdWithinBinArray(new BN(binId), a.account.index)
    );
    if (!ba) break;
    const bin = DLMM.getBinFromBinArray(binId, ba.account);
    const amt = swapForY ? bin.amountY : bin.amountX;
    if (!amt.isZero()) {
      return { binId, amount: amt, distance: d };
    }
  }
  return null;
}

function buildDirectionCases(pool, swapForY, activeBin, mode) {
  const direction = swapForY ? 'x_to_y' : 'y_to_x';
  const activeBinOutAmount = swapForY ? activeBin.yAmount : activeBin.xAmount;
  if (activeBinOutAmount === undefined || activeBinOutAmount === null) {
    throw new Error('active bin carries no output-side amount');
  }
  const liquid = firstLiquidBinOutAmount(pool, swapForY);
  const baseOutAmount = liquid ? liquid.amount : activeBinOutAmount;
  if (baseOutAmount.isZero()) {
    throw new Error(
      'no output-side liquidity found in the captured arrays for ' + direction
    );
  }
  const smallCap = SMALL_MAX_CROSSED + (liquid ? liquid.distance : 0);

  const baseOut = baseOutAmount;
  const smallInput = serveCase(pool, swapForY, mode, direction, 'exact_in_small',
    () => calibrateSmallInput(pool, swapForY, baseOut, smallCap));
  const deepIn = serveCase(pool, swapForY, mode, direction, 'exact_in_deep',
    () => calibrateDeepInput(pool, swapForY, 'exact_in', DEEP_MIN_CROSSED_IN));
  if (deepIn && deepIn.liquidity_limited) {
    console.warn(
      '  note: ' + direction + ' deep exact-in is liquidity-limited at ' +
        deepIn.crossed + ' bins crossed'
    );
  }
  const deepInput = deepIn ? deepIn.amount : null;

  let smallOut = serveCase(
    pool,
    swapForY,
    mode,
    direction,
    'exact_out_small',
    () => {
      const smallDesired = baseOut.muln(40).divn(100);
      let desired = smallDesired;
      for (let i = 0; i < MAX_ITERATIONS; i++) {
        const r = crossedFor(pool, swapForY, 'exact_out', desired);
        if (!r.insufficient && r.crossed <= smallCap) {
          return desired;
        }
        desired = desired.divn(2);
        if (desired.lten(1)) {
          throw new Error('could not calibrate a small exact-out amount');
        }
      }
      throw new Error('small exact-out calibration did not converge');
    }
  );

  const deepOutCal = serveCase(pool, swapForY, mode, direction, 'exact_out_deep',
    () => calibrateDeepInput(pool, swapForY, 'exact_out', DEEP_MIN_CROSSED_OUT));
  if (deepOutCal && deepOutCal.liquidity_limited) {
    console.warn(
      '  note: ' + direction + ' deep exact-out is liquidity-limited at ' +
        deepOutCal.crossed + ' bins crossed'
    );
  }

  const fin = (kind, rawAmount, kindName, useTraced) => {
    if (rawAmount === null || rawAmount === undefined) return null;
    const traced = useTraced ? tracedExactOut(pool, swapForY, rawAmount) : null;
    return finalizeCase(
      quoteCase(pool, binArraysRef.value, swapForY, kindName, rawAmount),
      pool.lbPair,
      swapForY,
      kindName,
      traced
    );
  };

  return {
    exact_in_small: fin(
      'exact_in_small',
      smallInput,
      'exact_in',
      false
    ),
    exact_in_deep: fin(
      'exact_in_deep',
      deepIn ? deepIn.amount : null,
      'exact_in',
      false
    ),
    exact_out_small: fin(
      'exact_out_small',
      smallOut,
      'exact_out',
      true
    ),
    exact_out_deep: fin(
      'exact_out_deep',
      deepOutCal ? deepOutCal.amount : null,
      'exact_out',
      true
    ),
  };
}

function buildPriceChecks(snapshot) {
  const binStep = new BN(snapshot.pair.binStep);
  const ids = new Set();
  for (const arr of snapshot.bin_arrays) {
    if (!arr.exists || arr.bins.length === 0) continue;
    ids.add(arr.bins[0].binId);
    ids.add(arr.bins[arr.bins.length - 1].binId);
  }
  const activeId = snapshot.pair.activeId;
  ids.add(activeId);
  ids.add(activeId - 1);
  ids.add(activeId + 1);
  return [...ids]
    .sort((a, b) => a - b)
    .map((binId) => ({
      bin_id: binId,
      price_q64: DLMM.getQPriceFromId(new BN(binId), binStep).toString(10),
    }));
}

async function main() {
  const rpcUrl = process.env.RPC_URL || 'https://api.mainnet-beta.solana.com';
  const poolAddress = process.env.POOL_ADDRESS || '5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6';
  const binArraysPerSide = Math.max(1, Number(process.env.BIN_ARRAYS_PER_SIDE || 8));

  console.log('capturing + quoting (READ-ONLY) pool ' + poolAddress);
  const captured = await captureSnapshot({ rpcUrl, poolAddress, binArraysPerSide });
  const { snapshot, pool, activeBin } = captured;

  // Fail-closed mint gate, mirroring the Raydium CLMM capture policy: the
  // engine rejects Token-2022 fail-closed (unsupported extensions), so any
  // pool carrying a Token-2022 mint is refused as a whole instead of being
  // quoted against SDK totals that silently fold transfer fees in.
  const SPL_TOKEN = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
  const probe = new (require('@solana/web3.js').Connection)(rpcUrl, 'confirmed');
  const mintInfos = await probe.getMultipleAccountsInfo([
    new PublicKey(snapshot.pair.tokenXMint),
    new PublicKey(snapshot.pair.tokenYMint),
  ]);
  const offOwner = ['tokenXMint', 'tokenYMint']
    .map((label, i) => ({ label, owner: mintInfos[i] && mintInfos[i].owner.toString() }))
    .filter((m) => m.owner !== SPL_TOKEN.toString());
  if (offOwner.length > 0) {
    throw new Error(
      'TOKEN_2022_MINT_REFUSED: pool mints are not all classic SPL Token: ' +
        JSON.stringify(offOwner)
    );
  }

  binArraysRef.value = captured.binArrays;
  lastRefusals = [];

  // Pin the SDK clock to the snapshot's own variable-fee timestamp so the
  // internal reference update is a no-op — the same timestamp the Rust
  // differential test passes to the engine under test.
  const pinnedMs = captured.lastUpdateTimestampSec * 1000;
  const realNow = Date.now.bind(global.Date);
  Date.now = () => pinnedMs;

  try {
    const capturedMode = snapshot.pair.collectFeeMode;
    if (capturedMode !== 0 && capturedMode !== 1) {
      throw new Error('unsupported CollectFeeMode ' + capturedMode + ' — refusing to quote');
    }
    const otherMode = capturedMode === 0 ? 1 : 0;

    const modeBlock = (mode) => {
      const real = pool.lbPair.parameters.collectFeeMode;
      if (mode !== real) {
        pool.lbPair.parameters.collectFeeMode = mode;
      }
      try {
        return {
          x_to_y: buildDirectionCases(pool, true, activeBin, mode),
          y_to_x: buildDirectionCases(pool, false, activeBin, mode),
        };
      } finally {
        pool.lbPair.parameters.collectFeeMode = real;
      }
    };

    const quotes = {};
    quotes['mode_' + capturedMode + (capturedMode === 0 ? '_input_only' : '_only_y')] =
      modeBlock(capturedMode);
    quotes['mode_' + otherMode + (otherMode === 0 ? '_input_only' : '_only_y')] =
      modeBlock(otherMode);

    const capturedRange = snapshot.bin_arrays
      .filter((a) => a.exists)
      .flatMap((a) => a.bins.map((b) => b.binId));
    const capturedBinRange = [Math.min(...capturedRange), Math.max(...capturedRange)];

    const fixture = {
      source_snapshot: 'fixtures/meteora_mainnet_snapshot.json',
      captured_at_utc: snapshot.captured_at_utc,
      rpc_slot: snapshot.rpc_slot,
      pool_address: snapshot.pool_address,
      program_id: snapshot.program_id,
      sdk_version: sdkVersion(),
      active_id: snapshot.pair.activeId,
      bin_step: snapshot.pair.binStep,
      volatility_update_timestamp: String(snapshot.pair.lastUpdateTimestamp),
      captured_bin_range: capturedBinRange,
      price_checks: buildPriceChecks(snapshot),
      parameters: {
        baseFactor: snapshot.pair.baseFactor,
        filterPeriod: snapshot.pair.filterPeriod,
        decayPeriod: snapshot.pair.decayPeriod,
        reductionFactor: snapshot.pair.reductionFactor,
        variableFeeControl: snapshot.pair.variableFeeControl,
        maxVolatilityAccumulator: snapshot.pair.maxVolatilityAccumulator,
        minBinId: snapshot.pair.minBinId,
        maxBinId: snapshot.pair.maxBinId,
        protocolShare: snapshot.pair.protocolShare,
        baseFeePowerFactor: snapshot.pair.baseFeePowerFactor,
        functionType: snapshot.pair.functionType,
        collectFeeMode: snapshot.pair.collectFeeMode,
      },
      variable_parameters: {
        volatilityAccumulator: snapshot.pair.volatilityAccumulator,
        volatilityReference: snapshot.pair.volatilityReference,
        indexReference: snapshot.pair.indexReference,
        lastUpdateTimestamp: String(snapshot.pair.lastUpdateTimestamp),
      },
      quotes,
    };

    fixture.refusals = lastRefusals.slice();

    // Fee-split sanity on every recorded (servable) case before anything is
    // written; refused cases must carry an entry in fixture.refusals.
    const allCases = [
      ['mode_0_input_only', 0, fixture.quotes.mode_0_input_only],
      ['mode_1_only_y', 1, fixture.quotes.mode_1_only_y],
    ];
    let servedCount = 0;
    for (const [modeName, modeNum, block] of allCases) {
      if (!block) continue;
      for (const dir of ['x_to_y', 'y_to_x']) {
        for (const kind of ['exact_in_small', 'exact_in_deep', 'exact_out_small', 'exact_out_deep']) {
          const c = block[dir][kind];
          if (c === null || c === undefined) {
            if (
              !fixture.refusals.some(
                (r) => r.mode === modeNum && r.direction === dir && r.case === kind
              )
            ) {
              throw new Error(
                modeName + '/' + dir + '/' + kind + ': absent without a refusal entry'
              );
            }
            continue;
          }
          servedCount += 1;
          const splitSum = BigInt(c.fee) + BigInt(c.protocol_fee);
          if (splitSum !== BigInt(c.total_fee)) {
            throw new Error(
              modeName + '/' + dir + '/' + kind + ': fee split inconsistent with total_fee'
            );
          }
        }
      }
    }
    if (servedCount === 0) {
      throw new Error('no servable comparable case for pool ' + poolAddress);
    }

    const snapPath = defaultSnapshotPath();
    const quotesPath = defaultQuotesPath();
    fs.mkdirSync(path.dirname(snapPath), { recursive: true });
    fs.mkdirSync(path.dirname(quotesPath), { recursive: true });
    fs.writeFileSync(snapPath, JSON.stringify(snapshot, null, 2) + '\n');
    fs.writeFileSync(quotesPath, JSON.stringify(fixture, null, 2) + '\n');

    console.log('wrote ' + snapPath);
    console.log('wrote ' + quotesPath);
    console.log(
      '  slot=' + fixture.rpc_slot +
        ' active_id=' + fixture.active_id +
        ' sdk=' + fixture.sdk_version +
        ' captured_range=[' + capturedBinRange[0] + ',' + capturedBinRange[1] + ']'
    );
    for (const [modeName, _modeNum, block] of allCases) {
      if (!block) continue;
      for (const dir of ['x_to_y', 'y_to_x']) {
        for (const kind of ['exact_in_small', 'exact_in_deep', 'exact_out_small', 'exact_out_deep']) {
          const c = block[dir][kind];
          if (!c) continue;
          console.log(
            '  ' + modeName + ' ' + dir + ' ' + kind +
              ': in=' + c.amount_in + ' out=' + c.amount_out +
              ' end=' + c.end_bin_id + ' crossed=' + c.bins_crossed
          );
        }
      }
    }
  } finally {
    Date.now = realNow;
  }
}

if (require.main === module) {
  main().catch((e) => {
    console.error('ERR:', e.message);
    process.exit(1);
  });
}

module.exports = { main };
