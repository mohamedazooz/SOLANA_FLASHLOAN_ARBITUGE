//! Transaction V1 Builder for Pump.fun-Raydium Atomic Arbitrage
//!
//! This module builds atomic Transaction V1 transactions for arbitrage between
//! Pump.fun Bonding Curve and Raydium pools using Kamino Flash Loans.
//!
//! # Transaction Structure
//!
//! A typical arbitrage transaction follows this structure:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    VERSIONED TRANSACTION V1                  │
//! ├─────────────────────────────────────────────────────────────┤
//! │ Version: 0x81 (V1)                                           │
//! │ Message:                                                    │
//! │   ┌─────────────────────────────────────────────────────┐   │
//! │   │ Header:                                             │   │
//! │   │   num_required_signatures: 1                       │   │
//! │   │   num_readonly_signed: 0                            │   │
//! │   │   num_readonly_unsigned: N                          │   │
//! │   └─────────────────────────────────────────────────────┘   │
//! │   ┌─────────────────────────────────────────────────────┐   │
//! │   │ Accounts: [payer, flash_loan_account, ...]         │   │
//! │   └─────────────────────────────────────────────────────┘   │
//! │   ┌─────────────────────────────────────────────────────┐   │
//! │   │ Recent Blockhash: ...                               │   │
//! │   └─────────────────────────────────────────────────────┘   │
//! │   ┌─────────────────────────────────────────────────────┐   │
//! │   │ Instructions:                                        │   │
//! │   │   0. Kamino FlashLoanBorrow                         │   │
//! │   │   1. Pump.fun Buy (or Sell)                         │   │
//! │   │   2. Raydium Swap (AMM v4/CPMM/CLMM)                 │   │
//! │   │   3. Kamino FlashLoanRepay                          │   │
//! │   │   4. AssertMinimumProfit (Guard)                   │   │
//! │   └─────────────────────────────────────────────────────┘   │
//! │   ┌─────────────────────────────────────────────────────┐   │
//! │   │ Config:                                              │   │
//! │   │   compute_unit_limit: 200_000                        │   │
//! │   │   loaded_accounts_data_size_limit: 10_000           │   │
//! │   │   heap_size: 32_768                                  │   │
//! │   │   priority_fee_lamports: X                           │   │
//! │   └─────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────┘
//! │ Signatures: [signature]                                      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Features
//!
//! - **Transaction V1**: Uses the new V1 format with 4096-byte capacity
//! - **No ComputeBudget Instructions**: Compute budget is in Message.config
//! - **Inline Accounts**: All accounts are inline (no ALTs)
//! - **Atomic Execution**: All-or-nothing execution

use arb_constants::{
    KAMINO_KLEND_PROGRAM_ID, KAMINO_MAIN_MARKET, KAMINO_USDC_RESERVE, KAMINO_WSOL_RESERVE,
    PUMP_FUN_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID, TOKEN_PROGRAM_ID,
};
use arb_engine::pump_raydium_arb::{ArbitrageDirection, PumpRaydiumOpportunity, RaydiumPool};
use arb_engine::pump_raydium_guard::AssertMinimumProfit;
use arb_types::{DexProtocol, Lamports, MintId, PoolId, Result};
use solana_pubkey::Pubkey;
use solana_transaction::VersionedTransaction;
use std::collections::HashSet;

// Re-export solana types for convenience
pub use solana_message::v1::MessageV1;
pub use solana_signer::Signer;

// ============================================================================
// Transaction V1 Builder Configuration
// ============================================================================

/// Configuration for building Transaction V1 arbitrage transactions.
#[derive(Debug, Clone)]
pub struct V1BuilderConfig {
    /// Compute unit limit for the transaction.
    pub compute_unit_limit: u32,
    /// Loaded accounts data size limit in bytes.
    pub loaded_accounts_data_size_limit: u32,
    /// Heap size in bytes (multiple of 1024).
    pub heap_size: u32,
    /// Priority fee in lamports.
    pub priority_fee_lamports: u64,
    /// Whether to use Kamino flash loans.
    pub use_kamino_flash: bool,
}

impl Default for V1BuilderConfig {
    fn default() -> Self {
        Self {
            compute_unit_limit: 200_000,
            loaded_accounts_data_size_limit: 10_000,
            heap_size: 32_768,
            priority_fee_lamports: 1_000_000, // 0.001 SOL
            use_kamino_flash: true,
        }
    }
}

