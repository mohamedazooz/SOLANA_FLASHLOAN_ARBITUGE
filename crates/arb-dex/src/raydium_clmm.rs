//! Raydium CLMM (`CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`) Account Deserializer and Quoting.

use crate::traits::AmmPool;
use arb_constants::RAYDIUM_CLMM_PROGRAM_ID;
use arb_math::clmm::{
    ClmmTickMath, TickArrayFacade, TickFacade, sqrt_price_x64_in_raydium_clmm_bounds,
};
use arb_types::{
    ArbError, ClmmState, ClmmTick, ClmmTickArray, DexProtocol, MintId, PoolExecutionAccounts,
    PoolId, Result, TokenProgramKind,
};
use ruint::aliases::U256;
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const RAYDIUM_CLMM_POOL_MIN_SIZE: usize = 300;
pub const RAYDIUM_CLMM_TICK_ARRAY_SIZE: usize = 60;
pub const RAYDIUM_CLMM_TICK_STATE_SIZE: usize = 168;

/// `PoolState.status` byte (bit 4 disables swaps), account offset.
pub const RAYDIUM_CLMM_STATUS_OFFSET: usize = 389;
/// `PoolState.fee_on` byte (0 = FromInput, 1 = Token0Only, 2 = Token1Only),
/// account offset. Official layout (`states/pool.rs`): after `status`.
pub const RAYDIUM_CLMM_FEE_ON_OFFSET: usize = 390;
/// `PoolState.dynamic_fee_info` (`DynamicFeeInfo`, 80 bytes), account offset.
/// All-zero means dynamic fee is disabled; anything else means the effective
/// swap fee is `trade_fee_rate + dynamic_fee_rate`, which depends on live
/// volatility and cannot be derived from the pool snapshot alone.
pub const RAYDIUM_CLMM_DYNAMIC_FEE_INFO_OFFSET: usize = 1096;
pub const RAYDIUM_CLMM_DYNAMIC_FEE_INFO_LEN: usize = 80;
pub const RAYDIUM_CLMM_TICK_ARRAY_MIN_SIZE: usize =
    44 + (RAYDIUM_CLMM_TICK_ARRAY_SIZE * RAYDIUM_CLMM_TICK_STATE_SIZE);

#[derive(Debug, Clone)]
pub struct RaydiumClmmPool {
    pub pool_id: PoolId,
    pub amm_config: Pubkey,
    pub token_mint_0: MintId,
    pub token_mint_1: MintId,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub token_0_program: TokenProgramKind,
    pub token_1_program: TokenProgramKind,
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    /// Fee rate in `1e6` precision (from `AmmConfig.trade_fee_rate`), e.g. `2500` = 25 bps.
    pub fee_rate_ppm: u32,
    pub status: u8,
    /// Official `PoolState::fee_on` (`CollectFeeOn`): 0 = FromInput,
    /// 1 = Token0Only, 2 = Token1Only. `None` when the account bytes are too
    /// short to prove the field (fail-closed: quote refuses unknown fee side).
    /// Non-zero modes charge the fee on the OUTPUT token in one direction;
    /// that math is not implemented, so those directions fail closed instead
    /// of quoting a fee-on-input approximation.
    pub fee_on: Option<u8>,
    /// Official `PoolState::dynamic_fee_info` (offset 1096..1176, verified
    /// 2026-11-20 against the deployed program source). `None` when the
    /// account bytes are too short to prove the dynamic fee is disabled.
    pub dynamic_fee_control: Option<u32>,
    pub max_volatility_accumulator: Option<u32>,
    /// Non-config inputs are informational only (the swap can raise the
    /// accumulator mid-swap); the fee quote uses the max-accumulator bound.
    pub volatility_accumulator: u32,
    pub tick_arrays: Vec<TickArrayFacade>,
    pub tick_array_pdas: Vec<Pubkey>,
    pub bitmap_extension: Option<Pubkey>,
    pub transfer_fee_0_bps: u16,
    pub transfer_fee_1_bps: u16,
    pub has_risky: bool,
}

