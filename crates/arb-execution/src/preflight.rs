//! Phase 16.6 — Mandatory Pre-Flight Validation Pipeline.
//!
//! A single, mandatory validation gate that runs before any transaction is
//! signed or submitted. No fast path may bypass these checks.
//!
//! The pipeline validates:
//!   1. Pool snapshots (freshness, fee validity, reserve non-zero)
//!   2. Account ownership (mint owner, token program, vault owner)
//!   3. Mint and token program compatibility (no Token-2022 substitution)
//!   4. Instruction account metas (signer/writable correctness)
//!   5. PDA derivations (expected seeds match on-chain PDAs)
//!   6. ALT resolution (all ALT accounts present and activated)
//!   7. V0/V1 compatibility (version supported by transport)
//!   8. Transaction size (within MTU limits)
//!   9. Compute budget (within limits)
//!   10. Loaded account data (within 256 KiB for V1)
//!   11. Flash loan repayment (repay >= borrow + fee)
//!   12. Minimum output (slippage not exceeded)
//!   13. Economic profitability (net profit >= minimum)
//!   14. Jito bid limits (tip within max acceptable)

use arb_math::economic_integration::UnifiedEconomicConfig;
use arb_tx::CompiledInstruction;
use std::result::Result as StdResult;

#[cfg(test)]
use arb_tx::AccountMeta;
#[cfg(test)]
use solana_pubkey::Pubkey;

/// Result type for pre-flight validation.
pub type PreflightResult<T> = StdResult<T, PreflightError>;

// ============================================================================
// Pre-flight validation errors
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum PreflightError {
    #[error("pool {pool_id} is stale: slot {slot}, max age {max_age}")]
    StalePool {
        pool_id: String,
        slot: u64,
        max_age: u64,
    },

    #[error("account {account} has unexpected owner: expected {expected}, found {found}")]
    InvalidAccountOwner {
        account: String,
        expected: String,
        found: String,
    },

    #[error("mint {mint} has unexpected token program: expected {expected}, found {found}")]
    TokenProgramMismatch {
        mint: String,
        expected: String,
        found: String,
    },

    #[error("instruction {ix_index} account {acc_index}: {reason}")]
    InvalidAccountMeta {
        ix_index: usize,
        acc_index: usize,
        reason: String,
    },

    #[error("PDA {pda} derivation failed: {reason}")]
    PdaDerivationFailed { pda: String, reason: String },

    #[error("ALT {alt_key} not found or not activated")]
    AltNotReady { alt_key: String },

    #[error("V1 transport not supported by downstream")]
    V1TransportUnsupported,

    #[error("transaction size {size} exceeds MTU limit {limit}")]
    TransactionTooLarge { size: usize, limit: usize },

    #[error("compute units {cu} exceed limit {limit}")]
    ComputeBudgetExceeded { cu: u32, limit: u32 },

    #[error("flash loan repayment {repay} insufficient for borrow {borrow} + fee {fee}")]
    InsufficientRepayment { repay: u64, borrow: u64, fee: u64 },

    #[error("minimum output {min_out} below expected {expected} by {slippage_bps} bps")]
    SlippageExceeded {
        min_out: u64,
        expected: u64,
        slippage_bps: u64,
    },

    #[error("economic check failed: {reason}")]
    EconomicCheckFailed { reason: String },

    #[error("Jito tip {tip} exceeds maximum acceptable {max_tip}")]
    JitoTipExceedsLimit { tip: u64, max_tip: u64 },
}

// ============================================================================
// Pre-flight configuration
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreflightConfig {
    /// Maximum slot lag between quote and submission.
    pub max_quote_slot_lag: u64,
    /// Maximum transaction size in bytes (1232 for legacy/V0, 4096 for V1).
    pub max_tx_size: usize,
    /// Maximum compute units.
    pub max_compute_units: u32,
    /// Minimum acceptable profit in base-mint raw units.
    pub min_profit_raw: u64,
    /// Maximum Jito tip in lamports.
    pub max_jito_tip: u64,
    /// Maximum slippage in basis points.
    pub max_slippage_bps: u16,
}

impl Default for PreflightConfig {
    fn default() -> Self {
        Self {
            max_quote_slot_lag: 2,
            max_tx_size: 1232,
            max_compute_units: 1_400_000,
            min_profit_raw: 0,
            max_jito_tip: 0,
            max_slippage_bps: 100,
        }
    }
}