// ============================================================================
// Account Metadata
// ============================================================================

/// Account metadata for transaction construction.
#[derive(Debug, Clone)]
pub struct AccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

// ============================================================================
// Transaction V1 Builder
// ============================================================================

/// Builder for creating atomic Transaction V1 arbitrage transactions.
pub struct PumpRaydiumV1Builder {
    config: V1BuilderConfig,
    /// Current set of accounts in the transaction.
    accounts: HashSet<Pubkey>,
    /// Account metadata (signer, writable flags).
    account_metas: Vec<AccountMeta>,
    /// Instructions to be added to the transaction.
    instructions: Vec<Vec<u8>>,
    /// The opportunity being executed.
    opportunity: Option<PumpRaydiumOpportunity>,
}

impl PumpRaydiumV1Builder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: V1BuilderConfig::default(),
            accounts: HashSet::new(),
            account_metas: Vec::new(),
            instructions: Vec::new(),
            opportunity: None,
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: V1BuilderConfig) -> Self {
        Self {
            config,
            accounts: HashSet::new(),
            account_metas: Vec::new(),
            instructions: Vec::new(),
            opportunity: None,
        }
    }

    /// Set the arbitrage opportunity to execute.
    pub fn with_opportunity(&mut self, opportunity: PumpRaydiumOpportunity) -> &mut Self {
        self.opportunity = Some(opportunity);
        self
    }

    /// Add an account to the transaction.
    pub fn add_account(&mut self, pubkey: Pubkey, is_signer: bool, is_writable: bool) -> &mut Self {
        if !self.accounts.contains(&pubkey) {
            self.accounts.insert(pubkey);
            self.account_metas.push(AccountMeta {
                pubkey,
                is_signer,
                is_writable,
            });
        }
        self
    }

    /// Add a signer account.
    pub fn add_signer(&mut self, pubkey: Pubkey) -> &mut Self {
        self.add_account(pubkey, true, true)
    }

    /// Add a writable account.
    pub fn add_writable(&mut self, pubkey: Pubkey) -> &mut Self {
        self.add_account(pubkey, false, true)
    }

    /// Add a readonly account.
    pub fn add_readonly(&mut self, pubkey: Pubkey) -> &mut Self {
        self.add_account(pubkey, false, false)
    }

    /// Add an instruction to the transaction.
    pub fn add_instruction(&mut self, program_id: Pubkey, data: Vec<u8>) -> &mut Self {
        self.instructions
            .push(self.build_instruction(program_id, data));
        self
    }

    /// Build a complete instruction with accounts.
    fn build_instruction(&self, program_id: Pubkey, data: Vec<u8>) -> Vec<u8> {
        // In Transaction V1, instructions are serialized as:
        // [program_id_index: u8, account_indices: Vec<u8>, data: Vec<u8>]
        // For now, we'll use a simplified format

        let mut instruction = Vec::new();

        // Find or add program_id to accounts
        let program_index = self
            .account_metas
            .iter()
            .position(|meta| meta.pubkey == program_id)
            .unwrap_or_else(|| {
                // This shouldn't happen - program_id should be added first
                0
            });

        instruction.push(program_index as u8);

        // Add account indices (simplified - in reality this is more complex)
        // For now, we'll just include the data
        instruction.extend(data);

        instruction
    }

    /// Build the complete Transaction V1.
    pub fn build(
        &self,
        payer: &Pubkey,
        recent_blockhash: &[u8; 32],
    ) -> Result<VersionedTransaction> {
        if self.opportunity.is_none() {
            return Err(arb_types::ArbError::InvalidInput(
                "No arbitrage opportunity set".into(),
            ));
        }

        let opportunity = self.opportunity.as_ref().unwrap();

        // Create MessageV1
        let message = self.build_message_v1(payer, recent_blockhash)?;

        Ok(VersionedTransaction::V1(message))
    }

    /// Build MessageV1 for the transaction.
    fn build_message_v1(&self, payer: &Pubkey, recent_blockhash: &[u8; 32]) -> Result<MessageV1> {
        // Ensure payer is in accounts
        if !self.accounts.contains(payer) {
            return Err(arb_types::ArbError::InvalidInput(
                "Payer not in transaction accounts".into(),
            ));
        }

        // Build account metas for MessageV1
        let mut account_metas = Vec::new();

        for meta in &self.account_metas {
            let is_signer = meta.is_signer;
            let is_writable = meta.is_writable;

            // In Solana MessageV1, account metas are represented as:
            // - is_signer: bool
            // - is_writable: bool
            // We'll use a simple struct for now
            account_metas.push(solana_message::AccountMeta::new(
                meta.pubkey,
                is_signer,
                is_writable,
            ));
        }

        // Build the message header
        let header = solana_message::MessageHeader {
            num_required_signatures: self.account_metas.iter().filter(|m| m.is_signer).count()
                as u8,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: self
                .account_metas
                .iter()
                .filter(|m| !m.is_signer && !m.is_writable)
                .count() as u8,
        };

        // Build instructions
        let mut compiled_instructions = Vec::new();

        for instruction in &self.instructions {
            // Parse the instruction
            // This is a placeholder - actual implementation would properly deserialize
            let program_id_index = instruction[0] as usize;
            let data = &instruction[1..];

            let program_id = self.account_metas[program_id_index].pubkey;

            compiled_instructions.push(solana_instruction::Instruction {
                program_id,
                accounts: Vec::new(), // Will be populated properly
                data: data.to_vec(),
            });
        }

        // Build MessageV1 with config
        let message_config = solana_message::v1::MessageConfig {
            compute_unit_limit: self.config.compute_unit_limit,
            loaded_accounts_data_size_limit: self.config.loaded_accounts_data_size_limit,
            heap_size: self.config.heap_size,
            priority_fee_lamports: self.config.priority_fee_lamports,
        };

        // Create the message
        let message = MessageV1::new(
            header,
            account_metas,
            *recent_blockhash,
            compiled_instructions,
            message_config,
        );

        Ok(message)
    }

    /// Build a complete arbitrage transaction from an opportunity.
    pub fn build_arbitrage_transaction(
        &mut self,
        opportunity: &PumpRaydiumOpportunity,
        payer: &Pubkey,
        recent_blockhash: &[u8; 32],
    ) -> Result<VersionedTransaction> {
        self.with_opportunity(opportunity.clone());

        // Add required accounts
        self.add_required_accounts(opportunity)?;

        // Add instructions in order
        self.add_flash_loan_borrow(opportunity)?;
        self.add_pump_swap(opportunity)?;
        self.add_raydium_swap(opportunity)?;
        self.add_flash_loan_repay(opportunity)?;
        self.add_profit_guard(opportunity)?;

        self.build(payer, recent_blockhash)
    }

    /// Add all required accounts for the arbitrage transaction.
    fn add_required_accounts(&mut self, opportunity: &PumpRaydiumOpportunity) -> Result<()> {
        // Add payer (will be added by build())

        // Add Pump.fun accounts
        self.add_writable(opportunity.pump_pool.pool_id.0);
        self.add_writable(opportunity.pump_pool.base_mint);
        self.add_writable(opportunity.pump_pool.quote_mint);

        // Add Raydium accounts
        match &opportunity.raydium_pool {
            RaydiumPool::AMMv4(p) => {
                self.add_writable(p.pool_id.0);
                self.add_writable(p.base_vault);
                self.add_writable(p.quote_vault);
                self.add_readonly(p.market_program);
                self.add_readonly(p.serum_market);
                self.add_readonly(p.open_orders);
            }
            RaydiumPool::CPMM(p) => {
                self.add_writable(p.pool_id.0);
                self.add_writable(p.token_0_vault);
                self.add_writable(p.token_1_vault);
                self.add_readonly(p.amm_config);
            }
            RaydiumPool::CLMM(p) => {
                self.add_writable(p.pool_id.0);
                self.add_writable(p.token_vault_0);
                self.add_writable(p.token_vault_1);
                self.add_readonly(p.amm_config);
                // Add tick arrays
                for ta in &p.tick_array_pdas {
                    self.add_readonly(*ta);
                }
            }
        }

        // Add Kamino flash loan accounts
        if self.config.use_kamino_flash {
            self.add_readonly(Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap());
            self.add_writable(Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap());

            // Add reserve accounts based on token
            let token_mint = opportunity.pump_pool.quote_mint;
            if token_mint == MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap()) {
                self.add_writable(Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap());
            } else if token_mint == MintId(Pubkey::from_str(arb_constants::USDC_MINT).unwrap()) {
                self.add_writable(Pubkey::from_str(KAMINO_USDC_RESERVE).unwrap());
            }
        }

        // Add token program
        self.add_readonly(Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap());

        // Add Pump.fun program
        self.add_readonly(Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap());

        // Add Raydium program
        let raydium_program = match opportunity.raydium_pool.pool_type() {
            arb_engine::pump_raydium_arb::RaydiumPoolType::AMMv4 => {
                Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap()
            }
            arb_engine::pump_raydium_arb::RaydiumPoolType::CPMM => {
                Pubkey::from_str(arb_constants::RAYDIUM_CPMM_PROGRAM_ID).unwrap()
            }
            arb_engine::pump_raydium_arb::RaydiumPoolType::CLMM => {
                Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID).unwrap()
            }
        };
        self.add_readonly(raydium_program);

        Ok(())
    }

    /// Add Kamino flash loan borrow instruction.
    fn add_flash_loan_borrow(&mut self, opportunity: &PumpRaydiumOpportunity) -> Result<()> {
        if !self.config.use_kamino_flash {
            return Ok(());
        }

        let amount = opportunity.optimal_input_lamports;
        let token_mint = opportunity.pump_pool.quote_mint.0;

        // Kamino flash loan borrow instruction
        // This is a placeholder - actual implementation would use Kamino SDK
        let mut data = Vec::new();

        // Instruction discriminator for Kamino flash borrow
        data.push(0x01); // Borrow discriminator

        // Amount to borrow (8 bytes, little-endian)
        data.extend_from_slice(&amount.to_le_bytes());

        // Token mint (32 bytes)
        data.extend_from_slice(token_mint.as_ref());

        self.add_instruction(Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap(), data);

        Ok(())
    }

    /// Add Pump.fun swap instruction.
    fn add_pump_swap(&mut self, opportunity: &PumpRaydiumOpportunity) -> Result<()> {
        let amount = opportunity.optimal_input_lamports;

        let (is_buy, input_mint, output_mint) = match opportunity.direction {
            ArbitrageDirection::PumpBuyRaydiumSell => {
                // Buy on Pump: SOL -> Token
                (
                    true,
                    opportunity.pump_pool.base_mint.0,
                    opportunity.pump_pool.quote_mint.0,
                )
            }
            ArbitrageDirection::PumpSellRaydiumBuy => {
                // Sell on Pump: Token -> SOL
                (
                    false,
                    opportunity.pump_pool.quote_mint.0,
                    opportunity.pump_pool.base_mint.0,
                )
            }
        };

        // Pump.fun swap instruction
        let mut data = Vec::new();

        if is_buy {
            // Buy instruction
            data.extend_from_slice(&arb_constants::PUMP_FUN_BUY_DISCRIMINATOR);
        } else {
            // Sell instruction
            data.extend_from_slice(&arb_constants::PUMP_FUN_SELL_DISCRIMINATOR);
        }

        // Amount (8 bytes, little-endian)
        data.extend_from_slice(&amount.to_le_bytes());

        // Minimum output (8 bytes, little-endian) - use 0 for now
        data.extend_from_slice(&0u64.to_le_bytes());

        self.add_instruction(Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap(), data);

        Ok(())
    }

    /// Add Raydium swap instruction.
    fn add_raydium_swap(&mut self, opportunity: &PumpRaydiumOpportunity) -> Result<()> {
        let amount = match opportunity.direction {
            ArbitrageDirection::PumpBuyRaydiumSell => {
                // We received tokens from Pump, now swap them on Raydium
                // This is the Pump.fun output, which we need to calculate
                // For now, use the optimal input as a placeholder
                opportunity.optimal_input_lamports
            }
            ArbitrageDirection::PumpSellRaydiumBuy => {
                // We received SOL from Pump, now swap it on Raydium
                opportunity.optimal_input_lamports
            }
        };

        let raydium_program = match opportunity.raydium_pool.pool_type() {
            arb_engine::pump_raydium_arb::RaydiumPoolType::AMMv4 => {
                Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap()
            }
            arb_engine::pump_raydium_arb::RaydiumPoolType::CPMM => {
                Pubkey::from_str(arb_constants::RAYDIUM_CPMM_PROGRAM_ID).unwrap()
            }
            arb_engine::pump_raydium_arb::RaydiumPoolType::CLMM => {
                Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID).unwrap()
            }
        };

        // Raydium swap instruction (placeholder)
        let mut data = Vec::new();

        // For now, just add a dummy instruction
        // Actual implementation would use the specific Raydium swap instruction
        data.push(0x02); // Swap discriminator
        data.extend_from_slice(&amount.to_le_bytes());

        self.add_instruction(raydium_program, data);

        Ok(())
    }

    /// Add Kamino flash loan repay instruction.
    fn add_flash_loan_repay(&mut self, opportunity: &PumpRaydiumOpportunity) -> Result<()> {
        if !self.config.use_kamino_flash {
            return Ok(());
        }

        let amount = opportunity.optimal_input_lamports;
        let token_mint = opportunity.pump_pool.quote_mint.0;

        // Kamino flash loan repay instruction
        let mut data = Vec::new();

        // Instruction discriminator for Kamino flash repay
        data.push(0x03); // Repay discriminator

        // Amount to repay (8 bytes, little-endian)
        data.extend_from_slice(&amount.to_le_bytes());

        // Token mint (32 bytes)
        data.extend_from_slice(token_mint.as_ref());

        self.add_instruction(Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap(), data);

        Ok(())
    }

    /// Add profit assertion guard instruction.
    fn add_profit_guard(&mut self, opportunity: &PumpRaydiumOpportunity) -> Result<()> {
        // Use the payer as the profit account for now
        // In reality, this would be a dedicated profit account
        let profit_account = Pubkey::default(); // Placeholder

        let pre_balance = 0; // Will be set at execution time
        let min_profit = opportunity.net_profit_lamports as u64;

        let guard = AssertMinimumProfit::new(profit_account, pre_balance, min_profit);
        let data = guard.serialize();

        // Add the arb-executor program (placeholder)
        // In reality, this would be the actual program ID
        let arb_executor_program = Pubkey::new_unique();

        self.add_instruction(arb_executor_program, data);

        Ok(())
    }
}

