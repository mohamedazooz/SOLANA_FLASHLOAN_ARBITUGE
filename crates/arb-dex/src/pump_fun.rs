//! Pump.fun Bonding Curve State Deserializer and Quoting Engine.
//!
//! Handles the virtual Constant-Product curve `(x + dx)(y - dy) = k` with an explicit
//! fee rate supplied from the current Pump Fees/GlobalConfig account state. Enforces strict bounds checking against `real_token_reserves`,
//! `real_sol_reserves`, and the `complete` (graduation) flag.

use crate::traits::AmmPool;
use arb_constants::{PUMP_FUN_BONDING_CURVE_DISCRIMINATOR, PUMP_FUN_GRADUATION_LAMPORTS};
use arb_math::cpmm_out_fee;
use arb_types::{ArbError, DexProtocol, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

/// Current Pump.fun BondingCurve account length: discriminator + 5 u64s + bool + Pubkey + 2 bools.
pub const PUMP_FUN_BONDING_CURVE_SIZE: usize = 83;

/// Pump.fun Bonding Curve Pool representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpFunBondingCurve {
    pub pool_id: PoolId,
    pub base_mint: MintId, // Quote asset mint (currently WSOL-only in this adapter)
    pub quote_mint: MintId, // Meme token mint
    pub virtual_token_reserves: u64,
    pub virtual_sol_reserves: u64,
    pub real_token_reserves: u64,
    pub real_sol_reserves: u64,
    pub token_total_supply: u64,
    pub complete: bool,
    pub creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    pub mode_flags_present: bool,
    /// Fee rate must come from the current Pump Fees/GlobalConfig state.
    pub fee_bps: Option<u16>,
    /// Token decimals must come from the mint account snapshot.
    pub token_decimals: Option<u8>,
}

impl PumpFunBondingCurve {
    /// Construct a new uninitialized Pump.fun pool instance.
    pub fn new(pool_id: PoolId, base_mint: MintId, quote_mint: MintId) -> Self {
        Self {
            pool_id,
            base_mint,
            quote_mint,
            virtual_token_reserves: 0,
            virtual_sol_reserves: 0,
            real_token_reserves: 0,
            real_sol_reserves: 0,
            token_total_supply: 0,
            complete: false,
            creator: Pubkey::default(),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            mode_flags_present: false,
            fee_bps: None,
            token_decimals: None,
        }
    }

    /// Construct a Pump.fun pool instance from on-chain PumpFunState.
    pub fn from_state(
        pool_id: PoolId,
        base_mint: MintId,
        quote_mint: MintId,
        state: &arb_types::PumpFunState,
    ) -> Self {
        Self {
            pool_id,
            base_mint,
            quote_mint,
            virtual_token_reserves: state.virtual_token_reserves,
            virtual_sol_reserves: state.virtual_sol_reserves,
            real_token_reserves: state.real_token_reserves,
            real_sol_reserves: state.real_sol_reserves,
            token_total_supply: state.token_total_supply,
            complete: state.complete,
            creator: state.creator,
            is_mayhem_mode: state.is_mayhem_mode,
            is_cashback_coin: state.is_cashback_coin,
            mode_flags_present: state.mode_flags_present,
            fee_bps: None,
            token_decimals: None,
        }
    }

    pub fn from_state_with_fee_bps(
        pool_id: PoolId,
        base_mint: MintId,
        quote_mint: MintId,
        state: &arb_types::PumpFunState,
        fee_bps: u16,
    ) -> Result<Self> {
        Self::from_state(pool_id, base_mint, quote_mint, state).with_fee_bps(fee_bps)
    }

    pub fn with_fee_bps(mut self, fee_bps: u16) -> Result<Self> {
        if fee_bps > 10_000 {
            return Err(ArbError::InvalidInput("Pump.fun fee exceeds 100%".into()));
        }
        self.fee_bps = Some(fee_bps);
        Ok(self)
    }

    pub fn with_token_decimals(mut self, decimals: u8) -> Self {
        self.token_decimals = Some(decimals);
        self
    }

    pub fn with_mode_flags(mut self, is_mayhem_mode: bool, is_cashback_coin: bool) -> Self {
        self.is_mayhem_mode = is_mayhem_mode;
        self.is_cashback_coin = is_cashback_coin;
        self.mode_flags_present = true;
        self
    }

