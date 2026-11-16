//! Orca Whirlpools Account Deserializer and CLMM Quoting.

use crate::traits::AmmPool;
use arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID;
use arb_math::clmm::{TickArrayFacade, parse_tick_array_account, quote_clmm};
use arb_types::{
    ArbError, ClmmState, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result,
    TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const WHIRLPOOL_MIN_SIZE: usize = 300;
pub const TICK_ARRAY_SIZE: i32 = 88;

/// Orca Whirlpool pool state representation.
#[derive(Debug, Clone)]
pub struct OrcaWhirlpool {
    pub pool_id: PoolId,
    pub token_mint_a: MintId,
    pub token_mint_b: MintId,
    pub token_vault_a: Pubkey,
    pub token_vault_b: Pubkey,
    pub token_program_a: TokenProgramKind,
    pub token_program_b: TokenProgramKind,
    pub tick_spacing: u16,
    pub fee_rate: u16, // in hundredths of a bip (10_000 = 100 bps = 1%)
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current_index: i32,
    pub transfer_fee_a_bps: u16,
    pub transfer_fee_b_bps: u16,
    pub has_risky: bool,
    /// Coherent, protocol-decoded tick arrays for the current pool snapshot.
    pub tick_arrays: Vec<TickArrayFacade>,
    pub tick_array_pdas: Vec<Pubkey>,
}

impl OrcaWhirlpool {
    pub fn new(
        pool_id: PoolId,
        token_mint_a: MintId,
        token_mint_b: MintId,
        token_vault_a: Pubkey,
        token_vault_b: Pubkey,
        tick_spacing: u16,
        fee_rate: u16,
    ) -> Self {
        Self {
            pool_id,
            token_mint_a,
            token_mint_b,
            token_vault_a,
            token_vault_b,
            token_program_a: TokenProgramKind::SplToken,
            token_program_b: TokenProgramKind::SplToken,
            tick_spacing,
            fee_rate,
            liquidity: 0,
            sqrt_price_x64: 0,
            tick_current_index: 0,
            transfer_fee_a_bps: 0,
            transfer_fee_b_bps: 0,
            has_risky: false,
            tick_arrays: Vec::new(),
            tick_array_pdas: Vec::new(),
        }
    }

    /// Replace the adapter's tick-array view with arrays decoded from one
    /// coherent account snapshot.
    pub fn set_tick_arrays(&mut self, tick_arrays: Vec<TickArrayFacade>) {
        self.tick_arrays = tick_arrays;
    }

    /// Derive Whirlpool TickArray PDA for a given start tick index.
    pub fn derive_tick_array_pda(&self, start_tick: i32) -> Pubkey {
        let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap_or_default();
        let start_tick_str = start_tick.to_string();
        let seeds: &[&[u8]] = &[
            b"tick_array",
            self.pool_id.0.as_ref(),
            start_tick_str.as_bytes(),
        ];
        let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
        pda
    }

    /// Derive an ordered TickArray PDA window for a swap.
    pub fn tick_array_pdas_for_swap(&self, a_to_b: bool, array_count: usize) -> Vec<Pubkey> {
        if array_count == 0 || self.tick_spacing == 0 {
            return Vec::new();
        }
        let ticks_per_array = TICK_ARRAY_SIZE * i32::from(self.tick_spacing);
        let mut start_tick = self.tick_current_index.div_euclid(ticks_per_array) * ticks_per_array;
        if a_to_b && self.tick_current_index == start_tick {
            let Some(previous_start) = start_tick.checked_sub(ticks_per_array) else {
                return Vec::new();
            };
            start_tick = previous_start;
        }
        (0..array_count)
            .filter_map(|index| {
                let distance = i32::try_from(index).ok()?.checked_mul(ticks_per_array)?;
                let start = if a_to_b {
                    start_tick.checked_sub(distance)?
                } else {
                    start_tick.checked_add(distance)?
                };
                Some(self.derive_tick_array_pda(start))
            })
            .collect()
    }

    /// Derive Oracle PDA for the whirlpool.
    pub fn derive_oracle_pda(&self) -> Pubkey {
        let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap_or_default();
        let seeds: &[&[u8]] = &[b"oracle", self.pool_id.0.as_ref()];
        let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
        pda
    }
}

impl AmmPool for OrcaWhirlpool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::OrcaWhirlpool
    }

    fn base_mint(&self) -> MintId {
        self.token_mint_a
    }

    fn quote_mint(&self) -> MintId {
        self.token_mint_b
    }

    fn reserves(&self) -> (u128, u128) {
        if self.liquidity == 0 || self.sqrt_price_x64 == 0 {
            return (0, 0);
        }
        let l_u256 = ruint::aliases::U256::from(self.liquidity);
        let p_u256 = ruint::aliases::U256::from(self.sqrt_price_x64);
        let num: ruint::aliases::U256 = l_u256 << 64usize;
        let x_quot: ruint::aliases::U256 = num / p_u256;
        let y_prod: ruint::aliases::U256 = (l_u256 * p_u256) >> 64usize;
        let x_virt = x_quot.to::<u128>();
        let y_virt = y_prod.to::<u128>();
        (x_virt, y_virt)
    }

    fn fee_bps(&self) -> Option<u16> {
        if self.fee_rate == 0 {
            None
        } else {
            Some(self.fee_rate / 100)
        }
    }

    fn token_programs(&self) -> [TokenProgramKind; 2] {
        [self.token_program_a, self.token_program_b]
    }

    fn transfer_fees_bps(&self) -> (u16, u16) {
        (self.transfer_fee_a_bps, self.transfer_fee_b_bps)
    }

    fn has_risky_token(&self) -> bool {
        self.has_risky
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.token_vault_a == Pubkey::default() || self.token_vault_b == Pubkey::default() {
            return None;
        }
        let tick_array_pdas = if self.tick_array_pdas.len() >= 3 {
            [
                self.tick_array_pdas[0],
                self.tick_array_pdas[1],
                self.tick_array_pdas[2],
            ]
        } else {
            let derived = self.tick_array_pdas_for_swap(true, 3);
            if derived.len() < 3 {
                return None;
            }
            [derived[0], derived[1], derived[2]]
        };

        Some(PoolExecutionAccounts::OrcaWhirlpool {
            vault_a: self.token_vault_a,
            vault_b: self.token_vault_b,
            oracle: self.derive_oracle_pda(),
            tick_array_pdas: tick_array_pdas.to_vec(),
            pool_mint_a: self.token_mint_a.0,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let mut keys = vec![
            self.token_vault_a,
            self.token_vault_b,
            self.derive_oracle_pda(),
        ];
        keys.extend(self.tick_array_pdas_for_swap(true, 3));
        keys
    }

    fn apply_dependency_account(&mut self, pubkey: &Pubkey, data: &[u8]) -> Result<bool> {
        if let Ok(ta) = parse_tick_array_account(data, self.tick_spacing) {
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

    fn clmm_state(&self) -> Option<ClmmState> {
        if self.liquidity == 0 || self.sqrt_price_x64 == 0 {
            return None;
        }
        Some(ClmmState {
            sqrt_price: self.sqrt_price_x64,
            liquidity: self.liquidity,
            tick_current: self.tick_current_index,
            tick_spacing: self.tick_spacing,
            fee_rate: self.fee_rate,
            tick_arrays: self
                .tick_arrays
                .iter()
                .map(|array| arb_types::ClmmTickArray {
                    start_tick_index: array.start_tick_index,
                    ticks: array
                        .ticks
                        .iter()
                        .filter(|tick| tick.initialized)
                        .map(|tick| arb_types::ClmmTick {
                            tick_index: tick.tick_index,
                            initialized: true,
                            liquidity_net: tick.liquidity_net,
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < WHIRLPOOL_MIN_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Whirlpool account data size {} < minimum {}",
                data.len(),
                WHIRLPOOL_MIN_SIZE
            )));
        }

        if let Ok(b) = data[41..43].try_into() {
            self.tick_spacing = u16::from_le_bytes(b);
        }
        if let Ok(b) = data[45..47].try_into() {
            self.fee_rate = u16::from_le_bytes(b);
        }
        if let Ok(b) = data[49..65].try_into() {
            self.liquidity = u128::from_le_bytes(b);
        }
        if let Ok(b) = data[65..81].try_into() {
            self.sqrt_price_x64 = u128::from_le_bytes(b);
        }
        if let Ok(b) = data[81..85].try_into() {
            self.tick_current_index = i32::from_le_bytes(b);
        }
        if let Ok(b) = data[101..133].try_into() {
            self.token_mint_a = MintId(Pubkey::new_from_array(b));
        }
        if let Ok(b) = data[133..165].try_into() {
            self.token_vault_a = Pubkey::new_from_array(b);
        }
        if let Ok(b) = data[181..213].try_into() {
            self.token_mint_b = MintId(Pubkey::new_from_array(b));
        }
        if let Ok(b) = data[213..245].try_into() {
            self.token_vault_b = Pubkey::new_from_array(b);
        }

        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if self.has_risky {
            return Err(ArbError::InvalidInput("risky Token-2022 extension".into()));
        }
        if self.liquidity == 0 || self.sqrt_price_x64 == 0 {
            return Err(ArbError::Internal(
                "Whirlpool has zero liquidity or price".into(),
            ));
        }
        if self.tick_arrays.is_empty() {
            return Err(ArbError::Internal(
                "Whirlpool quote requires decoded tick arrays from a coherent snapshot".into(),
            ));
        }
        if self.fee_rate == 0 {
            return Err(ArbError::Internal("Pool fee not configured".into()));
        }

        if self.transfer_fee_a_bps != 0 || self.transfer_fee_b_bps != 0 {
            return Err(ArbError::InvalidInput(
                "Whirlpool Token-2022 transfer fees require epoch/max-fee-aware mint context"
                    .into(),
            ));
        }
        let net_in = amount_in;

        let gross_out = quote_clmm(
            net_in,
            self.sqrt_price_x64,
            self.tick_current_index,
            self.liquidity,
            self.tick_spacing,
            self.fee_rate,
            a_to_b,
            &self.tick_arrays,
        )
        .map_err(|error| ArbError::Internal(format!("Whirlpool quote rejected: {error}")))?;

        Ok(gross_out)
    }
}
