//! Tests for Multi-Lending Provider Abstraction (Feature 4, Tier 1).
//!
//! Validates:
//! 1. Kamino provider implements FlashLoanProvider trait correctly.
//! 2. Multi-provider ranking selects the lowest fee provider.
//! 3. Borrow and repay instruction generation preserves principal invariance.
//! 4. Fail-closed behavior when borrow amount exceeds live available liquidity.
//! 5. Polymorphic provider dispatch in UniversalFlashTxBuilder.

use arb_tx::flash::{
    FlashBorrowParams, FlashLoanProvider, FlashRepayParams, KaminoFlashLoanProvider, LendingError,
    LendingProviderId, ReserveSnapshot, UniversalFlashTxBuilder, select_cheapest_flash_provider,
};
use arb_tx::primitives::error::TxError;
use arb_tx::primitives::instruction::CompiledInstruction;
use arb_types::ArbError;
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[derive(Debug, Clone)]
struct MockSolendProvider {
    fee_sf: u64,
}

impl MockSolendProvider {
    fn new(fee_sf: u64) -> Self {
        Self { fee_sf }
    }
}

impl FlashLoanProvider for MockSolendProvider {
    fn provider_id(&self) -> LendingProviderId {
        LendingProviderId::Solend
    }

    fn calculate_fee(&self, amount: u64, reserve: &ReserveSnapshot) -> Result<u64, ArbError> {
        let sf = if self.fee_sf > 0 {
            self.fee_sf
        } else {
            reserve.fee_sf
        };
        arb_math::economic::calculate_flash_fee_raw(amount, sf)
    }

    fn available_liquidity(&self, reserve: &ReserveSnapshot) -> u64 {
        reserve.available_liquidity
    }

    fn build_borrow_ix(&self, _params: &FlashBorrowParams) -> Result<CompiledInstruction, TxError> {
        Ok(CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![0x01],
        })
    }

    fn build_repay_ix(&self, _params: &FlashRepayParams) -> Result<CompiledInstruction, TxError> {
        Ok(CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![0x02],
        })
    }
}

#[derive(Debug, Clone)]
struct MockMarginfiProvider {
    _fee_sf: u64,
}

impl MockMarginfiProvider {
    fn new(_fee_sf: u64) -> Self {
        Self { _fee_sf }
    }
}

impl FlashLoanProvider for MockMarginfiProvider {
    fn provider_id(&self) -> LendingProviderId {
        LendingProviderId::Marginfi
    }

    fn calculate_fee(&self, _amount: u64, _reserve: &ReserveSnapshot) -> Result<u64, ArbError> {
        // Zero fee promotion / edge model
        Ok(0)
    }

    fn available_liquidity(&self, reserve: &ReserveSnapshot) -> u64 {
        reserve.available_liquidity
    }

    fn build_borrow_ix(&self, _params: &FlashBorrowParams) -> Result<CompiledInstruction, TxError> {
        Ok(CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![0x03],
        })
    }

    fn build_repay_ix(&self, _params: &FlashRepayParams) -> Result<CompiledInstruction, TxError> {
        Ok(CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![0x04],
        })
    }
}

fn sample_wsol_reserve() -> ReserveSnapshot {
    ReserveSnapshot {
        provider: LendingProviderId::Kamino,
        reserve: Pubkey::new_unique(),
        mint: Pubkey::from_str(arb_constants::WSOL_MINT).unwrap(),
        decimals: 9,
        active: true,
        fee_sf: 11_529_215_046_068,          // 1 bps
        available_liquidity: 10_000_000_000, // 10 SOL
        supply_vault: Pubkey::new_unique(),
        fee_vault: Pubkey::new_unique(),
    }
}

fn sample_borrow_params(amount: u64, reserve: ReserveSnapshot) -> FlashBorrowParams {
    FlashBorrowParams {
        user_authority: Pubkey::new_unique(),
        user_liquidity_ata: Pubkey::new_unique(),
        borrow_amount: amount,
        reserve,
        lending_market: Pubkey::from_str(arb_constants::KAMINO_MAIN_MARKET).unwrap(),
        token_program: Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap(),
    }
}

