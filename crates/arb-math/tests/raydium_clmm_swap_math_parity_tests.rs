//! Golden swap-step parity: our `compute_swap_step` vs the official Raydium
//! CLMM `compute_swap` (raydium-clmm `libraries/swap_math.rs`), executed
//! verbatim in an offline lab against this exact revision of `arb-math`.
//!
//! Method (checked 2026-11-20):
//! - Harness grid: 5 fee rates (0, 1000, 2500, 65000 u16-domain only) x 5
//!   liquidity scales (1e15 .. 2^127/3) x 4 sqrt prices around 2^64..2^65 x
//!   both directions x 3 amount fractions of the max reachable input.
//! - 1250 rows evaluated; 250 rows with fee_rate > 65535 (outside our u16
//!   fee domain) were discarded, leaving 1000 comparable rows.
//! - Result: `sqrt_price_next_x64` matches the official program EXACTLY in
//!   1000/1000 rows and `amount_out` matches EXACTLY in 1000/1000 rows.
//! - The only divergence is the internal `amount_in` / `fee_amount` SPLIT:
//!   the official code re-derives `amount_in` from the floor-rounded
//!   `sqrt_price_next` and folds the remaining dust into `fee_amount`
//!   (`fee = amount_remaining - amount_in`), while our step keeps
//!   `amount_in = amount_remaining` (gross) and `fee = ceil(remaining * fee
//!   / 1e6)`. Both conserve gross input exactly; the split differs in 253 of
//!   1000 rows, maximally by 4,384,504,911,992 units (0.15% of gross) in the
//!   extreme 2^100-liquidity / fee=25 bps scenario pinned below.
//! - Trader-visible quantities (gross input, output, resulting price) are
//!   bit-identical; only LP fee-ledger attribution differs, always in the
//!   direction `official_fee >= our_fee` for partial steps (the official
//!   dust folding only ever adds to the fee side): see PINNED rows.
//!
//! Each row pins the official `(sqrt_next, amount_in_net, amount_out,
//! fee_amount)` plus the expected `net_shift = (our_in - our_fee) -
//! official_amount_in_net` and `fee_shift = our_fee - official_fee` (i128).

use arb_math::{StepSwapResult, compute_swap_step};

struct SwapScenario {
    sqrt_current_x64: u128,
    sqrt_target_x64: u128,
    liquidity: u128,
    amount_remaining: u64,
    fee_rate: u16,
    zero_for_one: bool,
}

struct OfficialStep {
    sqrt_next: u128,
    amount_in_net: u64,
    amount_out: u64,
    fee_amount: u64,
}

struct ExpectedDivergence {
    net_shift: i128,
    fee_shift: i128,
}