// ============================================================================
// Transaction V1 Helper Functions
// ============================================================================

/// Create a MessageV1 config for arbitrage transactions.
pub fn create_arbitrage_message_config(
    compute_unit_limit: u32,
    priority_fee_lamports: u64,
) -> solana_message::v1::MessageConfig {
    solana_message::v1::MessageConfig {
        compute_unit_limit,
        loaded_accounts_data_size_limit: 10_000, // Conservative default
        heap_size: 32_768,
        priority_fee_lamports,
    }
}

/// Estimate compute units for a Pump.fun-Raydium arbitrage transaction.
pub fn estimate_arbitrage_cu(
    raydium_pool_type: arb_engine::pump_raydium_arb::RaydiumPoolType,
) -> u32 {
    match raydium_pool_type {
        arb_engine::pump_raydium_arb::RaydiumPoolType::AMMv4 => 45_000,
        arb_engine::pump_raydium_arb::RaydiumPoolType::CPMM => 40_000,
        arb_engine::pump_raydium_arb::RaydiumPoolType::CLMM => 55_000,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_builder_creation() {
        let builder = PumpRaydiumV1Builder::new();
        assert_eq!(builder.config.compute_unit_limit, 200_000);
        assert!(builder.accounts.is_empty());
        assert!(builder.instructions.is_empty());
    }

    #[test]
    fn test_add_accounts() {
        let mut builder = PumpRaydiumV1Builder::new();
        let pubkey1 = Pubkey::new_unique();
        let pubkey2 = Pubkey::new_unique();

        builder.add_signer(pubkey1);
        builder.add_writable(pubkey2);

        assert_eq!(builder.accounts.len(), 2);
        assert_eq!(builder.account_metas.len(), 2);
    }

    #[test]
    fn test_estimate_cu() {
        use arb_engine::pump_raydium_arb::RaydiumPoolType;

        assert_eq!(estimate_arbitrage_cu(RaydiumPoolType::AMMv4), 45_000);
        assert_eq!(estimate_arbitrage_cu(RaydiumPoolType::CPMM), 40_000);
        assert_eq!(estimate_arbitrage_cu(RaydiumPoolType::CLMM), 55_000);
    }

    #[test]
    fn test_message_config_creation() {
        let config = create_arbitrage_message_config(200_000, 1_000_000);

        assert_eq!(config.compute_unit_limit, 200_000);
        assert_eq!(config.priority_fee_lamports, 1_000_000);
    }
}
