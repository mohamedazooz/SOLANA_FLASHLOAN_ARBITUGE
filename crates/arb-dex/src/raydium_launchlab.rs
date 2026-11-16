//! Raydium LaunchLab (`LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj`) Adapter.

use crate::traits::AmmPool;
use arb_constants::RAYDIUM_LAUNCHLAB_PROGRAM_ID;
use arb_math::{quote_launchlab_buy_exact_in, quote_launchlab_sell_exact_in};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const RAYDIUM_LAUNCHLAB_POOL_MIN_SIZE: usize = 366;

#[derive(Debug, Clone)]
pub struct RaydiumLaunchLabPool {
    pub pool_id: PoolId,
    pub authority: Pubkey,
    pub event_authority: Pubkey,
    pub global_config: Pubkey,
    pub platform_config: Pubkey,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub token_base_program: TokenProgramKind,
    pub token_quote_program: TokenProgramKind,
    pub status: u8,
    pub virtual_base: u64,
    pub virtual_quote: u64,
    pub real_base: u64,
    pub real_quote: u64,
    pub total_fee_bps: u16,
    /// True only after the authoritative GlobalConfig/PlatformConfig fee has
    /// been decoded and joined to this pool. PoolState bytes alone do not prove it.
    pub fee_config_verified: bool,
    pub decimals_base: u8,
    pub decimals_quote: u8,
    pub transfer_fee_base_bps: u16,
    pub transfer_fee_quote_bps: u16,
    pub has_risky: bool,
}

impl RaydiumLaunchLabPool {
    pub fn from_account(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        let prog = Pubkey::from_str(RAYDIUM_LAUNCHLAB_PROGRAM_ID).unwrap_or_default();
        let (authority, _) = Pubkey::find_program_address(&[b"vault_auth_seed"], &prog);
        let (event_authority, _) = Pubkey::find_program_address(&[b"__event_authority"], &prog);
        let mut pool = Self {
            pool_id,
            authority,
            event_authority,
            global_config: Pubkey::default(),
            platform_config: Pubkey::default(),
            base_mint: MintId::default(),
            quote_mint: MintId::default(),
            base_vault: Pubkey::default(),
            quote_vault: Pubkey::default(),
            token_base_program: TokenProgramKind::SplToken,
            token_quote_program: TokenProgramKind::SplToken,
            status: 0,
            virtual_base: 0,
            virtual_quote: 0,
            real_base: 0,
            real_quote: 0,
            total_fee_bps: 0,
            fee_config_verified: false,
            decimals_base: 0,
            decimals_quote: 9,
            transfer_fee_base_bps: 0,
            transfer_fee_quote_bps: 0,
            has_risky: false,
        };
        pool.update_from_account(data)?;
        Ok(pool)
    }
}

impl AmmPool for RaydiumLaunchLabPool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::RaydiumLaunchLab
    }

    fn base_mint(&self) -> MintId {
        self.base_mint
    }

    fn quote_mint(&self) -> MintId {
        self.quote_mint
    }

    fn reserves(&self) -> (u128, u128) {
        (
            u128::from(self.virtual_base).saturating_add(u128::from(self.real_base)),
            u128::from(self.virtual_quote).saturating_add(u128::from(self.real_quote)),
        )
    }

    fn fee_bps(&self) -> Option<u16> {
        self.fee_config_verified.then_some(self.total_fee_bps)
    }

    fn decimals(&self) -> (u8, u8) {
        (self.decimals_base, self.decimals_quote)
    }

    fn token_programs(&self) -> [TokenProgramKind; 2] {
        [self.token_base_program, self.token_quote_program]
    }

    fn transfer_fees_bps(&self) -> (u16, u16) {
        (self.transfer_fee_base_bps, self.transfer_fee_quote_bps)
    }

    fn has_risky_token(&self) -> bool {
        self.has_risky
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.base_vault == Pubkey::default() || self.quote_vault == Pubkey::default() {
            return None;
        }
        Some(PoolExecutionAccounts::RaydiumLaunchLab {
            authority: self.authority,
            global_config: self.global_config,
            platform_config: self.platform_config,
            base_vault: self.base_vault,
            quote_vault: self.quote_vault,
            event_authority: self.event_authority,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        vec![
            self.global_config,
            self.platform_config,
            self.base_mint.0,
            self.quote_mint.0,
        ]
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < RAYDIUM_LAUNCHLAB_POOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium LaunchLab PoolState len {} < {}",
                data.len(),
                RAYDIUM_LAUNCHLAB_POOL_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        // LaunchLab PoolState layout from official IDL (8-byte Anchor discriminator):
        // 17: status (u8)
        // 18: base_decimals (u8)
        // 19: quote_decimals (u8)
        // 37..45: virtual_base (u64)
        // 45..53: virtual_quote (u64)
        // 53..61: real_base (u64)
        // 61..69: real_quote (u64)
        // 141..173: global_config (Pubkey)
        // 173..205: platform_config (Pubkey)
        // 205..237: base_mint (Pubkey)
        // 237..269: quote_mint (Pubkey)
        // 269..301: base_vault (Pubkey)
        // 301..333: quote_vault (Pubkey)
        // 365: token_program_flag (u8)
        self.status = data[17];
        self.decimals_base = data[18];
        self.decimals_quote = data[19];
        self.virtual_base = u64::from_le_bytes(data[37..45].try_into().unwrap());
        self.virtual_quote = u64::from_le_bytes(data[45..53].try_into().unwrap());
        self.real_base = u64::from_le_bytes(data[53..61].try_into().unwrap());
        self.real_quote = u64::from_le_bytes(data[61..69].try_into().unwrap());

        self.global_config = read_pk(141)?;
        self.platform_config = read_pk(173)?;
        self.base_mint = MintId(read_pk(205)?);
        self.quote_mint = MintId(read_pk(237)?);
        self.base_vault = read_pk(269)?;
        self.quote_vault = read_pk(301)?;

        let tp_flag = data[365];
        self.token_base_program = if (tp_flag & 1) != 0 {
            TokenProgramKind::Token2022
        } else {
            TokenProgramKind::SplToken
        };
        self.token_quote_program = if (tp_flag & 2) != 0 {
            TokenProgramKind::Token2022
        } else {
            TokenProgramKind::SplToken
        };
        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if self.has_risky {
            return Err(ArbError::InvalidInput("risky Token-2022 extension".into()));
        }
        if self.status != 0 {
            return Err(ArbError::Internal(
                "Raydium LaunchLab curve graduated/inactive".into(),
            ));
        }
        if !self.fee_config_verified {
            return Err(ArbError::InvalidInput(
                "LaunchLab PlatformConfig fee has not been verified".into(),
            ));
        }
        if self.transfer_fee_base_bps != 0 || self.transfer_fee_quote_bps != 0 {
            return Err(ArbError::InvalidInput(
                "LaunchLab Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        let net_in = amount_in;
        // a_to_b == true: base -> quote (sell_exact_in); a_to_b == false: quote -> base (buy_exact_in)
        let gross_out = if a_to_b {
            quote_launchlab_sell_exact_in(
                net_in,
                self.virtual_base,
                self.virtual_quote,
                self.real_base,
                self.real_quote,
                self.total_fee_bps,
            )
        } else {
            quote_launchlab_buy_exact_in(
                net_in,
                self.virtual_base,
                self.virtual_quote,
                self.real_base,
                self.real_quote,
                self.total_fee_bps,
            )
        }
        .ok_or_else(|| ArbError::InvalidInput("LaunchLab insufficient curve liquidity".into()))?;
        Ok(gross_out)
    }
}
