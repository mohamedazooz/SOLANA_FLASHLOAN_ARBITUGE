//! On-Chain Profit Assertion Guard for Pump.fun-Raydium Arbitrage
//!
//! This module implements the profit assertion mechanism that ensures atomic
//! arbitrage transactions only execute when they are profitable. If the
//! post-trade balance is less than the pre-trade balance plus minimum profit,
//! the transaction will revert on-chain.
//!
//! # Implementation
//!
//! The guard works by:
//! 1. Recording the pre-balance of the profit account
//! 2. Executing all swap instructions
//! 3. Recording the post-balance
//! 4. Asserting that post-balance >= pre-balance + min_profit
//!
//! This is implemented as a custom instruction that can be added to any
//! arbitrage transaction.

use arb_types::{ArbError, Lamports, Result};
use solana_pubkey::Pubkey;

// ============================================================================
// Profit Guard Configuration
// ============================================================================

/// Configuration for the profit guard.
#[derive(Debug, Clone)]
pub struct ProfitGuardConfig {
    /// Minimum net profit required in lamports.
    pub min_net_profit_lamports: u64,
    /// Whether to include flash loan fees in the assertion.
    pub include_flash_fee: bool,
    /// Whether to include priority fees in the assertion.
    pub include_priority_fee: bool,
}

impl Default for ProfitGuardConfig {
    fn default() -> Self {
        Self {
            min_net_profit_lamports: arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS,
            include_flash_fee: true,
            include_priority_fee: true,
        }
    }
}

// ============================================================================
// Profit Guard Instruction
// ============================================================================

/// Profit assertion guard instruction.
///
/// This instruction is added at the END of an arbitrage transaction to ensure
/// that the transaction is only successful if it's profitable.
#[derive(Debug, Clone)]
pub struct AssertMinimumProfit {
    /// The account that receives the profit (usually the fee payer).
    pub profit_account: Pubkey,
    /// The pre-trade balance of the profit account (stored in instruction data).
    pub pre_balance_lamports: u64,
    /// Minimum required net profit in lamports.
    pub min_net_profit_lamports: u64,
}

impl AssertMinimumProfit {
    /// Create a new profit assertion instruction.
    pub fn new(profit_account: Pubkey, pre_balance: u64, min_profit: u64) -> Self {
        Self {
            profit_account,
            pre_balance_lamports: pre_balance,
            min_net_profit_lamports: min_profit,
        }
    }

    /// Serialize the instruction for on-chain execution.
    ///
    /// Format: [discriminator: u8, pre_balance: u64, min_profit: u64]
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(17);
        
        // Discriminator for AssertMinimumProfit
        data.push(0x42); // 'B' for "Balance check"
        
        // Pre-balance (8 bytes, little-endian)
        data.extend_from_slice(&self.pre_balance_lamports.to_le_bytes());
        
        // Min profit (8 bytes, little-endian)
        data.extend_from_slice(&self.min_net_profit_lamports.to_le_bytes());
        
        data
    }

    /// Deserialize from instruction data.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        if data.len() < 17 {
            return Err(ArbError::DeserializationError(
                "AssertMinimumProfit: insufficient data".into(),
            ));
        }
        
        if data[0] != 0x42 {
            return Err(ArbError::DeserializationError(
                "AssertMinimumProfit: invalid discriminator".into(),
            ));
        }
        
        let pre_balance = u64::from_le_bytes(data[1..9].try_into().map_err(|_| {
            ArbError::DeserializationError("AssertMinimumProfit: invalid pre_balance".into())
        })?);
        
        let min_profit = u64::from_le_bytes(data[9..17].try_into().map_err(|_| {
            ArbError::DeserializationError("AssertMinimumProfit: invalid min_profit".into())
        })?);
        
        // We don't have the profit_account in the data, it's in the account list
        Ok(Self {
            profit_account: Pubkey::default(), // Will be set from account list
            pre_balance_lamports: pre_balance,
            min_net_profit_lamports: min_profit,
        })
    }

    /// Get the required accounts for this instruction.
    pub fn accounts(&self) -> Vec<Pubkey> {
        vec![self.profit_account]
    }
}

// ============================================================================
// Profit Guard Builder
// ============================================================================

/// Builder for creating profit guard instructions.
pub struct ProfitGuardBuilder {
    config: ProfitGuardConfig,
}

