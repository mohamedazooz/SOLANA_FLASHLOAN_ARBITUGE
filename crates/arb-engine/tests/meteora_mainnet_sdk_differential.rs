//! Differential: Rust DLMM engine vs the pinned official SDK over a CAPTURED
//! Mainnet snapshot.
//!
//! Inputs
//! ------
//! * `fixtures/meteora_mainnet_snapshot.json` — raw accounts captured from Mainnet
//!   by `scripts/differential-harness/capture_mainnet_snapshot.js`
//!   (pool `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6`, DLMM 0.12.0).
//! * `fixtures/meteora_mainnet_sdk_quotes.json` — quotes produced by the official
//!   `@meteora-ag/dlmm@1.9.14` client over that exact snapshot via
//!   `scripts/differential-harness/run_mainnet_snapshot_differential.js`.
//!
//! Both sides therefore receive byte-identical pool parameters and bin amounts.
//! A difference here is an engine difference, not a data difference.
//!
//! This is a genuine SDK differential test. It is NOT an invariant test: every
//! expected value comes from the SDK, not from our own arithmetic.
//!
//! Coverage per direction (`swap_for_y` in {true,false}) and per fee mode
//! (`CollectFeeMode` 0 = InputOnly, 1 = OnlyY):
//! * exact-in  / small — stays within one bin
//! * exact-in  / deep  — crosses 45–121 bins, spanning bin-array boundaries
//! * exact-out / small — stays within one bin
//! * exact-out / deep  — crosses 45–121 bins
//!
//! Note on fee semantics: the SDK returns `fee` = LP share and `protocolFee` =
//! protocol share, so total trading fee = `fee + protocolFee`. The Rust engine's
//! `fee_amount` is the total trading fee and `protocol_fee` is its protocol part.

use arb_math::dlmm::{
    DlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, bin_array_index_for_bin,
    get_price_from_id, quote_dlmm_exact_out, quote_dlmm_multi_bin,
};
use serde::Deserialize;
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Deserialize)]
struct Params {
    #[serde(rename = "baseFactor")]
    base_factor: u16,
    #[serde(rename = "filterPeriod")]
    filter_period: u16,
    #[serde(rename = "decayPeriod")]
    decay_period: u16,
    #[serde(rename = "reductionFactor")]
    reduction_factor: u16,
    #[serde(rename = "variableFeeControl")]
    variable_fee_control: u32,
    #[serde(rename = "maxVolatilityAccumulator")]
    max_volatility_accumulator: u32,
    #[serde(rename = "protocolShare")]
    protocol_share: u16,
    #[serde(rename = "baseFeePowerFactor")]
    base_fee_power_factor: u8,
    #[serde(rename = "collectFeeMode")]
    collect_fee_mode: u8,
}

#[derive(Debug, Deserialize)]
struct VariableParams {
    #[serde(rename = "volatilityAccumulator")]
    volatility_accumulator: u32,
    #[serde(rename = "volatilityReference")]
    volatility_reference: u32,
    #[serde(rename = "indexReference")]
    index_reference: i32,
    #[serde(rename = "lastUpdateTimestamp")]
    last_update_timestamp: String,
}

#[derive(Debug, Deserialize)]
struct SdkQuote {
    swap_for_y: bool,
    fee_on_input: bool,
    amount_in: String,
    amount_out: String,
    fee: String,
    protocol_fee: String,
    total_fee: String,
    end_bin_id: i32,
    bins_crossed: u32,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    desired_out: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    exact_in_small: SdkQuote,
    exact_in_deep: SdkQuote,
    exact_out_small: SdkQuote,
    exact_out_deep: SdkQuote,
}

#[derive(Debug, Deserialize)]
struct ModeQuotes {
    x_to_y: CaseSet,
    y_to_x: CaseSet,
}

#[derive(Debug, Deserialize)]
struct Quotes {
    mode_0_input_only: ModeQuotes,
    mode_1_only_y: ModeQuotes,
}

