//! Meteora DAMM v2 (`cp_amm`, `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`) Adapter.

use crate::traits::AmmPool;
use arb_constants::{METEORA_DAMM_V2_POOL_AUTHORITY, METEORA_DAMM_V2_PROGRAM_ID};
use arb_math::{
    DammV2FeeParams, damm_v2_virtual_reserves, sqrt_price_x64_in_meteora_damm_v2_bounds,
};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

// Anchor discriminator (8) + official Pool::INIT_SPACE (1,104).
pub const METEORA_DAMM_V2_POOL_MIN_SIZE: usize = 8 + 1_104;

#[derive(Debug, Clone)]
pub struct MeteoraDammV2Pool {
    pub pool_id: PoolId,
    /// On-chain Pool::whitelisted_vault (offset 296..328), not a pool config.
    /// Kept named explicitly so it cannot be mistaken for the swap's config account.
    pub whitelisted_vault: Pubkey,
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
    /// On-chain `Pool::pool_status`: 0 = Enable, 1 = Disable (official cp_amm
    /// `state/pool.rs`). Any other value is treated as disabled (fail-closed).
    pub pool_status: u8,
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
            whitelisted_vault: Pubkey::default(),
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
            pool_status: 1,
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

    /// Verified against the deployed cp_amm program layout
    /// (`programs/cp-amm/src/state/pool.rs`, checked 2026-11-20).
    pub fn is_tradable(&self) -> bool {
        self.pool_status == 0
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
        // The current adapter does not yet join the full official dynamic-fee
        // state and current-point context. A boolean flag is not a fee snapshot.
        None
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
        // The current account parser has only verified whitelisted_vault at
        // offset 296..328. It has not resolved the complete official swap
        // account graph, so do not expose this value as the PoolExecutionAccounts
        // config field or advertise the pool as executable.
        None
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let mut deps = vec![
            self.token_a_vault,
            self.token_b_vault,
            self.token_a_mint.0,
            self.token_b_mint.0,
            self.whitelisted_vault,
        ];
        deps.retain(|key| *key != Pubkey::default());
        deps
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
        // 480: activation_type (u8)
        // 481: pool_status (u8; 0 = enable, 1 = disable)
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
        self.whitelisted_vault = read_pk(296)?;

        self.liquidity = u128::from_le_bytes(data[360..376].try_into().unwrap());
        self.sqrt_min_price_x64 = u128::from_le_bytes(data[424..440].try_into().unwrap());
        self.sqrt_max_price_x64 = u128::from_le_bytes(data[440..456].try_into().unwrap());
        self.sqrt_price_x64 = u128::from_le_bytes(data[456..472].try_into().unwrap());
        if self.sqrt_price_x64 != 0
            && !sqrt_price_x64_in_meteora_damm_v2_bounds(self.sqrt_price_x64)
        {
            return Err(ArbError::InvalidInput(format!(
                "Meteora DAMM v2 sqrt_price_x64 {} outside official program bounds",
                self.sqrt_price_x64
            )));
        }
        self.activation_point = u64::from_le_bytes(data[472..480].try_into().unwrap());
        self.pool_status = data[481];

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

    fn quote(&self, _amount_in: u64, _a_to_b: bool) -> Result<u64> {
        Err(ArbError::UnsupportedOperation(
            "Meteora DAMM v2 adapter is quote-only until the full official PoolFeesStruct/dynamic-fee state, activation point, collect-fee mode, layout-version reserves, and epoch-aware mint context are joined"
                .into(),
        ))
    }
}
