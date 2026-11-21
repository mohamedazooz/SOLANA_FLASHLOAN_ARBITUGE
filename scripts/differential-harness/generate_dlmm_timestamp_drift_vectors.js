'use strict';

/*
 * generate_dlmm_timestamp_drift_vectors.js — deterministic timestamp-drift
 * vector generator for the Meteora DLMM variable-fee volatility state machine.
 *
 * Closes the parity gap "Variable-fee volatility state depends on
 * last_update_timestamp handling; re-quotes on live snapshots exercise decay
 * paths only in unit tests" (docs/quality/QUOTE_PARITY_REPORT.md §4.3) by
 * generating explicit elapsed-time fixtures whose expected values are computed
 * by the OFFICIAL SDK code paths, not by the engine under test:
 *
 *   - volatility_reference decay state machine: transcribed VERBATIM from
 *     @meteora-ag/dlmm's dist/index.mjs `DLMM.updateReference` /
 *     `DLMM.updateVolatilityAccumulator` statics (the class is not re-exported
 *     by the package index; these exact statics are what the SDK's
 *     getDynamicFee / swapQuoteExactOut / swapQuoteByInputToken paths call);
 *   - fee math: the officially EXPORTED helpers `getBaseFee`, `getVariableFee`,
 *     `getTotalFee`, `computeFee`, `computeFeeFromAmount`, `computeProtocolFee`
 *     (require('@meteora-ag/dlmm')) — no reimplementation;
 *   - decay semantics independently cross-checked against the official spec
 *     https://docs.meteora.ag/core-products/dlmm/formulas (Variable Fee
 *     section decay table, verified 2026-11-21).
 *
 * Determinism: a seeded LCG picks the probe values; every scenario is
 * reproducible from the seed alone. No wall-clock time enters the fixture.
 *
 * Usage: node generate_dlmm_timestamp_drift_vectors.js
 * Output: ../../fixtures/dlmm_timestamp_drift_vectors.json (checked in)
 */

const fs = require('fs');
const path = require('path');
const { BN } = require('@coral-xyz/anchor');
const dlmm = require('@meteora-ag/dlmm');

const SDK_VERSION = (() => {
  const pkg = JSON.parse(
    fs.readFileSync(path.join(__dirname, 'node_modules', '@meteora-ag', 'dlmm', 'package.json'), 'utf8')
  );
  return pkg.version;
})();

/* ------------------------------------------------------------------ *
 * Official SDK code paths (reference oracle)                          *
 * ------------------------------------------------------------------ */

// Transcribed verbatim from @meteora-ag/dlmm@<SDK_VERSION> dist/index.mjs
// `updateVolatilityAccumulator` / `updateReference` statics. Keep in sync with
// the bundle; any drift here invalidates the fixture provenance.
class DlmmOfficialStatics {
  static updateVolatilityAccumulator(vParameter, sParameter, activeId) {
    const deltaId = Math.abs(vParameter.indexReference - activeId);
    const newVolatilityAccumulator =
      vParameter.volatilityReference + deltaId * dlmm.BASIS_POINT_MAX;
    vParameter.volatilityAccumulator = Math.min(
      newVolatilityAccumulator,
      sParameter.maxVolatilityAccumulator
    );
  }

  static updateReference(activeId, vParameter, sParameter, currentTimestamp) {
    const elapsed =
      currentTimestamp - vParameter.lastUpdateTimestamp.toNumber();
    if (elapsed >= sParameter.filterPeriod) {
      vParameter.indexReference = activeId;
      if (elapsed < sParameter.decayPeriod) {
        const decayedVolatilityReference = Math.floor(
          (vParameter.volatilityAccumulator * sParameter.reductionFactor) /
            dlmm.BASIS_POINT_MAX
        );
        vParameter.volatilityReference = decayedVolatilityReference;
      } else {
        vParameter.volatilityReference = 0;
      }
    }
  }
}

/* ------------------------------------------------------------------ *
 * Deterministic RNG                                                   *
 * ------------------------------------------------------------------ */

function makeRng(seed) {
  let state = seed >>> 0;
  const next = () => {
    // Numerical Recipes LCG constants; exact 32-bit integer math.
    state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
    return state / 0x1_0000_0000;
  };
  const intBetweenBig = (minBig, maxBig) => {
    const span = maxBig - minBig + 1n;
    return minBig + BigInt(Math.floor(next() * Number(span)));
  };
  return { next, intBetweenBig };
}

