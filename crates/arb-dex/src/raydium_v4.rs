//! Raydium AMM v4 State Deserializer, OpenBook Market Dependency Tracking, and Quoting.

use crate::traits::AmmPool;
use arb_constants::{RAYDIUM_V4_AUTHORITY, RAYDIUM_V4_PROGRAM_ID};
use arb_math::cpmm_out_upfront_fee_ceil;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolExecutionAccounts, PoolId, Result, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const RAYDIUM_V4_LAYOUT_SIZE: usize = 752;
pub const OPENBOOK_MARKET_MIN_SIZE: usize = 388;

/// Raydium AMM v4 Pool representation.
#[derive(Debug, Clone)]
pub struct RaydiumV4Pool {
    pub pool_id: PoolId,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub base_vault_balance: u64,
    pub quote_vault_balance: u64,
    pub open_orders: Pubkey,
    pub target_orders: Pubkey,
    pub market_program: Pubkey,
    pub serum_market: Pubkey,
    pub serum_bids: Pubkey,
    pub serum_asks: Pubkey,
    pub serum_event_queue: Pubkey,
    pub serum_coin_vault: Pubkey,
    pub serum_pc_vault: Pubkey,
    pub serum_vault_signer: Pubkey,
    pub base_reserve: u128,
    pub quote_reserve: u128,
    pub need_take_pnl_coin: u64,
    pub need_take_pnl_pc: u64,
    pub fee_numerator: u64,
    pub fee_denominator: u64,
    pub status: u64,
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
            base_vault_balance: 0,
            quote_vault_balance: 0,
            open_orders,
            target_orders: Pubkey::default(),
            market_program: Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX")
                .unwrap(),
            serum_market: Pubkey::default(),
            serum_bids: Pubkey::default(),
            serum_asks: Pubkey::default(),
            serum_event_queue: Pubkey::default(),
            serum_coin_vault: Pubkey::default(),
            serum_pc_vault: Pubkey::default(),
            serum_vault_signer: Pubkey::default(),
            base_reserve: 0,
            quote_reserve: 0,
            need_take_pnl_coin: 0,
            need_take_pnl_pc: 0,
            // Unknown until the pool account is decoded. Never synthesize a
            // fee schedule or active status for a newly constructed pool.
            fee_numerator: 0,
            fee_denominator: 0,
            status: 0,
        }
    }

    /// Update reserves directly (from token account balances).
    pub fn update_reserves(&mut self, base_reserve: u128, quote_reserve: u128) {
        self.base_reserve = base_reserve;
        self.quote_reserve = quote_reserve;
    }

    /// Refresh effective reserves deducting need_take_pnl fees from vault balances.
    pub fn refresh_effective_reserves(&mut self) {
        let eff_base = self
            .base_vault_balance
            .saturating_sub(self.need_take_pnl_coin);
        let eff_quote = self
            .quote_vault_balance
            .saturating_sub(self.need_take_pnl_pc);
        if eff_base > 0 && eff_quote > 0 {
            self.base_reserve = eff_base as u128;
            self.quote_reserve = eff_quote as u128;
        }
    }

    /// Check if this pool is suitable for arbitrage.
    pub fn is_arbitrage_eligible(&self) -> bool {
        self.status == 6
            && self.base_reserve > 0
            && self.quote_reserve > 0
            && self.fee_numerator > 0
            && self.fee_denominator > 0
    }

    /// Calculate maximum tradable amount for arbitrage.
    pub fn max_arbitrage_amount(&self) -> u64 {
        std::cmp::min(
            self.base_reserve.try_into().unwrap_or(u64::MAX),
            self.quote_reserve.try_into().unwrap_or(u64::MAX),
        )
    }

    /// Get reserves as u64 for compatibility.
    pub fn reserves_u64(&self) -> (u64, u64) {
        (
            self.base_reserve.try_into().unwrap_or(u64::MAX),
            self.quote_reserve.try_into().unwrap_or(u64::MAX),
        )
    }

    /// Raydium authority PDA: [b"amm authority"] under Raydium V4 program.
    pub fn amm_authority() -> Pubkey {
        Pubkey::from_str(RAYDIUM_V4_AUTHORITY).unwrap_or_else(|_| {
            let prog = Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap();
            Pubkey::find_program_address(&[b"amm authority"], &prog).0
        })
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

    fn token_programs(&self) -> [TokenProgramKind; 2] {
        [TokenProgramKind::SplToken, TokenProgramKind::SplToken]
    }

    fn transfer_fees_bps(&self) -> (u16, u16) {
        (0, 0)
    }

    fn has_risky_token(&self) -> bool {
        false
    }

    fn execution_accounts(&self) -> Option<PoolExecutionAccounts> {
        if self.serum_market == Pubkey::default() || self.serum_vault_signer == Pubkey::default() {
            return None;
        }
        Some(PoolExecutionAccounts::RaydiumV4 {
            amm_authority: Self::amm_authority(),
            open_orders: self.open_orders,
            target_orders: if self.target_orders == Pubkey::default() {
                self.pool_id.0
            } else {
                self.target_orders
            },
            base_vault: self.base_vault,
            quote_vault: self.quote_vault,
            serum_program: self.market_program,
            serum_market: self.serum_market,
            serum_bids: self.serum_bids,
            serum_asks: self.serum_asks,
            serum_event_queue: self.serum_event_queue,
            serum_coin_vault: self.serum_coin_vault,
            serum_pc_vault: self.serum_pc_vault,
            serum_vault_signer: self.serum_vault_signer,
            swap_version: arb_types::RaydiumV4SwapVersion::V2,
        })
    }

    fn required_dependency_pubkeys(&self) -> Vec<Pubkey> {
        let mut keys = Vec::with_capacity(3);
        if self.base_vault != Pubkey::default() {
            keys.push(self.base_vault);
        }
        if self.quote_vault != Pubkey::default() {
            keys.push(self.quote_vault);
        }
        if self.serum_market != Pubkey::default() {
            keys.push(self.serum_market);
        }
        keys
    }

    fn apply_dependency_account(&mut self, pubkey: &Pubkey, data: &[u8]) -> Result<bool> {
        if *pubkey == self.base_vault {
            if data.len() >= 72 {
                let amount_bytes: [u8; 8] = data[64..72].try_into().map_err(|_| {
                    ArbError::DeserializationError("invalid base vault data".into())
                })?;
                self.base_vault_balance = u64::from_le_bytes(amount_bytes);
                self.refresh_effective_reserves();
                return Ok(true);
            }
        } else if *pubkey == self.quote_vault {
            if data.len() >= 72 {
                let amount_bytes: [u8; 8] = data[64..72].try_into().map_err(|_| {
                    ArbError::DeserializationError("invalid quote vault data".into())
                })?;
                self.quote_vault_balance = u64::from_le_bytes(amount_bytes);
                self.refresh_effective_reserves();
                return Ok(true);
            }
        } else if *pubkey == self.serum_market {
            if data.len() >= OPENBOOK_MARKET_MIN_SIZE {
                let nonce_bytes: [u8; 8] = data[45..53]
                    .try_into()
                    .map_err(|_| ArbError::DeserializationError("invalid market nonce".into()))?;
                let vault_signer_nonce = u64::from_le_bytes(nonce_bytes);

                let coin_vault_bytes: [u8; 32] = data[117..149]
                    .try_into()
                    .map_err(|_| ArbError::DeserializationError("invalid coin vault".into()))?;
                let pc_vault_bytes: [u8; 32] = data[165..197]
                    .try_into()
                    .map_err(|_| ArbError::DeserializationError("invalid pc vault".into()))?;
                let event_q_bytes: [u8; 32] = data[253..285]
                    .try_into()
                    .map_err(|_| ArbError::DeserializationError("invalid event queue".into()))?;
                let bids_bytes: [u8; 32] = data[285..317]
                    .try_into()
                    .map_err(|_| ArbError::DeserializationError("invalid bids".into()))?;
                let asks_bytes: [u8; 32] = data[317..349]
                    .try_into()
                    .map_err(|_| ArbError::DeserializationError("invalid asks".into()))?;

                self.serum_coin_vault = Pubkey::new_from_array(coin_vault_bytes);
                self.serum_pc_vault = Pubkey::new_from_array(pc_vault_bytes);
                self.serum_event_queue = Pubkey::new_from_array(event_q_bytes);
                self.serum_bids = Pubkey::new_from_array(bids_bytes);
                self.serum_asks = Pubkey::new_from_array(asks_bytes);

                // Derive serum_vault_signer: seeds = [market.as_ref(), &nonce.to_le_bytes()]
                if let Ok(signer) = Pubkey::create_program_address(
                    &[
                        self.serum_market.as_ref(),
                        &vault_signer_nonce.to_le_bytes(),
                    ],
                    &self.market_program,
                ) {
                    self.serum_vault_signer = signer;
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn update_from_account(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < RAYDIUM_V4_LAYOUT_SIZE {
            return Err(ArbError::DeserializationError(format!(
                "Raydium v4 data size {} < expected {}",
                data.len(),
                RAYDIUM_V4_LAYOUT_SIZE
            )));
        }

        // Status at 0..8
        if let Ok(status_bytes) = data[0..8].try_into() {
            self.status = u64::from_le_bytes(status_bytes);
        }

        // Parse fees at offset 176 (SwapFeeNumerator: u64, SwapFeeDenominator: u64)
        if let Ok(fee_num_bytes) = data[176..184].try_into() {
            self.fee_numerator = u64::from_le_bytes(fee_num_bytes);
        }
        if let Ok(fee_denom_bytes) = data[184..192].try_into() {
            self.fee_denominator = u64::from_le_bytes(fee_denom_bytes);
        }

        // need_take_pnl at 192..208
        if let Ok(pnl_coin) = data[192..200].try_into() {
            self.need_take_pnl_coin = u64::from_le_bytes(pnl_coin);
        }
        if let Ok(pnl_pc) = data[200..208].try_into() {
            self.need_take_pnl_pc = u64::from_le_bytes(pnl_pc);
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

        // Market (Serum / OpenBook market) at offset 528 (32 bytes)
        if let Ok(bytes) = data[528..560].try_into() {
            self.serum_market = Pubkey::new_from_array(bytes);
        }

        // Market Program at offset 560 (32 bytes)
        if let Ok(bytes) = data[560..592].try_into() {
            self.market_program = Pubkey::new_from_array(bytes);
        }

        // Target orders at offset 592 (32 bytes)
        if let Ok(bytes) = data[592..624].try_into() {
            self.target_orders = Pubkey::new_from_array(bytes);
        }

        self.refresh_effective_reserves();

        Ok(())
    }

    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> {
        if !self.is_arbitrage_eligible() {
            return Err(ArbError::InvalidInput(
                "Raydium AMM v4 pool is inactive or has invalid reserves/fees".into(),
            ));
        }
        let (r_in, r_out) = if a_to_b {
            (self.base_reserve, self.quote_reserve)
        } else {
            (self.quote_reserve, self.base_reserve)
        };

        if r_in == 0 || r_out == 0 {
            return Err(ArbError::Internal("Pool reserves are zero".into()));
        }

        let out = cpmm_out_upfront_fee_ceil(
            u128::from(amount_in),
            r_in,
            r_out,
            u128::from(self.fee_numerator),
            u128::from(self.fee_denominator),
        )
        .ok_or_else(|| {
            ArbError::Internal("Raydium V4 native-fee quote overflow or invalid fee".into())
        })?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raydium_v4_binary_offsets() {
        let mut data = vec![0u8; RAYDIUM_V4_LAYOUT_SIZE];
        data[0..8].copy_from_slice(&6u64.to_le_bytes()); // Status = 6
        data[176..184].copy_from_slice(&25u64.to_le_bytes()); // fee_num = 25
        data[184..192].copy_from_slice(&10_000u64.to_le_bytes()); // fee_denom = 10000
        data[192..200].copy_from_slice(&500u64.to_le_bytes()); // need_take_pnl_coin
        data[200..208].copy_from_slice(&300u64.to_le_bytes()); // need_take_pnl_pc

        let base_vault = Pubkey::new_unique();
        let quote_vault = Pubkey::new_unique();
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();
        let open_orders = Pubkey::new_unique();
        let market = Pubkey::new_unique();
        let market_prog = Pubkey::new_unique();
        let target_orders = Pubkey::new_unique();

        data[336..368].copy_from_slice(base_vault.as_ref());
        data[368..400].copy_from_slice(quote_vault.as_ref());
        data[400..432].copy_from_slice(base_mint.as_ref());
        data[432..464].copy_from_slice(quote_mint.as_ref());
        data[496..528].copy_from_slice(open_orders.as_ref());
        data[528..560].copy_from_slice(market.as_ref());
        data[560..592].copy_from_slice(market_prog.as_ref());
        data[592..624].copy_from_slice(target_orders.as_ref());

        let mut pool = RaydiumV4Pool::new(
            PoolId::new([1; 32]),
            MintId::new([0; 32]),
            MintId::new([0; 32]),
            Pubkey::default(),
            Pubkey::default(),
            Pubkey::default(),
        );

        pool.update_from_account(&data).unwrap();

        assert_eq!(pool.status, 6);
        assert_eq!(pool.fee_bps(), Some(25));
        assert_eq!(pool.base_vault, base_vault);
        assert_eq!(pool.quote_vault, quote_vault);
        assert_eq!(pool.base_mint.0, base_mint);
        assert_eq!(pool.quote_mint.0, quote_mint);
        assert_eq!(pool.open_orders, open_orders);
        assert_eq!(pool.serum_market, market);
        assert_eq!(pool.market_program, market_prog);
        assert_eq!(pool.target_orders, target_orders);
        assert_eq!(pool.need_take_pnl_coin, 500);
        assert_eq!(pool.need_take_pnl_pc, 300);
    }
}