#[derive(Debug, Deserialize)]
struct PriceCheck {
    bin_id: i32,
    price_q64: String,
}

#[derive(Debug, Deserialize)]
struct SdkFixture {
    rpc_slot: u64,
    pool_address: String,
    program_id: String,
    sdk_version: String,
    active_id: i32,
    bin_step: u16,
    volatility_update_timestamp: String,
    captured_bin_range: [i32; 2],
    price_checks: Vec<PriceCheck>,
    parameters: Params,
    variable_parameters: VariableParams,
    quotes: Quotes,
}

// --- captured Mainnet bin data ------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawBin {
    #[serde(rename = "binId")]
    bin_id: i32,
    #[serde(rename = "amountX")]
    amount_x: String,
    #[serde(rename = "amountY")]
    amount_y: String,
}

#[derive(Debug, Deserialize)]
struct RawBinArray {
    #[serde(default)]
    index: Option<i64>,
    #[serde(default)]
    pda: Option<String>,
    exists: bool,
    #[serde(default)]
    slot: Option<u64>,
    bins: Vec<RawBin>,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    #[serde(default)]
    rpc_slot: Option<u64>,
    #[serde(default)]
    pool_address: Option<String>,
    #[serde(default)]
    program_id: Option<String>,
    #[serde(default)]
    data_len: Option<usize>,
    #[serde(default)]
    captured_at_utc: Option<String>,
    #[serde(default)]
    oracle_pda: Option<String>,
    #[serde(default)]
    bitmap_extension_pda: Option<String>,
    bin_arrays: Vec<RawBinArray>,
}

fn find_fixture(name: &str) -> String {
    for p in &[
        format!("fixtures/{name}"),
        format!("../../fixtures/{name}"),
        format!("../fixtures/{name}"),
    ] {
        if Path::new(p).exists() {
            return p.clone();
        }
    }
    panic!("{name} not found; run the capture + SDK harness first");
}

fn load() -> SdkFixture {
    let s = fs::read_to_string(find_fixture("meteora_mainnet_sdk_quotes.json"))
        .expect("read SDK quotes fixture");
    serde_json::from_str(&s).expect("parse SDK quotes fixture")
}

fn load_bins() -> Vec<DlmmBin> {
    let s = fs::read_to_string(find_fixture("meteora_mainnet_snapshot.json"))
        .expect("read Mainnet snapshot");
    let snap: Snapshot = serde_json::from_str(&s).expect("parse Mainnet snapshot");
    let mut bins = Vec::new();
    for arr in snap.bin_arrays {
        if !arr.exists {
            continue;
        }
        for b in arr.bins {
            bins.push(DlmmBin {
                id: b.bin_id,
                amount_x: b.amount_x.parse().expect("amount_x"),
                amount_y: b.amount_y.parse().expect("amount_y"),
                // 0 => the engine must derive the price itself.
                price_q64: 0,
            });
        }
    }
    assert!(!bins.is_empty(), "snapshot carried no bins");
    bins.sort_by_key(|b| b.id);
    bins
}

fn pool_for(f: &SdkFixture, collect_fee_mode: u8) -> DlmmPoolState {
    DlmmPoolState {
        active_id: f.active_id,
        bin_step: f.bin_step,
        status: 0,
        parameters: DlmmParameters {
            bin_step: f.bin_step,
            base_factor: f.parameters.base_factor,
            filter_period: f.parameters.filter_period,
            decay_period: f.parameters.decay_period,
            reduction_factor: f.parameters.reduction_factor,
            variable_fee_control: f.parameters.variable_fee_control,
            max_volatility_accumulator: f.parameters.max_volatility_accumulator,
            protocol_share: f.parameters.protocol_share,
            base_fee_power_factor: f.parameters.base_fee_power_factor,
            collect_fee_mode,
        },
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: f.variable_parameters.volatility_accumulator,
            volatility_reference: f.variable_parameters.volatility_reference,
            index_reference: f.variable_parameters.index_reference,
            last_update_timestamp: f
                .variable_parameters
                .last_update_timestamp
                .parse()
                .expect("timestamp"),
        },
    }
}

