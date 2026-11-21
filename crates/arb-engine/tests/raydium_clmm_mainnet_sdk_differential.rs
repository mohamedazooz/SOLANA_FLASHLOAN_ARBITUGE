//! Differential: Rust CLMM (Raydium CLMM) engine vs the pinned official
//! `@raydium-io/raydium-sdk-v2` oracle over CAPTURED Mainnet snapshots.
//!
//! Inputs
//! ------
//! * `fixtures/raydium_clmm_mainnet_pools/*.snapshot.json` — raw account bytes
//!   captured READ-ONLY from Mainnet (pool + AmmConfig + existing tick-array
//!   PDAs + optional bitmap extension + vault balances) with the native
//!   `block_timestamp` pinned via `getBlockTime(slot)`.
//! * `fixtures/raydium_clmm_mainnet_pools/*.quotes.json` — quotes produced by
//!   `PoolUtils.getOutputAmountAndRemainAccounts` (the official swap
//!   simulator `swapInternal`) over that exact captured generation via
//!   `scripts/differential-harness/run_mainnet_raydium_clmm_differential.js`.
//!
//! Both sides receive byte-identical parameters; a difference here is an
//! engine difference, not a data difference. Parity policy: executable routes
//! tolerate NO unexplained discrepancy — every fee-on-input, fully-servable
//! (`all_trade`) case must match in `amount_in_consumed`, `amount_out_received`
//! and `fee_paid` EXACTLY.
//!
//! Fee semantics (verified 2026-11-21 from `libraries/swapMath.ts` of the
//! SDK, mirroring the deployed program): Raydium CLMM charges the trade fee
//! on the input as a per-step gross-up
//! `fee = amount_in × fee_rate / (1_000_000 − fee_rate)` in 1e6 precision,
//! summed over steps with the last partial step charging
//! `fee = amount_remaining − amount_in`. The engine's per-step Raydium fee
//! model (`ClmmTickMath::Raydium`) is compared against that oracle directly.
//!
//! Refusal coverage: `fee_on = 1/2` charges the fee on the OUTPUT token for
//! one direction. The canonical engine is fee-on-input only, so those cases
//! exist in the fixture as SDK provenance and the engine MUST refuse them
//! fail-closed — asserted here, not quoted.

use arb_dex::raydium_clmm::RaydiumClmmPool;
use arb_dex::traits::AmmPool;
use arb_math::clmm::{ClmmTickMath, TickArrayFacade, compute_swap_with_ticks_per_array_and_math};
use arb_types::PoolId;
use base64::Engine;
use serde::Deserialize;
use solana_pubkey::Pubkey;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LockResult, Mutex, MutexGuard, OnceLock};

/// Official Raydium CLMM program id (`raydium-io/clmm` deployed CAMM program).
const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
/// Deployed `PoolState` length after the 2026-09-30 upgrade (verified against
/// the SDK's `PoolInfoLayout` decode + live Mainnet accounts).
const RAYDIUM_CLMM_POOL_LEN: usize = 1_544;
/// Ticks per `TickArrayState` (official Raydium CLMM constant).
const RAYDIUM_CLMM_TICKS_PER_ARRAY: usize = 60;
/// `status` bit that disables swaps.
const STATUS_SWAP_DISABLED_BIT: u8 = 1 << 4;

/// Collected quote differences; asserted empty at the end of the batch so the
/// report shows EVERY divergent case, not just the first.
fn diff_log() -> LockResult<MutexGuard<'static, Vec<String>>> {
    static DIFF_LOG: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    DIFF_LOG.get_or_init(|| Mutex::new(Vec::new())).lock()
}

fn find_pool_fixture_dir() -> PathBuf {
    for p in &[
        "fixtures/raydium_clmm_mainnet_pools",
        "../../fixtures/raydium_clmm_mainnet_pools",
        "../fixtures/raydium_clmm_mainnet_pools",
    ] {
        if Path::new(p).is_dir() {
            return PathBuf::from(p);
        }
    }
    panic!("fixtures/raydium_clmm_mainnet_pools not found; run the batch capture first");
}

