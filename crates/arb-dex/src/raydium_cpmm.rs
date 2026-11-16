//! Raydium CP-Swap (CPMM) Account Deserializer and Exact-In Quoting.

use crate::traits::AmmPool;
use arb_constants::RAYDIUM_CPMM_PROGRAM_ID;
use arb_math::cpmm_out_upfront_fee_ceil;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const RAYDIUM_CPMM_POOL_MIN_SIZE: usize = 373;
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
    /// Combined fee rate in 1e6 PPM units (trade_fee + creator_fee).
    pub trade_fee_rate_ppm: u64,
    pub creator_fee_rate_ppm: u64,
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
            trade_fee_rate_ppm: 2_500, // default 25 bps until AmmConfig loaded
            creator_fee_rate_ppm: 0,
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
        if data.len() < 20 {
            return Err(ArbError::DeserializationError(
                "Raydium CPMM AmmConfig account truncated".into(),
            ));
        }
        let trade_fee = u64::from_le_bytes(
            data[12..20]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad amm_config trade fee".into()))?,
        );
        if trade_fee > 0 && trade_fee <= 100_000 {
            self.trade_fee_rate_ppm = trade_fee;
        }

        if data.len() >= 116 {
            let creator_fee = u64::from_le_bytes(data[108..116].try_into().map_err(|_| {
                ArbError::DeserializationError("bad amm_config creator fee".into())
            })?);
            if creator_fee <= 50_000 {
                self.creator_fee_rate_ppm = creator_fee;
            }
        }
        Ok(())
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
        let eff_ppm = self.effective_fee_rate_ppm();
        let bps = (u128::from(eff_ppm) * 10_000) / RAYDIUM_CPMM_FEE_DENOMINATOR;
        u16::try_from(bps).ok().filter(|b| *b <= 10_000)
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
        if data.len() < RAYDIUM_CPMM_POOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium CPMM pool data {} < {}",
                data.len(),
                RAYDIUM_CPMM_POOL_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey slice".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        let t2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap_or_default();

        self.amm_config = read_pk(8)?;
        self.token_0_vault = read_pk(72)?;
        self.token_1_vault = read_pk(104)?;
        self.token_0_mint = MintId(read_pk(168)?);
        self.token_1_mint = MintId(read_pk(200)?);
        let p0 = read_pk(232)?;
        let p1 = read_pk(264)?;
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
        self.status = data[329];
        self.mint_0_decimals = data[331];
        self.mint_1_decimals = data[332];
        self.protocol_fees_0 = u64::from_le_bytes(data[341..349].try_into().unwrap());
        self.protocol_fees_1 = u64::from_le_bytes(data[349..357].try_into().unwrap());
        self.fund_fees_0 = u64::from_le_bytes(data[357..365].try_into().unwrap());
        self.fund_fees_1 = u64::from_le_bytes(data[365..373].try_into().unwrap());

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
        if self.reserve_0 == 0 || self.reserve_1 == 0 {
            return Err(ArbError::InvalidInput("Raydium CPMM zero reserves".into()));
        }
        if self.transfer_fee_0_bps != 0 || self.transfer_fee_1_bps != 0 {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        let net_in = amount_in;
        let (r_in, r_out) = if a_to_b {
            (self.reserve_0, self.reserve_1)
        } else {
            (self.reserve_1, self.reserve_0)
        };
        let gross_out = cpmm_out_upfront_fee_ceil(
            u128::from(net_in),
            r_in,
            r_out,
            u128::from(self.effective_fee_rate_ppm()),
            RAYDIUM_CPMM_FEE_DENOMINATOR,
        )
        .ok_or(ArbError::Overflow)?;
        Ok(gross_out)
    }
}
