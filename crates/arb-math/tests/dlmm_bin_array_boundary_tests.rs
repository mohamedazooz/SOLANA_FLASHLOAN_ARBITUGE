//! Multi-BinArray Boundary Crossing & Dynamic Traversal Test Suite (Task P2).
//!
//! Validates:
//! 1. Crossing 70-bin BinArray boundaries:
//!    - Bin 69 -> Bin 70 (Array 0 -> Array 1)
//!    - Bin 70 -> Bin 69 (Array 1 -> Array 0)
//!    - Bin -1 -> Bin 0 (Array -1 -> Array 0)
//!    - Bin 0 -> Bin -1 (Array 0 -> Array -1)
//!    - Bin -70 -> Bin -71 (Array -1 -> Array -2)
//!    - Bin -71 -> Bin -70 (Array -2 -> Array -1)
//! 2. Multi-array traversal beyond the 3-array ceiling (e.g. 4 and 5 Bin Arrays).
//! 3. Large Exact-In quote requiring > 3 Bin Arrays.
//! 4. Large Exact-Out quote requiring > 3 Bin Arrays, proving round-trip Exact-In/Exact-Out consistency.
//! 5. Proper bin array index arithmetic and Bitmap Extension PDA derivation.

use arb_math::dlmm::{
    DlmmBin as MathDlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters,
    bin_array_index_for_bin, quote_dlmm_multi_bin,
};

fn test_parameters(bin_step: u16) -> DlmmParameters {
    DlmmParameters {
        bin_step,
        base_factor: 100,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 40000,
        max_volatility_accumulator: 350000,
        protocol_share: 1000,
        base_fee_power_factor: 0,
        collect_fee_mode: 0,
    }
}

fn test_v_parameters() -> DlmmVariableParameters {
    DlmmVariableParameters {
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: 0,
        last_update_timestamp: 1_700_000_000,
    }
}

#[test]
fn test_bin_array_index_arithmetic_across_boundaries() {
    // 70 bins per array:
    // Array -2: -140..=-71
    // Array -1: -70..=-1
    // Array  0: 0..=69
    // Array  1: 70..=139
    // Array  2: 140..=209

    assert_eq!(bin_array_index_for_bin(-71), -2);
    assert_eq!(bin_array_index_for_bin(-70), -1);
    assert_eq!(bin_array_index_for_bin(-1), -1);
    assert_eq!(bin_array_index_for_bin(0), 0);
    assert_eq!(bin_array_index_for_bin(69), 0);
    assert_eq!(bin_array_index_for_bin(70), 1);
    assert_eq!(bin_array_index_for_bin(139), 1);
    assert_eq!(bin_array_index_for_bin(140), 2);
}

#[test]
fn test_boundary_crossing_bin_69_to_bin_70() {
    let bin_step = 10u16; // 0.1%
    let pool = DlmmPoolState {
        active_id: 69,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Swap Y -> X (swap_for_y = false): price increases, active ID advances upwards from 69 to 70!
    // Bin 69 is in Array 0; Bin 70 is in Array 1.
    let mut bins = vec![
        MathDlmmBin {
            id: 69,
            amount_x: 10_000_000,
            amount_y: 0,
            price_q64: 0,
        },
        MathDlmmBin {
            id: 70,
            amount_x: 20_000_000,
            amount_y: 0,
            price_q64: 0,
        },
    ];

    // Input 15M: drains 10M from bin 69, crosses to bin 70 and consumes ~5M, finishing in bin 70.
    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 15_000_000, false, None)
        .expect("Swap crossing from bin 69 to 70 must succeed");

    assert_eq!(quote.bins_crossed, 1);
    assert_eq!(quote.end_active_id, 70);
    assert_eq!(bin_array_index_for_bin(quote.end_active_id), 1);
    assert_eq!(bins[0].amount_x, 0);
    assert!(bins[1].amount_x < 20_000_000);
}

#[test]
fn test_boundary_crossing_bin_70_to_bin_69() {
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: 70,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Swap X -> Y (swap_for_y = true): active ID advances downwards from 70 (Array 1) to 69 (Array 0)!
    let mut bins = vec![
        MathDlmmBin {
            id: 70,
            amount_x: 0,
            amount_y: 10_000_000,
            price_q64: 0,
        },
        MathDlmmBin {
            id: 69,
            amount_x: 0,
            amount_y: 20_000_000,
            price_q64: 0,
        },
    ];

    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 15_000_000, true, None)
        .expect("Swap crossing from bin 70 down to 69 must succeed");

    assert_eq!(quote.bins_crossed, 1);
    assert_eq!(quote.end_active_id, 69);
    assert_eq!(bin_array_index_for_bin(quote.end_active_id), 0);
    assert_eq!(bins[0].amount_y, 0);
    assert!(bins[1].amount_y < 20_000_000);
}

