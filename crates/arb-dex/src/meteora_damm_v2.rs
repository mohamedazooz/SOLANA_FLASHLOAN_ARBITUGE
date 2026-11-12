//! Meteora DAMM v2 (`cp_amm`, `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`) Adapter.

use crate::traits::AmmPool;
use arb_constants::{METEORA_DAMM_V2_POOL_AUTHORITY, METEORA_DAMM_V2_PROGRAM_ID};
use arb_math::{DammV2FeeParams, damm_v2_virtual_reserves};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const METEORA_DAMM_V2_POOL_MIN_SIZE: usize = 485;

#[derive(Debug, Clone)]
pub struct MeteoraDammV2Pool {
    pub pool_id: PoolId,
    pub config: Pubkey,
    pub authority: Pubkey,
    pub event_authority: Pubkey,
    pub token_a_mint: MintId,
    pub token_b_mint: MintId,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub token_a_program: TokenProgramKind,
    pub token_b_program: TokenProgramKind,
    pub sqrt_price_x64: u128,
    pub liquidity: u128,
    pub sqrt_min_price_x64: u128,
    pub sqrt_max_price_x64: u128,
    pub activation_point: u64,
    pub current_point: u64,
    pub fee_params: DammV2FeeParams,
    /// False until the complete current fee context (including dynamic-fee
    /// config and current slot/timestamp) has been joined from its source accounts.
    pub fee_context_verified: bool,
    pub decimals_a: u8,
    pub decimals_b: u8,
    pub transfer_fee_a_bps: u16,
    pub transfer_fee_b_bps: u16,
    pub has_risky: bool,
}

impl MeteoraDammV2Pool {
    pub fn from_account(pool_id: PoolId, data: &[u8]) -> Result<Self> {
        let prog = Pubkey::from_str(METEORA_DAMM_V2_PROGRAM_ID).unwrap_or_default();
        let authority = Pubkey::from_str(METEORA_DAMM_V2_POOL_AUTHORITY).unwrap_or_default();
        let (event_authority, _) = Pubkey::find_program_address(&[b"__event_authority"], &prog);
        let mut pool = Self {
            pool_id,
            config: Pubkey::default(),
            authority,
            event_authority,
            token_a_mint: MintId::default(),
            token_b_mint: MintId::default(),
            token_a_vault: Pubkey::default(),
            token_b_vault: Pubkey::default(),
            token_a_program: TokenProgramKind::SplToken,
            token_b_program: TokenProgramKind::SplToken,
            sqrt_price_x64: 0,
            liquidity: 0,
            sqrt_min_price_x64: 0,
            sqrt_max_price_x64: 0,
            activation_point: 0,
            current_point: 0,
            fee_params: DammV2FeeParams::default(),
            fee_context_verified: false,
            decimals_a: 0,
            decimals_b: 0,
            transfer_fee_a_bps: 0,
            transfer_fee_b_bps: 0,
            has_risky: false,
        };
        pool.update_from_account(data)?;
        Ok(pool)
    }
}

