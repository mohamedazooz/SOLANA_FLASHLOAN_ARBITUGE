//! Differential: Rust CLMM (Orca Whirlpool) engine vs the pinned official
//! `@orca-so/whirlpools-core` oracle over CAPTURED Mainnet snapshots.
//!
//! Inputs
//! ------
//! * `fixtures/whirlpool_mainnet_pools/*.snapshot.json` — raw account bytes
//!   captured READ-ONLY from Mainnet by
//!   `scripts/differential-harness/capture_mainnet_whirlpool.js`
//!   (whirlpool account + every required tick-array PDA + vault balances).
//! * `fixtures/whirlpool_mainnet_pools/*.quotes.json` — quotes produced by the
//!   official `@orca-so/whirlpools-core` wasm (the same Rust math library the
//!   Whirlpool program logic derives from) over that exact snapshot via
//!   `scripts/differential-harness/run_mainnet_whirlpool_differential.js`.
//!
//! Both sides therefore receive byte-identical pool parameters and tick data.
//! A difference here is an engine difference, not a data difference.
//!
//! Coverage per pool and per direction (`a_to_b` / `b_to_a`):
//! * exact-in / small — largest input whose quote stays inside the current
//!   tick ARRAY (verified in the harness by byte-comparing full-arrays vs
//!   current-array-only SDK quotes before the fixture was written)
//! * exact-in / deep  — largest input fully servable by the captured arrays
//!   (crosses many initialized ticks and typically several arrays)
//! * exact-out small/deep — recorded as SDK provenance only: the production
//!   engine is exact-in, so the Rust side does NOT recompute them; the harness
//!   verified SDK-internal relay consistency (exact-in of the returned
//!   `tokenEstIn` reproduces `tokenOut` exactly) before writing anything.
//!
//! Fee semantics: Whirlpool charges the full trade fee on the input token
//! (LP + protocol shares combined at quote time), so the engine's `fee_paid`
//! is compared against the SDK's `tradeFee` directly.
//!
//! Classic-pool policy: the canonical engine implements classic Whirlpool
//! math only. `feeTierIndex = seed[0] + seed[1] * 256` must equal
//! `tick_spacing` (official adaptive-fee detection rule, verified against
//! orca-so/whirlpools `rust-sdk/core/src/quote/swap.rs` — note its
//! `swap_quote_by_output_token` inverts the `specified_token_a` flag to the
//! OUTPUT side, which the harness honours). A non-classic pool would have
//! been refused at capture time; this test re-derives the verdict from the
//! raw account bytes fail-closed on every run.

use arb_math::clmm::{
    TickArrayFacade, WHIRLPOOL_TICK_ARRAY_SIZE, compute_whirlpool_swap, get_start_tick_index,
    parse_tick_array_account,
};
use base64::Engine;
use serde::Deserialize;
use solana_pubkey::Pubkey;
use std::fs;
use std::path::Path;
use std::sync::{LockResult, Mutex, MutexGuard, OnceLock};

/// Collected quote differences; asserted empty at the end of the batch so the
/// report shows EVERY divergent case, not just the first.
fn diff_log() -> LockResult<MutexGuard<'static, Vec<String>>> {
    static DIFF_LOG: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    DIFF_LOG.get_or_init(|| Mutex::new(Vec::new())).lock()
}
fn find_pool_fixture_dir() -> String {
    for p in &[
        "fixtures/whirlpool_mainnet_pools",
        "../../fixtures/whirlpool_mainnet_pools",
        "../fixtures/whirlpool_mainnet_pools",
    ] {
        if Path::new(p).is_dir() {
            return p.to_string();
        }
    }
    panic!("fixtures/whirlpool_mainnet_pools not found; run the batch capture first");
}

