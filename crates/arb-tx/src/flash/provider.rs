//! Multi-Lending Provider Abstraction for Flash Loans.
//!
//! Provides a protocol-agnostic interface (`FlashLoanProvider`) across Solana
//! lending protocols (Kamino Lend, Solend/Save, Marginfi) while preserving
//! provider-specific security and atomicity invariants.

use crate::flash::kamino::{
    KaminoFlashAccounts, KaminoReserveCandidate, build_kamino_flash_borrow,
    build_kamino_flash_repay, derive_lending_market_authority,
};
use crate::primitives::error::TxError;
use crate::primitives::instruction::CompiledInstruction;
use arb_types::ArbError;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

/// Errors specific to lending provider abstractions and flash operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LendingError {
    #[error("insufficient liquidity: requested {requested}, available {available}")]
    InsufficientLiquidity { requested: u64, available: u64 },
    #[error("invalid provider: expected {expected:?}, got {actual:?}")]
    InvalidProvider {
        expected: LendingProviderId,
        actual: LendingProviderId,
    },
    #[error("provider error: {0}")]
    ProviderError(String),
}

/// Supported Lending Protocol Identifiers for Flash Loans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LendingProviderId {
    Kamino,
    Solend,
    Marginfi,
}

/// Normalized Snapshot of a Lending Reserve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveSnapshot {
    pub provider: LendingProviderId,
    pub reserve: Pubkey,
    pub mint: Pubkey,
    pub decimals: u8,
    pub active: bool,
    /// Raw U68F60 fee scale factor or equivalent fee representation.
    pub fee_sf: u64,
    /// Supply-vault balance currently available to borrow.
    pub available_liquidity: u64,
    pub supply_vault: Pubkey,
    pub fee_vault: Pubkey,
}

impl ReserveSnapshot {
    pub fn default_wsol() -> Self {
        use std::str::FromStr;
        Self {
            provider: LendingProviderId::Kamino,
            reserve: Pubkey::new_unique(),
            mint: Pubkey::from_str(crate::WSOL_MINT).unwrap_or_default(),
            decimals: 9,
            active: true,
            fee_sf: 11_529_215_046_068,          // 1e-5 (0.1 bps)
            available_liquidity: 10_000_000_000, // 10 SOL
            supply_vault: Pubkey::new_unique(),
            fee_vault: Pubkey::new_unique(),
        }
    }
}

impl From<&KaminoReserveCandidate> for ReserveSnapshot {
    fn from(c: &KaminoReserveCandidate) -> Self {
        Self {
            provider: LendingProviderId::Kamino,
            reserve: c.reserve,
            mint: c.mint,
            decimals: c.decimals,
            active: c.active,
            fee_sf: c.flash_loan_fee_sf,
            available_liquidity: c.available_liquidity,
            supply_vault: c.supply_vault,
            fee_vault: c.fee_vault,
        }
    }
}

/// Generic Flash Borrow Parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashBorrowParams {
    pub user_authority: Pubkey,
    pub user_liquidity_ata: Pubkey,
    pub borrow_amount: u64,
    pub reserve: ReserveSnapshot,
    pub lending_market: Pubkey,
    pub token_program: Pubkey,
}

/// Generic Flash Repay Parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashRepayParams {
    pub user_authority: Pubkey,
    pub user_liquidity_ata: Pubkey,
    /// Must equal the borrowed principal only. Fee is collected separately from caller's ATA.
    pub repay_amount: u64,
    pub borrow_instruction_index: u8,
    pub reserve: ReserveSnapshot,
    pub lending_market: Pubkey,
    pub token_program: Pubkey,
}

/// Universal Trait for Flash Loan Lending Providers.
///
/// Implementations must enforce:
/// 1. Zero floating point in calculations.
/// 2. Fail closed when liquidity or fees are invalid.
/// 3. Repay instruction arguments must encode borrowed principal only (where expected by protocol).
pub trait FlashLoanProvider: Send + Sync {
    /// Return the unique provider identifier.
    fn provider_id(&self) -> LendingProviderId;

