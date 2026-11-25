//! Raydium CP-Swap (CPMM) Account Deserializer and Exact-In Quoting.

use crate::traits::AmmPool;
use arb_constants::RAYDIUM_CPMM_PROGRAM_ID;
use arb_math::cpmm_out_upfront_fee_ceil;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const RAYDIUM_CPMM_POOL_MIN_SIZE: usize = 637;
pub const RAYDIUM_CPMM_FEE_DENOMINATOR: u128 = 1_000_000;

#[derive(Debug, Clone)]
pub struct RaydiumCpmmPool {
    pub pool_id: PoolId,
    pub amm_config: Pubkey,
    pub authority: Pubkey,
    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,
    pub token_0_mint: MintId,
    pub token_1_mint: MintId,
    pub token_0_program: TokenProgramKind,
    pub token_1_program: TokenProgramKind,
    pub observation_key: Pubkey,
    pub status: u8,
    pub mint_0_decimals: u8,
    pub mint_1_decimals: u8,
    pub reserve_0: u128,
    pub reserve_1: u128,
    pub protocol_fees_0: u64,
    pub protocol_fees_1: u64,
    pub fund_fees_0: u64,
    pub fund_fees_1: u64,
    pub creator_fees_0: u64,
    pub creator_fees_1: u64,
    pub enable_creator_fee: bool,
    /// Official `PoolState.creator_fee_on`: `0` = both tokens, `1` = token0
    /// only, `2` = token1 only. Controls whether the creator fee is taken from
    /// the input token or from the output token for a given direction.
    pub creator_fee_on: u8,
    /// Combined fee rate in 1e6 PPM units (trade_fee + creator_fee).
    pub trade_fee_rate_ppm: u64,
    pub creator_fee_rate_ppm: u64,
    /// True once the `AmmConfig` dependency has been decoded. Until then the
    /// fee schedule is unknown and quoting must fail closed.
    pub amm_config_loaded: bool,
    pub transfer_fee_0_bps: u16,
    pub transfer_fee_1_bps: u16,
    pub has_risky: bool,
}