#[derive(Debug, Deserialize)]
struct Pair {
    #[serde(rename = "tickSpacing")]
    #[expect(dead_code)]
    tick_spacing: u16,
    #[serde(rename = "feeTierIndexSeedHex")]
    #[expect(dead_code)]
    fee_tier_index_seed_hex: String,
    #[serde(rename = "feeRate")]
    #[expect(dead_code)]
    fee_rate: u16,
    #[serde(rename = "protocolFeeRate")]
    #[expect(dead_code)]
    protocol_fee_rate: u16,
    #[serde(rename = "liquidity")]
    #[expect(dead_code)]
    liquidity: String,
    #[serde(rename = "sqrtPrice")]
    #[expect(dead_code)]
    sqrt_price: String,
    #[serde(rename = "tickCurrentIndex")]
    #[expect(dead_code)]
    tick_current_index: i32,
    #[serde(rename = "tokenMintA")]
    token_mint_a: String,
    #[serde(rename = "tokenMintB")]
    token_mint_b: String,
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
struct Probe {
    tick_index: i32,
    sdk_start_tick: i32,
}
#[derive(Debug, Deserialize)]
struct Snapshot {
    rpc_slot: u64,
    pool_address: String,
    program_id: String,
    sdk_core_version: String,
    owner: String,
    data_len: usize,
    data_base64: String,
    pair: Pair,
    token_vault_a_balance: String,
    token_vault_b_balance: String,
    tick_arrays: Vec<RawTickArray>,
}

#[derive(Debug, Deserialize)]
struct ExactInCase {
    #[expect(dead_code)]
    class: String,
    amount_in: String,
    amount_out: String,
    trade_fee: String,
}

/// Exact-out cases are provenance-only (production engine is exact-in); they
/// are still deserialized so a malformed fixture is caught here too.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProvenanceCase {
    class: String,
    desired_out: String,
    amount_in: String,
    amount_out: String,
    trade_fee: String,
}

#[derive(Debug, Deserialize)]
struct DirectionCases {
    exact_in_small: ExactInCase,
    exact_in_deep: ExactInCase,
    #[expect(dead_code)]
    exact_out_small: ProvenanceCase,
    #[expect(dead_code)]
    exact_out_deep: ProvenanceCase,
}

#[derive(Debug, Deserialize)]
struct Cases {
    a_to_b: DirectionCases,
    b_to_a: DirectionCases,
}

#[derive(Debug, Deserialize)]
struct Quotes {
    rpc_slot: u64,
    pool_address: String,
    program_id: String,
    sdk_core_version: String,
    tick_spacing: u16,
    fee_tier_index_seed: String,
    fee_rate: u16,
    protocol_fee_rate: u16,
    liquidity: String,
    sqrt_price: String,
    tick_current_index: i32,
    token_mint_a: String,
    token_mint_b: String,
    captured_arrays: usize,
    cases: Cases,
    tick_math_probes: Vec<Probe>,
}

/// Offsets into the whirlpool account (8-byte anchor discriminator included),
/// from the official whirlpools-sdk 0.22.0 IDL artifacts/whirlpool.json.
const WHIRLPOOL_ACCOUNT_SIZE: usize = 8 + 645;
const OFF_TICK_SPACING: usize = 41;
const OFF_FEE_TIER_INDEX_SEED: usize = 43;
const OFF_FEE_RATE: usize = 45;
const OFF_PROTOCOL_FEE_RATE: usize = 47;
const OFF_LIQUIDITY: usize = 49;
const OFF_SQRT_PRICE: usize = 65;
const OFF_TICK_CURRENT_INDEX: usize = 81;

struct RawPool {
    tick_spacing: u16,
    fee_tier_index_seed: [u8; 2],
    fee_rate: u16,
    protocol_fee_rate: u16,
    liquidity: u128,
    sqrt_price: u128,
    tick_current_index: i32,
}