    /// Calculate the flash loan fee in raw atomic units.
    fn calculate_fee(&self, amount: u64, reserve: &ReserveSnapshot) -> Result<u64, ArbError>;

    /// Return available supply-vault liquidity bounded by the live reserve state.
    fn available_liquidity(&self, reserve: &ReserveSnapshot) -> u64;

    /// Build the flash borrow instruction.
    fn build_borrow_ix(&self, params: &FlashBorrowParams) -> Result<CompiledInstruction, TxError>;

    /// Build the flash repay instruction.
    fn build_repay_ix(&self, params: &FlashRepayParams) -> Result<CompiledInstruction, TxError>;
}

/// Kamino Lend Flash Loan Provider implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct KaminoFlashLoanProvider {
    pub fee_sf_override: Option<u64>,
}

impl KaminoFlashLoanProvider {
    pub fn new(fee_sf: u64) -> Self {
        Self {
            fee_sf_override: Some(fee_sf),
        }
    }

    pub fn validate_and_build_borrow(
        &self,
        params: &FlashBorrowParams,
        reserve: &ReserveSnapshot,
    ) -> Result<CompiledInstruction, LendingError> {
        let available = self.available_liquidity(reserve);
        if params.borrow_amount > available {
            return Err(LendingError::InsufficientLiquidity {
                requested: params.borrow_amount,
                available,
            });
        }
        self.build_borrow_ix(params)
            .map_err(|e| LendingError::ProviderError(e.to_string()))
    }
}

impl FlashLoanProvider for KaminoFlashLoanProvider {
    fn provider_id(&self) -> LendingProviderId {
        LendingProviderId::Kamino
    }

    fn calculate_fee(&self, amount: u64, reserve: &ReserveSnapshot) -> Result<u64, ArbError> {
        let sf = self.fee_sf_override.unwrap_or(reserve.fee_sf);
        if self.fee_sf_override.is_none() && reserve.provider != LendingProviderId::Kamino {
            return Err(ArbError::InvalidInput(format!(
                "expected Kamino reserve snapshot, got {:?}",
                reserve.provider
            )));
        }
        arb_math::economic::calculate_flash_fee_raw(amount, sf)
    }

    fn available_liquidity(&self, reserve: &ReserveSnapshot) -> u64 {
        if !reserve.active || reserve.fee_sf == u64::MAX {
            0
        } else {
            reserve.available_liquidity
        }
    }

    fn build_borrow_ix(&self, params: &FlashBorrowParams) -> Result<CompiledInstruction, TxError> {
        if params.borrow_amount == 0 {
            return Err(TxError::ZeroBorrowAmount);
        }
        if !params.reserve.active {
            return Err(TxError::InvalidInput("reserve is inactive".to_string()));
        }
        if params.borrow_amount > params.reserve.available_liquidity {
            return Err(TxError::InvalidInput(format!(
                "borrow amount {} exceeds available liquidity {}",
                params.borrow_amount, params.reserve.available_liquidity
            )));
        }
        let (market_auth, _) = derive_lending_market_authority(
            &params.lending_market,
            &Pubkey::from_str_const(crate::KAMINO_KLEND_PROGRAM_ID),
        );
        let accounts = KaminoFlashAccounts {
            user_authority: params.user_authority,
            lending_market: params.lending_market,
            lending_market_authority: market_auth,
            reserve: params.reserve.reserve,
            reserve_liquidity_mint: params.reserve.mint,
            reserve_supply_vault: params.reserve.supply_vault,
            user_liquidity_ata: params.user_liquidity_ata,
            reserve_fee_vault: params.reserve.fee_vault,
            referrer_token_state: None,
            referrer_account: None,
            sysvar_instructions: Pubkey::from_str_const(crate::SYSVAR_INSTRUCTIONS_ID),
            token_program: params.token_program,
            klend_program_id: Pubkey::from_str_const(crate::KAMINO_KLEND_PROGRAM_ID),
        };
        Ok(build_kamino_flash_borrow(&accounts, params.borrow_amount))
    }