fn cmp(label: &str, rust: i128, sdk: i128) {
    let diff = rust - sdk;
    println!("{label:<62} rust={rust:<18} sdk={sdk:<18} diff={diff}");
    assert!(
        diff == 0,
        "{label}: rust {rust} vs sdk {sdk} (diff {diff}, tolerance 0)"
    );
}

#[test]
fn mainnet_snapshot_prices_match_official_sdk() {
    let f = load();
    println!(
        "\n=== Mainnet snapshot Q64.64 price differential ===\nslot={} pool={} sdk={}",
        f.rpc_slot, f.pool_address, f.sdk_version
    );
    for p in &f.price_checks {
        let rust = get_price_from_id(p.bin_id, f.bin_step).expect("price");
        let sdk: u128 = p.price_q64.parse().expect("sdk price");
        println!(
            "bin {:<7} rust={:<22} sdk={:<22} diff={}",
            p.bin_id,
            rust,
            sdk,
            rust as i128 - sdk as i128
        );
        assert_eq!(rust, sdk, "price mismatch at bin {}", p.bin_id);
    }
}

/// Rust-side evaluation of one SDK fixture case. Single computation path shared
/// by the assertion test and the manifest emitter, so the artifact can never
/// describe numbers that the assertions did not actually verify.
struct CaseEval {
    fee_on_input_rust: bool,
    fee_on_input_sdk: bool,
    amount_in: u64,
    amount_out: u64,
    total_fee: u64,
    protocol_fee: u64,
    end_bin_id: i32,
    bins_crossed: u32,
    start_bin_id: i32,
}

fn evaluate_case(
    f: &SdkFixture,
    bins: &[DlmmBin],
    label: &str,
    mode: u8,
    q: &SdkQuote,
) -> CaseEval {
    let pool = pool_for(f, mode);
    let ts = Some(
        f.volatility_update_timestamp
            .parse::<i64>()
            .expect("timestamp"),
    );

    let rust_fee_on_input =
        arb_math::dlmm::dlmm_fee_on_input(mode, q.swap_for_y).expect("valid collect fee mode");
    assert_eq!(
        rust_fee_on_input, q.fee_on_input,
        "{label}: fee_on_input mismatch (mode {mode}, swap_for_y {})",
        q.swap_for_y
    );

    let (amount_in, amount_out, fee, protocol_fee, end_id, crossed) = if let Some(input) = &q.input
    {
        let mut route = bins.to_vec();
        let quote = quote_dlmm_multi_bin(
            &pool,
            &mut route,
            input.parse().expect("input"),
            q.swap_for_y,
            ts,
        )
        .unwrap_or_else(|e| panic!("{label}: rust exact-in quote failed: {e}"));
        (
            quote.amount_in,
            quote.amount_out,
            quote.fee_amount,
            quote.protocol_fee,
            quote.end_active_id,
            quote.bins_crossed,
        )
    } else {
        let desired: u64 = q
            .desired_out
            .as_ref()
            .expect("exact-out desired")
            .parse()
            .expect("desired");
        let quote = quote_dlmm_exact_out(&pool, bins, desired, q.swap_for_y, ts)
            .unwrap_or_else(|e| panic!("{label}: rust exact-out quote failed: {e}"));
        (
            quote.amount_in,
            quote.amount_out,
            quote.fee_amount,
            quote.protocol_fee,
            quote.end_active_id,
            quote.bins_crossed,
        )
    };

    CaseEval {
        fee_on_input_rust: rust_fee_on_input,
        fee_on_input_sdk: q.fee_on_input,
        amount_in,
        amount_out,
        total_fee: fee,
        protocol_fee,
        end_bin_id: end_id,
        bins_crossed: crossed,
        start_bin_id: f.active_id,
    }
}

