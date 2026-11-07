//! Raydium AMM v4 State Deserializer and Quoting.

use crate::traits::AmmPool;
use arb_math::cpmm_out;
use arb_types::{ArbError, DexProtocol, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

pub const RAYDIUM_V4_LAYOUT_SIZE: usize = 752;

/// Raydium AMM v4 Pool representation.
#[derive(Debug, Clone)]
pub struct RaydiumV4Pool {
    pub pool_id: PoolId,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub open_orders: Pubkey,
    pub target_orders: Pubkey,
    pub serum_market: Pubkey,
    pub base_reserve: u128,
    pub quote_reserve: u128,
    pub fee_numerator: u64,
    pub fee_denominator: u64,
}

impl RaydiumV4Pool {
    pub fn new(
        pool_id: PoolId,
        base_mint: MintId,
        quote_mint: MintId,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        open_orders: Pubkey,
    ) -> Self {
        Self {
            pool_id,
            base_mint,
            quote_mint,
            base_vault,
            quote_vault,
            open_orders,
            target_orders: Pubkey::default(),
            serum_market: Pubkey::default(),
            base_reserve: 0,
            quote_reserve: 0,
            fee_numerator: 25,
            fee_denominator: 10_000,
        }
    }

    /// Update reserves directly (from token account balances).
    pub fn update_reserves(&mut self, base_reserve: u128, quote_reserve: u128) {
        self.base_reserve = base_reserve;
        self.quote_reserve = quote_reserve;
    }
}

impl AmmPool for RaydiumV4Pool {
    fn pool_id(&self) -> PoolId {
        self.pool_id
    }

    fn protocol(&self) -> DexProtocol {
        DexProtocol::RaydiumV4
    }

    fn base_mint(&self) -> MintId {
        self.base_mint
    }

    fn quote_mint(&self) -> MintId {
        self.quote_mint
    }

    fn reserves(&self) -> (u128, u128) {
        (self.base_reserve, self.quote_reserve)
    }

    fn fee_bps(&self) -> Option<u16> {
        if self.fee_denominator == 0 {
            None
        } else {
            Some(
                u16::try_from((self.fee_numerator as u128 * 10_000) / self.fee_denominator as u128)
                    .unwrap_or(10_000),
            )
        }
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < RAYDIUM_V4_LAYOUT_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium v4 data size {} < expected {}",
                data.len(),
                RAYDIUM_V4_LAYOUT_SIZE
            )));
        }

        // Parse fees at offset 176 (SwapFeeNumerator: u64, SwapFeeDenominator: u64)
        if let Ok(fee_num_bytes) = data[176..184].try_into() {
            self.fee_numerator = u64::from_le_bytes(fee_num_bytes);
        }
        if let Ok(fee_denom_bytes) = data[184..192].try_into() {
            self.fee_denominator = u64::from_le_bytes(fee_denom_bytes);
        }

        // Base vault at offset 336 (32 bytes)
        if let Ok(bytes) = data[336..368].try_into() {
            self.base_vault = Pubkey::new_from_array(bytes);
        }
        // Quote vault at offset 368 (32 bytes)
        if let Ok(bytes) = data[368..400].try_into() {
            self.quote_vault = Pubkey::new_from_array(bytes);
        }
        // Base mint at offset 400 (32 bytes)
        if let Ok(bytes) = data[400..432].try_into() {
            self.base_mint = MintId(Pubkey::new_from_array(bytes));
        }
        // Quote mint at offset 432 (32 bytes)
        if let Ok(bytes) = data[432..464].try_into() {
            self.quote_mint = MintId(Pubkey::new_from_array(bytes));
        }

        // Open orders at offset 496 (32 bytes)
        if let Ok(bytes) = data[496..528].try_into() {
            self.open_orders = Pubkey::new_from_array(bytes);
        }

        // Target orders at offset 528 (32 bytes)
        if let Ok(bytes) = data[528..560].try_into() {
            self.target_orders = Pubkey::new_from_array(bytes);
        }

        // Serum market at offset 560 (32 bytes)
        if let Ok(bytes) = data[560..592].try_into() {
            self.serum_market = Pubkey::new_from_array(bytes);
        }

        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        let (r_in, r_out) = if a_to_b {
            (self.base_reserve, self.quote_reserve)
        } else {
            (self.quote_reserve, self.base_reserve)
        };

        if r_in == 0 || r_out == 0 {
            return Err(ArbError::Internal("Pool reserves are zero".into()));
        }

        let Some(fee_bps) = self.fee_bps() else {
            return Err(ArbError::Internal("Pool fee not configured".into()));
        };
        let gamma = 10_000u128.saturating_sub(fee_bps as u128);
        let Some(out) = cpmm_out(u128::from(amount_in), r_in, r_out, gamma) else {
            return Err(ArbError::Internal("CPMM quote overflow".into()));
        };
        u64::try_from(out).map_err(|_| ArbError::Internal("CPMM quote overflow".into()))
    }
}