    fn build_repay_ix(&self, params: &FlashRepayParams) -> Result<CompiledInstruction, TxError> {
        if params.repay_amount == 0 {
            return Err(TxError::ZeroBorrowAmount);
        }
        let (market_auth, _) = derive_lending_market_authority(
            &params.lending_market,
            &Pubkey::from_str_const(crate::KAMINO_KLEND_PROGRAM_ID),
        );
        let accounts = KaminoFlashAccounts {
            user_authority: params.user_authority,
            lending_market: params.lending_market,
            lending_market_authority: market_auth,
            reserve: params.reserve.reserve,
            reserve_liquidity_mint: params.reserve.mint,
            reserve_supply_vault: params.reserve.supply_vault,
            user_liquidity_ata: params.user_liquidity_ata,
            reserve_fee_vault: params.reserve.fee_vault,
            referrer_token_state: None,
            referrer_account: None,
            sysvar_instructions: Pubkey::from_str_const(crate::SYSVAR_INSTRUCTIONS_ID),
            token_program: params.token_program,
            klend_program_id: Pubkey::from_str_const(crate::KAMINO_KLEND_PROGRAM_ID),
        };
        Ok(build_kamino_flash_repay(
            &accounts,
            params.repay_amount,
            params.borrow_instruction_index,
        ))
    }
}

/// Select the provider with the lowest fee among candidates having sufficient liquidity.
pub fn select_cheapest_flash_provider<'a>(
    providers: &'a [Box<dyn FlashLoanProvider>],
    amount: u64,
    reserve: &ReserveSnapshot,
) -> Result<&'a dyn FlashLoanProvider, ArbError> {
    let mut best: Option<(&'a dyn FlashLoanProvider, u64)> = None;
    for p in providers {
        if p.available_liquidity(reserve) >= amount {
            if let Ok(fee) = p.calculate_fee(amount, reserve) {
                if best.is_none() || fee < best.as_ref().unwrap().1 {
                    best = Some((p.as_ref(), fee));
                }
            }
        }
    }
    best.map(|(p, _)| p)
        .ok_or_else(|| ArbError::InsufficientLiquidity("no eligible lending provider".into()))
}

/// Universal Flash Transaction Builder supporting polymorphic lending providers.
pub struct UniversalFlashTxBuilder {
    pub provider: Box<dyn FlashLoanProvider>,
}

/// Standardized Flash Arbitrage Transaction Envelope.
#[derive(Debug, Clone)]
pub struct FlashArbitrageEnvelope {
    pub instructions: Vec<CompiledInstruction>,
    pub borrow_instruction_index: usize,
    pub repay_instruction_index: usize,
}

impl UniversalFlashTxBuilder {
    pub fn new(provider: Box<dyn FlashLoanProvider>) -> Self {
        Self { provider }
    }