fn check_case(f: &SdkFixture, bins: &[DlmmBin], label: &str, mode: u8, q: &SdkQuote) {
    let e = evaluate_case(f, bins, label, mode, q);

    println!("--- {label} ---");
    cmp(
        &format!("{label} amount_in"),
        e.amount_in as i128,
        q.amount_in.parse::<i128>().expect("sdk in"),
    );
    cmp(
        &format!("{label} amount_out"),
        e.amount_out as i128,
        q.amount_out.parse::<i128>().expect("sdk out"),
    );
    cmp(
        &format!("{label} total_fee"),
        e.total_fee as i128,
        q.total_fee.parse::<i128>().expect("sdk fee"),
    );
    cmp(
        &format!("{label} protocol_fee"),
        e.protocol_fee as i128,
        q.protocol_fee.parse::<i128>().expect("sdk pfee"),
    );
    // Sanity: the SDK's own split must add up to the total it reports.
    let sdk_split = q.fee.parse::<i128>().unwrap() + q.protocol_fee.parse::<i128>().unwrap();
    assert_eq!(
        sdk_split,
        q.total_fee.parse::<i128>().unwrap(),
        "{label}: SDK fixture fee split is inconsistent"
    );
    cmp(
        &format!("{label} end_bin_id"),
        e.end_bin_id as i128,
        q.end_bin_id as i128,
    );
    cmp(
        &format!("{label} bins_crossed"),
        e.bins_crossed as i128,
        q.bins_crossed as i128,
    );
}

#[test]
fn mainnet_snapshot_quotes_match_official_sdk() {
    let f = load();
    let bins = load_bins();
    println!(
        "\n=== Mainnet snapshot SDK differential ===\nslot={} pool={} sdk={} active_id={} captured={}..{} bins={}",
        f.rpc_slot,
        f.pool_address,
        f.sdk_version,
        f.active_id,
        f.captured_bin_range[0],
        f.captured_bin_range[1],
        bins.len()
    );
    // The fixture's declared pool mode must be the mode we drive for `mode_0`.
    assert_eq!(
        f.parameters.collect_fee_mode, 0,
        "captured pool CollectFeeMode changed; update the mode mapping"
    );

    for (mode, m) in [
        (0u8, &f.quotes.mode_0_input_only),
        (1u8, &f.quotes.mode_1_only_y),
    ] {
        for (dir, set) in [("x_to_y", &m.x_to_y), ("y_to_x", &m.y_to_x)] {
            check_case(
                &f,
                &bins,
                &format!("mode{mode} {dir} exact_in_small"),
                mode,
                &set.exact_in_small,
            );
            check_case(
                &f,
                &bins,
                &format!("mode{mode} {dir} exact_in_deep"),
                mode,
                &set.exact_in_deep,
            );
            check_case(
                &f,
                &bins,
                &format!("mode{mode} {dir} exact_out_small"),
                mode,
                &set.exact_out_small,
            );
            check_case(
                &f,
                &bins,
                &format!("mode{mode} {dir} exact_out_deep"),
                mode,
                &set.exact_out_deep,
            );
        }
    }
}

// ============================================================================
// Phase 15.2 — machine-readable differential manifest
// ============================================================================

fn reports_phase15_2_dir() -> String {
    for p in [
        "reports/phase15_2",
        "../../reports/phase15_2",
        "../reports/phase15_2",
    ] {
        if Path::new(p).is_dir() {
            return p.to_string();
        }
    }
    panic!("reports/phase15_2 not found; run from the repository root");
}

fn field(name: &str, rust: i128, sdk: i128) -> Value {
    let diff = rust - sdk;
    json!({
        "field": name,
        "rust": rust as i64,
        "sdk": sdk as i64,
        "diff": diff as i64,
        "tolerance": 0,
        "within_tolerance": diff == 0,
    })
}