/* ------------------------------------------------------------------ *
 * Scenario evaluation via the official code paths                     *
 * ------------------------------------------------------------------ */

function evaluateScenario(scn, rng) {
  const sParams = {
    baseFactor: scn.parameters.base_factor,
    binStep: scn.parameters.bin_step,
    baseFeePowerFactor: scn.parameters.base_fee_power_factor,
    variableFeeControl: scn.parameters.variable_fee_control,
    protocolShare: scn.parameters.protocol_share,
    reductionFactor: scn.parameters.reduction_factor,
    filterPeriod: scn.parameters.filter_period,
    decayPeriod: scn.parameters.decay_period,
    maxVolatilityAccumulator: scn.parameters.max_volatility_accumulator,
    minBinId: scn.parameters.min_bin_id,
    maxBinId: scn.parameters.max_bin_id,
  };
  const vParams = {
    volatilityAccumulator: scn.initial_v_parameters.volatility_accumulator,
    volatilityReference: scn.initial_v_parameters.volatility_reference,
    indexReference: scn.initial_v_parameters.index_reference,
    lastUpdateTimestamp: new BN(scn.initial_v_parameters.last_update_timestamp),
    isActiveBinReferenced: false,
  };

  DlmmOfficialStatics.updateReference(
    scn.step.active_id,
    vParams,
    sParams,
    scn.step.current_timestamp
  );
  DlmmOfficialStatics.updateVolatilityAccumulator(
    vParams,
    sParams,
    scn.step.active_id
  );

  const bn = (v) => new BN(v);

  // Probe amounts: one RNG-chosen odd value per probe (deterministic stream).
  const probeExcluded = rng.intBetweenBig(1n, 9_999_999_999n);
  const probeIncluded = rng.intBetweenBig(1n, 9_999_999_999n);

  const baseFee = dlmm.getBaseFee(scn.parameters.bin_step, sParams);
  const variableFee = dlmm.getVariableFee(scn.parameters.bin_step, sParams, vParams);
  const totalFee = dlmm.getTotalFee(scn.parameters.bin_step, sParams, vParams);
  const feeForInput = dlmm.computeFee(
    scn.parameters.bin_step,
    sParams,
    vParams,
    bn(probeExcluded)
  );
  const feeFromAmount = dlmm.computeFeeFromAmount(
    scn.parameters.bin_step,
    sParams,
    vParams,
    bn(probeIncluded)
  );
  const protocolFee = dlmm.computeProtocolFee(feeForInput, sParams);

  return {
    expected: {
      index_reference: vParams.indexReference,
      volatility_reference: vParams.volatilityReference,
      volatility_accumulator: vParams.volatilityAccumulator,
      base_fee: baseFee.toString(),
      variable_fee: variableFee.toString(),
      total_fee: totalFee.toString(),
      total_fee_clamped: totalFee.eq(new BN(dlmm.MAX_FEE_RATE)),
      probe_excluded_amount: probeExcluded.toString(),
      probe_fee_for_input: feeForInput.toString(),
      probe_included_amount: probeIncluded.toString(),
      probe_fee_from_amount: feeFromAmount.toString(),
      probe_protocol_fee: protocolFee.toString(),
    },
  };
}

/* ------------------------------------------------------------------ *
 * Scenario fixtures                                                   *
 * ------------------------------------------------------------------ */

function baseParameters(overrides = {}) {
  return Object.assign(
    {
      bin_step: 10,
      base_factor: 100,
      base_fee_power_factor: 0,
      variable_fee_control: 4_000_000,
      protocol_share: 1000,
      filter_period: 30,
      decay_period: 120,
      reduction_factor: 5000,
      max_volatility_accumulator: 150_000,
      min_bin_id: -69875,
      max_bin_id: 69875,
    },
    overrides
  );
}

function baseInitialV(overrides = {}) {
  return Object.assign(
    {
      volatility_accumulator: 50_000,
      volatility_reference: 40_000,
      index_reference: 818,
      last_update_timestamp: 1_000_000,
    },
    overrides
  );
}

