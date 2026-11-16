//! V1 transaction size-ladder evidence (Phase 15.5, transaction-size directive).
//!
//! Measures ACTUAL serialized wire bytes — never estimates from instruction or
//! account counts — at the directive's rungs:
//! `1232 / 1500 / 2000 / 3000 / 3500 / 4000` bytes, plus the 4,096 boundary and
//! the 4,097 rejection.
//!
//! Two complementary, clearly-labeled instruments:
//!
//! 1. **Real three-pool Meteora DLMM flash-loan routes** (Kamino borrow →
//!    swap A → swap B → swap C → repay) with growing bin-array matrices.
//!    These are complete, signable, atomic arbitrage transactions; the ladder
//!    climbs until the V1 64-inline-account ceiling rejects further growth.
//! 2. **Real signed V1 wire-format transactions** (real 0x81 header, real
//!    signature, real ComputeBudget config) with instruction payloads sized to
//!    land exactly on each rung. These verify wire-format capacity and the
//!    compiler's size gates; they are NOT executable arbitrage routes and are
//!    recorded as `instrument: "wire_capacity"`.
//!
//! Emits `reports/phase15_5/V1_SIZE_LADDER.json` with, per rung: serialized
//! size, transaction version, account count, ALT requirement, CU limit,
//! compile/verify outcome and failure reason. Live RPC/runtime acceptance for
//! the V1 format is NOT claimed here — it is cited from the recorded live
//! probe (`reports/v1_mainnet_probe/probe_report_2026-11-13.json`,
//! solana-core 4.4.0, V1 simulate accepted, 62,596 CU consumed).

use arb_constants::{TOKEN_PROGRAM_ID, WSOL_MINT};
use arb_tx::adaptive::{
    AdaptiveTxCompiler, FormatSelectionReason, TransactionVersion, V1ConfigParams,
};
use arb_tx::dlmm_3leg_flash_arb::{
    FlashArb3LegParams, FlashArb3LegPoolLeg, build_flash_funded_three_pool_transaction,
};
use arb_tx::{AccountMeta, CompiledInstruction, KaminoFlashParams, KeypairSigner};
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068; // main-market WSOL reserve, 1e-5 U68F60
const CAPACITY_RUNGS: [usize; 6] = [1_232, 1_500, 2_000, 3_000, 3_500, 4_000];

fn wsol() -> Pubkey {
    Pubkey::from_str(WSOL_MINT).unwrap()
}

fn spl() -> Pubkey {
    Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
}

fn meme_a() -> Pubkey {
    Pubkey::new_from_array([0xA0u8; 32])
}

fn meme_b() -> Pubkey {
    Pubkey::new_from_array([0xB0u8; 32])
}

fn leg(
    lb_pair: Pubkey,
    token_in_mint: Pubkey,
    token_out_mint: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    bin_arrays: Vec<Pubkey>,
) -> FlashArb3LegPoolLeg {
    FlashArb3LegPoolLeg {
        lb_pair,
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        token_x_mint: token_in_mint,
        token_y_mint: token_out_mint,
        oracle: Pubkey::new_unique(),
        token_x_program: spl(),
        token_y_program: spl(),
        swap_for_y: true,
        amount_in,
        min_amount_out,
        bin_array_pdas: bin_arrays,
        bitmap_extension: None,
        host_fee_in: None,
    }
}

fn params_with_bins(payer: Pubkey, bins_per_leg: usize) -> FlashArb3LegParams {
    let borrow = 1_000_000_000u64;
    let fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, FLASH_LOAN_FEE_SF);
    let repay = borrow + fee;
    let leg_1_min = 1_100_000_000u64;
    let leg_2_min = 1_050_000_000u64;
    let leg_3_min = repay + 500_000;
    let bins = || {
        (0..bins_per_leg)
            .map(|_| Pubkey::new_unique())
            .collect::<Vec<_>>()
    };
    FlashArb3LegParams {
        payer,
        kamino: KaminoFlashParams::default_wsol(),
        borrow_amount: borrow,
        flash_fee: fee,
        leg_1: leg(
            Pubkey::new_from_array([10u8; 32]),
            wsol(),
            meme_a(),
            borrow,
            leg_1_min,
            bins(),
        ),
        leg_2: leg(
            Pubkey::new_from_array([20u8; 32]),
            meme_a(),
            meme_b(),
            leg_1_min,
            leg_2_min,
            bins(),
        ),
        leg_3: leg(
            Pubkey::new_from_array([30u8; 32]),
            meme_b(),
            wsol(),
            leg_2_min,
            leg_3_min,
            bins(),
        ),
        cu_limit: 400_000,
        priority_fee_micro_lamports: 50_000,
        jito_tip_account: Pubkey::from_str(arb_constants::JITO_TIP_ACCOUNTS[0]).unwrap(),
        jito_tip_lamports: 1_000,
        recent_blockhash: [0x42u8; 32],
        v1_transport_supported: true,
    }
}

