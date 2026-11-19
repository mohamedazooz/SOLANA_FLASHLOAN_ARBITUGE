//! Bit-exact parity of the Orca Whirlpool tick conversion against
//! `orca_whirlpools_core` (pinned dev-dependency).
//!
//! The on-chain program's `sqrt_price_from_tick_index` is deterministic integer
//! math; there is no rounding tolerance. Any difference is a defect.

use arb_math::clmm::{MAX_TICK_INDEX, MIN_TICK_INDEX, tick_index_to_sqrt_price_x64};

#[test]
fn tick_to_sqrt_price_is_bit_exact_with_official_orca() {
    let ticks = [
        MIN_TICK_INDEX,
        MIN_TICK_INDEX + 1,
        -443_443,
        -100_000,
        -1,
        0,
        1,
        997,
        100_000,
        MAX_TICK_INDEX - 1,
        MAX_TICK_INDEX,
    ];

    for tick in ticks
        .into_iter()
        .chain((MIN_TICK_INDEX..=MAX_TICK_INDEX).step_by(997))
    {
        let local = tick_index_to_sqrt_price_x64(tick)
            .unwrap_or_else(|error| panic!("local tick conversion failed at {tick}: {error}"));
        let official: u128 = orca_whirlpools_core::tick_index_to_sqrt_price(tick);
        assert_eq!(
            local, official,
            "sqrt price differs from official Orca at tick {tick}"
        );
    }
}

#[test]
fn official_bit_table_values_are_reproduced_exactly() {
    // `test_exact_bit_values` from the official `tick_math.rs`.
    let conditions: &[(i32, u128, u128)] = &[
        (0, 18_446_744_073_709_551_616, 18_446_744_073_709_551_616),
        (1, 18_447_666_387_855_959_850, 18_445_821_805_675_392_311),
        (2, 18_448_588_748_116_922_571, 18_444_899_583_751_176_498),
        (4, 18_450_433_606_991_734_263, 18_443_055_278_223_354_162),
        (8, 18_454_123_878_217_468_680, 18_439_367_220_385_604_838),
        (16, 18_461_506_635_090_006_701, 18_431_993_317_065_449_817),
        (32, 18_476_281_010_653_910_144, 18_417_254_355_718_160_513),
        (64, 18_505_865_242_158_250_041, 18_387_811_781_193_591_352),
        (128, 18_565_175_891_880_433_522, 18_329_067_761_203_520_168),
        (256, 18_684_368_066_214_940_582, 18_212_142_134_806_087_854),
        (512, 18_925_053_041_275_764_671, 17_980_523_815_641_551_639),
        (1024, 19_415_764_168_677_886_926, 17_526_086_738_831_147_013),
        (2048, 20_435_687_552_633_177_494, 16_651_378_430_235_024_244),
        (4096, 22_639_080_592_224_303_007, 15_030_750_278_693_429_944),
        (8192, 27_784_196_929_998_399_742, 12_247_334_978_882_834_399),
        (16384, 41_848_122_137_994_986_128, 8_131_365_268_884_726_200),
        (32768, 94_936_283_578_220_370_716, 3_584_323_654_723_342_297),
        (65536, 488_590_176_327_622_479_860, 696_457_651_847_595_233),
        (
            131_072,
            12_941_056_668_319_229_769_860,
            26_294_789_957_452_057,
        ),
        (
            262_144,
            9_078_618_265_828_848_800_676_189,
            37_481_735_321_082,
        ),
    ];

    for &(tick, positive, negative) in conditions {
        assert_eq!(
            tick_index_to_sqrt_price_x64(tick).expect("positive tick"),
            positive,
            "positive bit {tick} mismatch"
        );
        assert_eq!(
            tick_index_to_sqrt_price_x64(-tick).expect("negative tick"),
            negative,
            "negative bit {tick} mismatch"
        );
    }
}
