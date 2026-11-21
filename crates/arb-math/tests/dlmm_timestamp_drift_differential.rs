//! Differential verification of the DLMM variable-fee volatility state machine
//! against the official `@meteora-ag/dlmm` SDK (see
//! `scripts/differential-harness/generate_dlmm_timestamp_drift_vectors.js` for
//! oracle provenance).
//!
//! Parity policy (`docs/quality/QUOTE_PARITY_REPORT.md` §1): any unexplained
//! discrepancy in elapsed-time decay semantics, volatility-accumulator
//! reconstruction, or the derived fee-rate/fee-amount family is a hard
//! failure.

use arb_math::dlmm::{
    DlmmParameters, DlmmVariableParameters, compute_base_fee, compute_fee_from_amount,
    compute_fee_to_add, compute_protocol_fee, compute_total_fee_rate, compute_variable_fee,
    update_references_with_decay, update_volatility_accumulator,
};

fn fixture() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/dlmm_timestamp_drift_vectors.json"
    );
    serde_json::from_reader(std::fs::File::open(path).expect("fixture readable"))
        .expect("fixture valid JSON")
}

fn expect_u32(v: &serde_json::Value, key: &str) -> u32 {
    u32::try_from(v[key].as_i64().expect(key)).expect("fits u32")
}

fn expect_u16(v: &serde_json::Value, key: &str) -> u16 {
    u16::try_from(v[key].as_i64().expect(key)).expect("fits u16")
}

fn expect_u128(v: &serde_json::Value, key: &str) -> u128 {
    v[key].as_str().expect(key).parse().expect("decimal u128")
}

fn expect_u64(v: &serde_json::Value, key: &str) -> u64 {
    u64::try_from(expect_u128(v, key)).expect("fits u64")
}

fn build_parameters(params: &serde_json::Value) -> DlmmParameters {
    DlmmParameters {
        bin_step: expect_u16(params, "bin_step"),
        base_factor: expect_u16(params, "base_factor"),
        filter_period: expect_u16(params, "filter_period"),
        decay_period: expect_u16(params, "decay_period"),
        reduction_factor: expect_u16(params, "reduction_factor"),
        variable_fee_control: u32::try_from(params["variable_fee_control"].as_i64().expect("vfc"))
            .expect("fits u32"),
        max_volatility_accumulator: u32::try_from(
            params["max_volatility_accumulator"]
                .as_i64()
                .expect("max_va"),
        )
        .expect("fits u32"),
        protocol_share: expect_u16(params, "protocol_share"),
        base_fee_power_factor: u8::try_from(
            params["base_fee_power_factor"].as_i64().expect("power"),
        )
        .expect("fits u8"),
        collect_fee_mode: 0,
    }
}

fn build_initial_v_params(initial: &serde_json::Value) -> DlmmVariableParameters {
    DlmmVariableParameters {
        volatility_accumulator: expect_u32(initial, "volatility_accumulator"),
        volatility_reference: expect_u32(initial, "volatility_reference"),
        index_reference: initial["index_reference"].as_i64().expect("index_ref") as i32,
        last_update_timestamp: initial["last_update_timestamp"]
            .as_i64()
            .expect("last_update_timestamp"),
    }
}

struct ProbeAmounts {
    excluded: u64,
    included: u64,
}