impl RaydiumCpmmPool {
    pub fn from_account(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID)
            .map_err(|_| ArbError::Internal("invalid RAYDIUM_CPMM_PROGRAM_ID".into()))?;
        let (authority, _) =
            Pubkey::find_program_address(&[arb_constants::RAYDIUM_CPMM_AUTH_SEED], &program_id);
        let mut pool = Self {
            pool_id,
            amm_config: Pubkey::default(),
            authority,
            token_0_vault: Pubkey::default(),
            token_1_vault: Pubkey::default(),
            token_0_mint: MintId::default(),
            token_1_mint: MintId::default(),
            token_0_program: TokenProgramKind::SplToken,
            token_1_program: TokenProgramKind::SplToken,
            observation_key: Pubkey::default(),
            status: 0,
            mint_0_decimals: 0,
            mint_1_decimals: 0,
            reserve_0: 0,
            reserve_1: 0,
            protocol_fees_0: 0,
            protocol_fees_1: 0,
            fund_fees_0: 0,
            fund_fees_1: 0,
            creator_fees_0: 0,
            creator_fees_1: 0,
            enable_creator_fee: false,
            creator_fee_on: 0,
            // No synthesized fee schedule: quoting stays disabled until the
            // pool's `AmmConfig` account has actually been decoded.
            trade_fee_rate_ppm: 0,
            creator_fee_rate_ppm: 0,
            amm_config_loaded: false,
            transfer_fee_0_bps: 0,
            transfer_fee_1_bps: 0,
            has_risky: false,
        };
        pool.update_from_account(data)?;
        Ok(pool)
    }

    pub fn update_reserves(&mut self, vault_0: u64, vault_1: u64) {
        let locked_0 = u128::from(self.protocol_fees_0)
            .saturating_add(u128::from(self.fund_fees_0))
            .saturating_add(u128::from(self.creator_fees_0));
        let locked_1 = u128::from(self.protocol_fees_1)
            .saturating_add(u128::from(self.fund_fees_1))
            .saturating_add(u128::from(self.creator_fees_1));
        self.reserve_0 = u128::from(vault_0).saturating_sub(locked_0);
        self.reserve_1 = u128::from(vault_1).saturating_sub(locked_1);
    }

    /// Decode Raydium CPMM `AmmConfig` account.
    /// trade_fee_rate: offset 12..20 (u64 @ 1e6 ppm).
    /// creator_fee_rate: offset 108..116 (u64 @ 1e6 ppm).
    pub fn apply_amm_config_data(&mut self, data: &[u8]) -> Result<()> {
        const AMM_CONFIG_LEN: usize = 236;
        const AMM_CONFIG_DISCRIMINATOR: [u8; 8] = [0xda, 0xf4, 0x21, 0x68, 0xcb, 0xcb, 0x2b, 0x6f];
        if data.len() < AMM_CONFIG_LEN || data.get(..8) != Some(&AMM_CONFIG_DISCRIMINATOR) {
            return Err(ArbError::DeserializationError(
                "Raydium CPMM AmmConfig has invalid length or discriminator".into(),
            ));
        }
        let trade_fee = u64::from_le_bytes(
            data[12..20]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad amm_config trade fee".into()))?,
        );
        if trade_fee == 0 || trade_fee > 100_000 {
            return Err(ArbError::InvalidInput(format!(
                "Raydium CPMM AmmConfig trade_fee_rate {trade_fee} exceeds the program maximum 100,000 ppm"
            )));
        }
        let protocol_fee = u64::from_le_bytes(data[20..28].try_into().unwrap());
        let fund_fee = u64::from_le_bytes(data[28..36].try_into().unwrap());
        if protocol_fee > 1_000_000 || fund_fee > 1_000_000 || protocol_fee + fund_fee > 1_000_000 {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM AmmConfig protocol/fund fee allocation is invalid".into(),
            ));
        }
        self.trade_fee_rate_ppm = trade_fee;
        {
            let creator_fee = u64::from_le_bytes(data[108..116].try_into().map_err(|_| {
                ArbError::DeserializationError("bad amm_config creator fee".into())
            })?);
            if creator_fee > 50_000 {
                return Err(ArbError::InvalidInput(format!(
                    "Raydium CPMM AmmConfig creator_fee_rate {creator_fee} exceeds the program maximum 50,000 ppm"
                )));
            }
            self.creator_fee_rate_ppm = creator_fee;
        }
        self.trade_fee_rate_ppm = trade_fee;
        self.amm_config_loaded = true;
        Ok(())
    }

    /// Fee charged on the input token for this direction, in 1e6 ppm units.
    ///
    /// Mirrors `PoolState::is_creator_fee_on_input` + `adjust_creator_fee_rate`:
    /// the creator fee is only charged when the pool enables it, and it lands on
    /// the input token only for the directions where `creator_fee_on` says so.
    /// In the remaining directions it is deducted from the *output* amount.
    pub fn creator_fee_on_input(&self, a_to_b: bool) -> Result<bool> {
        if !self.enable_creator_fee || self.creator_fee_rate_ppm == 0 {
            return Ok(true);
        }
        match self.creator_fee_on {
            0 => Ok(true),
            1 => Ok(a_to_b),
            2 => Ok(!a_to_b),
            other => Err(ArbError::InvalidInput(format!(
                "Raydium CPMM creator_fee_on={other} is not a valid CreatorFeeOn variant"
            ))),
        }
    }

    pub fn effective_fee_rate_ppm(&self) -> u64 {
        if self.enable_creator_fee {
            self.trade_fee_rate_ppm
                .saturating_add(self.creator_fee_rate_ppm)
        } else {
            self.trade_fee_rate_ppm
        }
    }
}

