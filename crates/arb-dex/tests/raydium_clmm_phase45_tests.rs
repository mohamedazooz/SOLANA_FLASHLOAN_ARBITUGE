//! Raydium CLMM Phase 4 dynamic-fee / fee-side gate tests.
//!
//! VERIFIED layout facts (deployed program source, checked 2026-11-20):
//! `PoolState` LEN = 1544 with `fee_on` (`CollectFeeOn`) @390,
//! `dynamic_fee_info` @1096..1176 (`dynamic_fee_control` u32 @1102,
//! `max_volatility_accumulator` u32 @1106, `volatility_accumulator` u32
//! @1118). Accounts shorter than the dynamic-fee tail cannot prove the fee
//! state, so every quote path must fail closed instead of assuming
//! `CollectFeeOn::FromInput` with the dynamic fee disabled.

use arb_dex::{AmmPool, RaydiumClmmPool};
use arb_math::clmm::{TickArrayFacade, TickFacade};
use arb_types::{ArbError, PoolId};

const FULL_LEN: usize = 1544;
const SQRT_X64_PRICE_1: u128 = 1u128 << 64;
const LIQUIDITY: u128 = 1 << 60;

struct PoolBytes {
    data: Vec<u8>,
}

impl PoolBytes {
    fn new(len: usize) -> Self {
        Self {
            data: vec![0u8; len],
        }
    }

    fn head(mut self) -> Self {
        self.data[0..8]
            .copy_from_slice(&arb_dex::raydium_clmm::RAYDIUM_CLMM_POOL_STATE_DISCRIMINATOR);
        self.data[9] = 9;
        self.data[73] = 10;
        self.data[105] = 11;
        self.data[137] = 12;
        self.data[169] = 13;
        self.data[201] = 14;
        self.data[233] = 6;
        self.data[234] = 9;
        self.data[235..237].copy_from_slice(&1u16.to_le_bytes());
        self.data[237..253].copy_from_slice(&LIQUIDITY.to_le_bytes());
        self.data[253..269].copy_from_slice(&SQRT_X64_PRICE_1.to_le_bytes());
        self.data[269..273].copy_from_slice(&0i32.to_le_bytes());
        self
    }

    fn status(mut self, status: u8) -> Self {
        self.data[389] = status;
        self
    }

    fn fee_on(mut self, fee_on: u8) -> Self {
        self.data[390] = fee_on;
        self
    }

    fn dynamic_fee(mut self, control: u32, max_va: u32, va: u32) -> Self {
        if control != 0 {
            self.data[1096..1098].copy_from_slice(&10u16.to_le_bytes());
            self.data[1098..1100].copy_from_slice(&100u16.to_le_bytes());
            self.data[1100..1102].copy_from_slice(&5_000u16.to_le_bytes());
            self.data[1102..1106].copy_from_slice(&control.to_le_bytes());
            self.data[1106..1110].copy_from_slice(&max_va.to_le_bytes());
            self.data[1118..1122].copy_from_slice(&va.to_le_bytes());
        }
        self
    }

    fn build(self) -> RaydiumClmmPool {
        let mut pool = RaydiumClmmPool::from_account(PoolId::new([7; 32]), &self.data)
            .expect("pool account decodes");
        // This fixture represents an already validated AmmConfig snapshot;
        // hydration integration tests exercise the on-chain PDA/owner checks.
        pool.amm_config_loaded = true;
        pool.amm_config_index = Some(0);
        pool.fee_rate_ppm = 2_500;
        pool
    }
}

#[test]
fn full_length_account_decodes_dynamic_fee_fields() {
    let pool = PoolBytes::new(FULL_LEN)
        .head()
        .status(0)
        .dynamic_fee(10_000, 10_000, 1_000)
        .build();
    assert_eq!(pool.dynamic_fee_control, Some(10_000));
    assert_eq!(pool.max_volatility_accumulator, Some(10_000));
    assert_eq!(pool.volatility_accumulator, 1_000);
    assert_eq!(pool.fee_on, Some(0));
}

#[test]
fn fee_bps_includes_dynamic_fee_upper_bound_with_ceil() {
    // control=10_000, max_va=10_000, ts=1 -> crossed=1e4, num=1e12,
    // denom=1e13 -> dynamic = 1 (ceil); total = 2_500 + 1 = 2501 ppm =
    // 25.01 bps -> rounded up to 26.
    let mut pool = PoolBytes::new(FULL_LEN)
        .head()
        .fee_on(0)
        .dynamic_fee(10_000, 10_000, 0)
        .build();
    pool.tick_arrays.push(stub_tick_array());
    assert_eq!(pool.fee_bps(), Some(26));
    let state = pool.clmm_state().expect("clmm state");
    assert_eq!(state.fee_rate, 2_501);
}