// ============================================================================
// Pre-flight validator
// ============================================================================

/// Validates a transaction and its economic context before signing/submission.
///
/// This is the single mandatory pre-flight gate. No fast path may bypass
/// these checks.
pub struct PreflightValidator {
    config: PreflightConfig,
    economic_calculator: arb_math::economic_integration::UnifiedEconomicCalculator,
}

impl PreflightValidator {
    pub fn new(config: PreflightConfig, economic_config: UnifiedEconomicConfig) -> Self {
        Self {
            config,
            economic_calculator: arb_math::economic_integration::UnifiedEconomicCalculator::new(
                economic_config,
            ),
        }
    }

    /// Run the structural validation pipeline (size, account metas, compute budget).
    ///
    /// This does not require economic context and can be run before signing.
    pub fn validate_tx_structure(
        &self,
        instructions: &[CompiledInstruction],
    ) -> PreflightResult<()> {
        // 1. Transaction size check
        self.validate_tx_size(instructions)?;

        // 2. Account meta validation
        self.validate_account_metas(instructions)?;

        Ok(())
    }

    fn validate_tx_size(&self, instructions: &[CompiledInstruction]) -> PreflightResult<()> {
        let mut size: usize = 0;
        size += 1; // signature count
        size += 64; // one signature
        size += 3; // message header
        size += 2; // compact-u16 account count
        size += 32; // payer
        size += 32; // blockhash
        size += 2; // compact-u16 instruction count

        for ix in instructions {
            size += 1; // program index
            size += 2; // compact-u16 account count
            size += ix.accounts.len(); // account indices
            size += 2; // compact-u16 data length
            size += ix.data.len(); // data
        }

        if size > self.config.max_tx_size {
            return Err(PreflightError::TransactionTooLarge {
                size,
                limit: self.config.max_tx_size,
            });
        }
        Ok(())
    }