#[test]
fn dlmm_timestamp_drift_matches_official_sdk_state_machine() {
    let doc = fixture();
    assert_eq!(
        doc["provenance"]["sdk"]["version"].as_str(),
        Some("1.9.14"),
        "fixture oracle drift: regenerate vectors and re-review provenance"
    );
    assert_eq!(
        doc["scenarios"].as_array().expect("scenarios").len(),
        13,
        "fixture drift: scenario set changed; regenerate vectors and re-review oracle provenance"
    );

    for scenario in doc["scenarios"].as_array().expect("scenarios") {
        let name = scenario["name"].as_str().expect("scenario name");
        let params = build_parameters(&scenario["parameters"]);
        let mut v_params = build_initial_v_params(&scenario["initial_v_parameters"]);
        let expected = &scenario["expected"];
        let active_id = scenario["step"]["active_id"].as_i64().expect("active_id") as i32;
        let current_timestamp = scenario["step"]["current_timestamp"]
            .as_i64()
            .expect("current_timestamp");

        update_references_with_decay(&mut v_params, &params, active_id, current_timestamp)
            .unwrap_or_else(|e| panic!("{name}: update_references_with_decay failed: {e:?}"));
        update_volatility_accumulator(&mut v_params, &params, active_id)
            .unwrap_or_else(|e| panic!("{name}: update_volatility_accumulator failed: {e:?}"));

        assert_eq!(
            v_params.index_reference,
            expected["index_reference"].as_i64().expect("index_ref") as i32,
            "{name}: index_reference mismatch"
        );
        assert_eq!(
            v_params.volatility_reference,
            expect_u32(expected, "volatility_reference"),
            "{name}: volatility_reference mismatch"
        );
        assert_eq!(
            v_params.volatility_accumulator,
            expect_u32(expected, "volatility_accumulator"),
            "{name}: volatility_accumulator mismatch"
        );

        let expected_base = expect_u128(expected, "base_fee");
        assert_eq!(
            compute_base_fee(&params, params.bin_step).expect("base fee"),
            expected_base,
            "{name}: base_fee mismatch"
        );
        let expected_variable = expect_u128(expected, "variable_fee");
        assert_eq!(
            compute_variable_fee(&params, params.bin_step, v_params.volatility_accumulator)
                .expect("variable fee"),
            expected_variable,
            "{name}: variable_fee mismatch"
        );
        let expected_total = expect_u128(expected, "total_fee");
        assert_eq!(
            compute_total_fee_rate(&params, params.bin_step, v_params.volatility_accumulator)
                .expect("total fee"),
            expected_total,
            "{name}: total_fee mismatch"
        );
        assert_eq!(
            expected_total == expect_u128(expected, "base_fee") + expected_variable,
            !expected["total_fee_clamped"].as_bool().expect("clamped"),
            "{name}: clamp flag must match base+variable equality"
        );

        let expected_clamp_bound = expected["total_fee_clamped"].as_bool().expect("clamped");
        assert_eq!(
            expected_total == arb_math::dlmm::MAX_FEE_RATE,
            expected_clamp_bound,
            "{name}: MAX_FEE_RATE clamp divergence"
        );

        let probes = ProbeAmounts {
            excluded: expect_u64(expected, "probe_excluded_amount"),
            included: expect_u64(expected, "probe_included_amount"),
        };
        assert_eq!(
            compute_fee_to_add(probes.excluded, expected_total).expect("fee_to_add"),
            expect_u64(expected, "probe_fee_for_input"),
            "{name}: fee_to_add mismatch on probe amount"
        );
        assert_eq!(
            compute_fee_from_amount(probes.included, expected_total).expect("fee_from_amount"),
            expect_u64(expected, "probe_fee_from_amount"),
            "{name}: fee_from_amount mismatch on probe amount"
        );
        assert_eq!(
            compute_protocol_fee(
                expect_u64(expected, "probe_fee_for_input"),
                params.protocol_share
            )
            .expect("protocol fee"),
            expect_u64(expected, "probe_protocol_fee"),
            "{name}: protocol_fee mismatch on probe amount"
        );
    }
}

#[test]
fn dlmm_drift_invariants_never_decrease_reference_below_zero_or_above_max() {
    let doc = fixture();
    for scenario in doc["scenarios"].as_array().expect("scenarios") {
        let params = build_parameters(&scenario["parameters"]);
        let mut v_params = build_initial_v_params(&scenario["initial_v_parameters"]);
        let active_id = scenario["step"]["active_id"].as_i64().expect("active_id") as i32;
        let current_timestamp = scenario["step"]["current_timestamp"]
            .as_i64()
            .expect("current_timestamp");

        update_references_with_decay(&mut v_params, &params, active_id, current_timestamp)
            .expect("decay update");
        update_volatility_accumulator(&mut v_params, &params, active_id).expect("accumulator");

        assert!(
            v_params.volatility_reference <= v_params.volatility_accumulator,
            "{}: volatility_reference must always be <= volatility_accumulator (SDK contract)",
            scenario["name"].as_str().expect("name")
        );
        assert!(
            v_params.volatility_accumulator <= params.max_volatility_accumulator,
            "{}: volatility_accumulator must respect its cap",
            scenario["name"].as_str().expect("name")
        );
    }
}