impl RaydiumClmmPool {
    pub fn from_account(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        let mut pool = Self {
            pool_id,
            amm_config: Pubkey::default(),
            token_mint_0: MintId::default(),
            token_mint_1: MintId::default(),
            token_vault_0: Pubkey::default(),
            token_vault_1: Pubkey::default(),
            observation_key: Pubkey::default(),
            token_0_program: TokenProgramKind::SplToken,
            token_1_program: TokenProgramKind::SplToken,
            mint_decimals_0: 0,
            mint_decimals_1: 0,
            tick_spacing: 1,
            liquidity: 0,
            sqrt_price_x64: 0,
            tick_current: 0,
            fee_rate_ppm: 2_500, // Updated by AmmConfig dependency
            status: 0,
            fee_on: None,
            dynamic_fee_control: None,
            max_volatility_accumulator: None,
            volatility_accumulator: 0,
            tick_arrays: Vec::new(),
            tick_array_pdas: Vec::new(),
            bitmap_extension: None,
            transfer_fee_0_bps: 0,
            transfer_fee_1_bps: 0,
            has_risky: false,
        };
        pool.update_from_account(data)?;
        Ok(pool)
    }

    pub fn derive_tick_array_pda(&self, start_tick_index: i32) -> Pubkey {
        let prog = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap_or_default();
        let (pda, _) = Pubkey::find_program_address(
            &[
                arb_constants::RAYDIUM_CLMM_TICK_ARRAY_SEED,
                self.pool_id.0.as_ref(),
                &start_tick_index.to_be_bytes(),
            ],
            &prog,
        );
        pda
    }

    pub fn tick_array_pdas_around_current(&self) -> Vec<Pubkey> {
        let spacing = i32::from(self.tick_spacing.max(1));
        let span = (RAYDIUM_CLMM_TICK_ARRAY_SIZE as i32) * spacing;
        if span == 0 {
            return Vec::new();
        }
        let start = self.tick_current.div_euclid(span) * span;
        [start - span, start, start + span]
            .into_iter()
            .map(|s| self.derive_tick_array_pda(s))
            .collect()
    }

    /// Parse Raydium CLMM `TickArrayState` account bytes (60 ticks per array).
    pub fn parse_tick_array_state(data: &[u8]) -> Result<TickArrayFacade> {
        // Discriminator (8) + pool_id (32) + start_tick_index (4) = 44 bytes header.
        if data.len() < RAYDIUM_CLMM_TICK_ARRAY_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium CLMM TickArrayState len {} < {}",
                data.len(),
                RAYDIUM_CLMM_TICK_ARRAY_MIN_SIZE
            )));
        }
        let start_tick_index = i32::from_le_bytes(data[40..44].try_into().unwrap());
        let mut ticks = Vec::with_capacity(RAYDIUM_CLMM_TICK_ARRAY_SIZE);
        let mut offset = 44usize;
        for _ in 0..RAYDIUM_CLMM_TICK_ARRAY_SIZE {
            let tick = i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
            let liquidity_net =
                i128::from_le_bytes(data[offset + 4..offset + 20].try_into().unwrap());
            let liquidity_gross =
                u128::from_le_bytes(data[offset + 20..offset + 36].try_into().unwrap());
            // TickState limit-order fields (VERIFIED layout, checked 2026-11-20).
            let orders_amount =
                u64::from_le_bytes(data[offset + 124..offset + 132].try_into().unwrap());
            let part_filled_remaining =
                u64::from_le_bytes(data[offset + 132..offset + 140].try_into().unwrap());
            let limit_order_unfilled = orders_amount
                .checked_add(part_filled_remaining)
                .ok_or(ArbError::Overflow)?;
            ticks.push(TickFacade {
                tick_index: tick,
                // Official is_initialized(): liquidity OR limit orders.
                initialized: liquidity_gross > 0 || limit_order_unfilled > 0,
                liquidity_net,
                liquidity_gross,
                limit_order_unfilled,
            });
            offset += RAYDIUM_CLMM_TICK_STATE_SIZE;
        }
        Ok(TickArrayFacade {
            start_tick_index,
            ticks,
        })
    }

    /// Worst-case dynamic fee rate (denominator `1e6`) this pool can charge in
    /// the current account epoch: the exact on-chain formula evaluated at
    /// `max_volatility_accumulator` (fee is monotonic in the volatility
    /// accumulator). `None` when the account bytes are too short to prove the
    /// dynamic-fee configuration (fail-closed).
    pub fn dynamic_fee_rate_upper_bound(&self) -> Option<u32> {
        Some(arb_math::raydium_clmm_dynamic_fee_rate_upper_bound(
            self.dynamic_fee_control?,
            self.max_volatility_accumulator?,
            self.tick_spacing,
        ))
    }

    /// `base_fee_rate + worst-case dynamic` total fee rate (denominator `1e6`),
    /// matching on-chain `get_total_fee_rate` under the max-accumulator bound.
    pub fn total_fee_rate_upper_bound(&self) -> Option<u64> {
        Some(u64::from(self.fee_rate_ppm) + u64::from(self.dynamic_fee_rate_upper_bound()?))
    }

    pub fn apply_amm_config_data(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 52 {
            return Err(ArbError::DeserializationError(
                "Raydium CLMM AmmConfig truncated".into(),
            ));
        }
        // AmmConfig: disc(8) + bump(1) + index(2) + owner(32) + protocol_fee_rate(4) + trade_fee_rate(4 @47..51)
        let trade_fee_rate = u32::from_le_bytes(data[47..51].try_into().unwrap());
        if trade_fee_rate <= 100_000 {
            self.fee_rate_ppm = trade_fee_rate;
        }
        Ok(())
    }
}

