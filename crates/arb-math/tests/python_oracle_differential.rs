//! Differential test: Rust production math vs the independent Python oracle
//! (`audit/math_oracle/oracle.py`, vectors in `audit/test_vectors/`).
//!
//! The oracle is derived from the protocol specifications (Raydium constant
//! product, Meteora DLMM bin swap semantics) with Python arbitrary-precision
//! integers; the only rounding decisions are the floor/ceil points documented
//! in the oracle header. A mismatch here means the production formula diverges
//! from the specification-derived model — exactly the bug class that silently
//! corrupts net-profit math.
//!
//! Run: `cargo test -p arb-math --test python_oracle_differential`
//! Regenerate vectors after a *verified-correct* formula change:
//! `python3 audit/math_oracle/oracle.py`

use serde_json::Value;

fn vectors_dir() -> std::path::PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest)
        .join("../../audit/test_vectors")
        .canonicalize()
        .expect("audit/test_vectors must exist (committed to the repo)")
}

#[test]
fn cpmm_matches_python_oracle() {
    let path = vectors_dir().join("cpmm_vectors.json");
    let raw = std::fs::read_to_string(&path).expect("read cpmm_vectors.json");
    let doc: Value = serde_json::from_str(&raw).expect("parse cpmm_vectors.json");

    for (i, v) in doc["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .enumerate()
    {
        let dx = v["dx"].as_u64().unwrap() as u128;
        let x = v["x"].as_u64().unwrap() as u128;
        let y = v["y"].as_u64().unwrap() as u128;
        let fee_num = v["fee_num"].as_u64().unwrap_or(0) as u128;
        let fee_den = v["fee_den"].as_u64().unwrap_or(0) as u128;
        let gamma = v["gamma"].as_u64().unwrap_or(0) as u128;

        let expected_upfront: Option<u64> = v["out_upfront_fee_ceil"].as_u64();
        let expected_gamma: Option<u64> = v["out_gamma_folded"].as_u64();

        let got_upfront = arb_math::cpmm::cpmm_out_upfront_fee_ceil(dx, x, y, fee_num, fee_den);
        let got_gamma = arb_math::cpmm::cpmm_out_fee(dx, x, y, gamma, fee_den);

        assert_eq!(
            got_upfront, expected_upfront,
            "cpmm_out_upfront_fee_ceil diverges from Python oracle at vector {i}"
        );
        assert_eq!(
            got_gamma.and_then(|o| u64::try_from(o).ok()),
            expected_gamma,
            "cpmm_out_fee diverges from Python oracle at vector {i}"
        );
    }
}

#[test]
fn dlmm_single_bin_matches_python_oracle() {
    let path = vectors_dir().join("dlmm_single_bin_vectors.json");
    let raw = std::fs::read_to_string(&path).expect("read dlmm_single_bin_vectors.json");
    let doc: Value = serde_json::from_str(&raw).expect("parse dlmm_single_bin_vectors.json");

    for (i, v) in doc["vectors"]
        .as_array()
        .expect("vectors array")
        .iter()
        .enumerate()
    {
        let price_q64: u128 = v["price_q64"]
            .as_str()
            .expect("price_q64 encoded as decimal string (exceeds u64/f64 precision)")
            .parse()
            .expect("parse price_q64");
        let swap_for_y = v["swap_for_y"].as_bool().unwrap();
        let fee_rate = v["fee_rate"].as_u64().unwrap() as u128;
        let share = v["protocol_share_bps"].as_u64().unwrap() as u16;

        for mode in ["fee_on_input", "fee_on_output"] {
            let c = &v[mode];
            let mut bin = arb_math::dlmm::DlmmBin {
                id: 0,
                amount_x: v["bin_x"].as_u64().unwrap(),
                amount_y: v["bin_y"].as_u64().unwrap(),
                price_q64,
            };
            let expected_in = v["amount_in"].as_u64().unwrap();
            let step = arb_math::dlmm::consume_bin_liquidity(
                &mut bin,
                expected_in,
                price_q64,
                swap_for_y,
                fee_rate,
                share,
                mode == "fee_on_input",
            )
            .expect("oracle fixtures are valid states");

            let get = |k: &str| c[k].as_u64().unwrap_or(0);
            assert_eq!(
                step.amount_in_consumed,
                get("amount_in_consumed"),
                "v{i} {mode} consumed"
            );
            assert_eq!(step.amount_out, get("amount_out"), "v{i} {mode} out");
            assert_eq!(step.fee_amount, get("fee_amount"), "v{i} {mode} fee");
            assert_eq!(
                step.protocol_fee,
                get("protocol_fee"),
                "v{i} {mode} protocol fee"
            );
            assert_eq!(
                step.drained_bin,
                c["drained_bin"].as_bool().unwrap(),
                "v{i} {mode} drained"
            );
            assert_eq!(bin.amount_x, get("bin_x_after"), "v{i} {mode} bin_x after");
            assert_eq!(bin.amount_y, get("bin_y_after"), "v{i} {mode} bin_y after");
        }
    }
}