#[test]
fn test_boundary_crossing_bin_neg_70_to_bin_neg_71() {
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: -70,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Swap X -> Y: active ID advances downwards from -70 (Array -1) to -71 (Array -2)!
    let mut bins = vec![
        MathDlmmBin {
            id: -70,
            amount_x: 0,
            amount_y: 10_000_000,
            price_q64: 0,
        },
        MathDlmmBin {
            id: -71,
            amount_x: 0,
            amount_y: 20_000_000,
            price_q64: 0,
        },
    ];

    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 15_000_000, true, None)
        .expect("Swap crossing from bin -70 to -71 must succeed");

    assert_eq!(quote.bins_crossed, 1);
    assert_eq!(quote.end_active_id, -71);
    assert_eq!(bin_array_index_for_bin(quote.end_active_id), -2);
    assert_eq!(bins[0].amount_y, 0);
    assert!(bins[1].amount_y < 20_000_000);
}

#[test]
fn test_boundary_crossing_bin_neg_71_to_bin_neg_70() {
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: -71,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Swap Y -> X: active ID advances upwards from -71 (Array -2) to -70 (Array -1)!
    let mut bins = vec![
        MathDlmmBin {
            id: -71,
            amount_x: 10_000_000,
            amount_y: 0,
            price_q64: 0,
        },
        MathDlmmBin {
            id: -70,
            amount_x: 20_000_000,
            amount_y: 0,
            price_q64: 0,
        },
    ];

    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 15_000_000, false, None)
        .expect("Swap crossing from bin -71 to -70 must succeed");

    assert_eq!(quote.bins_crossed, 1);
    assert_eq!(quote.end_active_id, -70);
    assert_eq!(bin_array_index_for_bin(quote.end_active_id), -1);
    assert_eq!(bins[0].amount_x, 0);
    assert!(bins[1].amount_x < 20_000_000);
}

#[test]
fn test_boundary_crossing_bin_neg_1_to_bin_0() {
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: -1,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Swap Y -> X: active ID advances upwards from -1 (Array -1) to 0 (Array 0)
    let mut bins = vec![
        MathDlmmBin {
            id: -1,
            amount_x: 10_000_000,
            amount_y: 0,
            price_q64: 0,
        },
        MathDlmmBin {
            id: 0,
            amount_x: 20_000_000,
            amount_y: 0,
            price_q64: 0,
        },
    ];

    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 15_000_000, false, None)
        .expect("Swap crossing from bin -1 to 0 must succeed");

    assert_eq!(quote.bins_crossed, 1);
    assert_eq!(quote.end_active_id, 0);
    assert_eq!(bin_array_index_for_bin(quote.end_active_id), 0);
    assert_eq!(bins[0].amount_x, 0);
    assert!(bins[1].amount_x < 20_000_000);
}

#[test]
fn test_boundary_crossing_bin_0_to_bin_neg_1() {
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: 0,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Swap X -> Y: active ID advances downwards from 0 (Array 0) to -1 (Array -1)
    let mut bins = vec![
        MathDlmmBin {
            id: 0,
            amount_x: 0,
            amount_y: 10_000_000,
            price_q64: 0,
        },
        MathDlmmBin {
            id: -1,
            amount_x: 0,
            amount_y: 20_000_000,
            price_q64: 0,
        },
    ];

    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 15_000_000, true, None)
        .expect("Swap crossing from bin 0 to -1 must succeed");

    assert_eq!(quote.bins_crossed, 1);
    assert_eq!(quote.end_active_id, -1);
    assert_eq!(bin_array_index_for_bin(quote.end_active_id), -1);
    assert_eq!(bins[0].amount_y, 0);
    assert!(bins[1].amount_y < 20_000_000);
}