impl AmmPool for RaydiumClmmPool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::RaydiumClmm
    }

    fn base_mint(&self) -> MintId {
        self.token_mint_0
    }

    fn quote_mint(&self) -> MintId {
        self.token_mint_1
    }

    fn reserves(&self) -> (u128, u128) {
        if self.liquidity == 0 || self.sqrt_price_x64 == 0 {
            return (0, 0);
        }
        let l = U256::from(self.liquidity);
        let sp = U256::from(self.sqrt_price_x64);
        let x = (l << 64usize) / sp;
        let y = (l * sp) >> 64usize;
        (
            u128::try_from(x).unwrap_or(u128::MAX),
            u128::try_from(y).unwrap_or(u128::MAX),
        )
    }

    fn fee_bps(&self) -> Option<u16> {
        let total = self.total_fee_rate_upper_bound()?;
        // Round UP to basis points: understating fees would overstate quotes.
        u16::try_from(total.div_ceil(100)).ok()
    }

    fn clmm_state(&self) -> Option<ClmmState> {
        // Token0/Token1-only fee collection is direction-dependent: a
        // direction-free `ClmmState` cannot express it, so exclude those pools
        // from sizing/routing instead of feeding wrong economics into the
        // engine. Unprovable fee side is equally excluded (fail-closed).
        if self.fee_on != Some(0) {
            return None;
        }
        Some(ClmmState {
            sqrt_price: self.sqrt_price_x64,
            liquidity: self.liquidity,
            tick_current: self.tick_current,
            tick_spacing: self.tick_spacing,
            // Fail-closed: use the base+dynamic upper bound and never clamp a
            // fee rate into the u16 domain (a clamped 10% fee would silently
            // undercharge and overstate out).
            fee_rate: u16::try_from(self.total_fee_rate_upper_bound()?).ok()?,
            tick_arrays: self
                .tick_arrays
                .iter()
                .map(|ta| ClmmTickArray {
                    start_tick_index: ta.start_tick_index,
                    ticks: ta
                        .ticks
                        .iter()
                        .filter(|t| t.initialized)
                        .map(|t| ClmmTick {
                            tick_index: t.tick_index,
                            initialized: true,
                            liquidity_net: t.liquidity_net,
                            limit_order_unfilled: t.limit_order_unfilled,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    fn decimals(&self) -> (u8, u8) {
        (self.mint_decimals_0, self.mint_decimals_1)
    }

    fn token_programs(&self) -> [TokenProgramKind; 2] {
        [self.token_0_program, self.token_1_program]
    }

    fn transfer_fees_bps(&self) -> (u16, u16) {
        (self.transfer_fee_0_bps, self.transfer_fee_1_bps)
    }

    fn has_risky_token(&self) -> bool {
        self.has_risky
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.amm_config == Pubkey::default()
            || self.token_vault_0 == Pubkey::default()
            || self.token_vault_1 == Pubkey::default()
        {
            return None;
        }
        let pdas = if self.tick_array_pdas.is_empty() {
            self.tick_array_pdas_around_current()
        } else {
            self.tick_array_pdas.clone()
        };
        Some(PoolExecutionAccounts::RaydiumClmm {
            amm_config: self.amm_config,
            vault_0: self.token_vault_0,
            vault_1: self.token_vault_1,
            observation_state: self.observation_key,
            tick_array_pdas: pdas,
            bitmap_extension: self.bitmap_extension,
            pool_mint_0: self.token_mint_0.0,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let mut keys = vec![
            self.amm_config,
            self.observation_key,
            self.token_mint_0.0,
            self.token_mint_1.0,
        ];
        keys.extend(self.tick_array_pdas_around_current());
        keys
    }

    fn apply_dependency_account(&mut self, pubkey: &Pubkey, data: &[u8]) -> Result<bool> {
        if *pubkey == self.amm_config {
            self.apply_amm_config_data(data)?;
            return Ok(true);
        }
        if let Ok(ta) = Self::parse_tick_array_state(data) {
            if let Some(existing) = self
                .tick_arrays
                .iter_mut()
                .find(|a| a.start_tick_index == ta.start_tick_index)
            {
                *existing = ta;
            } else {
                self.tick_arrays.push(ta);
            }
            if !self.tick_array_pdas.contains(pubkey) {
                self.tick_array_pdas.push(*pubkey);
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < RAYDIUM_CLMM_POOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium CLMM PoolState len {} < {}",
                data.len(),
                RAYDIUM_CLMM_POOL_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        // Raydium CLMM PoolState layout (after 8-byte Anchor discriminator + 1 bump):
        // 8: bump[1], 9..41: amm_config, 41..73: owner,
        // 73..105: token_mint_0, 105..137: token_mint_1,
        // 137..169: token_vault_0, 169..201: token_vault_1,
        // 201..233: observation_key,
        // 233: mint_decimals_0, 234: mint_decimals_1,
        // 235..237: tick_spacing (u16),
        // 237..253: liquidity (u128),
        // 253..269: sqrt_price_x64 (u128),
        // 269..273: tick_current (i32)
        self.amm_config = read_pk(9)?;
        self.token_mint_0 = MintId(read_pk(73)?);
        self.token_mint_1 = MintId(read_pk(105)?);
        self.token_vault_0 = read_pk(137)?;
        self.token_vault_1 = read_pk(169)?;
        self.observation_key = read_pk(201)?;
        self.mint_decimals_0 = data[233];
        self.mint_decimals_1 = data[234];
        self.tick_spacing = u16::from_le_bytes(data[235..237].try_into().unwrap()).max(1);
        self.liquidity = u128::from_le_bytes(data[237..253].try_into().unwrap());
        self.sqrt_price_x64 = u128::from_le_bytes(data[253..269].try_into().unwrap());
        if self.sqrt_price_x64 != 0 && !sqrt_price_x64_in_raydium_clmm_bounds(self.sqrt_price_x64) {
            return Err(ArbError::InvalidInput(format!(
                "Raydium CLMM sqrt_price_x64 {} outside official program bounds",
                self.sqrt_price_x64
            )));
        }
        self.tick_current = i32::from_le_bytes(data[269..273].try_into().unwrap());
        if data.len() > RAYDIUM_CLMM_STATUS_OFFSET {
            self.status = data[RAYDIUM_CLMM_STATUS_OFFSET];
        }
        // PoolState tail: fee_on @390, dynamic_fee_info @1096..1176 (verified
        // 2026-11-20; total account LEN = 1544). fee_on is readable from
        // offset 391; proving the dynamic-fee state needs the full tail
        // (>= 1122). Shorter accounts cannot prove the fee side or that the
        // dynamic fee is disabled -> remain None and fail closed at quote time.
        if data.len() > RAYDIUM_CLMM_FEE_ON_OFFSET {
            self.fee_on = Some(data[RAYDIUM_CLMM_FEE_ON_OFFSET]);
        }
        if data.len() >= RAYDIUM_CLMM_DYNAMIC_FEE_INFO_OFFSET + 32 {
            self.dynamic_fee_control =
                Some(u32::from_le_bytes(data[1102..1106].try_into().unwrap()));
            self.max_volatility_accumulator =
                Some(u32::from_le_bytes(data[1106..1110].try_into().unwrap()));
            self.volatility_accumulator = u32::from_le_bytes(data[1118..1122].try_into().unwrap());
        }
        self.tick_array_pdas = self.tick_array_pdas_around_current();
        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if self.has_risky {
            return Err(ArbError::InvalidInput("risky Token-2022 extension".into()));
        }
        // Bit 4 set in status disables swapping
        if (self.status & (1 << 4)) != 0 {
            return Err(ArbError::InvalidInput(
                "Raydium CLMM swap disabled in pool status".into(),
            ));
        }
        if self.liquidity == 0 || self.sqrt_price_x64 == 0 || self.tick_arrays.is_empty() {
            return Err(ArbError::InvalidInput(
                "Raydium CLMM missing liquidity or tick arrays".into(),
            ));
        }
        if self.transfer_fee_0_bps != 0 || self.transfer_fee_1_bps != 0 {
            return Err(ArbError::InvalidInput(
                "Raydium CLMM Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        // Fee side: our quote math charges the fee on the input token, which
        // matches the fee-on-input directions only. Official
        // `PoolState::is_fee_on_input(zero_for_one)`: 0 = FromInput -> always
        // true; 1 = Token0Only (zero_for_one -> the fee is in token0 == the
        // input); 2 = Token1Only (fee in token0 == the output for
        // zero_for_one). Fee-on-output uses a different invariant; quoting it
        // with the fee-on-input formula overstates output, so fail closed.
        match self.fee_on {
            None => {
                return Err(ArbError::InvalidInput(
                    "Raydium CLMM account too short to prove fee side (fee_on); reject rather than assume".into(),
                ));
            }
            Some(0) => {}
            Some(1) if a_to_b => {}
            Some(2) if !a_to_b => {}
            Some(other) => {
                return Err(ArbError::InvalidInput(format!(
                    "Raydium CLMM fee_on = {other}: fee-on-output direction for this swap side is not supported; reject rather than overstate output"
                )));
            }
        }
        let total_fee_rate = self.total_fee_rate_upper_bound().ok_or_else(|| {
            ArbError::InvalidInput(
                "Raydium CLMM account too short to prove the dynamic-fee state; reject rather than assume disabled".into(),
            )
        })?;
        let fee_rate = u16::try_from(total_fee_rate).map_err(|_| {
            ArbError::InvalidInput(
                "Raydium CLMM fee rate exceeds the current u16 quote-math domain; reject rather than clamp".into(),
            )
        })?;
        let net_in = amount_in;
        let gross_out = arb_math::clmm::quote_clmm_with_ticks_per_array_and_math(
            net_in,
            self.sqrt_price_x64,
            self.tick_current,
            self.liquidity,
            self.tick_spacing,
            fee_rate,
            a_to_b,
            &self.tick_arrays,
            RAYDIUM_CLMM_TICK_ARRAY_SIZE,
            ClmmTickMath::Raydium,
        )
        .map_err(|e| ArbError::InvalidInput(format!("Raydium CLMM quote error: {e:?}")))?;
        Ok(gross_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_math::clmm::TickFacade;

    /// Build a minimal live-shaped `PoolState` buffer at tick 0.
    fn pool_state_bytes(fee_on: u8, dynamic_fee: bool) -> Vec<u8> {
        let mut data = vec![0u8; 1_544];
        data[235..237].copy_from_slice(&1u16.to_le_bytes()); // tick_spacing
        data[237..253].copy_from_slice(&1_000_000_000_000u128.to_le_bytes()); // liquidity
        data[253..269].copy_from_slice(&(1u128 << 64).to_le_bytes()); // sqrt_price @ tick 0
        data[269..273].copy_from_slice(&0i32.to_le_bytes()); // tick_current
        data[RAYDIUM_CLMM_STATUS_OFFSET] = 0;
        data[RAYDIUM_CLMM_FEE_ON_OFFSET] = fee_on;
        if dynamic_fee {
            data[RAYDIUM_CLMM_DYNAMIC_FEE_INFO_OFFSET] = 1;
        }
        data
    }

    fn pool(fee_on: u8, dynamic_fee: bool) -> RaydiumClmmPool {
        let mut pool = RaydiumClmmPool::from_account(
            PoolId(Pubkey::new_unique()),
            &pool_state_bytes(fee_on, dynamic_fee),
        )
        .expect("valid pool state");
        pool.fee_rate_ppm = 2_500;
        pool.tick_arrays = vec![
            TickArrayFacade {
                start_tick_index: -60,
                ticks: Vec::<TickFacade>::new(),
            },
            TickArrayFacade {
                start_tick_index: 0,
                ticks: Vec::<TickFacade>::new(),
            },
        ];
        pool
    }

    #[test]
    fn fee_on_from_input_quotes_both_directions() {
        let pool = pool(0, false);
        assert!(pool.quote(1_000_000, true).expect("a_to_b quote") > 0);
        assert!(pool.quote(1_000_000, false).expect("b_to_a quote") > 0);
        assert!(pool.clmm_state().is_some());
    }

    #[test]
    fn fee_on_token0_rejects_the_fee_on_output_direction() {
        let pool = pool(1, false);
        // zero_for_one: fee is on token0 == input -> exact.
        assert!(pool.quote(1_000_000, true).expect("fee-on-input side") > 0);
        // one_for_zero: fee is on token0 == output -> must fail closed.
        let error = pool.quote(1_000_000, false).expect_err("must reject");
        assert!(error.to_string().contains("fee-on-output"));
        // `ClmmState` cannot express direction-dependent fees.
        assert!(pool.clmm_state().is_none());
    }

    #[test]
    fn fee_on_token1_rejects_the_fee_on_output_direction() {
        let pool = pool(2, false);
        assert!(pool.quote(1_000_000, false).expect("fee-on-input side") > 0);
        let error = pool.quote(1_000_000, true).expect_err("must reject");
        assert!(error.to_string().contains("fee-on-output"));
        assert!(pool.clmm_state().is_none());
    }

    #[test]
    fn dynamic_fee_pool_is_considered_but_never_inflates_output() {
        // A proven dynamic-fee state (control=10_000, max_va=10_000, ts=1 ->
        // dynamic bound 1 ppm) must not reject the pool outright: the quote
        // proceeds with the max-volatility-accumulator upper bound, so it can
        // only understate (never inflate) received output.
        let mut buf = pool_state_bytes(0, false);
        buf[1102..1106].copy_from_slice(&10_000u32.to_le_bytes());
        buf[1106..1110].copy_from_slice(&10_000u32.to_le_bytes());
        let mut pool = RaydiumClmmPool::from_account(PoolId(Pubkey::new_unique()), &buf)
            .expect("valid pool state");
        pool.fee_rate_ppm = 2_500;
        pool.tick_arrays = vec![
            TickArrayFacade {
                start_tick_index: -60,
                ticks: Vec::<TickFacade>::new(),
            },
            TickArrayFacade {
                start_tick_index: 0,
                ticks: Vec::<TickFacade>::new(),
            },
        ];
        assert_eq!(pool.dynamic_fee_control, Some(10_000));
        assert_eq!(pool.max_volatility_accumulator, Some(10_000));
        // total = 2_500 + 1 = 2501 ppm -> 25.01 bps rounded UP to 26.
        assert_eq!(pool.fee_bps(), Some(26));
        assert!(pool.quote(1_000_000, true).is_ok());
        assert!(pool.quote(1_000_000, false).is_ok());
        assert_eq!(pool.clmm_state().expect("state").fee_rate, 2_501);
    }

    #[test]
    fn fee_rate_beyond_u16_domain_is_not_silently_clamped() {
        let mut pool = pool(0, false);
        pool.fee_rate_ppm = 100_000; // official maximum is 10%
        assert!(pool.quote(1_000_000, true).is_err());
        assert!(pool.clmm_state().is_none());
    }
}