impl ProfitGuardBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: ProfitGuardConfig::default(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: ProfitGuardConfig) -> Self {
        Self { config }
    }

    /// Build a profit assertion instruction.
    pub fn build(&self, profit_account: Pubkey, pre_balance: u64) -> AssertMinimumProfit {
        AssertMinimumProfit::new(
            profit_account,
            pre_balance,
            self.config.min_net_profit_lamports,
        )
    }

    /// Build a profit assertion with custom minimum profit.
    pub fn build_with_min_profit(
        &self,
        profit_account: Pubkey,
        pre_balance: u64,
        min_profit: u64,
    ) -> AssertMinimumProfit {
        AssertMinimumProfit::new(profit_account, pre_balance, min_profit)
    }

    /// Calculate the expected post-balance for a profitable trade.
    pub fn expected_post_balance(&self, pre_balance: u64, net_profit: i64) -> u64 {
        if net_profit > 0 {
            pre_balance.saturating_add(net_profit as u64)
        } else {
            pre_balance
        }
    }
}

// ============================================================================
// Profit Guard Validator
// ============================================================================

/// Validator for checking if a trade would be profitable.
pub struct ProfitGuardValidator {
    config: ProfitGuardConfig,
}

impl ProfitGuardValidator {
    /// Create a new validator.
    pub fn new(config: ProfitGuardConfig) -> Self {
        Self { config }
    }

    /// Validate that a trade meets the profit requirements.
    ///
    /// Returns true if the trade is profitable and should be executed.
    pub fn validate(
        &self,
        pre_balance: u64,
        post_balance: u64,
        flash_loan_fee: u64,
        priority_fee: u64,
    ) -> bool {
        let net_profit = post_balance.saturating_sub(pre_balance);
        
        let required_profit = self.config.min_net_profit_lamports;
        
        if self.config.include_flash_fee {
            // Net profit must cover flash loan fee
            if net_profit <= flash_loan_fee {
                return false;
            }
        }
        
        if self.config.include_priority_fee {
            // Net profit must cover priority fee
            if net_profit <= priority_fee {
                return false;
            }
        }
        
        // Final check
        if self.config.include_flash_fee && self.config.include_priority_fee {
            net_profit > required_profit + flash_loan_fee + priority_fee
        } else if self.config.include_flash_fee {
            net_profit > required_profit + flash_loan_fee
        } else if self.config.include_priority_fee {
            net_profit > required_profit + priority_fee
        } else {
            net_profit > required_profit
        }
    }

    /// Calculate the minimum required post-balance.
    pub fn min_required_post_balance(
        &self,
        pre_balance: u64,
        flash_loan_fee: u64,
        priority_fee: u64,
    ) -> u64 {
        let mut required = pre_balance.saturating_add(self.config.min_net_profit_lamports);
        
        if self.config.include_flash_fee {
            required = required.saturating_add(flash_loan_fee);
        }
        
        if self.config.include_priority_fee {
            required = required.saturating_add(priority_fee);
        }
        
        required
    }
}

// ============================================================================
// Profit Guard Error
// ============================================================================

/// Custom error for profit guard failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfitGuardError {
    /// Transaction would result in a loss.
    UnprofitableTransaction,
    /// Flash loan fee exceeds profit.
    FlashLoanFeeTooHigh,
    /// Priority fee exceeds profit.
    PriorityFeeTooHigh,
    /// Minimum profit threshold not met.
    MinimumProfitNotMet,
}

impl std::fmt::Display for ProfitGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfitGuardError::UnprofitableTransaction => {
                write!(f, "Transaction would result in a loss")
            }
            ProfitGuardError::FlashLoanFeeTooHigh => {
                write!(f, "Flash loan fee exceeds profit")
            }
            ProfitGuardError::PriorityFeeTooHigh => {
                write!(f, "Priority fee exceeds profit")
            }
            ProfitGuardError::MinimumProfitNotMet => {
                write!(f, "Minimum profit threshold not met")
            }
        }
    }
}

impl std::error::Error for ProfitGuardError {}

// ============================================================================
// On-Chain Program (Reference Implementation)
// ============================================================================