#[test]
fn test_kamino_provider_implements_flash_loan_provider_trait() {
    let provider = KaminoFlashLoanProvider::default();
    assert_eq!(provider.provider_id(), LendingProviderId::Kamino);

    let reserve = sample_wsol_reserve();
    let fee = provider.calculate_fee(1_000_000_000, &reserve).unwrap();
    // 1 SOL with 1 bps (11_529_215_046_068 sf) is 10_000 lamports
    assert_eq!(fee, 10_000);
    assert_eq!(provider.available_liquidity(&reserve), 10_000_000_000);
}

#[test]
fn test_multi_provider_fee_ranking_and_selection() {
    let kamino = KaminoFlashLoanProvider::new(11_529_215_046_068); // 1 bps
    let solend = MockSolendProvider::new(30_000_000_000_000); // ~2.6 bps
    let marginfi = MockMarginfiProvider::new(0); // 0 bps

    let providers: Vec<Box<dyn FlashLoanProvider>> =
        vec![Box::new(kamino), Box::new(solend), Box::new(marginfi)];

    let reserve = sample_wsol_reserve();
    let best = select_cheapest_flash_provider(&providers, 1_000_000_000, &reserve).unwrap();
    assert_eq!(best.provider_id(), LendingProviderId::Marginfi);
}

#[test]
fn test_provider_borrow_and_repay_instruction_generation() {
    let provider = KaminoFlashLoanProvider::default();
    let reserve = sample_wsol_reserve();
    let params = sample_borrow_params(5_000_000_000, reserve.clone());

    let borrow_ix = provider.build_borrow_ix(&params).unwrap();
    let repay_params = FlashRepayParams {
        user_authority: params.user_authority,
        user_liquidity_ata: params.user_liquidity_ata,
        repay_amount: params.borrow_amount,
        borrow_instruction_index: 2,
        reserve: params.reserve.clone(),
        lending_market: params.lending_market,
        token_program: params.token_program,
    };
    let repay_ix = provider.build_repay_ix(&repay_params).unwrap();

    assert_eq!(borrow_ix.data[8..16], 5_000_000_000u64.to_le_bytes());
    assert_eq!(repay_ix.data[8..16], 5_000_000_000u64.to_le_bytes()); // Principal only
    assert_eq!(repay_ix.data[16], 2); // borrow_instruction_index

    // Sysvar instructions account must be at index 10
    let sysvar_pk = Pubkey::from_str(arb_constants::SYSVAR_INSTRUCTIONS_ID).unwrap();
    assert_eq!(repay_ix.accounts[10].pubkey, sysvar_pk);
}

#[test]
fn test_provider_fails_closed_when_borrow_exceeds_liquidity() {
    let provider = KaminoFlashLoanProvider::default();
    let mut reserve = sample_wsol_reserve();
    reserve.available_liquidity = 2_000_000_000; // 2 SOL

    let params = sample_borrow_params(2_000_000_001, reserve.clone()); // 2 SOL + 1 lamport
    let res = provider.validate_and_build_borrow(&params, &reserve);
    assert_eq!(
        res,
        Err(LendingError::InsufficientLiquidity {
            requested: 2_000_000_001,
            available: 2_000_000_000,
        })
    );

    let res_direct = provider.build_borrow_ix(&params);
    assert!(matches!(res_direct, Err(TxError::InvalidInput(_))));
}

#[test]
fn test_polymorphic_provider_dispatch_in_tx_builder() {
    let provider: Box<dyn FlashLoanProvider> = Box::new(KaminoFlashLoanProvider::default());
    let builder = UniversalFlashTxBuilder::new(provider);
    let reserve = sample_wsol_reserve();
    let params = sample_borrow_params(1_000_000_000, reserve);

    let hop1_ix = CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![],
        data: vec![0x10],
    };
    let hop2_ix = CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![],
        data: vec![0x20],
    };

    let envelope = builder
        .assemble_arbitrage_envelope(&params, vec![hop1_ix, hop2_ix])
        .unwrap();

    assert_eq!(envelope.instructions.len(), 7);
    assert_eq!(envelope.borrow_instruction_index, 2);
    assert_eq!(envelope.repay_instruction_index, 5);

    // Verify discriminator and arguments on borrow and repay
    assert_eq!(
        &envelope.instructions[2].data[0..8],
        &arb_constants::FLASH_BORROW_DISCRIMINATOR
    );
    assert_eq!(
        &envelope.instructions[5].data[0..8],
        &arb_constants::FLASH_REPAY_DISCRIMINATOR
    );
    assert_eq!(envelope.instructions[5].data[16], 2); // references borrow at index 2
}