    /// Assemble flash borrow + intermediate swap hops + flash repay into an atomic envelope.
    pub fn assemble_arbitrage_envelope(
        &self,
        borrow_params: &FlashBorrowParams,
        swap_instructions: Vec<CompiledInstruction>,
    ) -> Result<FlashArbitrageEnvelope, TxError> {
        let borrow_ix = self.provider.build_borrow_ix(borrow_params)?;
        let borrow_idx = 2; // following 2 compute budget instructions (CU limit + Priority fee)
        let repay_params = FlashRepayParams {
            user_authority: borrow_params.user_authority,
            user_liquidity_ata: borrow_params.user_liquidity_ata,
            repay_amount: borrow_params.borrow_amount,
            borrow_instruction_index: borrow_idx as u8,
            reserve: borrow_params.reserve.clone(),
            lending_market: borrow_params.lending_market,
            token_program: borrow_params.token_program,
        };
        let repay_ix = self.provider.build_repay_ix(&repay_params)?;

        let cu_limit_ix = CompiledInstruction {
            program_id: Pubkey::new_from_array(crate::COMPUTE_BUDGET_PROGRAM_ID),
            accounts: vec![],
            data: vec![2, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        let cu_price_ix = CompiledInstruction {
            program_id: Pubkey::new_from_array(crate::COMPUTE_BUDGET_PROGRAM_ID),
            accounts: vec![],
            data: vec![3, 0, 0, 0, 0, 0, 0, 0, 0],
        };

        let mut instructions = Vec::with_capacity(swap_instructions.len() + 4);
        instructions.push(cu_limit_ix);
        instructions.push(cu_price_ix);
        instructions.push(borrow_ix);
        instructions.extend(swap_instructions);
        let repay_idx = instructions.len();
        instructions.push(repay_ix);
        let tip_ix = CompiledInstruction {
            program_id: Pubkey::new_from_array(crate::SYSTEM_PROGRAM_ID),
            accounts: vec![],
            data: vec![0; 12],
        };
        instructions.push(tip_ix);

        Ok(FlashArbitrageEnvelope {
            instructions,
            borrow_instruction_index: borrow_idx,
            repay_instruction_index: repay_idx,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn sample_kamino_reserve() -> ReserveSnapshot {
        ReserveSnapshot {
            provider: LendingProviderId::Kamino,
            reserve: Pubkey::new_unique(),
            mint: Pubkey::from_str(crate::WSOL_MINT).unwrap(),
            decimals: 9,
            active: true,
            fee_sf: 11_529_215_046_068,          // 1e-5 (0.1 bps)
            available_liquidity: 50_000_000_000, // 50 SOL
            supply_vault: Pubkey::new_unique(),
            fee_vault: Pubkey::new_unique(),
        }
    }

    #[test]
    fn test_kamino_provider_conformance() {
        let provider = KaminoFlashLoanProvider::default();
        assert_eq!(provider.provider_id(), LendingProviderId::Kamino);

        let reserve = sample_kamino_reserve();
        assert_eq!(provider.available_liquidity(&reserve), 50_000_000_000);

        // 10 SOL borrow fee
        let fee = provider.calculate_fee(10_000_000_000, &reserve).unwrap();
        assert_eq!(fee, 100_000);

        let user = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let market = Pubkey::from_str(crate::KAMINO_MAIN_MARKET).unwrap();
        let token_program = Pubkey::from_str(crate::TOKEN_PROGRAM_ID).unwrap();

        let borrow_params = FlashBorrowParams {
            user_authority: user,
            user_liquidity_ata: user_ata,
            borrow_amount: 10_000_000_000,
            reserve: reserve.clone(),
            lending_market: market,
            token_program,
        };

        let borrow_ix = provider.build_borrow_ix(&borrow_params).unwrap();
        assert_eq!(borrow_ix.accounts.len(), 12);
        assert_eq!(&borrow_ix.data[0..8], &crate::FLASH_BORROW_DISCRIMINATOR);
        assert_eq!(&borrow_ix.data[8..16], &10_000_000_000u64.to_le_bytes());

        let repay_params = FlashRepayParams {
            user_authority: user,
            user_liquidity_ata: user_ata,
            repay_amount: 10_000_000_000,
            borrow_instruction_index: 3,
            reserve,
            lending_market: market,
            token_program,
        };

        let repay_ix = provider.build_repay_ix(&repay_params).unwrap();
        assert_eq!(repay_ix.accounts.len(), 12);
        assert_eq!(&repay_ix.data[0..8], &crate::FLASH_REPAY_DISCRIMINATOR);
        assert_eq!(&repay_ix.data[8..16], &10_000_000_000u64.to_le_bytes());
        assert_eq!(repay_ix.data[16], 3);
    }

    #[test]
    fn test_kamino_provider_bounds_borrow_to_available_liquidity() {
        let provider = KaminoFlashLoanProvider::default();
        let reserve = sample_kamino_reserve();
        let user = Pubkey::new_unique();
        let user_ata = Pubkey::new_unique();
        let market = Pubkey::from_str(crate::KAMINO_MAIN_MARKET).unwrap();
        let token_program = Pubkey::from_str(crate::TOKEN_PROGRAM_ID).unwrap();

        let borrow_params = FlashBorrowParams {
            user_authority: user,
            user_liquidity_ata: user_ata,
            borrow_amount: 50_000_000_001, // exceeds 50 SOL available
            reserve,
            lending_market: market,
            token_program,
        };

        let res = provider.build_borrow_ix(&borrow_params);
        assert!(matches!(res, Err(TxError::InvalidInput(_))));
    }
}