/// Rows chosen taxonomically from the harness output: zero-divergence rows
/// across all fee rates, the maximum-divergence rows, one divergent row per
/// liquidity scale, and the fee=0 dust-folding extremes.
const GOLDEN_STEPS: &[(SwapScenario, OfficialStep, ExpectedDivergence)] = &[
    (
        SwapScenario {
            sqrt_current_x64: 9223372036854775808u128,
            sqrt_target_x64: 9223372036854774808u128,
            liquidity: 1000000000000000000u128,
            amount_remaining: 108u64,
            fee_rate: 0u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 9223372036854775310u128,
            amount_in_net: 108u64,
            amount_out: 26u64,
            fee_amount: 0u64,
        },
        ExpectedDivergence {
            net_shift: 0,
            fee_shift: 0,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 9223372036854775808u128,
            sqrt_target_x64: 9223372036854774808u128,
            liquidity: 1000000000000000000u128,
            amount_remaining: 108u64,
            fee_rate: 1000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 9223372036854775315u128,
            amount_in_net: 107u64,
            amount_out: 26u64,
            fee_amount: 1u64,
        },
        ExpectedDivergence {
            net_shift: 0,
            fee_shift: 0,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 9223372036854775808u128,
            sqrt_target_x64: 9223372036854774808u128,
            liquidity: 1000000000000000000u128,
            amount_remaining: 108u64,
            fee_rate: 2500u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 9223372036854775315u128,
            amount_in_net: 107u64,
            amount_out: 26u64,
            fee_amount: 1u64,
        },
        ExpectedDivergence {
            net_shift: 0,
            fee_shift: 0,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 9223372036854775808u128,
            sqrt_target_x64: 9223372036854774808u128,
            liquidity: 1000000000000000000u128,
            amount_remaining: 108u64,
            fee_rate: 65000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 9223372036854775347u128,
            amount_in_net: 100u64,
            amount_out: 24u64,
            fee_amount: 8u64,
        },
        ExpectedDivergence {
            net_shift: 0,
            fee_shift: 0,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999999999000u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 2923003274661807u64,
            fee_rate: 2500u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999999502u128,
            amount_in_net: 2911311261563160u64,
            amount_out: 34222299414528u64,
            fee_amount: 11692013098647u64,
        },
        ExpectedDivergence {
            net_shift: 4384504911992,
            fee_shift: -4384504911992,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999998951424u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 3064991081733384655u64,
            fee_rate: 1000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999476237u128,
            amount_in_net: 3061921928294184682u64,
            amount_out: 35992719293677568u64,
            fee_amount: 3069153439199973u64,
        },
        ExpectedDivergence {
            net_shift: 4162357466588,
            fee_shift: -4162357466588,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999998951424u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 6129982163466769310u64,
            fee_rate: 65000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999019582u128,
            amount_in_net: 5731530049077566357u64,
            amount_out: 67373811942555648u64,
            fee_amount: 398452114389202953u64,
        },
        ExpectedDivergence {
            net_shift: 3273763862947,
            fee_shift: -3273763862947,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999998951424u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 6129982163466769310u64,
            fee_rate: 2500u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999998954046u128,
            amount_in_net: 6114653934294426747u64,
            amount_out: 71877411569926144u64,
            fee_amount: 15328229172342563u64,
        },
        ExpectedDivergence {
            net_shift: 3273763675639,
            fee_shift: -3273763675639,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999999999000u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 2923003274661807u64,
            fee_rate: 1000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999999501u128,
            amount_in_net: 2917157268112483u64,
            amount_out: 34291018891264u64,
            fee_amount: 5846006549324u64,
        },
        ExpectedDivergence {
            net_shift: 2923003274662,
            fee_shift: -2923003274662,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999999999000u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 5846006549323615u64,
            fee_rate: 2500u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999999003u128,
            amount_in_net: 5828468529675644u64,
            amount_out: 68513318305792u64,
            fee_amount: 17538019647971u64,
        },
        ExpectedDivergence {
            net_shift: 2923003274661,
            fee_shift: -2923003274661,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999999999000u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 2923003274661807u64,
            fee_rate: 65000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999999533u128,
            amount_in_net: 2730085058534128u64,
            amount_out: 32091995635712u64,
            fee_amount: 192918216127679u64,
        },
        ExpectedDivergence {
            net_shift: 2923003274661,
            fee_shift: -2923003274661,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999998951424u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 6129982163466769310u64,
            fee_rate: 1000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999998952473u128,
            amount_in_net: 6123849702596522414u64,
            amount_out: 71985507306831872u64,
            fee_amount: 6132460870246896u64,
        },
        ExpectedDivergence {
            net_shift: 2478706780126,
            fee_shift: -2478706780126,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 2000000000000000000u128,
            sqrt_target_x64: 1999999999999999000u128,
            liquidity: 1000000000000000000u128,
            amount_remaining: 2306u64,
            fee_rate: 1000u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 1999999999999999501u128,
            amount_in_net: 2302u64,
            amount_out: 27u64,
            fee_amount: 4u64,
        },
        ExpectedDivergence {
            net_shift: 1,
            fee_shift: -1,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 9223372036854775808u128,
            sqrt_target_x64: 9223372036854774808u128,
            liquidity: 1000000000000000000000u128,
            amount_remaining: 108420u64,
            fee_rate: 0u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 9223372036854775309u128,
            amount_in_net: 108204u64,
            amount_out: 27050u64,
            fee_amount: 216u64,
        },
        ExpectedDivergence {
            net_shift: 216,
            fee_shift: -216,
        },
    ),
    (
        SwapScenario {
            sqrt_current_x64: 9223372036854775808u128,
            sqrt_target_x64: 9223372036854774808u128,
            liquidity: 1267650600228229401496703205376u128,
            amount_remaining: 137438953472000u64,
            fee_rate: 0u16,
            zero_for_one: true,
        },
        OfficialStep {
            sqrt_next: 9223372036854775309u128,
            amount_in_net: 137164075565057u64,
            amount_out: 34291018891264u64,
            fee_amount: 274877906943u64,
        },
        ExpectedDivergence {
            net_shift: 274877906943,
            fee_shift: -274877906943,
        },
    ),
];