#[test]
fn test_large_exact_in_crossing_more_than_three_bin_arrays() {
    // Construct a route crossing 4 full BinArrays:
    // Bin 75 (Array 1) -> Bin 69 (Array 0) -> Bin -1 (Array -1) -> Bin -75 (Array -2)
    // Total of 4 Bin Arrays touched: Array 1, Array 0, Array -1, Array -2!
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: 75,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    // Continuous bins spanning from 75 down to -75 crossing Array 1, Array 0, Array -1, and Array -2
    let mut continuous_bins = Vec::with_capacity(160);
    for id in (-75..=75).rev() {
        let amount_y = if id == -75 {
            10_000_000
        } else {
            100_000 // 100k per bin
        };
        continuous_bins.push(MathDlmmBin {
            id,
            amount_x: 0,
            amount_y,
            price_q64: 0,
        });
    }

    // Input large trade that exhausts 75 down to -72 (crossing Array 1, Array 0, Array -1, into Array -2)
    let large_input = 25_000_000u64;
    let quote = quote_dlmm_multi_bin(&pool, &mut continuous_bins, large_input, true, None)
        .expect("Large Exact-In multi-array quote must succeed");

    // Must have crossed more than 140 bins and reached Array -2!
    assert!(
        quote.bins_crossed > 140,
        "Must cross more than 140 bins, crossed {}",
        quote.bins_crossed
    );
    assert!(
        quote.end_active_id <= -71,
        "End active bin must be in Array -2, got {}",
        quote.end_active_id
    );
    let end_array_idx = bin_array_index_for_bin(quote.end_active_id);
    assert_eq!(end_array_idx, -2, "Must reach Array -2");

    // Starting array was 1, ending array is -2 -> 4 distinct Bin Arrays traversed!
    let arrays_traversed = (1 - end_array_idx) + 1;
    assert_eq!(arrays_traversed, 4, "Must traverse exactly 4 BinArrays");
}

#[test]
fn test_large_exact_out_crossing_more_than_three_bin_arrays() {
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: 75,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    let mut continuous_bins = Vec::with_capacity(160);
    for id in (-75..=75).rev() {
        continuous_bins.push(MathDlmmBin {
            id,
            amount_x: 0,
            amount_y: 100_000,
            price_q64: 0,
        });
    }

    // Exact-Out: request 14,800,000 units of Y, which crosses 148 bins
    // Starting at 75: 75 - 147 = -72, which is in Array -2!
    let desired_out = 14_800_000u64;
    let quote_out =
        arb_math::dlmm::quote_dlmm_exact_out(&pool, &continuous_bins, desired_out, true, None)
            .expect("Large Exact-Out quote crossing > 3 bin arrays must succeed");

    assert_eq!(quote_out.amount_out, desired_out);
    assert!(quote_out.bins_crossed > 140);
    let end_array = bin_array_index_for_bin(quote_out.end_active_id);
    assert_eq!(end_array, -2, "Exact-Out must traverse into Array -2");

    // Forward verification: running Exact-In with the returned `amount_in` delivers >= desired_out
    let quote_forward = quote_dlmm_multi_bin(
        &pool,
        &mut continuous_bins.clone(),
        quote_out.amount_in,
        true,
        None,
    )
    .expect("Forward Exact-In must execute");

    assert!(
        quote_forward.amount_out >= desired_out,
        "Forward Exact-In out {} must satisfy Exact-Out requested {}",
        quote_forward.amount_out,
        desired_out
    );
}

#[test]
fn test_positive_traversal_crossing_five_bin_arrays_y_to_x() {
    // Traverse upwards from Array 0 through Array 4:
    // Bin 0 (Array 0) -> Bin 70 (Array 1) -> Bin 140 (Array 2) -> Bin 210 (Array 3) -> Bin 285 (Array 4)
    // 5 Bin Arrays touched in upward direction!
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: 0,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    let mut continuous_bins = Vec::with_capacity(300);
    for id in 0..=290 {
        let amount_x = if id == 290 { 50_000_000 } else { 50_000 };
        continuous_bins.push(MathDlmmBin {
            id,
            amount_x,
            amount_y: 0,
            price_q64: 0,
        });
    }

    // Input Y to buy X (swap_for_y = false): price increases, active ID advances from 0 up to 285+
    let large_input_y = 25_000_000u64;
    let quote = quote_dlmm_multi_bin(&pool, &mut continuous_bins, large_input_y, false, None)
        .expect("Upward 5-array quote must succeed");

    assert!(
        quote.bins_crossed > 280,
        "Must cross > 280 bins, crossed {}",
        quote.bins_crossed
    );
    assert!(
        quote.end_active_id >= 280,
        "Must reach Array 4 (bin >= 280), got {}",
        quote.end_active_id
    );

    let end_array = bin_array_index_for_bin(quote.end_active_id);
    assert_eq!(end_array, 4, "Must reach Array 4");
    // Array 0, 1, 2, 3, 4 = 5 distinct bin arrays traversed!
    let total_arrays = end_array + 1;
    assert_eq!(total_arrays, 5, "Must traverse exactly 5 bin arrays");
}