impl AmmPool for RaydiumCpmmPool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::RaydiumCpmm
    }

    fn base_mint(&self) -> MintId {
        self.token_0_mint
    }

    fn quote_mint(&self) -> MintId {
        self.token_1_mint
    }

    fn reserves(&self) -> (u128, u128) {
        (self.reserve_0, self.reserve_1)
    }

    fn fee_bps(&self) -> Option<u16> {
        if !self.amm_config_loaded {
            return None;
        }
        // Graph metadata is conservative: any non-zero sub-bps fee rounds up.
        u16::try_from(self.effective_fee_rate_ppm().div_ceil(100))
            .ok()
            .filter(|bps| *bps <= 10_000)
    }
    fn decimals(&self) -> (u8, u8) {
        (self.mint_0_decimals, self.mint_1_decimals)
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
            || self.token_0_vault == Pubkey::default()
            || self.token_1_vault == Pubkey::default()
        {
            return None;
        }
        Some(PoolExecutionAccounts::RaydiumCpmm {
            amm_config: self.amm_config,
            authority: self.authority,
            vault_0: self.token_0_vault,
            vault_1: self.token_1_vault,
            observation_state: self.observation_key,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        vec![
            self.token_0_vault,
            self.token_1_vault,
            self.amm_config,
            self.observation_key,
        ]
    }

    fn apply_dependency_account(&mut self, pubkey: &Pubkey, data: &[u8]) -> Result<bool> {
        if *pubkey == self.token_0_vault && data.len() >= 72 {
            let amt = u64::from_le_bytes(data[64..72].try_into().unwrap());
            let locked = u128::from(self.protocol_fees_0)
                .saturating_add(u128::from(self.fund_fees_0))
                .saturating_add(u128::from(self.creator_fees_0));
            self.reserve_0 = u128::from(amt).saturating_sub(locked);
            return Ok(true);
        }
        if *pubkey == self.token_1_vault && data.len() >= 72 {
            let amt = u64::from_le_bytes(data[64..72].try_into().unwrap());
            let locked = u128::from(self.protocol_fees_1)
                .saturating_add(u128::from(self.fund_fees_1))
                .saturating_add(u128::from(self.creator_fees_1));
            self.reserve_1 = u128::from(amt).saturating_sub(locked);
            return Ok(true);
        }
        if *pubkey == self.amm_config {
            self.apply_amm_config_data(data)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
        if data.len() < RAYDIUM_CPMM_POOL_MIN_SIZE
            || data.get(..8) != Some(&POOL_STATE_DISCRIMINATOR)
        {
            return Err(ArbError::DeserializationError(format!(
                "Raydium CPMM PoolState has invalid length or discriminator ({} bytes)",
                data.len()
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey slice".into()))?;
            let key = Pubkey::new_from_array(b);
            if key == Pubkey::default() {
                return Err(ArbError::DeserializationError(format!(
                    "Raydium CPMM PoolState has default pubkey at offset {off}"
                )));
            }
            Ok(key)
        };
        let t2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default();
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap_or_default();

        self.amm_config = read_pk(8)?;
        self.token_0_vault = read_pk(72)?;
        self.token_1_vault = read_pk(104)?;
        self.token_0_mint = MintId(read_pk(168)?);
        self.token_1_mint = MintId(read_pk(200)?);
        let p0 = read_pk(232)?;
        let p1 = read_pk(264)?;
        if ![spl, t2022].contains(&p0) || ![spl, t2022].contains(&p1) {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM has an unsupported token program id".into(),
            ));
        }
        self.token_0_program = if p0 == t2022 {
            TokenProgramKind::Token2022
        } else {
            TokenProgramKind::SplToken
        };
        self.token_1_program = if p1 == t2022 {
            TokenProgramKind::Token2022
        } else {
            TokenProgramKind::SplToken
        };
        self.observation_key = read_pk(296)?;
        if data[389] > 2 || data[390] > 1 {
            return Err(ArbError::DeserializationError(
                "Raydium CPMM PoolState has invalid creator-fee flags".into(),
            ));
        }
        if data[331] > 18 || data[332] > 18 {
            return Err(ArbError::DeserializationError(
                "Raydium CPMM PoolState has invalid mint decimals".into(),
            ));
        }
        self.status = data[329];
        self.mint_0_decimals = data[331];
        self.mint_1_decimals = data[332];
        self.protocol_fees_0 = u64::from_le_bytes(data[341..349].try_into().unwrap());
        self.protocol_fees_1 = u64::from_le_bytes(data[349..357].try_into().unwrap());
        self.fund_fees_0 = u64::from_le_bytes(data[357..365].try_into().unwrap());
        self.fund_fees_1 = u64::from_le_bytes(data[365..373].try_into().unwrap());
        if data.len() > 389 {
            self.creator_fee_on = data[389];
        }

        if data.len() >= 413 {
            self.enable_creator_fee = data[390] != 0;
            self.creator_fees_0 = u64::from_le_bytes(data[397..405].try_into().unwrap());
            self.creator_fees_1 = u64::from_le_bytes(data[405..413].try_into().unwrap());
        }

        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if self.has_risky {
            return Err(ArbError::InvalidInput("risky Token-2022 extension".into()));
        }
        // Bit 2 set in status disables swapping
        if (self.status & 4) != 0 {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM swap disabled in pool status".into(),
            ));
        }
        if !self.amm_config_loaded {
            return Err(ArbError::IncompleteState(
                "Raydium CPMM AmmConfig not loaded: fee schedule unknown, refusing to quote".into(),
            ));
        }
        if self.reserve_0 == 0 || self.reserve_1 == 0 {
            return Err(ArbError::InvalidInput("Raydium CPMM zero reserves".into()));
        }
        if self.transfer_fee_0_bps != 0 || self.transfer_fee_1_bps != 0 {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        let (r_in, r_out) = if a_to_b {
            (self.reserve_0, self.reserve_1)
        } else {
            (self.reserve_1, self.reserve_0)
        };

        // Official `CurveCalculator::swap_base_input`:
        //   creator fee on input  -> total fee = ceil(dx*(trade+creator)/1e6), one deduction
        //   creator fee on output -> trade fee = ceil(dx*trade/1e6), then
        //                            creator fee = ceil(out*creator/1e6) off the output
        let creator_rate = if self.enable_creator_fee {
            self.creator_fee_rate_ppm
        } else {
            0
        };
        let creator_on_input = self.creator_fee_on_input(a_to_b)?;
        if creator_rate == 0 || creator_on_input {
            let total_fee_rate = self.trade_fee_rate_ppm.saturating_add(creator_rate);
            return cpmm_out_upfront_fee_ceil(
                u128::from(amount_in),
                r_in,
                r_out,
                u128::from(total_fee_rate),
                RAYDIUM_CPMM_FEE_DENOMINATOR,
            )
            .ok_or(ArbError::Overflow);
        }

        let swapped = cpmm_out_upfront_fee_ceil(
            u128::from(amount_in),
            r_in,
            r_out,
            u128::from(self.trade_fee_rate_ppm),
            RAYDIUM_CPMM_FEE_DENOMINATOR,
        )
        .ok_or(ArbError::Overflow)?;
        let numerator = u128::from(swapped)
            .checked_mul(u128::from(creator_rate))
            .and_then(|v| v.checked_add(RAYDIUM_CPMM_FEE_DENOMINATOR - 1))
            .ok_or(ArbError::Overflow)?;
        let creator_fee = numerator / RAYDIUM_CPMM_FEE_DENOMINATOR;
        let net = u128::from(swapped)
            .checked_sub(creator_fee)
            .ok_or_else(|| {
                ArbError::InvalidInput("Raydium CPMM creator fee exceeds the swapped output".into())
            })?;
        u64::try_from(net).map_err(|_| ArbError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POOL_LEN: usize = 637;

    /// Minimal live-shaped CPMM `PoolState` (8-byte discriminator + 637 bytes).
    fn pool_state_bytes(creator_fee_on: u8, enable_creator_fee: bool) -> Vec<u8> {
        let mut data = vec![0u8; POOL_LEN];
        data[0..8].copy_from_slice(&[247, 237, 227, 245, 215, 195, 222, 70]);
        data[8..40].copy_from_slice(&[1u8; 32]);
        data[40..72].copy_from_slice(&[2u8; 32]);
        data[136..168].copy_from_slice(&[3u8; 32]);
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        data[232..264].copy_from_slice(spl.as_ref());
        data[264..296].copy_from_slice(spl.as_ref());
        data[72..104].copy_from_slice(&[7u8; 32]); // token_0_vault
        data[104..136].copy_from_slice(&[8u8; 32]); // token_1_vault
        data[168..200].copy_from_slice(&[9u8; 32]); // token_0_mint
        data[200..232].copy_from_slice(&[10u8; 32]); // token_1_mint
        data[296..328].copy_from_slice(&[11u8; 32]); // observation_key
        data[329] = 0; // swap enabled
        data[331] = 9;
        data[332] = 6;
        data[389] = creator_fee_on;
        data[390] = u8::from(enable_creator_fee);
        data
    }

    fn amm_config_bytes(trade_fee: u64, creator_fee: u64) -> Vec<u8> {
        let mut data = vec![0u8; 236];
        data[0..8].copy_from_slice(&[0xda, 0xf4, 0x21, 0x68, 0xcb, 0xcb, 0x2b, 0x6f]);
        data[12..20].copy_from_slice(&trade_fee.to_le_bytes());
        data[108..116].copy_from_slice(&creator_fee.to_le_bytes());
        data
    }

    fn pool(creator_fee_on: u8, enable_creator_fee: bool, creator_fee: u64) -> RaydiumCpmmPool {
        let mut pool = RaydiumCpmmPool::from_account(
            PoolId(Pubkey::new_unique()),
            &pool_state_bytes(creator_fee_on, enable_creator_fee),
        )
        .expect("valid pool state");
        pool.apply_amm_config_data(&amm_config_bytes(2_500, creator_fee))
            .expect("valid amm config");
        pool.update_reserves(1_000_000_000, 2_000_000_000);
        pool
    }

    #[test]
    fn quote_is_refused_until_the_amm_config_is_loaded() {
        let mut p = RaydiumCpmmPool::from_account(
            PoolId(Pubkey::new_unique()),
            &pool_state_bytes(0, false),
        )
        .expect("valid pool state");
        p.update_reserves(1_000_000_000, 2_000_000_000);
        let error = p.quote(1_000_000, true).expect_err("must refuse");
        assert!(error.to_string().contains("AmmConfig not loaded"));
        assert_eq!(p.fee_bps(), None);
    }

    #[test]
    fn fee_bps_metadata_rounds_up_and_respects_creator_enable_flag() {
        let mut p = pool(0, false, 5_000);
        p.trade_fee_rate_ppm = 50;
        assert_eq!(p.fee_bps(), Some(1));
        p.enable_creator_fee = true;
        assert_eq!(p.fee_bps(), Some(51));
        p.enable_creator_fee = false;
        assert_eq!(p.fee_bps(), Some(1));
    }
    #[test]
    fn trade_fee_only_quotes_match_the_official_curve() {
        let p = pool(0, false, 0);
        assert_eq!(p.quote(1_000_000, true).unwrap(), 1_993_011);
        assert_eq!(p.quote(1_000_000, false).unwrap(), 498_501);
    }

    #[test]
    fn creator_fee_on_both_tokens_is_charged_on_input_for_both_directions() {
        let p = pool(0, true, 5_000);
        assert_eq!(p.quote(1_000_000, true).unwrap(), 1_983_031);
        assert_eq!(p.quote(1_000_000, false).unwrap(), 496_003);
    }

    #[test]
    fn creator_fee_on_token0_moves_to_the_output_for_one_for_zero() {
        let p = pool(1, true, 5_000);
        // zero_for_one: token0 is the input -> creator fee on input.
        assert_eq!(p.quote(1_000_000, true).unwrap(), 1_983_031);
        // one_for_zero: token0 is the output -> creator fee off the output.
        assert_eq!(p.quote(1_000_000, false).unwrap(), 496_008);
    }

    #[test]
    fn creator_fee_on_token1_moves_to_the_output_for_zero_for_one() {
        let p = pool(2, true, 5_000);
        // zero_for_one: token1 is the output -> creator fee off the output.
        assert_eq!(p.quote(1_000_000, true).unwrap(), 1_983_045);
        // one_for_zero: token1 is the input -> creator fee on input.
        assert_eq!(p.quote(1_000_000, false).unwrap(), 496_003);
    }

    #[test]
    fn disabling_the_creator_fee_ignores_the_configured_rate() {
        let p = pool(1, false, 5_000);
        assert_eq!(p.quote(1_000_000, true).unwrap(), 1_993_011);
        assert_eq!(p.quote(1_000_000, false).unwrap(), 498_501);
    }

    #[test]
    fn invalid_creator_fee_mode_fails_closed() {
        assert!(RaydiumCpmmPool::from_account(
            PoolId(Pubkey::new_unique()),
            &pool_state_bytes(3, true),
        )
        .is_err());
    }
    #[test]
    fn out_of_range_config_fees_are_rejected_not_clamped() {
        let mut p =
            RaydiumCpmmPool::from_account(PoolId(Pubkey::new_unique()), &pool_state_bytes(0, true))
                .expect("valid pool state");
        assert!(
            p.apply_amm_config_data(&amm_config_bytes(100_001, 0))
                .is_err()
        );
        assert!(
            p.apply_amm_config_data(&amm_config_bytes(2_500, 50_001))
                .is_err()
        );
        assert!(!p.amm_config_loaded);
    }
}