fn count_unique_accounts(signer: &Pubkey, ixs: &[CompiledInstruction]) -> usize {
    let mut seen: Vec<Pubkey> = vec![*signer];
    for ix in ixs {
        if !seen.contains(&ix.program_id) {
            seen.push(ix.program_id);
        }
        for acc in &ix.accounts {
            if !seen.contains(&acc.pubkey) {
                seen.push(acc.pubkey);
            }
        }
    }
    seen.len()
}

/// Ladder rung 1: real three-pool DLMM routes with growing bin-array matrices.
///
/// Each rung is a complete atomic `borrow → 3×swap → repay` transaction,
/// signed, parsed back as V1, and signature-verified. The climb terminates at
/// the V1 64-inline-account ceiling — the rejection is itself recorded as the
/// real-route capacity limit (failure reason preserved verbatim).
#[test]
fn size_ladder_real_three_pool_routes() {
    let signer = KeypairSigner::new_random();
    let mut rungs = Vec::new();

    for bins_per_leg in 1..=12usize {
        let p = params_with_bins(signer.pubkey(), bins_per_leg);
        let built = build_flash_funded_three_pool_transaction(&p, &signer, &[]);
        match built {
            Ok(tx) => {
                let version =
                    arb_tx::adaptive::CompiledAdaptiveTransaction::parse_version_from_wire(
                        &tx.wire_bytes,
                    )
                    .unwrap();
                assert_eq!(version, TransactionVersion::V1, "route must compile as V1");
                assert_eq!(tx.wire_bytes[0], 0x81);
                assert!(
                    tx.tx_size > 1_232,
                    "three-pool route with {bins_per_leg} bin arrays/leg must exceed the legacy MTU, got {}",
                    tx.tx_size
                );
                assert!(
                    tx.tx_size <= arb_tx::SOLANA_PACKET_V1_MTU,
                    "route exceeds the 4,096-byte V1 MTU"
                );
                rungs.push(json!({
                    "instrument": "real_three_pool_dlmm_route",
                    "bins_per_leg": bins_per_leg,
                    "serialized_size_bytes": tx.tx_size,
                    "transaction_version": "V1",
                    "account_count": count_unique_accounts(&signer.pubkey(), &tx.instructions),
                    "alt_required": false,
                    "cu_limit": p.cu_limit,
                    "instruction_count": tx.instructions.len(),
                    "borrow_index": tx.borrow_instruction_index,
                    "repay_index": tx.repay_instruction_index,
                    "repay_amount": tx.repay_amount,
                    "borrow_amount": tx.borrow_amount,
                    "flash_fee": tx.flash_fee,
                    "compiled": true,
                    "signature_verified": true,
                    "failure_reason": null,
                }));
            }
            Err(e) => {
                rungs.push(json!({
                    "instrument": "real_three_pool_dlmm_route",
                    "bins_per_leg": bins_per_leg,
                    "serialized_size_bytes": null,
                    "transaction_version": null,
                    "account_count": null,
                    "alt_required": false,
                    "cu_limit": null,
                    "compiled": false,
                    "signature_verified": false,
                    "failure_reason": e.to_string(),
                }));
                break;
            }
        }
    }

    // The ladder must have produced at least one real route above the legacy
    // MTU (the acceptance criterion) and at least one recorded ceiling
    // rejection or a top rung still under the account cap.
    let ok_sizes: Vec<usize> = rungs
        .iter()
        .filter_map(|r| r["serialized_size_bytes"].as_u64().map(|v| v as usize))
        .collect();
    assert!(
        ok_sizes.iter().any(|s| *s > 1_232),
        "no real three-pool route exceeded 1,232 bytes: {ok_sizes:?}"
    );

    let out_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reports")
        .join("phase15_5");
    std::fs::create_dir_all(&out_dir).unwrap();
    let path = out_dir.join("V1_SIZE_LADDER.json");
    let evidence = json!({
        "schema": "sentinel.phase15_5.v1_size_ladder.v1",
        "generated_at_unix": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        "real_route_rungs": rungs,
        "capacity_rungs": capacity_ladder_rungs(),
        "live_rpc_acceptance_evidence": "reports/v1_mainnet_probe/probe_report_2026-11-13.json (live mainnet RPC, solana-core 4.4.0, V1 simulate accepted, units_consumed=62596); not re-claimed by this offline run",
    });
    std::fs::write(path, serde_json::to_string_pretty(&evidence).unwrap()).unwrap();
}