    /// Convert to canonical PumpFunState.
    pub fn to_state(&self) -> arb_types::PumpFunState {
        arb_types::PumpFunState {
            virtual_token_reserves: self.virtual_token_reserves,
            virtual_sol_reserves: self.virtual_sol_reserves,
            real_token_reserves: self.real_token_reserves,
            real_sol_reserves: self.real_sol_reserves,
            token_total_supply: self.token_total_supply,
            complete: self.complete,
            creator: self.creator,
            is_mayhem_mode: self.is_mayhem_mode,
            is_cashback_coin: self.is_cashback_coin,
            mode_flags_present: self.mode_flags_present,
        }
    }

    /// Decode the current Pump.fun BondingCurve account (Anchor discriminator + 75-byte body).
    /// Legacy/truncated layouts are rejected because they do not carry creator/mode flags.
    pub fn decode_account_data(data: &[u8]) -> Result<arb_types::PumpFunState> {
        if data.len() < PUMP_FUN_BONDING_CURVE_SIZE
            || data.get(..8) != Some(PUMP_FUN_BONDING_CURVE_DISCRIMINATOR.as_slice())
        {
            return Err(ArbError::DeserializationError(format!(
                "invalid or truncated Pump.fun BondingCurve: expected current {}-byte layout, got {}",
                PUMP_FUN_BONDING_CURVE_SIZE,
                data.len()
            )));
        }
        let read_u64 = |offset: usize| -> Result<u64> {
            let bytes = data.get(offset..offset + 8).ok_or_else(|| {
                ArbError::DeserializationError("truncated Pump.fun BondingCurve u64".into())
            })?;
            Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| {
                ArbError::DeserializationError("invalid Pump.fun BondingCurve u64".into())
            })?))
        };
        let read_bool = |offset: usize| -> Result<bool> {
            match data.get(offset).copied() {
                Some(0) => Ok(false),
                Some(1) => Ok(true),
                _ => Err(ArbError::DeserializationError(
                    "invalid Pump.fun BondingCurve bool".into(),
                )),
            }
        };
        let creator_bytes: [u8; 32] = data
            .get(49..81)
            .ok_or_else(|| ArbError::DeserializationError("truncated Pump.fun creator".into()))?
            .try_into()
            .map_err(|_| ArbError::DeserializationError("invalid Pump.fun creator".into()))?;
        Ok(arb_types::PumpFunState {
            virtual_token_reserves: read_u64(8)?,
            virtual_sol_reserves: read_u64(16)?,
            real_token_reserves: read_u64(24)?,
            real_sol_reserves: read_u64(32)?,
            token_total_supply: read_u64(40)?,
            complete: read_bool(48)?,
            creator: Pubkey::new_from_array(creator_bytes),
            is_mayhem_mode: read_bool(81)?,
            is_cashback_coin: read_bool(82)?,
            mode_flags_present: true,
        })
    }

    /// Bonding curve progress towards graduation in basis points [0..10_000].
    pub fn progress_bps(&self) -> u16 {
        if self.complete {
            return 10_000;
        }
        if self.real_sol_reserves == 0 {
            return 0;
        }
        let bps =
            (self.real_sol_reserves as u128 * 10_000) / (PUMP_FUN_GRADUATION_LAMPORTS as u128);
        bps.min(10_000) as u16
    }

    /// Exact-in quote using a fee rate supplied by the current on-chain Pump Fees
    /// configuration. A missing fee rate is a hard error; no static 1.25% fallback.
    /// Fees are denominated in the quote asset.
    pub fn quote_with_fee(&self, amount_in: u64, a_to_b: bool) -> Result<(u64, u64)> {
        let fee_bps = self.fee_bps.ok_or_else(|| {
            ArbError::InvalidInput(
                "Pump.fun quote requires an explicit current fee schedule".into(),
            )
        })?;
        self.quote_with_fee_bps(amount_in, a_to_b, fee_bps)
    }

    pub fn quote_with_fee_bps(
        &self,
        amount_in: u64,
        a_to_b: bool,
        fee_bps: u16,
    ) -> Result<(u64, u64)> {
        if fee_bps > 10_000 {
            return Err(ArbError::InvalidInput("Pump.fun fee exceeds 100%".into()));
        }
        if !self.mode_flags_present {
            return Err(ArbError::InvalidInput(
                "Pump.fun current mode flags are missing".into(),
            ));
        }
        if self.token_decimals.is_none() {
            return Err(ArbError::InvalidInput(
                "Pump.fun mint decimals are missing".into(),
            ));
        }
        if self.is_mayhem_mode || self.is_cashback_coin {
            return Err(ArbError::InvalidInput(
                "Pump.fun mayhem/cashback curves are not supported by this quote adapter".into(),
            ));
        }
        if self.complete {
            return Err(ArbError::Internal(
                "Pump.fun bonding curve is complete (migrated)".into(),
            ));
        }
        if amount_in == 0 {
            return Err(ArbError::InvalidInput("zero Pump.fun input".into()));
        }
        if self.virtual_sol_reserves == 0 || self.virtual_token_reserves == 0 {
            return Err(ArbError::Internal(
                "Pump.fun virtual reserves are zero".into(),
            ));
        }
        let fee_rate = fee_bps as u128;
        let (out, fee) = if a_to_b {
            let fee = (amount_in as u128)
                .checked_mul(fee_rate)
                .ok_or(ArbError::Overflow)?
                / 10_000;
            let net_quote_in = (amount_in as u128)
                .checked_sub(fee)
                .ok_or(ArbError::Overflow)?;
            let out = cpmm_out_fee(
                net_quote_in,
                self.virtual_sol_reserves as u128,
                self.virtual_token_reserves as u128,
                10_000,
                10_000,
            )
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| ArbError::Internal("Pump.fun buy quote failed".into()))?;
            if out > self.real_token_reserves {
                return Err(ArbError::PumpFunReserveExceeded {
                    requested: out,
                    available: self.real_token_reserves,
                });
            }
            (out, u64::try_from(fee).map_err(|_| ArbError::Overflow)?)
        } else {
            let gross = cpmm_out_fee(
                amount_in as u128,
                self.virtual_token_reserves as u128,
                self.virtual_sol_reserves as u128,
                10_000,
                10_000,
            )
            .and_then(|v| u64::try_from(v).ok())
            .ok_or_else(|| ArbError::Internal("Pump.fun sell quote failed".into()))?;
            if gross > self.real_sol_reserves {
                return Err(ArbError::PumpFunReserveExceeded {
                    requested: gross,
                    available: self.real_sol_reserves,
                });
            }
            let fee = (gross as u128)
                .checked_mul(fee_rate)
                .ok_or(ArbError::Overflow)?
                / 10_000;
            let net = (gross as u128).checked_sub(fee).ok_or(ArbError::Overflow)?;
            (
                u64::try_from(net).map_err(|_| ArbError::Overflow)?,
                u64::try_from(fee).map_err(|_| ArbError::Overflow)?,
            )
        };
        if out == 0 {
            return Err(ArbError::Internal(
                "Pump.fun quote has no output liquidity".into(),
            ));
        }
        Ok((out, fee))
    }

    /// Convenience quote returning only the received output amount.
    pub fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        self.quote_with_fee(amount_in, a_to_b).map(|(out, _)| out)
    }

    /// Convert to PumpFunParams for cross-venue arbitrage math.
    pub fn to_pump_fun_params(&self) -> arb_math::pump_fun_raydium::PumpFunParams {
        arb_math::pump_fun_raydium::PumpFunParams {
            reserve: self.virtual_sol_reserves as u128,
            supply: self.virtual_token_reserves as u128,
        }
    }

    /// Get real reserves (for liquidity constraints).
    pub fn real_reserves(&self) -> (u64, u64) {
        (self.real_sol_reserves, self.real_token_reserves)
    }

    /// Check if this pool is suitable for cross-venue arbitrage.
    pub fn is_arbitrage_eligible(&self) -> bool {
        !self.complete
            && !self.is_mayhem_mode
            && !self.is_cashback_coin
            && self.mode_flags_present
            && self.virtual_sol_reserves > 0
            && self.virtual_token_reserves > 0
            && self.real_sol_reserves > 0
            && self.real_token_reserves > 0
    }

    /// Calculate maximum tradable amount for arbitrage.
    pub fn max_arbitrage_amount(&self) -> u64 {
        std::cmp::min(self.real_sol_reserves, self.real_token_reserves)
    }
}