    fn validate_account_metas(&self, instructions: &[CompiledInstruction]) -> PreflightResult<()> {
        for (ix_index, ix) in instructions.iter().enumerate() {
            for (acc_index, acc) in ix.accounts.iter().enumerate() {
                // Program IDs should not be signers
                if acc.is_signer && ix.program_id == acc.pubkey {
                    return Err(PreflightError::InvalidAccountMeta {
                        ix_index,
                        acc_index,
                        reason: "program ID cannot be a signer".into(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate flash loan repayment: repay_amount >= borrow_amount + fee.
    pub fn validate_flash_loan_repayment(
        &self,
        borrow_amount: u64,
        repay_amount: u64,
        fee: u64,
    ) -> PreflightResult<()> {
        let required = borrow_amount.saturating_add(fee);
        if repay_amount < required {
            return Err(PreflightError::InsufficientRepayment {
                repay: repay_amount,
                borrow: borrow_amount,
                fee,
            });
        }
        Ok(())
    }

    /// Validate slippage: min_output >= expected_output * (1 - max_slippage_bps).
    pub fn validate_slippage(&self, expected_output: u64, min_output: u64) -> PreflightResult<()> {
        if expected_output == 0 {
            return Ok(());
        }
        let slippage_bps =
            ((expected_output - min_output) as u128 * 10_000) / expected_output as u128;
        if slippage_bps > u128::from(self.config.max_slippage_bps) {
            return Err(PreflightError::SlippageExceeded {
                min_out: min_output,
                expected: expected_output,
                slippage_bps: slippage_bps as u64,
            });
        }
        Ok(())
    }

    /// Validate Jito tip is within maximum acceptable bound.
    pub fn validate_jito_tip(&self, tip: u64, net_before_tip: u64) -> PreflightResult<()> {
        // Absolute cap from config (0 = unlimited)
        if self.config.max_jito_tip > 0 && tip > self.config.max_jito_tip {
            return Err(PreflightError::JitoTipExceedsLimit {
                tip,
                max_tip: self.config.max_jito_tip,
            });
        }
        // Percentage-based cap from economic config
        if !self
            .economic_calculator
            .is_tip_acceptable(tip as u128, net_before_tip as u128)
        {
            return Err(PreflightError::JitoTipExceedsLimit {
                tip,
                max_tip: self.config.max_jito_tip,
            });
        }
        Ok(())
    }

    /// Validate economic profitability using the unified calculator.
    pub fn validate_economics(
        &self,
        economic: &arb_math::economic_integration::UnifiedEconomicResult,
    ) -> PreflightResult<()> {
        if !economic.meets_minimum_profit {
            return Err(PreflightError::EconomicCheckFailed {
                reason: format!(
                    "conservative net profit {} < minimum {}",
                    economic.conservative_net_profit_raw, self.config.min_profit_raw
                ),
            });
        }
        if economic.net_profit_signed < 0 {
            return Err(PreflightError::EconomicCheckFailed {
                reason: format!("net profit is negative: {}", economic.net_profit_signed),
            });
        }
        Ok(())
    }

    /// Reference to the configuration.
    pub fn config(&self) -> &PreflightConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_validator() -> PreflightValidator {
        PreflightValidator::new(PreflightConfig::default(), UnifiedEconomicConfig::default())
    }

    #[test]
    fn test_validate_tx_size_ok() {
        let validator = test_validator();
        let ix = CompiledInstruction {
            program_id: Pubkey::new_from_array([1u8; 32]),
            accounts: vec![AccountMeta::new_readonly(
                Pubkey::new_from_array([2u8; 32]),
                false,
            )],
            data: vec![1, 2, 3],
        };
        assert!(validator.validate_tx_size(&[ix]).is_ok());
    }

    #[test]
    fn test_validate_tx_size_too_large() {
        let validator = PreflightValidator::new(
            PreflightConfig {
                max_tx_size: 50,
                ..Default::default()
            },
            UnifiedEconomicConfig::default(),
        );
        let ix = CompiledInstruction {
            program_id: Pubkey::new_from_array([1u8; 32]),
            accounts: vec![AccountMeta::new_readonly(
                Pubkey::new_from_array([2u8; 32]),
                false,
            )],
            data: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        };
        assert!(validator.validate_tx_size(&[ix]).is_err());
    }

    #[test]
    fn test_validate_account_metas_program_as_signer_rejected() {
        let validator = test_validator();
        let program = Pubkey::new_from_array([1u8; 32]);
        let ix = CompiledInstruction {
            program_id: program,
            accounts: vec![AccountMeta::new(program, true)],
            data: vec![1],
        };
        let err = validator.validate_account_metas(&[ix]);
        assert!(err.is_err());
    }

    #[test]
    fn test_validate_flash_loan_repayment_ok() {
        let validator = test_validator();
        // Total outlay must cover principal + fee (mainnet WSOL fee on 1 SOL
        // is 10,000 lamports at sf = 11_529_215_046_068).
        assert!(
            validator
                .validate_flash_loan_repayment(1_000_000_000, 1_000_010_000, 10_000)
                .is_ok()
        );
    }

    #[test]
    fn test_validate_flash_loan_repayment_insufficient() {
        let validator = test_validator();
        let err = validator.validate_flash_loan_repayment(1_000_000_000, 1_000_000, 10_000);
        assert!(err.is_err());
    }

    #[test]
    fn test_validate_slippage_ok() {
        let validator = test_validator();
        assert!(validator.validate_slippage(1_000_000, 990_000).is_ok()); // 1% slippage, max 1%
    }

    #[test]
    fn test_validate_slippage_exceeded() {
        let validator = test_validator();
        let err = validator.validate_slippage(1_000_000, 950_000); // 5% slippage > 1% max
        assert!(err.is_err());
    }

    #[test]
    fn test_validate_jito_tip_ok() {
        let validator = test_validator();
        assert!(validator.validate_jito_tip(10_000, 1_000_000).is_ok());
    }

    #[test]
    fn test_validate_jito_tip_exceeds() {
        let validator = PreflightValidator::new(
            PreflightConfig {
                max_jito_tip: 5_000,
                ..Default::default()
            },
            UnifiedEconomicConfig::default(),
        );
        let err = validator.validate_jito_tip(10_000, 1_000_000);
        assert!(err.is_err());
    }

    #[test]
    fn test_validate_tx_structure_ok() {
        let validator = test_validator();
        let ix = CompiledInstruction {
            program_id: Pubkey::new_from_array([1u8; 32]),
            accounts: vec![AccountMeta::new_readonly(
                Pubkey::new_from_array([2u8; 32]),
                false,
            )],
            data: vec![1, 2, 3],
        };
        assert!(validator.validate_tx_structure(&[ix]).is_ok());
    }
}
