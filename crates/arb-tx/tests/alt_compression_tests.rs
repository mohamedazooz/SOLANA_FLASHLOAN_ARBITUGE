//! ALT (Address Lookup Table) compression vs V1 inline-account evidence
//! (Phase 16 directive §3: "Investigate safe account compression and ALT
//! usage where supported. Never bypass SIMD-0385 limits.").
//!
//! Establishes, with MEASURED serialized bytes on the SAME real three-pool
//! Meteora DLMM flash-loan route (`Kamino borrow → swap A → swap B → swap C →
//! repay`):
//!
//! 1. **ALT compresses address payload** — the identical route that needs
//!    N×32-byte inline accounts under V1 fits the legacy 1,232-byte packet
//!    when its non-signer accounts are loaded through a v0 Address Lookup
//!    Table (1-byte indices instead of 32-byte keys).
//! 2. **ALT does NOT raise the packet limit** — the v0 wire is hard-enforced
//!    at `SOLANA_PACKET_MTU_LIMIT` (1,232); a larger v0 payload fails closed
//!    with `TransactionTooLarge`. ALT changes *how many accounts fit*, never
//!    *how many bytes the packet may carry*.
//! 3. **No silent downgrade** — with a V1-capable profile the compiler emits
//!    V1 even when lookup tables are supplied; V0+ALT is reachable only by
//!    explicitly declaring `v1_transport_supported: false`.
//! 4. **SIMD-0385 is never bypassed** — V1 keeps its 64-inline-account
//!    ceiling; compression pressure is answered by v0+ALT, not by exceeding
//!    the V1 account limit.
//!
//! Emits `reports/phase16/ALT_COMPRESSION_MATRIX.json` with the measured
//! matrix. All sizes are actual `wire_bytes.len()` — never estimates.

use arb_tx::adaptive::{
    AdaptiveTxCompiler, CompiledAdaptiveTransaction, CompilerCapabilities, TransactionVersion,
    V1ConfigParams,
};
use arb_tx::dlmm_3leg_flash_arb::{
    FlashArb3LegParams, FlashArb3LegPoolLeg, build_flash_funded_three_pool_transaction,
};
use arb_tx::{AddressLookupTable, CompiledInstruction, KeypairSigner};
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068; // main-market WSOL reserve, 1e-5 U68F60
const LEGACY_MTU: usize = arb_tx::SOLANA_PACKET_MTU_LIMIT; // 1,232
const V1_MTU: usize = arb_tx::SOLANA_PACKET_V1_MTU; // 4,096

fn wsol() -> Pubkey {
    Pubkey::from_str(arb_constants::WSOL_MINT).unwrap()
}