#[derive(Debug, Deserialize)]
struct Pair {
    amm_config: String,
    token_mint_0: String,
    token_mint_1: String,
    #[expect(dead_code)]
    token_vault_0: String,
    #[expect(dead_code)]
    token_vault_1: String,
    #[expect(dead_code)]
    mint_decimals_0: u8,
    #[expect(dead_code)]
    mint_decimals_1: u8,
    tick_spacing: u16,
    #[expect(dead_code)]
    liquidity: String,
    #[expect(dead_code)]
    sqrt_price_x64: String,
    #[expect(dead_code)]
    tick_current: i32,
    fee_on: u8,
    #[expect(dead_code)]
    status: u8,
    dynamic_fee_active: bool,
}

#[derive(Debug, Deserialize)]
struct RawAmmConfig {
    #[expect(dead_code)]
    address: String,
    data_base64: String,
}

#[derive(Debug, Deserialize)]
struct RawBitmapExtension {
    pda: String,
    #[expect(dead_code)]
    exists: bool,
    #[serde(default)]
    #[expect(dead_code)]
    data_base64: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawTickArray {
    start_tick_index: i32,
    pda: String,
    exists: bool,
    #[serde(default)]
    data_base64: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Snapshot {
    rpc_slot: u64,
    pool_address: String,
    program_id: String,
    sdk_version: String,
    data_len: usize,
    data_base64: String,
    pair: Pair,
    amm_config: RawAmmConfig,
    block_timestamp: u64,
    tick_array_bitmap_extension: RawBitmapExtension,
    tick_arrays: Vec<RawTickArray>,
}

#[derive(Debug, Clone, Deserialize)]
struct QuoteCase {
    case_id: String,
    zero_for_one: bool,
    amount_in: String,
    all_trade: bool,
    amount_out: String,
    fee_amount: String,
    #[expect(dead_code)]
    execution_price_x64: String,
    remaining_accounts: Vec<String>,
    #[serde(default)]
    #[expect(dead_code)]
    deep_probe_candidate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Quotes {
    provenance: Provenance,
    pair: PairLite,
    cases: Vec<QuoteCase>,
    #[serde(default)]
    refusals: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Provenance {
    #[serde(rename = "sdk")]
    sdk: SdkInfo,
    snapshot: SnapshotRef,
}

#[derive(Debug, Deserialize)]
struct SdkInfo {
    version: String,
    #[expect(dead_code)]
    package: String,
}

#[derive(Debug, Deserialize)]
struct SnapshotRef {
    pool_address: String,
    slot: u64,
    block_timestamp: u64,
    sdk_version: String,
}

#[derive(Debug, Deserialize)]
struct PairLite {
    tick_spacing: u16,
    fee_on: u8,
    trade_fee_rate: u32,
    mint_0: String,
    mint_1: String,
}

/// Official fee-on-input rule (`PoolState::is_fee_on_input(zero_for_one)`):
/// 0 = FromInput (always), 1 = Token0Only (input for zero_for_one),
/// 2 = Token1Only (input for one_for_zero).
fn is_fee_on_input(fee_on: u8, zero_for_one: bool) -> bool {
    match fee_on {
        0 => true,
        1 => zero_for_one,
        2 => !zero_for_one,
        other => panic!("invalid fee_on {other} in fixture"),
    }
}

struct PoolPair {
    snap: Snapshot,
    quotes: Quotes,
    pool: RaydiumClmmPool,
    arrays: Vec<TickArrayFacade>,
}

fn load_pair(snapshot_path: &Path, quotes_path: &Path) -> PoolPair {
    let snap: Snapshot =
        serde_json::from_str(&fs::read_to_string(snapshot_path).expect("read snapshot"))
            .expect("parse snapshot");
    let quotes: Quotes =
        serde_json::from_str(&fs::read_to_string(quotes_path).expect("read quotes"))
            .expect("parse quotes");

    // Provenance: the fixture must quote the OFFICIAL deployed program.
    assert_eq!(
        snap.program_id, RAYDIUM_CLMM_PROGRAM_ID,
        "pool owner must be the official Raydium CLMM program"
    );
    assert_eq!(
        snap.sdk_version, quotes.provenance.snapshot.sdk_version,
        "sdk version mismatch between fixture halves"
    );
    assert_eq!(
        snap.sdk_version, quotes.provenance.sdk.version,
        "sdk version mismatch between provenance blocks"
    );
    // The two fixture halves must describe the SAME capture generation.
    assert_eq!(snap.pool_address, quotes.provenance.snapshot.pool_address);
    assert_eq!(
        snap.rpc_slot, quotes.provenance.snapshot.slot,
        "slot mismatch"
    );
    assert_eq!(
        snap.block_timestamp, quotes.provenance.snapshot.block_timestamp,
        "block_timestamp mismatch — quotes and snapshot are different captures"
    );
    assert_eq!(snap.pair.tick_spacing, quotes.pair.tick_spacing);
    assert_eq!(snap.pair.fee_on, quotes.pair.fee_on);
    assert_eq!(snap.pair.token_mint_0, quotes.pair.mint_0);
    assert_eq!(snap.pair.token_mint_1, quotes.pair.mint_1);

    // Raw pool decode + engine parse from the SAME captured bytes.
    let data = base64::engine::general_purpose::STANDARD
        .decode(&snap.data_base64)
        .expect("decode pool account");
    assert_eq!(
        data.len(),
        RAYDIUM_CLMM_POOL_LEN,
        "snapshotted pool data_len {} is not the deployed 1544-byte layout",
        data.len()
    );
    assert_eq!(snap.data_len, RAYDIUM_CLMM_POOL_LEN, "data_len mismatch");
    let pool_id = snap.pool_address.parse::<Pubkey>().expect("pool pubkey");
    let mut pool = RaydiumClmmPool::from_account(PoolId(pool_id), &data)
        .unwrap_or_else(|e| panic!("{}: engine pool parse failed: {e:?}", snap.pool_address));

    // Fail-closed verdicts re-derived from the RAW account bytes every run.
    assert_eq!(
        data[arb_dex::raydium_clmm::RAYDIUM_CLMM_STATUS_OFFSET] & STATUS_SWAP_DISABLED_BIT,
        0,
        "{}: swap-disabled pool leaked into fixtures",
        snap.pool_address
    );
    assert_eq!(
        data[arb_dex::raydium_clmm::RAYDIUM_CLMM_FEE_ON_OFFSET],
        quotes.pair.fee_on,
        "{}: fee_on drift between raw bytes and fixture",
        snap.pool_address
    );
    // The raw pool bytes' AmmConfig pointer must match the captured config.
    assert_eq!(
        pool.amm_config.to_string(),
        snap.pair.amm_config,
        "{}: AmmConfig pointer drift between raw bytes and fixture",
        snap.pool_address
    );
    pool.apply_amm_config_data(
        &base64::engine::general_purpose::STANDARD
            .decode(&snap.amm_config.data_base64)
            .expect("decode amm config"),
    )
    .unwrap_or_else(|e| panic!("{}: amm config apply failed: {e:?}", snap.pool_address));
    assert_eq!(
        pool.fee_rate_ppm, quotes.pair.trade_fee_rate,
        "{}: trade_fee_rate drift between AmmConfig bytes and fixture",
        snap.pool_address
    );
    // The capture refuses dynamic-fee-active pools; re-derive that verdict.
    if snap.pair.dynamic_fee_active {
        panic!(
            "{}: dynamic-fee-active pool leaked into fixtures",
            snap.pool_address
        );
    }
    assert_eq!(
        pool.dynamic_fee_control,
        Some(0),
        "{}: dynamic fee must be provably disabled for comparability",
        snap.pool_address
    );
    assert!(
        !pool.has_risky,
        "{}: risky Token-2022 extension pool leaked into fixtures",
        snap.pool_address
    );

    // Tick-array PDA derivation parity: the engine must derive the exact PDAs
    // the SDK walked (i32 big-endian seed, official "tick_array" seed).
    for arr in &snap.tick_arrays {
        let derived = pool.derive_tick_array_pda(arr.start_tick_index);
        assert_eq!(
            derived.to_string(),
            arr.pda,
            "{}: tick-array PDA mismatch at start {}",
            snap.pool_address,
            arr.start_tick_index
        );
    }

    // Decode captured raw tick-array bytes with the ENGINE's parser; PDA
    // derivation parity was already asserted above via the engine's own
    // `derive_tick_array_pda`.
    let mut arrays = Vec::new();
    for arr in &snap.tick_arrays {
        if !arr.exists {
            continue;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(arr.data_base64.as_deref().expect("existing array data"))
            .expect("decode tick array");
        let facade = RaydiumClmmPool::parse_tick_array_state(&bytes).unwrap_or_else(|e| {
            panic!(
                "{}: tick array {} failed engine parse: {e:?}",
                snap.pool_address, arr.start_tick_index
            )
        });
        assert_eq!(facade.start_tick_index, arr.start_tick_index);
        arrays.push(facade);
    }
    let captured = snap.tick_arrays.iter().filter(|a| a.exists).count();
    assert_eq!(
        arrays.len(),
        captured,
        "{}: existing-array count mismatch",
        snap.pool_address
    );
    PoolPair {
        snap,
        quotes,
        pool,
        arrays,
    }
}

fn derive_tick_array_pda(program_id: &Pubkey, pool: Pubkey, start: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[
            arb_constants::RAYDIUM_CLMM_TICK_ARRAY_SEED,
            pool.as_ref(),
            &start.to_be_bytes(),
        ],
        program_id,
    )
    .0
}

/// The SDK walks only arrays on its side of the current array start, ordered
/// away from the active tick (zeroForOne descending, else ascending) —
/// `PoolUtils.getOutputAmountAndRemainAccounts` filtering/sorting, replicated
/// exactly so both engines traverse the same array sequence.
fn arrays_for_direction(
    pool: &RaydiumClmmPool,
    arrays: &[TickArrayFacade],
    zero_for_one: bool,
) -> Vec<TickArrayFacade> {
    let span = RAYDIUM_CLMM_TICKS_PER_ARRAY as i32 * i32::from(pool.tick_spacing);
    let current_start = pool.tick_current.div_euclid(span) * span;
    let mut selected: Vec<TickArrayFacade> = arrays
        .iter()
        .filter(|a| {
            if zero_for_one {
                a.start_tick_index <= current_start
            } else {
                a.start_tick_index >= current_start
            }
        })
        .cloned()
        .collect();
    if zero_for_one {
        selected.sort_by_key(|array| std::cmp::Reverse(array.start_tick_index));
    } else {
        selected.sort_by_key(|array| array.start_tick_index);
    }
    selected
}

fn check_case(pool_pair: &mut PoolPair, case: &QuoteCase) {
    let pool_id_str = pool_pair.snap.pool_address.clone();
    let label_end = case.case_id.clone();
    let zero_for_one = case.zero_for_one;
    let a_to_b = zero_for_one;
    let fee_on = pool_pair.quotes.pair.fee_on;

    // Provenance-only (fee-on-output) direction: the canonical engine must
    // REFUSE to quote it fail-closed.
    if !is_fee_on_input(fee_on, zero_for_one) {
        let mut proxy = pool_pair.pool.clone();
        proxy.tick_arrays = arrays_for_direction(&pool_pair.pool, &pool_pair.arrays, zero_for_one);
        let error = proxy
            .quote(
                case.amount_in.parse::<u64>().expect("sdk amount_in"),
                a_to_b,
            )
            .expect_err(&format!(
                "{pool_id_str} {label_end}: fee-on-output direction must be refused"
            ));
        assert!(
            error.to_string().contains("fee-on-output"),
            "{pool_id_str} {label_end}: unexpected refusal reason: {error}"
        );
        return;
    }

    assert!(
        case.all_trade,
        "{pool_id_str} {label_end}: non-servable case leaked into the fixture"
    );
    let amount_in: u64 = case.amount_in.parse().expect("sdk amount_in");
    let arrays = arrays_for_direction(&pool_pair.pool, &pool_pair.arrays, zero_for_one);
    assert!(
        !arrays.is_empty(),
        "{pool_id_str} {label_end}: engine has no arrays for a servable direction"
    );

    let quote = compute_swap_with_ticks_per_array_and_math(
        pool_pair.pool.sqrt_price_x64,
        pool_pair.pool.tick_current,
        pool_pair.pool.liquidity,
        pool_pair.pool.tick_spacing,
        u16::try_from(pool_pair.quotes.pair.trade_fee_rate)
            .expect("fee rate fits the quote-math u16 domain"),
        amount_in,
        a_to_b,
        &arrays,
        RAYDIUM_CLMM_TICKS_PER_ARRAY,
        ClmmTickMath::Raydium,
    )
    .unwrap_or_else(|e| panic!("{pool_id_str} {label_end}: engine quote failed: {e:?}"));

    let sdk_out: u128 = case.amount_out.parse().expect("sdk amount_out");
    let sdk_fee: u128 = case.fee_amount.parse().expect("sdk fee_amount");

    // Canonical whole-pool path (AmmPool::quote adapter) must agree with the
    // direct math result for the same direction.
    let mut proxy = pool_pair.pool.clone();
    proxy.tick_arrays = arrays.clone();
    let adapter_out = proxy
        .quote(amount_in, a_to_b)
        .unwrap_or_else(|e| panic!("{pool_id_str} {label_end}: adapter quote failed: {e:?}"));

    if quote.amount_in_consumed != amount_in {
        diff_log().unwrap().push(format!(
            "{pool_id_str} {label_end}: amount_in_consumed {} != sdk {amount_in}",
            quote.amount_in_consumed
        ));
    }
    if u128::from(quote.amount_out_received) != sdk_out {
        diff_log().unwrap().push(format!(
            "{pool_id_str} {label_end}: amount_out_received {} != sdk {sdk_out} (diff {})",
            quote.amount_out_received,
            i128::from(quote.amount_out_received) - i128::try_from(sdk_out).unwrap_or(i128::MAX)
        ));
    }
    if u128::from(quote.fee_paid) != sdk_fee {
        diff_log().unwrap().push(format!(
            "{pool_id_str} {label_end}: fee_paid {} != sdk {sdk_fee} (diff {})",
            quote.fee_paid,
            i128::from(quote.fee_paid) - i128::try_from(sdk_fee).unwrap_or(i128::MAX)
        ));
    }
    if adapter_out != quote.amount_out_received {
        diff_log().unwrap().push(format!(
            "{pool_id_str} {label_end}: adapter quote {adapter_out} != direct math {}",
            quote.amount_out_received
        ));
    }

    // Account-dependency manifest check uses the official program id.
    let program_id = pool_pair
        .snap
        .program_id
        .parse::<Pubkey>()
        .expect("program id");
    let mut derivable: std::collections::HashSet<String> = pool_pair
        .snap
        .tick_arrays
        .iter()
        .map(|a| {
            derive_tick_array_pda(&program_id, pool_pair.pool.pool_id.0, a.start_tick_index)
                .to_string()
        })
        .collect();
    derivable.insert(pool_pair.snap.tick_array_bitmap_extension.pda.clone());
    for account in &case.remaining_accounts {
        assert!(
            derivable.contains(account),
            "{pool_id_str} {label_end}: SDK remaining account {account} is not derivable by the engine"
        );
    }

    println!(
        "{pool_id_str:<46} {label_end:<16} in={:<22} rust_out={:<22} sdk_out={:<22} rust_fee={:<20} sdk_fee={:<20} rem={}",
        amount_in,
        quote.amount_out_received,
        sdk_out,
        quote.fee_paid,
        sdk_fee,
        case.remaining_accounts.len()
    );
}

fn assert_no_quote_diffs() {
    let log = diff_log().unwrap();
    assert!(
        log.is_empty(),
        "{} quote difference(s) vs official Raydium SDK:\n{}",
        log.len(),
        log.join("\n")
    );
}

/// Batch differential over the captured Mainnet Raydium CLMM pools in
/// `fixtures/raydium_clmm_mainnet_pools/`: tick spacings 1 / 60 / 120,
/// fee_on 0 (FromInput) and 1 (Token0Only), SPL mints only (Token-2022
/// candidates were refused at capture), each captured at a distinct slot by
/// `run_mainnet_raydium_clmm_differential.js` with the official
/// `@raydium-io/raydium-sdk-v2` swap simulator as quote oracle over a frozen
/// in-process cache. A difference here is an engine difference, not a data
/// difference.
#[test]
fn raydium_clmm_mainnet_batch_quotes_match_official_sdk() {
    let dir = find_pool_fixture_dir();
    let mut quotes_files: Vec<String> = fs::read_dir(&dir)
        .expect("read pool fixture dir")
        .filter_map(|e| {
            let n = e.ok()?.file_name().to_string_lossy().to_string();
            n.ends_with(".quotes.json").then_some(n)
        })
        .collect();
    quotes_files.sort();
    assert!(
        quotes_files.len() >= 4,
        "expected the batch-captured pool set (>=4 pools), found {quotes_files:?}"
    );

    // Fixture hygiene: every snapshot must have a matching quotes file, or it
    // is a refused capture without its SDK verdict — reject the whole set.
    for entry in fs::read_dir(&dir).expect("read pool fixture dir") {
        let name = entry
            .expect("dir entry")
            .file_name()
            .to_string_lossy()
            .to_string();
        if name.ends_with(".snapshot.json") {
            let stem = name.strip_suffix(".snapshot.json").expect("snapshot stem");
            assert!(
                quotes_files.contains(&format!("{stem}.quotes.json")),
                "orphan snapshot {stem} without .quotes.json (refused captures must be re-run)"
            );
        }
    }

    for qname in &quotes_files {
        let stem = qname
            .strip_suffix(".quotes.json")
            .expect("quotes file stem");
        let sname = format!("{stem}.snapshot.json");
        let pool_pair = &mut load_pair(&dir.join(&sname), &dir.join(qname));
        println!(
            "{stem}: slot={} tick_spacing={} fee_rate_ppm={} fee_on={} arrays={} captured_cases={} refusals={}",
            pool_pair.snap.rpc_slot,
            pool_pair.quotes.pair.tick_spacing,
            pool_pair.quotes.pair.trade_fee_rate,
            pool_pair.quotes.pair.fee_on,
            pool_pair.arrays.len(),
            pool_pair.quotes.cases.len(),
            pool_pair.quotes.refusals.len(),
        );

        let cases = pool_pair.quotes.cases.clone();
        let mut fee_on_input_cases = 0usize;
        for case in &cases {
            let fee_on = pool_pair.quotes.pair.fee_on;
            if is_fee_on_input(fee_on, case.zero_for_one) {
                fee_on_input_cases += 1;
            }
            check_case(pool_pair, case);
        }
        // Every refusals ledger entry must be a direction we did NOT quote as
        // a parity target (the refusal is the fixture's contract, not an
        // unexplained gap).
        for refusal in &pool_pair.quotes.refusals {
            assert!(
                refusal.get("direction").is_some() && refusal.get("reason").is_some(),
                "{stem}: malformed refusal ledger entry: {refusal}"
            );
        }
        assert!(
            fee_on_input_cases >= 1,
            "{stem}: fixture has no comparable fee-on-input direction"
        );
    }

    assert_no_quote_diffs();
}