/// Decode the pool fields this test consumes directly from the captured raw
/// account bytes — the same bytes the SDK oracle quoted over — instead of
/// trusting the fixture's pre-decoded copies.
fn decode_raw_pool(data: &[u8]) -> RawPool {
    assert_eq!(
        data.len(),
        WHIRLPOOL_ACCOUNT_SIZE,
        "whirlpool account length {} is not the classic 653-byte layout",
        data.len()
    );
    RawPool {
        tick_spacing: u16::from_le_bytes(
            data[OFF_TICK_SPACING..OFF_TICK_SPACING + 2]
                .try_into()
                .expect("tick_spacing"),
        ),
        fee_tier_index_seed: data[OFF_FEE_TIER_INDEX_SEED..OFF_FEE_TIER_INDEX_SEED + 2]
            .try_into()
            .expect("seed"),
        fee_rate: u16::from_le_bytes(
            data[OFF_FEE_RATE..OFF_FEE_RATE + 2]
                .try_into()
                .expect("fee_rate"),
        ),
        protocol_fee_rate: u16::from_le_bytes(
            data[OFF_PROTOCOL_FEE_RATE..OFF_PROTOCOL_FEE_RATE + 2]
                .try_into()
                .expect("protocol_fee_rate"),
        ),
        liquidity: u128::from_le_bytes(
            data[OFF_LIQUIDITY..OFF_LIQUIDITY + 16]
                .try_into()
                .expect("liquidity"),
        ),
        sqrt_price: u128::from_le_bytes(
            data[OFF_SQRT_PRICE..OFF_SQRT_PRICE + 16]
                .try_into()
                .expect("sqrt_price"),
        ),
        tick_current_index: i32::from_le_bytes(
            data[OFF_TICK_CURRENT_INDEX..OFF_TICK_CURRENT_INDEX + 4]
                .try_into()
                .expect("tick_current"),
        ),
    }
}