const SCENARIOS = [
  {
    name: 'before_filter_period_reference_unchanged',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_029, active_id: 823 },
  },
  {
    name: 'elapsed_equals_filter_period_exactly_decays',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_030, active_id: 818 },
  },
  {
    name: 'inside_decay_window_decays',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_095, active_id: 811 },
  },
  {
    name: 'elapsed_equals_decay_period_exactly_resets',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_120, active_id: 818 },
  },
  {
    name: 'beyond_decay_period_resets_zero',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_600, active_id: 840 },
  },
  {
    name: 'pre_filter_large_price_move_accumulates_from_old_reference',
    parameters: baseParameters({ max_volatility_accumulator: 1_000_000 }),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_010, active_id: 840 },
  },
  {
    name: 'accumulator_clamped_at_max_volatility_accumulator',
    parameters: baseParameters({ max_volatility_accumulator: 150_000 }),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_600, active_id: 900 },
  },
  {
    name: 'reduction_factor_zero_decays_to_zero_within_decay_window',
    parameters: baseParameters({ reduction_factor: 0 }),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_050, active_id: 815 },
  },
  {
    name: 'variable_fee_control_zero_yields_zero_variable_fee',
    parameters: baseParameters({ variable_fee_control: 0 }),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_050, active_id: 830 },
  },
  {
    name: 'total_fee_clamped_at_max_fee_rate',
    parameters: baseParameters({
      bin_step: 200,
      base_factor: 800,
      variable_fee_control: 400_000_000,
      max_volatility_accumulator: 200_000,
    }),
    initial_v_parameters: baseInitialV({
      volatility_accumulator: 120_000,
      volatility_reference: 100_000,
    }),
    step: { current_timestamp: 1_000_050, active_id: 860 },
  },
  {
    name: 'backward_timestamp_is_noop',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 999_000, active_id: 830 },
  },
  {
    name: 'chained_post_swap_state_pre_filter_move',
    parameters: baseParameters(),
    initial_v_parameters: baseInitialV({
      volatility_accumulator: 65_000,
      volatility_reference: 25_000,
      index_reference: 771,
    }),
    step: { current_timestamp: 1_000_040, active_id: 765 },
  },
  {
    name: 'zero_filter_period_immediately_references',
    parameters: baseParameters({ filter_period: 0 }),
    initial_v_parameters: baseInitialV(),
    step: { current_timestamp: 1_000_000, active_id: 818 },
  },
];

/* ------------------------------------------------------------------ *
 * Main                                                                *
 * ------------------------------------------------------------------ */

function main() {
  const rng = makeRng(2026_1121);
  const scenarios = SCENARIOS.map((scn) =>
    Object.assign({}, scn, evaluateScenario(scn, rng))
  );

  const fixture = {
    provenance: {
      kind: 'dlmm_timestamp_drift_vectors',
      description:
        'Variable-fee volatility state-machine parity vectors: elapsed-time decay of volatility_reference, volatility_accumulator rebuild, and downstream fee rates/amounts. Expected values computed by official SDK code paths, not the engine under test.',
      sdk: {
        package: '@meteora-ag/dlmm',
        version: SDK_VERSION,
        decay_statics_source:
          'dist/index.mjs DLMM.updateReference / DLMM.updateVolatilityAccumulator statics (transcribed verbatim in the generator; call sites on the SDK swap path: getDynamicFee, swapQuoteExactOut, swapQuoteByInputToken)',
        fee_helpers_source:
          'officially exported getBaseFee/getVariableFee/getTotalFee/computeFee/computeFeeFromAmount/computeProtocolFee',
        spec_cross_check:
          'https://docs.meteora.ag/core-products/dlmm/formulas (Variable Fee decay table, verified 2026-11-21)',
      },
      engine_under_test:
        'arb_math::dlmm::fee::update_references_with_decay + update_volatility_accumulator + compute_base_fee/compute_variable_fee/compute_total_fee_rate/compute_fee_to_add/compute_fee_from_amount/compute_protocol_fee',
      deterministic_rng: 'LCG (Numerical Recipes constants), seed 20261121',
      parity_policy:
        'docs/quality/QUOTE_PARITY_REPORT.md §1 — unexplained discrepancies are a hard failure',
    },
    scenarios,
  };

  const outPath = path.resolve(
    __dirname,
    '..',
    '..',
    'fixtures',
    'dlmm_timestamp_drift_vectors.json'
  );
  fs.writeFileSync(outPath, JSON.stringify(fixture, null, 2) + '\n');
  console.log(`wrote ${outPath} (${scenarios.length} scenarios)`);
}

main();