impl AmmPool for PumpFunBondingCurve {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::PumpFun
    }

    fn base_mint(&self) -> MintId {
        self.base_mint
    }

    fn quote_mint(&self) -> MintId {
        self.quote_mint
    }

    fn reserves(&self) -> (u128, u128) {
        (
            self.virtual_sol_reserves as u128,
            self.virtual_token_reserves as u128,
        )
    }

    fn fee_bps(&self) -> Option<u16> {
        self.fee_bps
    }

    fn decimals(&self) -> (u8, u8) {
        // Unknown token decimals are represented as 0; callers must use the mint snapshot.
        (9, self.token_decimals.unwrap_or(0))
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        let state = Self::decode_account_data(data)?;
        self.virtual_token_reserves = state.virtual_token_reserves;
        self.virtual_sol_reserves = state.virtual_sol_reserves;
        self.real_token_reserves = state.real_token_reserves;
        self.real_sol_reserves = state.real_sol_reserves;
        self.token_total_supply = state.token_total_supply;
        self.complete = state.complete;
        self.creator = state.creator;
        self.is_mayhem_mode = state.is_mayhem_mode;
        self.is_cashback_coin = state.is_cashback_coin;
        self.mode_flags_present = state.mode_flags_present;
        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        PumpFunBondingCurve::quote(self, amount_in, a_to_b)
    }

    fn pump_fun_state(&self) -> Option<arb_types::PumpFunState> {
        Some(self.to_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_pump_fun_decode_and_quote() {
        let pool_id = PoolId(Pubkey::new_unique());
        let wsol = MintId(Pubkey::new_unique());
        let meme = MintId(Pubkey::new_unique());

        let mut pool = PumpFunBondingCurve::new(pool_id, wsol, meme);

        // Current 83-byte Anchor BondingCurve account including creator and mode flags.
        let mut data = Vec::new();
        data.extend_from_slice(&PUMP_FUN_BONDING_CURVE_DISCRIMINATOR);
        data.extend_from_slice(&1_073_000_000_000_000u64.to_le_bytes());
        data.extend_from_slice(&30_000_000_000u64.to_le_bytes());
        data.extend_from_slice(&793_100_000_000_000u64.to_le_bytes());
        data.extend_from_slice(&5_000_000_000u64.to_le_bytes());
        data.extend_from_slice(&1_000_000_000_000_000u64.to_le_bytes());
        data.push(0);
        data.extend_from_slice(Pubkey::new_unique().as_ref());
        data.push(0);
        data.push(0);

        pool.update_from_account(&data)
            .expect("decode should succeed");

        assert_eq!(pool.virtual_sol_reserves, 30_000_000_000);
        assert_eq!(pool.virtual_token_reserves, 1_073_000_000_000_000);
        assert_eq!(pool.real_sol_reserves, 5_000_000_000);
        assert!(!pool.complete);
        assert!(pool.mode_flags_present);
        assert!(!pool.is_mayhem_mode);
        assert!(!pool.is_cashback_coin);
        assert_ne!(pool.creator, Pubkey::default());
        assert_eq!(pool.fee_bps(), None);
        assert!(pool.quote_with_fee(1_000_000_000, true).is_err());
        pool = pool.with_fee_bps(125).unwrap().with_token_decimals(6);
        assert_eq!(pool.fee_bps(), Some(125));

        // Test BUY: 1 SOL gross in, with the verified quote-asset fee deducted from input.
        let sol_in = 1_000_000_000u64;
        let (tokens_out, buy_fee) = pool
            .quote_with_fee(sol_in, true)
            .expect("quote buy should succeed");
        assert_eq!(buy_fee, 12_500_000);
        assert!(tokens_out > 0);
        assert!(tokens_out < pool.real_token_reserves);

        // Test SELL: tokens in
        let (sol_out, sell_fee) = pool
            .quote_with_fee(tokens_out, false)
            .expect("quote sell should succeed");
        assert!(sol_out > 0);
        assert!(sell_fee > 0);
        // Due to 1.25% fee on buy and 1.25% fee on sell, sol_out must be less than sol_in
        assert!(sol_out < sol_in);

        // Unsupported curve modes must not be quoted by this adapter.
        let mayhem = pool.clone().with_mode_flags(true, false);
        assert!(mayhem.quote_with_fee(sol_in, true).is_err());
        let cashback = pool.clone().with_mode_flags(false, true);
        assert!(cashback.quote_with_fee(sol_in, true).is_err());

        // Test completed curve rejection
        pool.complete = true;
        assert!(pool.quote(sol_in, true).is_err());
    }
}

#[cfg(test)]
mod cross_venue_tests {
    use super::*;
    use arb_math::pump_fun_raydium::{
        DEFAULT_FLASH_LOAN_FEE_BPS, RaydiumParams, calculate_optimal_trade,
    };
    use solana_pubkey::Pubkey;

    #[test]
    fn test_cross_venue_arbitrage_params() {
        let pool_id = PoolId(Pubkey::new_unique());
        let wsol = MintId(Pubkey::new_unique());
        let meme = MintId(Pubkey::new_unique());

        let mut data = Vec::new();
        data.extend_from_slice(&PUMP_FUN_BONDING_CURVE_DISCRIMINATOR);
        // virtual_token_reserves = 1_073_000_000_000_000
        data.extend_from_slice(&1_073_000_000_000_000u64.to_le_bytes());
        // virtual_sol_reserves = 30_000_000_000
        data.extend_from_slice(&30_000_000_000u64.to_le_bytes());
        // real_token_reserves = 793_100_000_000_000
        data.extend_from_slice(&793_100_000_000_000u64.to_le_bytes());
        // real_sol_reserves = 5_000_000_000
        data.extend_from_slice(&5_000_000_000u64.to_le_bytes());
        // token_total_supply = 1_000_000_000_000_000
        data.extend_from_slice(&1_000_000_000_000_000u64.to_le_bytes());
        data.push(0); // complete = false
        data.extend_from_slice(Pubkey::new_unique().as_ref()); // creator
        data.push(0); // is_mayhem_mode = false
        data.push(0); // is_cashback_coin = false

        let mut pool = PumpFunBondingCurve::new(pool_id, wsol, meme);
        pool.update_from_account(&data).unwrap();
        pool = pool.with_fee_bps(125).unwrap().with_token_decimals(6);

        // Convert to params
        let pump_params = pool.to_pump_fun_params();

        // Create Raydium params (higher price for arbitrage)
        let raydium_params = RaydiumParams {
            token_reserve: 500_000 * 1_000_000,  // 500K tokens (6 decimals)
            sol_reserve: 15_000 * 1_000_000_000, // 15,000 SOL
            fee_bps: 25,                         // 0.25%
        };

        // Calculate optimal trade
        let max_delta_t = pool.max_arbitrage_amount();
        let result = calculate_optimal_trade(
            pump_params,
            raydium_params,
            DEFAULT_FLASH_LOAN_FEE_BPS,
            max_delta_t as u128,
        );

        assert!(result.is_ok(), "Should find arbitrage opportunity");
        let trade = result.unwrap();
        assert!(trade.is_profitable());
    }

    #[test]
    fn test_arbitrage_eligibility() {
        let pool_id = PoolId(Pubkey::new_unique());
        let wsol = MintId(Pubkey::new_unique());
        let meme = MintId(Pubkey::new_unique());

        let pool = PumpFunBondingCurve::new(pool_id, wsol, meme);

        // Without liquidity, not eligible
        assert!(!pool.is_arbitrage_eligible());

        // With liquidity
        let mut data = Vec::new();
        data.extend_from_slice(&PUMP_FUN_BONDING_CURVE_DISCRIMINATOR);
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // virtual_token_reserves
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // virtual_sol_reserves
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // real_token_reserves
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // real_sol_reserves
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // token_total_supply
        data.push(0); // complete = false
        data.extend_from_slice(Pubkey::new_unique().as_ref()); // creator
        data.push(0); // is_mayhem_mode = false
        data.push(0); // is_cashback_coin = false

        let mut pool = PumpFunBondingCurve::new(pool_id, wsol, meme);
        pool.update_from_account(&data).unwrap();
        pool = pool.with_fee_bps(125).unwrap().with_token_decimals(6);

        assert!(pool.is_arbitrage_eligible());

        // Mark as complete (migrated)
        pool.complete = true;
        assert!(!pool.is_arbitrage_eligible());

        // Mark as mayhem mode
        pool.complete = false;
        pool.is_mayhem_mode = true;
        assert!(!pool.is_arbitrage_eligible());
    }
}