/// Reference implementation of the on-chain profit guard logic.
///
/// This would be implemented in the arb-executor program.
///
/// ```rust,ignore
/// #[program]
/// pub mod arb_executor {
///     use solana_program::{
///         account_info::AccountInfo,
///         entrypoint::ProgramResult,
///         msg,
///         program_error::ProgramError,
///     };
///     
///     /// Custom error codes
///     #[error_code]
///     pub enum ArbError {
///         #[msg("Unprofitable arbitrage")]
///         UnprofitableArbitrage = 0x1000,
///     }
///     
///     pub fn process_instruction(
///         program_id: &Pubkey,
///         accounts: &[AccountInfo],
///         instruction_data: &[u8],
///     ) -> ProgramResult {
///         let discriminator = instruction_data[0];
///         
///         if discriminator == 0x42 {
///             // AssertMinimumProfit instruction
///             if instruction_data.len() < 17 {
///                 return Err(ProgramError::InvalidInstructionData);
///             }
///             
///             let pre_balance = u64::from_le_bytes(
///                 instruction_data[1..9].try_into().map_err(|_| ProgramError::InvalidInstructionData)?
///             );
///             let min_profit = u64::from_le_bytes(
///                 instruction_data[9..17].try_into().map_err(|_| ProgramError::InvalidInstructionData)?
///             );
///             
///             let profit_account = &accounts[0];
///             let post_balance = profit_account.lamports();
///             
///             if post_balance < pre_balance.saturating_add(min_profit) {
///                 msg!("Profit guard failed: post_balance={} < required={}", 
///                     post_balance, pre_balance + min_profit);
///                 return Err(ArbError::UnprofitableArbitrage.into());
///             }
///             
///             Ok(())
///         } else {
///             Err(ProgramError::InvalidInstructionData)
///         }
///     }
/// }
/// ```

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_assert_minimum_profit_serialization() {
        let profit_account = Pubkey::new_unique();
        let guard = AssertMinimumProfit::new(profit_account, 1_000_000, 50_000);
        
        let data = guard.serialize();
        assert_eq!(data.len(), 17);
        assert_eq!(data[0], 0x42);
        
        // Check pre_balance
        let pre_balance = u64::from_le_bytes(data[1..9].try_into().unwrap());
        assert_eq!(pre_balance, 1_000_000);
        
        // Check min_profit
        let min_profit = u64::from_le_bytes(data[9..17].try_into().unwrap());
        assert_eq!(min_profit, 50_000);
    }

    #[test]
    fn test_assert_minimum_profit_deserialization() {
        let mut data = vec![0x42];
        data.extend_from_slice(&1_000_000u64.to_le_bytes());
        data.extend_from_slice(&50_000u64.to_le_bytes());
        
        let guard = AssertMinimumProfit::deserialize(&data).unwrap();
        assert_eq!(guard.pre_balance_lamports, 1_000_000);
        assert_eq!(guard.min_net_profit_lamports, 50_000);
    }

    #[test]
    fn test_profit_guard_validation() {
        let config = ProfitGuardConfig {
            min_net_profit_lamports: 50_000,
            include_flash_fee: true,
            include_priority_fee: true,
        };
        
        let validator = ProfitGuardValidator::new(config);
        
        // Test profitable trade
        assert!(validator.validate(
            1_000_000, // pre_balance
            1_060_000, // post_balance (+60k)
            5_000,     // flash_fee
            5_000,     // priority_fee
        ));
        
        // Test unprofitable trade (net profit < min)
        assert!(!validator.validate(
            1_000_000,
            1_040_000, // +40k (less than min 50k)
            5_000,
            5_000,
        ));
        
        // Test flash fee too high
        assert!(!validator.validate(
            1_000_000,
            1_050_000, // +50k
            60_000,    // flash fee > profit
            5_000,
        ));
    }

    #[test]
    fn test_min_required_post_balance() {
        let config = ProfitGuardConfig {
            min_net_profit_lamports: 50_000,
            include_flash_fee: true,
            include_priority_fee: true,
        };
        
        let validator = ProfitGuardValidator::new(config);
        
        let min_post = validator.min_required_post_balance(
            1_000_000,
            5_000,
            5_000,
        );
        
        // 1_000_000 + 50_000 + 5_000 + 5_000 = 1_060_000
        assert_eq!(min_post, 1_060_000);
    }

    #[test]
    fn test_profit_guard_builder() {
        let builder = ProfitGuardBuilder::new();
        let profit_account = Pubkey::new_unique();
        
        let guard = builder.build(profit_account, 1_000_000);
        
        assert_eq!(guard.profit_account, profit_account);
        assert_eq!(guard.pre_balance_lamports, 1_000_000);
        assert_eq!(guard.min_net_profit_lamports, arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS);
    }
}