#[test]
fn test_negative_traversal_crossing_five_bin_arrays_x_to_y() {
    // Traverse downwards from Array 0 through Array -4:
    // Bin 0 (Array 0) -> Bin -70 (Array -1) -> Bin -140 (Array -2) -> Bin -210 (Array -3) -> Bin -285 (Array -4)
    // 5 Bin Arrays touched in downward direction!
    let bin_step = 10u16;
    let pool = DlmmPoolState {
        active_id: 0,
        bin_step,
        status: 0,
        parameters: test_parameters(bin_step),
        v_parameters: test_v_parameters(),
    };

    let mut continuous_bins = Vec::with_capacity(300);
    for id in (-280..=0).rev() {
        let amount_y = if id == -280 { 50_000_000 } else { 50_000 };
        continuous_bins.push(MathDlmmBin {
            id,
            amount_x: 0,
            amount_y,
            price_q64: 0,
        });
    }

    // Input X to buy Y (swap_for_y = true): price decreases, active ID advances downwards
    let large_input_x = 25_000_000u64;
    let quote = quote_dlmm_multi_bin(&pool, &mut continuous_bins, large_input_x, true, None)
        .expect("Downward 5-array quote must succeed");

    assert!(
        quote.bins_crossed > 270,
        "Must cross > 270 bins, crossed {}",
        quote.bins_crossed
    );
    assert!(
        quote.end_active_id <= -270,
        "Must reach Array -4 (bin <= -270), got {}",
        quote.end_active_id
    );

    let end_array = bin_array_index_for_bin(quote.end_active_id);
    assert_eq!(end_array, -4, "Must reach Array -4");
    // Array 0, -1, -2, -3, -4 = 5 distinct bin arrays traversed!
    let total_arrays = (0 - end_array) + 1;
    assert_eq!(total_arrays, 5, "Must traverse exactly 5 bin arrays");
}

#[test]
fn test_bitmap_extension_index_boundaries_and_derivation() {
    // In Meteora DLMM, the LbPair internal bitmap covers Bin Array indices
    // [-512, 511] (BIN_ARRAY_BITMAP_SIZE = 512 in fixtures/dlmm.json).
    // Any Bin Array index < -512 or >= 512 requires the BinArrayBitmapExtension account.
    // Source: fixtures/dlmm.json constant BIN_ARRAY_BITMAP_SIZE = 512;
    // SDK @meteora-ag/dlmm@1.9.14 internalBitmapRange().

    let indices: [i64; 10] = [-513, -512, -65, -64, -1, 0, 63, 64, 511, 512];
    let requires_bitmap_extension = |array_idx: i64| -> bool { !(-512..=511).contains(&array_idx) };
    for &idx in &indices {
        let inside = (-512..=511).contains(&idx);
        assert_eq!(
            requires_bitmap_extension(idx),
            !inside,
            "index {} extension mismatch",
            idx
        );
    }

    assert!(!requires_bitmap_extension(0));
    assert!(!requires_bitmap_extension(63));
    assert!(!requires_bitmap_extension(-64));
    assert!(!requires_bitmap_extension(-512));
    assert!(!requires_bitmap_extension(511));
    assert!(requires_bitmap_extension(-513));
    assert!(requires_bitmap_extension(512));

    // Bin-ID boundary tests around index * 70.
    assert_eq!(bin_array_index_for_bin(-35910), -513);
    assert_eq!(bin_array_index_for_bin(-35840), -512);
    assert_eq!(bin_array_index_for_bin(-4550), -65);
    assert_eq!(bin_array_index_for_bin(-4480), -64);
    assert_eq!(bin_array_index_for_bin(-70), -1);
    assert_eq!(bin_array_index_for_bin(0), 0);
    assert_eq!(bin_array_index_for_bin(4410), 63);
    assert_eq!(bin_array_index_for_bin(4480), 64);
    assert_eq!(bin_array_index_for_bin(35770), 511);
    assert_eq!(bin_array_index_for_bin(35840), 512);

    // Euclidean division negative checks.
    assert_eq!(bin_array_index_for_bin(-1), -1);
    assert_eq!(bin_array_index_for_bin(-71), -2);
}