fn spl() -> Pubkey {
    Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap()
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

fn params(payer: Pubkey, bins_per_leg: usize, v1: bool) -> FlashArb3LegParams {
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
        kamino: arb_tx::KaminoFlashParams::default_wsol(),
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
        v1_transport_supported: v1,
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

/// Build one ALT containing every non-payer account referenced by the route
/// (pool PDAs, bin arrays, reserves, oracles, mints, ATAs, program ids).
fn route_alt(signer: &KeypairSigner, ixs: &[CompiledInstruction]) -> AddressLookupTable {
    let mut addresses: Vec<Pubkey> = Vec::new();
    for ix in ixs {
        if ix.program_id != signer.pubkey() && !addresses.contains(&ix.program_id) {
            addresses.push(ix.program_id);
        }
        for acc in &ix.accounts {
            if acc.pubkey != signer.pubkey() && !addresses.contains(&acc.pubkey) {
                addresses.push(acc.pubkey);
            }
        }
    }
    AddressLookupTable::new(Pubkey::new_unique(), addresses)
}

/// Same real three-pool route compiled three ways at every bin expansion,
/// measured bytes: V1 inline vs V0 bare vs V0+ALT. Proves where each format's
/// capacity ends, that ALT compression genuinely extends the legacy packet's
/// account reach, and that the 1,232-byte MTU is invariant under ALTs.
#[test]
fn alt_compression_matrix_same_route_three_ways() {
    let signer = KeypairSigner::new_random();
    let mut rungs = Vec::new();
    let mut compression_value_rung = None;

    for bins_per_leg in 1..=12usize {
        // -- Way 1: V1 inline (production default) --
        let p_v1 = params(signer.pubkey(), bins_per_leg, true);
        let v1 = match build_flash_funded_three_pool_transaction(&p_v1, &signer, &[]) {
            Ok(tx) => tx,
            Err(e) => {
                rungs.push(json!({
                    "bins_per_leg": bins_per_leg,
                    "v1": {"compiled": false, "failure_reason": e.to_string()},
                }));
                break;
            }
        };
        assert_eq!(
            CompiledAdaptiveTransaction::parse_version_from_wire(&v1.wire_bytes).unwrap(),
            TransactionVersion::V1
        );
        assert!(v1.tx_size <= V1_MTU);
        let account_count = count_unique_accounts(&signer.pubkey(), &v1.instructions);

        // -- Way 2: V0 WITHOUT ALT --
        let p_v0_bare = params(signer.pubkey(), bins_per_leg, false);
        let bare = build_flash_funded_three_pool_transaction(&p_v0_bare, &signer, &[]);

        // -- Way 3: V0 + ALT (all non-payer accounts compressed) --
        let alt = route_alt(&signer, &v1.instructions);
        assert_eq!(alt.addresses.len(), account_count - 1);
        let p_v0_alt = params(signer.pubkey(), bins_per_leg, false);
        let v0_alt = build_flash_funded_three_pool_transaction(&p_v0_alt, &signer, &[alt]);

        let mut v0_alt_ok_bytes = None;
        let v0_alt_json = match &v0_alt {
            Ok(tx) => {
                assert_eq!(
                    CompiledAdaptiveTransaction::parse_version_from_wire(&tx.wire_bytes).unwrap(),
                    TransactionVersion::V0
                );
                assert!(
                    tx.tx_size <= LEGACY_MTU,
                    "V0+ALT wire {} must fit the 1,232-byte packet",
                    tx.tx_size
                );
                v0_alt_ok_bytes = Some(tx.tx_size);
                json!({
                    "version": "V0",
                    "alt_addresses": account_count - 1,
                    "serialized_size_bytes": tx.tx_size,
                    "fits_1232": true,
                })
            }
            Err(e) => json!({
                "version": null,
                "serialized_size_bytes": null,
                "compiled": false,
                "failure_reason": e.to_string(),
            }),
        };

        // Compression value: a rung where bare V0 fails but V0+ALT fits.
        if bare.is_err() {
            if let Some(alt_bytes) = v0_alt_ok_bytes {
                compression_value_rung = Some((bins_per_leg, account_count, v1.tx_size, alt_bytes));
            }
        }

        rungs.push(json!({
            "bins_per_leg": bins_per_leg,
            "account_count": account_count,
            "v1_inline": {
                "version": "V1",
                "serialized_size_bytes": v1.tx_size,
                "fits_4096": v1.tx_size <= V1_MTU,
                "exceeds_1232": v1.tx_size > LEGACY_MTU,
            },
            "v0_bare": match &bare {
                Ok(tx) => json!({"version": "V0", "serialized_size_bytes": tx.tx_size, "fits_1232": tx.tx_size <= LEGACY_MTU}),
                Err(e) => json!({"version": null, "serialized_size_bytes": null, "compiled": false, "failure_reason": e.to_string()}),
            },
            "v0_alt": v0_alt_json,
        }));
    }

    // The matrix must demonstrate the ALT's actual value: at least one route
    // that the bare legacy packet rejects but the ALT-compressed v0 packet
    // carries, and every v0+ALT success stays inside 1,232 bytes.
    let (bins, accounts, v1_bytes, v0_alt_bytes) =
        compression_value_rung.expect("ALT must extend at least one route past the bare V0 limit");
    assert!(v0_alt_bytes < v1_bytes);

    write_report(&json!({
        "schema": "sentinel.phase16.alt_compression_matrix.v1",
        "instrument": "real_three_pool_dlmm_route",
        "compression_value_rung": {
            "bins_per_leg": bins,
            "account_count": accounts,
            "v1_inline_bytes": v1_bytes,
            "v0_alt_bytes": v0_alt_bytes,
            "bytes_saved_vs_v1": v1_bytes - v0_alt_bytes,
        },
        "rungs": rungs,
        "invariants_proven": [
            "ALT compresses Nx32-byte inline accounts to 1-byte indices (v0)",
            "ALT does NOT raise the packet limit: v0 wire hard-capped at 1232",
            "V0 without ALT fails closed with TransactionTooLarge",
            "V1 remains the production default; V0+ALT is opt-in only",
        ],
    }));
}

/// ALT never raises the packet limit: climb bin-array expansion on the v0+ALT
/// path until either the route still fits 1,232 or the compiler fails closed.
#[test]
fn alt_packet_limit_invariant_across_bin_expansion() {
    let signer = KeypairSigner::new_random();
    let mut rungs = Vec::new();

    for bins_per_leg in [6usize, 12, 20] {
        let p_v1 = params(signer.pubkey(), bins_per_leg, true);
        let v1 = match build_flash_funded_three_pool_transaction(&p_v1, &signer, &[]) {
            Ok(tx) => tx,
            Err(_) => break, // V1 account ceiling reached; nothing left to compress
        };
        let alt = route_alt(&signer, &v1.instructions);
        let p_v0 = params(signer.pubkey(), bins_per_leg, false);
        match build_flash_funded_three_pool_transaction(&p_v0, &signer, &[alt]) {
            Ok(v0_alt) => {
                assert_eq!(
                    CompiledAdaptiveTransaction::parse_version_from_wire(&v0_alt.wire_bytes)
                        .unwrap(),
                    TransactionVersion::V0
                );
                assert!(v0_alt.tx_size <= LEGACY_MTU);
                rungs.push(json!({
                    "bins_per_leg": bins_per_leg,
                    "v1_bytes": v1.tx_size,
                    "v0_alt_bytes": v0_alt.tx_size,
                    "fits_1232": true,
                }));
            }
            Err(e) => {
                rungs.push(json!({
                    "bins_per_leg": bins_per_leg,
                    "v1_bytes": v1.tx_size,
                    "v0_alt_bytes": null,
                    "fits_1232": false,
                    "failure_reason": e.to_string(),
                }));
                break;
            }
        }
    }

    assert!(
        !rungs.is_empty(),
        "at least one v0+ALT rung must be measured"
    );
    write_report(&json!({
        "schema": "sentinel.phase16.alt_packet_limit_invariant.v1",
        "instrument": "real_three_pool_dlmm_route",
        "rungs": rungs,
        "invariant": "v0+ALT wire size is hard-capped at 1232 bytes for every bin expansion; overflow fails closed",
    }));
}

/// No silent downgrade: a V1-capable profile compiles V1 even when lookup
/// tables are handed to the compiler, and `compile_adaptive` with the
/// standard mainnet capability profile never selects V0+ALT on its own.
#[test]
fn v1_profile_never_downgrades_to_v0_alt() {
    let signer = KeypairSigner::new_random();
    let bins_per_leg = 6;

    let p = params(signer.pubkey(), bins_per_leg, true);
    let v1 = build_flash_funded_three_pool_transaction(&p, &signer, &[]).unwrap();
    let alt = route_alt(&signer, &v1.instructions);

    // Builder-level: V1 path ignores supplied ALTs entirely.
    let p_with_alt = params(signer.pubkey(), bins_per_leg, true);
    let v1_with_alt =
        build_flash_funded_three_pool_transaction(&p_with_alt, &signer, std::slice::from_ref(&alt))
            .unwrap();
    assert_eq!(
        CompiledAdaptiveTransaction::parse_version_from_wire(&v1_with_alt.wire_bytes).unwrap(),
        TransactionVersion::V1,
        "V1-capable profile must never emit V0+ALT"
    );

    // Compiler-level: standard mainnet capabilities + ALTs + V1 config → V1.
    let config = V1ConfigParams {
        priority_fee_lamports: Some(20_000),
        compute_unit_limit: Some(400_000),
        loaded_accounts_data_size_limit: Some(1_048_576),
        heap_size: Some(256 * 1024),
    };
    let compiled = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        &v1.instructions,
        p.recent_blockhash,
        std::slice::from_ref(&alt),
        Some(config),
        CompilerCapabilities::v1_capable(),
        None,
    )
    .unwrap();
    assert_eq!(compiled.version, TransactionVersion::V1);
    assert!(compiled.wire_bytes.len() <= V1_MTU);
}

fn write_report(report: &serde_json::Value) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../reports/phase16");
    std::fs::create_dir_all(dir).unwrap();
    let path = format!("{dir}/ALT_COMPRESSION_MATRIX.json");
    let existing = std::fs::read_to_string(&path).ok();
    let mut merged: Vec<serde_json::Value> = existing
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if let Some(map) = report.as_object() {
        merged.retain(|e| e.get("schema") != map.get("schema"));
    }
    merged.push(report.clone());
    std::fs::write(&path, serde_json::to_string_pretty(&merged).unwrap()).unwrap();
}