impl AmmPool for MeteoraDammV2Pool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::MeteoraDammV2
    }

    fn base_mint(&self) -> MintId {
        self.token_a_mint
    }

    fn quote_mint(&self) -> MintId {
        self.token_b_mint
    }

    fn reserves(&self) -> (u128, u128) {
        damm_v2_virtual_reserves(self.sqrt_price_x64, self.liquidity)
    }

    fn fee_bps(&self) -> Option<u16> {
        self.fee_context_verified.then(|| {
            self.fee_params
                .fee_bps(self.current_point, self.activation_point)
        })
    }

    fn decimals(&self) -> (u8, u8) {
        (self.decimals_a, self.decimals_b)
    }

    fn token_programs(&self) -> [TokenProgramKind; 2] {
        [self.token_a_program, self.token_b_program]
    }

    fn transfer_fees_bps(&self) -> (u16, u16) {
        (self.transfer_fee_a_bps, self.transfer_fee_b_bps)
    }

    fn has_risky_token(&self) -> bool {
        self.has_risky
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.token_a_vault == Pubkey::default() || self.token_b_vault == Pubkey::default() {
            return None;
        }
        Some(PoolExecutionAccounts::MeteoraDammV2 {
            config: self.config,
            authority: self.authority,
            vault_a: self.token_a_vault,
            vault_b: self.token_b_vault,
            event_authority: self.event_authority,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        vec![
            self.token_a_vault,
            self.token_b_vault,
            self.token_a_mint.0,
            self.token_b_mint.0,
        ]
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < METEORA_DAMM_V2_POOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Meteora DAMM v2 Pool len {} < {}",
                data.len(),
                METEORA_DAMM_V2_POOL_MIN_SIZE
            )));
        }
        let read_pk = |off: usize| -> Result<Pubkey> {
            let b: [u8; 32] = data[off..off + 32]
                .try_into()
                .map_err(|_| ArbError::DeserializationError("bad pubkey".into()))?;
            Ok(Pubkey::new_from_array(b))
        };
        // Meteora cp_amm::Pool verified binary layout:
        // 0..8: Anchor discriminator
        // 8..168: PoolFeesStruct (BaseFeeStruct + DynamicFeeStruct)
        //   8..16: cliff_fee_numerator (u64)
        //   16: base_fee_mode (u8)
        //   22..24: number_of_period (u16)
        //   24..32: period_frequency (u64)
        //   32..40: reduction_factor (u64)
        // 168..200: token_a_mint
        // 200..232: token_b_mint
        // 232..264: token_a_vault
        // 264..296: token_b_vault
        // 296..328: whitelisted_vault
        // 360..376: liquidity (u128)
        // 424..440: sqrt_min_price (u128)
        // 440..456: sqrt_max_price (u128)
        // 456..472: sqrt_price (u128)
        // 472..480: activation_point (u64)
        // 482: token_a_flag (u8)
        // 483: token_b_flag (u8)
        // 484: collect_fee_mode (u8)
        self.fee_params.cliff_fee_numerator = u64::from_le_bytes(data[8..16].try_into().unwrap());
        self.fee_params.fee_scheduler_mode = data[16];
        self.fee_params.number_of_period = u16::from_le_bytes(data[22..24].try_into().unwrap());
        self.fee_params.period_frequency = u64::from_le_bytes(data[24..32].try_into().unwrap());
        self.fee_params.reduction_factor = u64::from_le_bytes(data[32..40].try_into().unwrap());

        self.token_a_mint = MintId(read_pk(168)?);
        self.token_b_mint = MintId(read_pk(200)?);
        self.token_a_vault = read_pk(232)?;
        self.token_b_vault = read_pk(264)?;
        self.config = read_pk(296)?;

        self.liquidity = u128::from_le_bytes(data[360..376].try_into().unwrap());
        self.sqrt_min_price_x64 = u128::from_le_bytes(data[424..440].try_into().unwrap());
        self.sqrt_max_price_x64 = u128::from_le_bytes(data[440..456].try_into().unwrap());
        self.sqrt_price_x64 = u128::from_le_bytes(data[456..472].try_into().unwrap());
        self.activation_point = u64::from_le_bytes(data[472..480].try_into().unwrap());

        let flag_a = data[482];
        let flag_b = data[483];
        self.token_a_program = if flag_a == 1 {
            TokenProgramKind::Token2022
        } else {
            TokenProgramKind::SplToken
        };
        self.token_b_program = if flag_b == 1 {
            TokenProgramKind::Token2022
        } else {
            TokenProgramKind::SplToken
        };
        self.fee_params.collect_fee_mode = data[484];
        // Pool bytes alone do not prove the full dynamic-fee/config/current-point
        // context. Keep this false until the account-join layer supplies it.
        self.fee_context_verified = false;
        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if self.has_risky {
            return Err(ArbError::InvalidInput("risky Token-2022 extension".into()));
        }
        if !self.fee_context_verified {
            return Err(ArbError::InvalidInput(
                "Meteora DAMM v2 fee config/current point has not been coherently joined".into(),
            ));
        }
        if self.fee_params.collect_fee_mode == 2 {
            return Err(ArbError::InvalidInput(
                "Meteora DAMM v2 compounding mode requires layout-version reserve balances; concentrated-liquidity quote is invalid".into(),
            ));
        }
        if self.fee_params.collect_fee_mode > 2 {
            return Err(ArbError::InvalidInput(
                "unknown Meteora DAMM v2 collect fee mode".into(),
            ));
        }
        if self.sqrt_price_x64 == 0 || self.liquidity == 0 {
            return Err(ArbError::InvalidInput(
                "Meteora DAMM v2 zero sqrt_price or liquidity".into(),
            ));
        }
        if self.transfer_fee_a_bps != 0 || self.transfer_fee_b_bps != 0 {
            return Err(ArbError::InvalidInput(
                "Meteora DAMM v2 Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        let net_in = amount_in;
        let fee_num = self
            .fee_params
            .effective_fee_numerator(self.current_point, self.activation_point);
        let fees_on_input = match self.fee_params.collect_fee_mode {
            1 => !a_to_b, // OnlyB: fees on input only when selling B (B -> A)
            2 => !a_to_b, // Compounding: B -> A fee on input
            _ => false,   // BothToken (0): fee on output
        };
        let gross_out = arb_math::damm_v2::quote_damm_v2_exact_in_with_fee_mode(
            net_in,
            a_to_b,
            self.sqrt_price_x64,
            self.liquidity,
            self.sqrt_min_price_x64,
            self.sqrt_max_price_x64,
            fee_num,
            fees_on_input,
        )
        .ok_or(ArbError::Overflow)?;
        Ok(gross_out)
    }
}