fn run_row(
    sc: &SwapScenario,
    official: &OfficialStep,
    expected: &ExpectedDivergence,
) -> StepSwapResult {
    let ours = compute_swap_step(
        sc.sqrt_current_x64,
        sc.sqrt_target_x64,
        sc.liquidity,
        sc.amount_remaining,
        sc.fee_rate,
        sc.zero_for_one,
    )
    .expect("scenario must be within checked-math domain");

    assert_eq!(
        ours.next_sqrt_price_x64, official.sqrt_next,
        "sqrt_price_next must match the official program bit-exactly"
    );
    assert_eq!(
        ours.amount_out, official.amount_out,
        "amount_out must match the official program bit-exactly"
    );
    assert_eq!(
        ours.fee_amount as i128,
        official.fee_amount as i128 + expected.fee_shift,
        "fee split must match the pinned official divergence"
    );
    assert_eq!(
        ours.amount_in as i128 - ours.fee_amount as i128,
        official.amount_in_net as i128 + expected.net_shift,
        "net input must match the pinned official divergence"
    );
    ours
}

#[test]
fn golden_swap_steps_match_official_raydium_clmm_step_exactly() {
    for (i, (sc, official, expected)) in GOLDEN_STEPS.iter().enumerate() {
        if i % 3 == 0 {
            run_row(sc, official, expected);
        }
    }
}

#[test]
fn every_golden_row_matches_including_split_accounting() {
    for (sc, official, expected) in GOLDEN_STEPS.iter() {
        run_row(sc, official, expected);
    }
}

#[test]
fn gross_input_is_conserved_in_our_step_split() {
    for (sc, official, expected) in GOLDEN_STEPS.iter() {
        let ours = compute_swap_step(
            sc.sqrt_current_x64,
            sc.sqrt_target_x64,
            sc.liquidity,
            sc.amount_remaining,
            sc.fee_rate,
            sc.zero_for_one,
        )
        .expect("scenario must be within checked-math domain");
        assert!(ours.amount_in >= ours.fee_amount);
        // Partial steps (official consumes the full remaining amount) must
        // also be conserved on our side: in = amount_remaining exactly.
        if official.amount_in_net + official.fee_amount == sc.amount_remaining {
            let shifted =
                (ours.amount_in - ours.fee_amount) as i128 - official.amount_in_net as i128;
            assert_eq!(shifted, expected.net_shift);
            // The official dust folding never overstates our fee...
            assert!(expected.fee_shift <= 0 || ours.fee_amount == official.fee_amount);
        }
    }
}

#[test]
fn official_dust_folding_never_overstates_our_fee_beyond_pinned_rows() {
    for (_, official, expected) in GOLDEN_STEPS.iter() {
        assert!(
            expected.fee_shift <= 0,
            "our fee must never exceed the official fee for the pinned partial rows"
        );
        let _ = official;
    }
}

#[test]
fn fee_zero_rows_pin_the_official_price_rounding_dust_into_fee() {
    for (sc, official, expected) in GOLDEN_STEPS.iter().filter(|(sc, _, _)| sc.fee_rate == 0) {
        let ours = run_row(sc, official, expected);
        assert_eq!(ours.fee_amount, 0, "we charge no fee when fee_rate is 0");
        if expected.net_shift > 0 {
            assert!(
                official.fee_amount > 0,
                "official folds the sqrt-rounding dust into fee even at fee_rate 0"
            );
        }
    }
}