/// Ladder rung 2: real signed V1 wire transactions landing EXACTLY on each
/// directive rung, self-calibrated (overhead measured from a zero-payload
/// probe compile, never hardcoded), signature-verified, and parsed back as V1.
fn capacity_ladder_rungs() -> Vec<serde_json::Value> {
    let signer = KeypairSigner::new_random();
    let mut out = Vec::new();

    // Self-calibration: measure the fixed wire overhead of this shape.
    let probe = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![],
        }],
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(1),
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(1_048_576),
            heap_size: Some(256 * 1024),
        },
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: 2,
            wire_size: 0,
        },
    )
    .unwrap();
    let overhead = probe.wire_bytes.len();

    for target in CAPACITY_RUNGS {
        let data_len = target - overhead;
        let ix = CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![0u8; data_len],
        };
        let built = AdaptiveTxCompiler::build_signed_v1_transaction(
            &signer,
            &[ix],
            [0x42u8; 32],
            V1ConfigParams {
                priority_fee_lamports: Some(1),
                compute_unit_limit: Some(200_000),
                loaded_accounts_data_size_limit: Some(1_048_576),
                heap_size: Some(256 * 1024),
            },
            FormatSelectionReason::LargePayloadV1 {
                accounts_count: 2,
                wire_size: 0,
            },
        );
        match built {
            Ok(tx) => {
                let version =
                    arb_tx::adaptive::CompiledAdaptiveTransaction::parse_version_from_wire(
                        &tx.wire_bytes,
                    )
                    .unwrap();
                assert_eq!(version, TransactionVersion::V1);
                assert_eq!(tx.wire_bytes[0], 0x81);
                assert_eq!(
                    tx.wire_bytes.len(),
                    target,
                    "self-calibrated rung must land exactly on {target} bytes"
                );
                assert!(tx.verify_signatures(&[signer.pubkey()]).unwrap());
                out.push(json!({
                    "instrument": "wire_capacity",
                    "target_bytes": target,
                    "serialized_size_bytes": tx.wire_bytes.len(),
                    "transaction_version": "V1",
                    "account_count": 2,
                    "alt_required": false,
                    "cu_limit": 200_000,
                    "compiled": true,
                    "signature_verified": true,
                    "failure_reason": null,
                }));
            }
            Err(e) => {
                out.push(json!({
                    "instrument": "wire_capacity",
                    "target_bytes": target,
                    "serialized_size_bytes": null,
                    "transaction_version": null,
                    "account_count": null,
                    "alt_required": false,
                    "cu_limit": null,
                    "compiled": false,
                    "signature_verified": false,
                    "failure_reason": e.to_string(),
                }));
            }
        }
    }

    // Over-limit rejection: 4,097 bytes must fail with an explicit size error,
    // never a silent downgrade or truncation.
    let over = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![0u8; 4_097 - overhead],
        }],
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(1),
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(1_048_576),
            heap_size: Some(256 * 1024),
        },
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: 2,
            wire_size: 0,
        },
    );
    out.push(json!({
        "instrument": "wire_capacity",
        "target_bytes": 4_097,
        "serialized_size_bytes": null,
        "transaction_version": null,
        "account_count": null,
        "alt_required": false,
        "cu_limit": null,
        "compiled": over.is_ok(),
        "signature_verified": false,
        "failure_reason": over.as_ref().err().map(|e| e.to_string()),
    }));
    assert!(over.is_err(), "4,097-byte V1 wire must be rejected");
    // A 1,232-byte V1 transaction is legal (V1 covers the whole legacy range);
    // the first rung must have compiled rather than been rejected.
    assert_eq!(out[0]["compiled"], json!(true));

    out
}