/// Emit `reports/phase15_2/sdk_differential_manifest.json`.
///
/// Every number in the artifact is produced by the SAME `evaluate_case` path the
/// assertion test uses, and every diff is re-asserted here, so the manifest
/// cannot claim a match the suite did not verify.
#[test]
fn phase15_2_sdk_differential_manifest() {
    let f = load();
    let bins = load_bins();
    let snap: Snapshot = serde_json::from_str(
        &fs::read_to_string(find_fixture("meteora_mainnet_snapshot.json"))
            .expect("read Mainnet snapshot"),
    )
    .expect("parse Mainnet snapshot");

    let idl_path = find_fixture("meteora_sdk_idl_fingerprint.json");
    let idl: Value = serde_json::from_str(&fs::read_to_string(idl_path).expect(
        "fixtures/meteora_sdk_idl_fingerprint.json missing — run \
         `node scripts/differential-harness/fingerprint_sdk_idl.js` first",
    ))
    .expect("parse IDL fingerprint");

    let pool = Pubkey::from_str(&f.pool_address).expect("pool pubkey");

    // --- PDA derivations -----------------------------------------------------
    let mut bin_arrays = Vec::new();
    let mut pda_checks = 0usize;
    let mut pda_failures = 0usize;
    for arr in &snap.bin_arrays {
        let idx = arr.index.expect("snapshot bin array index");
        let derived = arb_tx::derive_meteora_bin_array_pda(&pool, idx);
        let fixture = arr.pda.clone().unwrap_or_default();
        let ok = derived.to_string() == fixture;
        pda_checks += 1;
        if !ok {
            pda_failures += 1;
        }
        bin_arrays.push(json!({
            "index": idx,
            "fixture_pda": fixture,
            "rust_derived_pda": derived.to_string(),
            "match": ok,
            "exists": arr.exists,
            "account_slot": arr.slot,
            "bins_captured": arr.bins.len(),
            "bin_id_range": if arr.bins.is_empty() {
                Value::Null
            } else {
                json!([
                    arr.bins.iter().map(|b| b.bin_id).min(),
                    arr.bins.iter().map(|b| b.bin_id).max(),
                ])
            },
        }));
    }

    let bitmap_derived = arb_tx::derive_meteora_bin_array_bitmap_extension_pda(&pool);
    let oracle_derived = arb_tx::derive_meteora_oracle_pda(&pool);
    let bitmap_fixture = snap.bitmap_extension_pda.clone().unwrap_or_default();
    let oracle_fixture = snap.oracle_pda.clone().unwrap_or_default();
    let bitmap_ok = bitmap_derived.to_string() == bitmap_fixture;
    let oracle_ok = oracle_derived.to_string() == oracle_fixture;
    pda_checks += 2;
    pda_failures += usize::from(!bitmap_ok) + usize::from(!oracle_ok);

    // --- quotes --------------------------------------------------------------
    let mut quote_results: Vec<Value> = Vec::new();
    let mut comparisons = 0usize;
    let mut max_abs_diff: i128 = 0;
    let mut arrays_spanned_max = 0usize;

    let mode_sets: [(u8, &ModeQuotes); 2] = [
        (0, &f.quotes.mode_0_input_only),
        (1, &f.quotes.mode_1_only_y),
    ];
    for (mode, m) in mode_sets {
        for (dir, set) in [("x_to_y", &m.x_to_y), ("y_to_x", &m.y_to_x)] {
            let cases: [(&str, &SdkQuote); 4] = [
                ("exact_in_small", &set.exact_in_small),
                ("exact_in_deep", &set.exact_in_deep),
                ("exact_out_small", &set.exact_out_small),
                ("exact_out_deep", &set.exact_out_deep),
            ];
            for (kind, q) in cases {
                let label = format!("mode{mode}_{dir}_{kind}");
                let e = evaluate_case(&f, &bins, &label, mode, q);

                let sdk_total: i128 = q.total_fee.parse().expect("sdk total_fee");
                let sdk_protocol: i128 = q.protocol_fee.parse().expect("sdk protocol_fee");
                let sdk_lp: i128 = q.fee.parse().expect("sdk fee");
                let rust_lp = e.total_fee as i128 - e.protocol_fee as i128;

                let fields = vec![
                    field(
                        "amount_in",
                        e.amount_in as i128,
                        q.amount_in.parse().expect("sdk amount_in"),
                    ),
                    field(
                        "amount_out",
                        e.amount_out as i128,
                        q.amount_out.parse().expect("sdk amount_out"),
                    ),
                    field("total_fee", e.total_fee as i128, sdk_total),
                    field("protocol_fee", e.protocol_fee as i128, sdk_protocol),
                    field("lp_fee", rust_lp, sdk_lp),
                    field("end_bin_id", e.end_bin_id as i128, q.end_bin_id as i128),
                    field(
                        "bins_crossed",
                        e.bins_crossed as i128,
                        q.bins_crossed as i128,
                    ),
                ];

                for v in &fields {
                    let diff = v["diff"].as_i64().expect("diff");
                    comparisons += 1;
                    assert_eq!(
                        diff,
                        0,
                        "{label}.{}: rust {} vs sdk {} (diff {diff}, tolerance 0)",
                        v["field"].as_str().unwrap(),
                        v["rust"],
                        v["sdk"]
                    );
                    max_abs_diff = max_abs_diff.max((diff as i128).abs());
                }

                let mut spanned: BTreeSet<i64> = BTreeSet::new();
                spanned.insert(bin_array_index_for_bin(e.start_bin_id));
                spanned.insert(bin_array_index_for_bin(e.end_bin_id));
                arrays_spanned_max = arrays_spanned_max.max(spanned.len());

                quote_results.push(json!({
                    "case": label,
                    "collect_fee_mode": mode,
                    "collect_fee_mode_name": if mode == 0 { "InputOnly" } else { "OnlyY" },
                    "direction": dir,
                    "swap_for_y": q.swap_for_y,
                    "kind": kind,
                    "fee_on_input": {
                        "rust": e.fee_on_input_rust,
                        "sdk": e.fee_on_input_sdk,
                        "match": e.fee_on_input_rust == e.fee_on_input_sdk,
                    },
                    "start_bin_id": e.start_bin_id,
                    "fields": fields,
                    "bin_array_indices_touched": spanned.iter().collect::<Vec<_>>(),
                }));
            }
        }
    }

    // --- Q64.64 prices -------------------------------------------------------
    let mut price_checks = Vec::new();
    for p in &f.price_checks {
        let rust = get_price_from_id(p.bin_id, f.bin_step).expect("price");
        let sdk: u128 = p.price_q64.parse().expect("sdk price");
        let diff = rust as i128 - sdk as i128;
        assert_eq!(rust, sdk, "price mismatch at bin {}", p.bin_id);
        comparisons += 1;
        max_abs_diff = max_abs_diff.max(diff.abs());
        price_checks.push(json!({
            "bin_id": p.bin_id,
            "rust_q64": rust.to_string(),
            "sdk_q64": sdk.to_string(),
            "diff": diff as i64,
            "match": diff == 0,
        }));
    }

    let live_arrays = snap
        .bin_arrays
        .iter()
        .filter(|a| a.exists)
        .filter_map(|a| a.index)
        .collect::<Vec<_>>();
    let live_bin_ids = snap
        .bin_arrays
        .iter()
        .flat_map(|a| a.bins.iter().map(|b| b.bin_id))
        .collect::<Vec<_>>();
    let live_bin_range = (
        live_bin_ids.iter().min().copied().unwrap_or(0),
        live_bin_ids.iter().max().copied().unwrap_or(0),
    );

    // --- explicitly uncovered cases -----------------------------------------
    let unsupported = vec![
        json!({
            "case": "five_consecutive_bin_array_live_snapshot",
            "status": "NOT_COVERED_ON_LIVE_SNAPSHOT",
            "detail": format!(
                "Captured snapshot contains only {} existing bin arrays {:?}; the deepest SDK case \
                 spans {} array(s). A 5-array hysteresis/boundary claim is therefore NOT evidenced \
                 by this differential.",
                live_arrays.len(), live_arrays, arrays_spanned_max
            ),
            "required_evidence": "Capture a pool with >=5 consecutive existing bin arrays and \
                regenerate SDK quotes that cross all five, then extend this manifest.",
        }),
        json!({
            "case": "token_2022_transfer_fee_inside_dlmm_quote",
            "status": "PARTIAL_STANDALONE_ONLY",
            "detail": "Token-2022 transfer fee is verified as standalone fee math in \
                crates/arb-engine/tests/meteora_sdk_differential_tests.rs; the captured Mainnet \
                pool uses SPL Token, so no DLMM quote in this manifest applies a Token-2022 fee.",
            "required_evidence": "Capture a Token-2022 DLMM pool with a non-zero transfer fee and \
                obtain official SDK quotes for it.",
        }),
        json!({
            "case": "synthetic_variable_fee_sweep",
            "status": "NOT_COVERED",
            "detail": format!(
                "Variable fee is exercised only through the live snapshot's accumulator \
                 (volatility_accumulator={}, volatility_reference={}, index_reference={}). \
                 No synthetic sweep across variable_fee_control values is included.",
                f.variable_parameters.volatility_accumulator,
                f.variable_parameters.volatility_reference,
                f.variable_parameters.index_reference
            ),
            "required_evidence": "Generate a parameter matrix (variable_fee_control x accumulator) \
                with official SDK quotes and add it to the differential.",
        }),
        json!({
            "case": "bitmap_extension_account_decoding",
            "status": "PDA_DERIVATION_ONLY",
            "detail": "The bitmap-extension PDA derivation is compared against the SDK, but no \
                pool in the snapshot requires an extension account, so extension-account parsing \
                and array-index selection beyond the default bitmap are not exercised.",
            "required_evidence": "Capture a pool whose active bin falls outside the default bitmap \
                range and decode its extension account.",
        }),
        json!({
            "case": "collect_fee_mode_1_live_pool",
            "status": "SIMULATED_ONLY",
            "detail": "The captured pool is CollectFeeMode 0. Mode 1 cases reuse the same bin data \
                with the fee mode overridden, which validates the fee branch but not a real \
                mode-1 pool's on-chain parameters.",
            "required_evidence": "Capture a genuine CollectFeeMode 1 pool and regenerate SDK quotes.",
        }),
        json!({
            "case": "quotes_outside_captured_bin_range",
            "status": "NOT_COVERED",
            "detail": format!(
                "All quotes are confined to the captured bin range {:?}. Behaviour when a quote \
                 exhausts the available bin arrays (partial fill / TooManyBins) is not evidenced.",
                live_bin_range
            ),
            "required_evidence": "Add cases sized to exhaust the captured arrays and record the \
                SDK's error or partial-fill result.",
        }),
        json!({
            "case": "instruction_encoding_and_flash_loan_cpi",
            "status": "OUT_OF_SCOPE",
            "detail": "This manifest compares QUOTE arithmetic only. Meteora swap instruction \
                encoding, account ordering and the Kamino flash-loan CPI wrapper are covered by \
                separate transaction-construction tests, not by the SDK.",
            "required_evidence": "n/a for quote differential",
        }),
    ];

    let all_diffs_zero = max_abs_diff == 0 && pda_failures == 0;
    assert!(
        all_diffs_zero,
        "differential produced non-zero diffs (max_abs_diff={max_abs_diff}, pda_failures={pda_failures})"
    );

    let manifest = json!({
        "artifact": "sdk_differential_manifest.json",
        "phase": "15.2",
        "generated_at_utc": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "verdict": if unsupported.is_empty() { "PASS" } else { "PASS_WITH_DECLARED_GAPS" },
        "tolerance_raw_units": 0,
        "sdk": {
            "package": idl["sdk_package"],
            "version": idl["sdk_version"],
            "bundle_path": idl["sdk_bundle_path"],
            "bundle_bytes": idl["sdk_bundle_bytes"],
            "bundle_sha256": idl["sdk_bundle_sha256"],
            "idl_source": idl["idl_source"],
            "idl_sha256": idl["idl_sha256"],
            "idl_sha256_method": idl["idl_sha256_method"],
            "idl_instruction_count": idl["idl_instructions"],
            "idl_account_count": idl["idl_accounts"],
            "idl_type_count": idl["idl_types"],
            "program_id": f.program_id,
            "fixture_sdk_version": f.sdk_version,
            "pin_match": idl["sdk_version"].as_str() == Some(f.sdk_version.as_str()),
        },
        "rust_engine": {
            "quote_functions": [
                "arb_math::dlmm::quote_dlmm_multi_bin",
                "arb_math::dlmm::quote_dlmm_exact_out",
                "arb_math::dlmm::get_price_from_id",
                "arb_math::dlmm::dlmm_fee_on_input",
            ],
            "pda_functions": [
                "arb_tx::derive_meteora_bin_array_pda",
                "arb_tx::derive_meteora_bin_array_bitmap_extension_pda",
                "arb_tx::derive_meteora_oracle_pda",
            ],
            "source": "crates/arb-math/src/dlmm.rs, crates/arb-tx/src/lib.rs",
        },
        "snapshot": {
            "source_file": "fixtures/meteora_mainnet_snapshot.json",
            "quotes_file": "fixtures/meteora_mainnet_sdk_quotes.json",
            "captured_at_utc": snap.captured_at_utc,
            "rpc_slot": snap.rpc_slot,
            "quotes_rpc_slot": f.rpc_slot,
            "slot_consistent": snap.rpc_slot == Some(f.rpc_slot),
            "pool_address": snap.pool_address,
            "program_id": snap.program_id,
            "pool_data_len": snap.data_len,
            "active_id": f.active_id,
            "bin_step_bps": f.bin_step,
            "collect_fee_mode_declared": f.parameters.collect_fee_mode,
            "volatility_update_timestamp": f.volatility_update_timestamp,
            "captured_bin_range": f.captured_bin_range,
            "live_bin_id_range": [live_bin_range.0, live_bin_range.1],
            "bins_decoded": bins.len(),
            "existing_bin_arrays": live_arrays.clone(),
            "max_bin_arrays_spanned_by_any_case": arrays_spanned_max,
        },
        "bin_arrays": bin_arrays,
        "bitmap_extension_pda": {
            "fixture": bitmap_fixture,
            "rust_derived": bitmap_derived.to_string(),
            "match": bitmap_ok,
        },
        "oracle_pda": {
            "fixture": oracle_fixture,
            "rust_derived": oracle_derived.to_string(),
            "match": oracle_ok,
        },
        "coverage_summary": {
            "quote_cases": quote_results.len(),
            "expected_quote_cases": 16,
            "field_comparisons": comparisons,
            "max_abs_diff_raw": max_abs_diff as i64,
            "all_diffs_zero": all_diffs_zero,
            "pda_checks": pda_checks,
            "pda_failures": pda_failures,
        },
        "quote_results": quote_results,
        "price_checks": price_checks,
        "unsupported": unsupported,
    });

    let out_dir = reports_phase15_2_dir();
    let out_path = Path::new(&out_dir).join("sdk_differential_manifest.json");
    fs::write(
        &out_path,
        serde_json::to_string_pretty(&manifest).unwrap() + "\n",
    )
    .expect("write sdk_differential_manifest.json");
    println!(
        "\nwrote {} (cases={}, comparisons={}, max_abs_diff={}, pda_failures={})",
        out_path.display(),
        quote_results.len(),
        comparisons,
        max_abs_diff,
        pda_failures
    );
}
