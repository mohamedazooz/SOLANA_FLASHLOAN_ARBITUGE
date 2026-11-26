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
/// Anchor discriminator for account:Pool (SHA-256 prefix).
pub const METEORA_DAMM_V2_POOL_DISCRIMINATOR: [u8; 8] =
    [0xf1, 0x9a, 0x6d, 0x04, 0x11, 0xb1, 0x6d, 0xbc];

/// Clock sysvar values decoded from the same getMultipleAccounts context as the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DammV2ClockSnapshot {
    pub context_slot: u64,
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: i64,
}

impl DammV2ClockSnapshot {
    /// Solana Clock layout: slot @0, epoch_start_timestamp @8, epoch @16,
    /// leader_schedule_epoch @24, unix_timestamp @32.
    pub fn decode(data: &[u8], context_slot: u64) -> Option<Self> {
        if data.len() < 40 || context_slot == 0 {
            return None;
        }
        let slot = u64::from_le_bytes(data.get(0..8)?.try_into().ok()?);
        let epoch = u64::from_le_bytes(data.get(16..24)?.try_into().ok()?);
        let unix_timestamp = i64::from_le_bytes(data.get(32..40)?.try_into().ok()?);
        (slot > 0).then_some(Self {
            context_slot,
            slot,
            epoch,
            unix_timestamp,
        })
    }
}
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
    pub activation_type: u8,
    pub fee_version: u8,
    pub layout_version: u8,
    /// V1-only tracked compounding reserves; V0 does not carry these counters.
    pub token_a_amount: Option<u64>,
    pub token_b_amount: Option<u64>,
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
            activation_type: 0,
            fee_version: 0,
            layout_version: 0,
            token_a_amount: None,
            token_b_amount: None,
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
    /// Supply a slot or Unix timestamp from a coherent provider snapshot.
    /// Do not pass local wall-clock time for timestamp-activated pools.
    /// Bind the fee scheduler to a real Clock sysvar snapshot fetched in the
    /// same RPC context as this pool. Timestamp activation never uses local time.
    pub fn set_clock_snapshot(
        &mut self,
        clock: DammV2ClockSnapshot,
        pool_context_slot: u64,
    ) -> bool {
        if pool_context_slot == 0
            || clock.context_slot != pool_context_slot
            || clock.slot != pool_context_slot
        {
            self.fee_context_verified = false;
            return false;
        }
        let current_point = match self.activation_type {
            0 => clock.slot,
            1 => match u64::try_from(clock.unix_timestamp) {
                Ok(timestamp) if timestamp > 0 => timestamp,
                _ => {
                    self.fee_context_verified = false;
                    return false;
                }
            },
            _ => {
                self.fee_context_verified = false;
                return false;
            }
        };
        self.set_current_point(self.activation_type, current_point);
        self.fee_context_verified
    }
    pub fn set_current_point(&mut self, activation_type: u8, current_point: u64) {
        if activation_type != self.activation_type {
            self.fee_context_verified = false;
            return;
        }
        self.current_point = current_point;
        self.fee_context_verified = self.fee_snapshot_is_valid();
    }

    fn fee_snapshot_is_valid(&self) -> bool {
        let mode = self.fee_params.fee_scheduler_mode;
        let scheduler_valid = match mode {
            0 | 1 => {
                let configured = self.fee_params.period_frequency != 0
                    || self.fee_params.number_of_period != 0
                    || self.fee_params.reduction_factor != 0;
                !configured
                    || (self.fee_params.period_frequency > 0
                        && self.fee_params.number_of_period > 0
                        && self.fee_params.reduction_factor > 0)
            }
            2 => true,
            3 | 4 => {
                self.fee_params.number_of_period > 0
                    && self.fee_params.sqrt_price_step_bps > 0
                    && self.fee_params.scheduler_expiration_duration > 0
                    && self.fee_params.reduction_factor > 0
            }
            _ => false,
        };
        let Some(fee_cap) = self.fee_params.max_fee_numerator() else {
            return false;
        };
        let base_fee_valid = self
            .fee_params
            .min_base_fee_numerator()
            .is_some_and(|min| min >= 100_000)
            && u128::from(self.fee_params.cliff_fee_numerator) <= fee_cap;
        let split_valid = self.fee_params.protocol_fee_percent <= 100
            && self.fee_params.referral_fee_percent <= 100
            && self.fee_params.compounding_fee_bps <= 10_000;
        let dynamic_valid = !self.fee_params.dynamic_fee_initialized
            || (self.fee_params.bin_step > 0
                && self.fee_params.variable_fee_control > 0
                && self.fee_params.max_volatility_accumulator > 0
                && self.fee_params.volatility_accumulator
                    <= u128::from(self.fee_params.max_volatility_accumulator));
        scheduler_valid
            && base_fee_valid
            && split_valid
            && dynamic_valid
            && self.fee_version <= 1
            && self.layout_version <= 1
            && self.activation_type <= 1
            && self.pool_status == 0
            && self.sqrt_price_x64 > 0
            && self.liquidity > 0
            && (mode < 3 || self.fee_params.init_sqrt_price_x64 > 0)
            && (self.fee_params.collect_fee_mode != 2
                || (self.layout_version == 1
                    && self.token_a_amount.is_some()
                    && self.token_b_amount.is_some()))
    }
    /// Exact-in quote requiring a Clock-matched fee context. Token-2022 remains
    /// fail-closed until epoch-aware fees and hook accounts are fully resolved.
    pub fn quote_exact_in_verified(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if !self.fee_context_verified || !self.is_tradable() {
            return Err(ArbError::IncompleteState(
                "DAMM v2 quote requires an enabled pool and verified current-point fee snapshot"
                    .into(),
            ));
        }
        if self.has_risky
            || self.token_a_program != TokenProgramKind::SplToken
            || self.token_b_program != TokenProgramKind::SplToken
        {
            return Err(ArbError::UnsupportedOperation(
                "DAMM v2 Token-2022 requires epoch-aware transfer-fee and transfer-hook context"
                    .into(),
            ));
        }
        let fee = self
            .fee_params
            .effective_fee_numerator_at(
                self.current_point,
                self.activation_point,
                self.sqrt_price_x64,
            )
            .ok_or_else(|| ArbError::IncompleteState("DAMM v2 fee schedule is invalid".into()))?;
        let fees_on_input = self.fee_params.fees_on_input(a_to_b).ok_or_else(|| {
            ArbError::UnsupportedOperation("unsupported DAMM v2 collect-fee mode".into())
        })?;
        let output = if self.fee_params.collect_fee_mode == 2 {
            if self.layout_version != 1 {
                return Err(ArbError::UnsupportedOperation(
                    "DAMM v2 Compounding requires LayoutVersion::V1 tracked reserves".into(),
                ));
            }
            arb_math::quote_damm_v2_compounding_exact_in(
                amount_in,
                a_to_b,
                self.token_a_amount
                    .ok_or_else(|| ArbError::IncompleteState("missing V1 token A amount".into()))?,
                self.token_b_amount
                    .ok_or_else(|| ArbError::IncompleteState("missing V1 token B amount".into()))?,
                fee,
                fees_on_input,
            )
        } else {
            arb_math::quote_damm_v2_exact_in_with_fee_mode(
                amount_in,
                a_to_b,
                self.sqrt_price_x64,
                self.liquidity,
                self.sqrt_min_price_x64,
                self.sqrt_max_price_x64,
                fee,
                fees_on_input,
            )
        }
        .ok_or_else(|| {
            ArbError::CalculationError(
                "DAMM v2 exact-in quote overflowed or crossed a price-range boundary".into(),
            )
        })?;
        Ok(output)
    }
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
        if self.fee_params.collect_fee_mode == 2 {
            return match (
                self.layout_version,
                self.token_a_amount,
                self.token_b_amount,
            ) {
                (1, Some(a), Some(b)) => (u128::from(a), u128::from(b)),
                _ => (0, 0),
            };
        }
        damm_v2_virtual_reserves(self.sqrt_price_x64, self.liquidity)
    }

    fn fee_bps(&self) -> Option<u16> {
        if !self.fee_context_verified {
            return None;
        }
        self.fee_params.fee_bps_at(
            self.current_point,
            self.activation_point,
            self.sqrt_price_x64,
        )
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
        if data.get(..8) != Some(METEORA_DAMM_V2_POOL_DISCRIMINATOR.as_slice()) {
            return Err(ArbError::DeserializationError(
                "Meteora DAMM v2 Pool discriminator mismatch".into(),
            ));
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
        self.fee_params.protocol_fee_percent = data[48];
        self.fee_params.referral_fee_percent = data[50];
        self.fee_params.compounding_fee_bps = u16::from_le_bytes(data[54..56].try_into().unwrap());
        self.fee_params.cliff_fee_numerator = u64::from_le_bytes(data[8..16].try_into().unwrap());
        self.fee_params.fee_scheduler_mode = data[16];
        self.fee_params.number_of_period = u16::from_le_bytes(data[22..24].try_into().unwrap());
        match self.fee_params.fee_scheduler_mode {
            0 | 1 => {
                self.fee_params.period_frequency =
                    u64::from_le_bytes(data[24..32].try_into().unwrap());
                self.fee_params.reduction_factor =
                    u64::from_le_bytes(data[32..40].try_into().unwrap());
            }
            2 => {
                self.fee_params.period_frequency = 0;
                self.fee_params.reduction_factor = 0;
            }
            3 | 4 => {
                self.fee_params.sqrt_price_step_bps =
                    u32::from_le_bytes(data[24..28].try_into().unwrap());
                self.fee_params.scheduler_expiration_duration =
                    u32::from_le_bytes(data[28..32].try_into().unwrap());
                self.fee_params.reduction_factor =
                    u64::from_le_bytes(data[32..40].try_into().unwrap());
            }
            _ => {}
        }
        // PoolFeesStruct::dynamic_fee begins at account offset 56.
        self.fee_params.dynamic_fee_initialized = data[56] != 0;
        self.fee_params.max_volatility_accumulator =
            u32::from_le_bytes(data[64..68].try_into().unwrap());
        self.fee_params.variable_fee_control = u32::from_le_bytes(data[68..72].try_into().unwrap());
        self.fee_params.bin_step = u16::from_le_bytes(data[72..74].try_into().unwrap());
        self.fee_params.volatility_accumulator =
            u128::from_le_bytes(data[120..136].try_into().unwrap());
        self.fee_params.init_sqrt_price_x64 =
            u128::from_le_bytes(data[152..168].try_into().unwrap());

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
        self.activation_type = data[480];
        self.pool_status = data[481];
        self.fee_version = data[486];
        self.layout_version = data[696];
        if self.layout_version == 1 {
            self.token_a_amount = Some(u64::from_le_bytes(data[680..688].try_into().unwrap()));
            self.token_b_amount = Some(u64::from_le_bytes(data[688..696].try_into().unwrap()));
        } else {
            self.token_a_amount = None;
            self.token_b_amount = None;
        }
        self.fee_params.fee_version = self.fee_version;

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
            "Meteora DAMM v2 adapter is quote-only until the full official PoolFeesStruct/dynamic-fee state, activation point plus a clock-matched current point, collect-fee mode, layout-version reserves, and epoch-aware mint context are joined"
                .into(),
        ))
    }
}