#[test]
fn fee_bps_with_disabled_dynamic_fee_is_base_only() {
    let mut pool = PoolBytes::new(FULL_LEN)
        .head()
        .fee_on(0)
        .dynamic_fee(0, 0, 0)
        .build();
    pool.tick_arrays.push(stub_tick_array());
    assert_eq!(pool.fee_bps(), Some(25));
    assert_eq!(pool.clmm_state().expect("clmm state").fee_rate, 2_500);
}

#[test]
fn fee_bps_none_when_dynamic_fee_state_unprovable() {
    // len 500 proves fee_on but not the dynamic-fee tail (>= 1122 bytes).
    let pool = PoolBytes::new(500).head().build();
    assert_eq!(pool.fee_on, Some(0));
    assert_eq!(pool.dynamic_fee_control, None);
    assert_eq!(pool.fee_bps(), None);
    assert!(pool.clmm_state().is_none());
}

#[test]
fn fee_bps_none_when_even_fee_side_unprovable() {
    let pool = PoolBytes::new(350).head().build();
    assert_eq!(pool.fee_on, None);
    assert_eq!(pool.fee_bps(), None);
    assert!(pool.clmm_state().is_none());
}

#[test]
fn quote_rejects_token0_or_token1_only_fee_collection() {
    let mut pool = PoolBytes::new(FULL_LEN)
        .head()
        .fee_on(1)
        .dynamic_fee(0, 0, 0)
        .build();
    pool.tick_arrays.push(stub_tick_array());
    let err = pool
        .quote(1_000_000, false)
        .expect_err("fee_on != FromInput");
    match err {
        ArbError::InvalidInput(msg) => {
            assert!(msg.contains("fee_on = 1"), "unexpected: {msg}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn quote_rejects_unprovable_dynamic_fee_state() {
    let mut pool = PoolBytes::new(1_000).head().fee_on(0).build();
    pool.tick_arrays.push(stub_tick_array());
    let err = pool
        .quote(1_000_000, false)
        .expect_err("dynamic-fee state unknown");
    match err {
        ArbError::InvalidInput(msg) => {
            assert!(msg.contains("dynamic-fee state"), "unexpected: {msg}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn quote_rejects_short_account_fee_side() {
    let mut pool = PoolBytes::new(350).head().build();
    pool.tick_arrays.push(stub_tick_array());
    let err = pool.quote(1_000_000, false).expect_err("fee side unknown");
    match err {
        ArbError::InvalidInput(msg) => {
            assert!(msg.contains("fee side"), "unexpected: {msg}");
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn quote_rejects_total_fee_rate_above_u16_domain() {
    // The total (base + dynamic) fee is capped at 100_000 ppm by Raydium.
    let mut pool = PoolBytes::new(FULL_LEN)
        .head()
        .fee_on(0)
        .dynamic_fee(99_999, u32::MAX, u32::MAX)
        .build();
    pool.tick_arrays.push(stub_tick_array());
    let err = pool
        .quote(1_000_000, false)
        .expect_err("total fee out of u16 domain");
    match err {
        ArbError::InvalidInput(msg) => {
            assert!(
                msg.contains("exceeds the current u16 quote-math domain"),
                "unexpected: {msg}"
            );
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
    // clmm_state fails closed when the bounded fee cannot fit u16.
    assert!(pool.clmm_state().is_none());
    // fee_bps still reports the rounded-up bps for observability.
    assert_eq!(pool.fee_bps(), Some(1_000));
}

#[test]
fn quote_succeeds_with_proven_from_input_fee_side() {
    let mut pool = PoolBytes::new(FULL_LEN)
        .head()
        .fee_on(0)
        .dynamic_fee(0, 0, 0)
        .build();
    pool.tick_arrays.push(stub_tick_array());
    let out = pool
        .quote(1_000, false)
        .expect("quote flows through the gates");
    assert!(out > 0);
}

fn stub_tick_array() -> TickArrayFacade {
    TickArrayFacade {
        start_tick_index: 0,
        ticks: (0..60)
            .map(|i| TickFacade {
                tick_index: i,
                initialized: false,
                liquidity_net: 0,
                liquidity_gross: 0,
                limit_order_unfilled: 0,
            })
            .collect(),
    }
}