fn load_pair(snapshot_path: &Path, quotes_path: &Path) -> (Snapshot, Quotes, Vec<TickArrayFacade>) {
    let snap: Snapshot =
        serde_json::from_str(&fs::read_to_string(snapshot_path).expect("read snapshot"))
            .expect("parse snapshot");
    let quotes: Quotes =
        serde_json::from_str(&fs::read_to_string(quotes_path).expect("read quotes"))
            .expect("parse quotes");

    // Provenance: the fixture must quote the OFFICIAL deployed program.
    assert_eq!(
        snap.program_id, WHIRLPOOL_PROGRAM_ID,
        "pool owner must be the official Whirlpool program"
    );

    // The two fixture halves must describe the same capture.
    assert_eq!(snap.pool_address, quotes.pool_address, "pool mismatch");
    assert_eq!(snap.rpc_slot, quotes.rpc_slot, "slot mismatch");
    assert_eq!(snap.program_id, quotes.program_id, "program mismatch");
    assert_eq!(
        snap.sdk_core_version, quotes.sdk_core_version,
        "sdk version mismatch"
    );

    // Classic-pool verdict re-derived from the RAW account bytes (fail-closed).
    assert_eq!(
        snap.data_len, WHIRLPOOL_ACCOUNT_SIZE,
        "snapshotted pool data_len {} is not the classic 653-byte layout",
        snap.data_len
    );
    let raw = decode_raw_pool(
        &base64::engine::general_purpose::STANDARD
            .decode(&snap.data_base64)
            .expect("decode pool account"),
    );
    let fee_tier_index =
        u16::from(raw.fee_tier_index_seed[0]) | (u16::from(raw.fee_tier_index_seed[1]) << 8);
    assert_eq!(
        raw.tick_spacing, fee_tier_index,
        "{0}: adaptive-fee whirlpool (seed {1:?}) leaked into fixtures — canonical engine is classic-only",
        snap.pool_address, raw.fee_tier_index_seed
    );
    // Raw bytes vs fixture-encoded fields (catches any decode drift).
    assert_eq!(raw.tick_spacing, quotes.tick_spacing);
    assert_eq!(raw.fee_rate, quotes.fee_rate);
    assert_eq!(raw.protocol_fee_rate, quotes.protocol_fee_rate);
    assert_eq!(raw.liquidity.to_string(), quotes.liquidity);
    assert_eq!(raw.sqrt_price.to_string(), quotes.sqrt_price);
    assert_eq!(raw.tick_current_index, quotes.tick_current_index);
    let seed_hex = hex(&raw.fee_tier_index_seed);
    assert_eq!(seed_hex, quotes.fee_tier_index_seed);
    assert_eq!(snap.pair.token_mint_a, quotes.token_mint_a);
    assert_eq!(snap.pair.token_mint_b, quotes.token_mint_b);
    assert_eq!(
        snap.owner, snap.program_id,
        "pool owner must be the whirlpool program"
    );

    // Tick-array PDA derivation from raw bytes: the start tick index is an
    // ASCII decimal string seed (VERIFIED from whirlpools-sdk pda-utils) —
    // NOT an i32 LE seed, which is a classic silent-corruption trap.
    let pool_pk = snap.pool_address.parse::<Pubkey>().expect("pool pubkey");
    let program_id = snap.program_id.parse::<Pubkey>().expect("program id");
    for arr in &snap.tick_arrays {
        let derived = Pubkey::find_program_address(
            &[
                b"tick_array",
                pool_pk.as_ref(),
                arr.start_tick_index.to_string().as_bytes(),
            ],
            &program_id,
        )
        .0;
        assert_eq!(
            derived.to_string(),
            arr.pda,
            "{0}: tick-array PDA mismatch at start {1}",
            snap.pool_address,
            arr.start_tick_index
        );
    }

    // Decode the captured raw tick-array bytes with the ENGINE's parser; the
    // SDK quoted over the equivalent facade, so any parse divergence fails here.
    let mut arrays = Vec::new();
    for arr in &snap.tick_arrays {
        if !arr.exists {
            continue;
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(arr.data_base64.as_deref().expect("existing array data"))
            .expect("decode tick array");
        assert_eq!(
            data.len(),
            WHIRLPOOL_TICK_ARRAY_SIZE,
            "{0}: tick array at start {1} has length {2}, expected {3}",
            snap.pool_address,
            arr.start_tick_index,
            data.len(),
            WHIRLPOOL_TICK_ARRAY_SIZE
        );
        let facade = parse_tick_array_account(&data, quotes.tick_spacing).unwrap_or_else(|e| {
            panic!(
                "{}: tick array {} failed engine parse: {e}",
                snap.pool_address, arr.start_tick_index
            )
        });
        assert_eq!(facade.start_tick_index, arr.start_tick_index);
        arrays.push(facade);
    }
    assert_eq!(
        arrays.len(),
        quotes.captured_arrays,
        "existing-array count mismatch"
    );
    (snap, quotes, arrays)
}

/// Official Whirlpool program id (orca-so/whirlpools, whirlpools-sdk 0.22.0
/// `public/constants`).
const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn check_exact_in(
    pool_address: &str,
    quotes: &Quotes,
    arrays: &[TickArrayFacade],
    label: &str,
    a_to_b: bool,
    case: &ExactInCase,
) {
    let amount_in: u64 = case.amount_in.parse().expect("sdk amount_in");
    let quote = compute_whirlpool_swap(
        quotes.sqrt_price.parse::<u128>().expect("sqrt_price"),
        quotes.tick_current_index,
        quotes.liquidity.parse::<u128>().expect("liquidity"),
        quotes.tick_spacing,
        quotes.fee_rate,
        amount_in,
        a_to_b,
        arrays,
    )
    .unwrap_or_else(|e| panic!("{pool_address} {label}: engine quote failed: {e}"));

    let sdk_out: i128 = case.amount_out.parse().expect("sdk amount_out");
    let sdk_fee: i128 = case.trade_fee.parse().expect("sdk trade_fee");
    println!(
        "{pool_address:<46} {label:<28} rust_in={:<12} sdk_in={:<12} rust_out={:<18} sdk_out={:<18} rust_fee={:<12} sdk_fee={:<12}",
        quote.amount_in_consumed,
        amount_in,
        quote.amount_out_received,
        sdk_out,
        quote.fee_paid,
        sdk_fee
    );
    if quote.amount_in_consumed != amount_in {
        diff_log().unwrap().push(format!(
            "{pool_address} {label}: amount_in_consumed {} != sdk {amount_in}",
            quote.amount_in_consumed
        ));
    }
    if quote.amount_out_received as i128 != sdk_out {
        diff_log().unwrap().push(format!(
            "{pool_address} {label}: amount_out_received {} != sdk {sdk_out} (diff {})",
            quote.amount_out_received,
            quote.amount_out_received as i128 - sdk_out
        ));
    }
    if quote.fee_paid as i128 != sdk_fee {
        diff_log().unwrap().push(format!(
            "{pool_address} {label}: fee_paid {} != sdk {sdk_fee} (diff {})",
            quote.fee_paid,
            quote.fee_paid as i128 - sdk_fee
        ));
    }
}

/// Fail if any exact-in case diverged from the SDK (parity policy: executable
/// routes tolerate NO unexplained discrepancy).
fn assert_no_quote_diffs() {
    let log = diff_log().unwrap();
    assert!(
        log.is_empty(),
        "{} exact-in quote difference(s) vs official SDK:\n{}",
        log.len(),
        log.join("\n")
    );
}

/// Multi-pool batch differential over the batch-captured Mainnet Whirlpools in
/// `fixtures/whirlpool_mainnet_pools/`: tick spacings 1 / 4 / 8 / 64, SOL/USDC
/// and USDC/USDT, fee rates 100 / 400 / 500 / 3000 hundredths-of-a-bip, each
/// captured from Mainnet at a distinct slot by
/// `run_mainnet_whirlpool_differential.js` with the official
/// `@orca-so/whirlpools-core` wasm as quote oracle over a frozen in-process
/// cache. A difference here is an engine difference, not a data difference.
#[test]
fn whirlpool_mainnet_batch_quotes_match_official_sdk() {
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

    for qname in &quotes_files {
        let stem = qname
            .strip_suffix(".quotes.json")
            .expect("quotes file stem");
        let sname = format!("{stem}.snapshot.json");
        let (snap, quotes, arrays) =
            load_pair(&Path::new(&dir).join(&sname), &Path::new(&dir).join(qname));
        println!(
            "{stem}: slot={} tick_spacing={} fee_rate={} protocol_fee_rate={} arrays={} sqrt={} tick_current={}",
            quotes.rpc_slot,
            quotes.tick_spacing,
            quotes.fee_rate,
            quotes.protocol_fee_rate,
            quotes.captured_arrays,
            quotes.sqrt_price,
            quotes.tick_current_index
        );

        // Tick-math parity: the engine's start-tick derivation must agree with
        // whirlpools-core for every probe (active tick, its neighbours and
        // every captured array boundary).
        for p in &quotes.tick_math_probes {
            let rust = get_start_tick_index(p.tick_index, quotes.tick_spacing);
            assert_eq!(
                rust, p.sdk_start_tick,
                "{}: start-tick mismatch at tick {}",
                snap.pool_address, p.tick_index
            );
        }

        // Full exact-in comparison against this pool's SDK-generated quotes.
        check_exact_in(
            &snap.pool_address,
            &quotes,
            &arrays,
            "a_to_b exact_in_small",
            true,
            &quotes.cases.a_to_b.exact_in_small,
        );
        check_exact_in(
            &snap.pool_address,
            &quotes,
            &arrays,
            "a_to_b exact_in_deep",
            true,
            &quotes.cases.a_to_b.exact_in_deep,
        );
        check_exact_in(
            &snap.pool_address,
            &quotes,
            &arrays,
            "b_to_a exact_in_small",
            false,
            &quotes.cases.b_to_a.exact_in_small,
        );
        check_exact_in(
            &snap.pool_address,
            &quotes,
            &arrays,
            "b_to_a exact_in_deep",
            false,
            &quotes.cases.b_to_a.exact_in_deep,
        );

        println!(
            "{stem}: sdk={} vaults_a={} vaults_b={} — exact-in cases verified (exact-out cases are SDK provenance only)",
            quotes.sdk_core_version, snap.token_vault_a_balance, snap.token_vault_b_balance
        );
    }

    assert_no_quote_diffs();
}
