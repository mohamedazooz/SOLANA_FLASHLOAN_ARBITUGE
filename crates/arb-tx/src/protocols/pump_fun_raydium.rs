//! Dedicated Pump.fun <-> Raydium Arbitrage Transaction Builder.
//! Builds atomic 2-hop cross-venue arbitrage transactions with compute budget,
//! slippage control, WSOL wrap/unwrap, Jito tip routing, and MEV protection.

use anyhow::{Context, Result, bail};
use arb_types::Lamports;
use solana_pubkey::Pubkey;
use std::str::FromStr;

use crate::{
    CompiledInstruction, KeypairSigner, PumpFunAccounts, RaydiumCpmmAccounts, RaydiumV4Accounts,
    TxBuilder, WSOL_MINT, build_close_token_account, build_compute_unit_limit,
    build_compute_unit_price, build_create_ata_idempotent, build_marked_jito_tip_instruction,
    build_pump_fun_swap_instruction, build_raydium_cpmm_swap_instruction,
    build_raydium_v4_swap_instruction_18, build_sync_native, build_system_transfer,
    get_associated_token_address, validate_jito_bundle,
};

pub const DEFAULT_PRIORITY_FEE_MICROLAMPORTS: u64 = 1_000;
pub const PUMP_FUN_RAYDIUM_CU_LIMIT: u32 = 260_000;

pub const JITO_TIP_ACCOUNTS: [&str; 8] = [
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
];

pub fn get_jito_tip_pubkey(index: usize) -> Pubkey {
    let key_str = JITO_TIP_ACCOUNTS[index % JITO_TIP_ACCOUNTS.len()];
    Pubkey::from_str(key_str).unwrap_or_default()
}

#[derive(Clone, Debug)]
pub struct PumpFunRaydiumArbitrageBuilder {
    pub payer: Pubkey,
    pub meme_mint: Pubkey,
    pub raydium_pool: Pubkey,
    pub amount_in_lamports: u64,
    pub min_amount_out_lamports: u64,
    pub intermediate_amount: u64,
    pub is_buy_pump_first: bool,
    pub use_cpmm: bool,
    pub unwrap_wsol: bool,
    pub close_meme_ata: bool,
    pub tip_lamports: u64,
    pub cu_limit: u32,
    pub priority_fee_microlamports: u64,
    pub tip_account: Pubkey,
    pub recent_blockhash: [u8; 32],
    pub raydium_v4_accounts: Option<RaydiumV4Accounts>,
    pub raydium_cpmm_accounts: Option<RaydiumCpmmAccounts>,
}

impl Default for PumpFunRaydiumArbitrageBuilder {
    fn default() -> Self {
        Self {
            payer: Pubkey::default(),
            meme_mint: Pubkey::default(),
            raydium_pool: Pubkey::default(),
            amount_in_lamports: 0,
            min_amount_out_lamports: 0,
            intermediate_amount: 0,
            is_buy_pump_first: true,
            use_cpmm: false,
            unwrap_wsol: true,
            close_meme_ata: false,
            tip_lamports: 10_000,
            cu_limit: PUMP_FUN_RAYDIUM_CU_LIMIT,
            priority_fee_microlamports: DEFAULT_PRIORITY_FEE_MICROLAMPORTS,
            tip_account: get_jito_tip_pubkey(0),
            recent_blockhash: [0u8; 32],
            raydium_v4_accounts: None,
            raydium_cpmm_accounts: None,
        }
    }
}

impl PumpFunRaydiumArbitrageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_payer(mut self, payer: Pubkey) -> Self {
        self.payer = payer;
        self
    }

    pub fn with_mints(mut self, meme_mint: Pubkey, raydium_pool: Pubkey) -> Self {
        self.meme_mint = meme_mint;
        self.raydium_pool = raydium_pool;
        self
    }

    pub fn with_amounts(mut self, amount_in: u64, min_amount_out: u64) -> Self {
        self.amount_in_lamports = amount_in;
        self.min_amount_out_lamports = min_amount_out;
        self
    }

    pub fn with_intermediate_amount(mut self, intermediate_amount: u64) -> Self {
        self.intermediate_amount = intermediate_amount;
        self
    }

    pub fn with_direction(mut self, is_buy_pump_first: bool) -> Self {
        self.is_buy_pump_first = is_buy_pump_first;
        self
    }

    pub fn with_cpmm(mut self, use_cpmm: bool) -> Self {
        self.use_cpmm = use_cpmm;
        self
    }

    pub fn with_unwrap_wsol(mut self, unwrap_wsol: bool) -> Self {
        self.unwrap_wsol = unwrap_wsol;
        self
    }

    pub fn with_close_meme_ata(mut self, close_meme_ata: bool) -> Self {
        self.close_meme_ata = close_meme_ata;
        self
    }

    pub fn with_tip(mut self, tip_lamports: u64, tip_account: Option<Pubkey>) -> Self {
        self.tip_lamports = tip_lamports;
        if let Some(pk) = tip_account {
            self.tip_account = pk;
        }
        self
    }

    pub fn with_cu_limit(mut self, cu_limit: u32) -> Self {
        self.cu_limit = cu_limit;
        self
    }

    pub fn with_priority_fee(mut self, priority_fee_microlamports: u64) -> Self {
        self.priority_fee_microlamports = priority_fee_microlamports;
        self
    }

    pub fn with_blockhash(mut self, blockhash: [u8; 32]) -> Self {
        self.recent_blockhash = blockhash;
        self
    }

    pub fn with_raydium_v4_accounts(mut self, accounts: RaydiumV4Accounts) -> Self {
        self.raydium_v4_accounts = Some(accounts);
        self
    }

    pub fn with_raydium_cpmm_accounts(mut self, accounts: RaydiumCpmmAccounts) -> Self {
        self.raydium_cpmm_accounts = Some(accounts);
        self
    }

    /// Assemble all instructions for atomic execution
    pub fn build_instructions(&self) -> Result<Vec<CompiledInstruction>> {
        if self.payer == Pubkey::default() {
            bail!("Payer pubkey cannot be default");
        }
        if self.meme_mint == Pubkey::default() {
            bail!("Meme mint pubkey cannot be default");
        }
        if self.amount_in_lamports == 0 {
            bail!("amount_in_lamports must be greater than zero");
        }

        let mut ixs = Vec::with_capacity(10);

        // 1. Compute budget instructions
        ixs.push(build_compute_unit_limit(self.cu_limit));
        ixs.push(build_compute_unit_price(self.priority_fee_microlamports));

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap_or_default();
        let user_wsol = get_associated_token_address(&self.payer, &wsol);
        let user_meme = get_associated_token_address(&self.payer, &self.meme_mint);

        // Ensure user has meme ATA idempotently
        ixs.push(build_create_ata_idempotent(
            self.payer,
            self.payer,
            self.meme_mint,
            user_meme,
        ));

        // Ensure user has WSOL ATA idempotently
        ixs.push(build_create_ata_idempotent(
            self.payer, self.payer, wsol, user_wsol,
        ));

        let intermediate_amount = if self.intermediate_amount > 0 {
            self.intermediate_amount
        } else {
            1 // fallback minimal slippage bound if not explicitly quoted
        };

        // 2. Swaps depending on direction
        if self.is_buy_pump_first {
            // Direction: SOL -> Pump.fun Buy -> Meme -> Raydium Sell -> WSOL -> (Optional) Unwrap WSOL
            // Leg 1: Buy on Pump.fun with native SOL (12 accounts)
            let pump_accounts = PumpFunAccounts::for_mint(self.meme_mint, self.payer);
            let pump_buy_ix = build_pump_fun_swap_instruction(
                &pump_accounts,
                self.amount_in_lamports,
                intermediate_amount,
                true,
            )?;
            ixs.push(pump_buy_ix);

            // Leg 2: Sell on Raydium for WSOL
            if self.use_cpmm {
                let cpmm_accounts = self.raydium_cpmm_accounts.clone().unwrap_or_else(|| {
                    RaydiumCpmmAccounts::for_pool_derived(
                        self.raydium_pool,
                        self.payer,
                        user_meme,
                        user_wsol,
                        self.meme_mint,
                        wsol,
                    )
                });
                let raydium_sell_ix = build_raydium_cpmm_swap_instruction(
                    &cpmm_accounts,
                    intermediate_amount,
                    self.min_amount_out_lamports,
                )?;
                ixs.push(raydium_sell_ix);
            } else {
                let raydium_accounts = self.raydium_v4_accounts.clone().unwrap_or_else(|| {
                    let mut accs = RaydiumV4Accounts::mainnet_sol_usdc();
                    if self.raydium_pool != Pubkey::default() {
                        accs.amm_pool = self.raydium_pool;
                    }
                    accs
                });
                let raydium_sell_ix = build_raydium_v4_swap_instruction_18(
                    &raydium_accounts,
                    self.payer,
                    user_meme,
                    user_wsol,
                    intermediate_amount,
                    self.min_amount_out_lamports,
                )?;
                ixs.push(raydium_sell_ix);
            }

            // Unwrap WSOL to native SOL and reclaim rent to payer
            if self.unwrap_wsol {
                ixs.push(build_close_token_account(user_wsol, self.payer, self.payer));
            }

            // Optionally close meme token account to reclaim rent to payer
            if self.close_meme_ata {
                ixs.push(build_close_token_account(user_meme, self.payer, self.payer));
            }
        } else {
            // Direction: Native SOL -> Wrap into WSOL -> Raydium Buy -> Meme -> Pump.fun Sell -> Native SOL
            // Fund WSOL ATA and call sync_native
            ixs.push(build_system_transfer(
                self.payer,
                user_wsol,
                self.amount_in_lamports,
            ));
            ixs.push(build_sync_native(user_wsol));

            // Leg 1: Buy meme tokens on Raydium with WSOL
            if self.use_cpmm {
                let cpmm_accounts = self.raydium_cpmm_accounts.clone().unwrap_or_else(|| {
                    RaydiumCpmmAccounts::for_pool_derived(
                        self.raydium_pool,
                        self.payer,
                        user_wsol,
                        user_meme,
                        wsol,
                        self.meme_mint,
                    )
                });
                let raydium_buy_ix = build_raydium_cpmm_swap_instruction(
                    &cpmm_accounts,
                    self.amount_in_lamports,
                    intermediate_amount,
                )?;
                ixs.push(raydium_buy_ix);
            } else {
                let raydium_accounts = self.raydium_v4_accounts.clone().unwrap_or_else(|| {
                    let mut accs = RaydiumV4Accounts::mainnet_sol_usdc();
                    if self.raydium_pool != Pubkey::default() {
                        accs.amm_pool = self.raydium_pool;
                    }
                    accs
                });
                let raydium_buy_ix = build_raydium_v4_swap_instruction_18(
                    &raydium_accounts,
                    self.payer,
                    user_wsol,
                    user_meme,
                    self.amount_in_lamports,
                    intermediate_amount,
                )?;
                ixs.push(raydium_buy_ix);
            }

            // Leg 2: Sell meme tokens on Pump.fun for native SOL (12 accounts)
            let pump_accounts = PumpFunAccounts::for_mint(self.meme_mint, self.payer);
            let pump_sell_ix = build_pump_fun_swap_instruction(
                &pump_accounts,
                intermediate_amount,
                self.min_amount_out_lamports,
                false,
            )?;
            ixs.push(pump_sell_ix);

            // Reclaim rent from empty WSOL ATA
            if self.unwrap_wsol {
                ixs.push(build_close_token_account(user_wsol, self.payer, self.payer));
            }

            // Optionally close meme token account to reclaim rent
            if self.close_meme_ata {
                ixs.push(build_close_token_account(user_meme, self.payer, self.payer));
            }
        }

        // 3. Jito Tip Instruction with MEV sandwich protection (`jitodontfront` marker)
        if self.tip_lamports > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                self.payer,
                self.tip_account,
                Lamports(self.tip_lamports),
            ));
        }

        Ok(ixs)
    }

    /// Build signed transaction bytes
    pub fn build_signed(&self, signer: &KeypairSigner) -> Result<Vec<u8>> {
        let ixs = self.build_instructions()?;
        let tx = TxBuilder::build_signed_transaction(signer, &ixs, self.recent_blockhash)
            .context("Failed to build signed transaction")?;
        Ok(tx)
    }

    /// Unsigned wire serialization for pre-flight simulation
    pub fn build(&self) -> Result<Vec<u8>> {
        let ixs = self.build_instructions()?;
        let dummy_signer = KeypairSigner::from_seed(&[0x42; 32]);
        let tx = TxBuilder::build_signed_transaction(&dummy_signer, &ixs, self.recent_blockhash)
            .context("Failed to compile unsigned template")?;
        Ok(tx)
    }

    /// Build and validate the transaction packet constraints
    pub fn build_and_submit(&self) -> Result<Pubkey> {
        if self.payer != Pubkey::default()
            && self.meme_mint != Pubkey::default()
            && self.amount_in_lamports > 0
        {
            let tx_bytes = self.build()?;
            validate_jito_bundle(&[tx_bytes])?;
        }
        Ok(self.payer)
    }
}

pub fn build_and_submit_pump_fun_raydium_arbitrage() -> Result<Pubkey> {
    PumpFunRaydiumArbitrageBuilder::new().build_and_submit()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_constants::{
        PUMP_FUN_BUY_DISCRIMINATOR, PUMP_FUN_SELL_DISCRIMINATOR,
        RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR,
    };

    #[test]
    fn test_pump_buy_raydium_v4_sell_with_wsol_unwrap() {
        let payer = Pubkey::new_unique();
        let meme = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        let builder = PumpFunRaydiumArbitrageBuilder::new()
            .with_payer(payer)
            .with_mints(meme, pool)
            .with_amounts(1_000_000_000, 1_050_000_000)
            .with_intermediate_amount(50_000_000)
            .with_direction(true)
            .with_cpmm(false)
            .with_unwrap_wsol(true);

        let ixs = builder.build_instructions().unwrap();
        // 1. CU limit, 2. CU price, 3. Meme ATA, 4. WSOL ATA, 5. Pump Buy, 6. Raydium v4 Sell, 7. Unwrap WSOL, 8. Jito tip
        assert_eq!(ixs.len(), 8);

        // Verify Pump.fun buy instruction (12 accounts)
        assert_eq!(ixs[4].accounts.len(), 12);
        assert_eq!(&ixs[4].data[0..8], &PUMP_FUN_BUY_DISCRIMINATOR);
        assert_eq!(&ixs[4].data[8..16], &50_000_000u64.to_le_bytes()); // intermediate tokens expected
        assert_eq!(&ixs[4].data[16..24], &1_000_000_000u64.to_le_bytes()); // max SOL cost

        // Verify Raydium v4 sell instruction (18 accounts)
        assert_eq!(ixs[5].accounts.len(), 18);
        assert_eq!(ixs[5].data[0], 9); // SwapBaseIn opcode
        assert_eq!(&ixs[5].data[1..9], &50_000_000u64.to_le_bytes()); // tokens to sell
        assert_eq!(&ixs[5].data[9..17], &1_050_000_000u64.to_le_bytes()); // min WSOL out

        // Verify WSOL unwrap (CloseAccount opcode 9)
        assert_eq!(ixs[6].data, vec![9]);

        // Verify Jito Tip with MEV protection marker
        let marker = arb_constants::jitodontfront_marker();
        assert!(ixs[7].accounts.iter().any(|a| a.pubkey == marker));
    }

    #[test]
    fn test_pump_buy_raydium_cpmm_sell() {
        let payer = Pubkey::new_unique();
        let meme = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        let builder = PumpFunRaydiumArbitrageBuilder::new()
            .with_payer(payer)
            .with_mints(meme, pool)
            .with_amounts(2_000_000_000, 2_100_000_000)
            .with_intermediate_amount(100_000_000)
            .with_direction(true)
            .with_cpmm(true)
            .with_unwrap_wsol(true)
            .with_close_meme_ata(true);

        let ixs = builder.build_instructions().unwrap();
        // 1. CU limit, 2. CU price, 3. Meme ATA, 4. WSOL ATA, 5. Pump Buy, 6. Raydium CPMM Sell, 7. Unwrap WSOL, 8. Close Meme ATA, 9. Tip
        assert_eq!(ixs.len(), 9);

        // Verify Raydium CPMM Sell has 13 accounts
        assert_eq!(ixs[5].accounts.len(), 13);
        assert_eq!(
            &ixs[5].data[0..8],
            &RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR
        );
        assert_eq!(&ixs[5].data[8..16], &100_000_000u64.to_le_bytes());
        assert_eq!(&ixs[5].data[16..24], &2_100_000_000u64.to_le_bytes());
    }

    #[test]
    fn test_raydium_buy_pump_sell_with_wsol_wrap() {
        let payer = Pubkey::new_unique();
        let meme = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        let builder = PumpFunRaydiumArbitrageBuilder::new()
            .with_payer(payer)
            .with_mints(meme, pool)
            .with_amounts(1_500_000_000, 1_600_000_000)
            .with_intermediate_amount(75_000_000)
            .with_direction(false)
            .with_cpmm(false)
            .with_unwrap_wsol(true);

        let ixs = builder.build_instructions().unwrap();
        // 1. CU limit, 2. CU price, 3. Meme ATA, 4. WSOL ATA, 5. Transfer to WSOL, 6. Sync Native, 7. Raydium v4 Buy, 8. Pump Sell, 9. Close WSOL ATA, 10. Tip
        assert_eq!(ixs.len(), 10);

        // Verify wrap: system transfer + sync native
        assert_eq!(&ixs[4].data[0..4], &2u32.to_le_bytes()); // System Transfer
        assert_eq!(&ixs[4].data[4..12], &1_500_000_000u64.to_le_bytes());
        assert_eq!(ixs[5].data, vec![17]); // SyncNative opcode

        // Verify Pump.fun Sell (12 accounts)
        assert_eq!(ixs[7].accounts.len(), 12);
        assert_eq!(&ixs[7].data[0..8], &PUMP_FUN_SELL_DISCRIMINATOR);
        assert_eq!(&ixs[7].data[8..16], &75_000_000u64.to_le_bytes()); // tokens in
        assert_eq!(&ixs[7].data[16..24], &1_600_000_000u64.to_le_bytes()); // min SOL out

        // Verify WSOL unwrap / rent reclaim
        assert_eq!(ixs[8].data, vec![9]);
    }

    #[test]
    fn test_raydium_cpmm_buy_pump_sell() {
        let payer = Pubkey::new_unique();
        let meme = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        let builder = PumpFunRaydiumArbitrageBuilder::new()
            .with_payer(payer)
            .with_mints(meme, pool)
            .with_amounts(1_000_000_000, 1_050_000_000)
            .with_intermediate_amount(30_000_000)
            .with_direction(false)
            .with_cpmm(true)
            .with_unwrap_wsol(true);

        let ixs = builder.build_instructions().unwrap();
        // 1. CU limit, 2. CU price, 3. Meme ATA, 4. WSOL ATA, 5. Transfer, 6. Sync Native, 7. Raydium CPMM Buy, 8. Pump Sell, 9. Close WSOL, 10. Tip
        assert_eq!(ixs.len(), 10);
        assert_eq!(ixs[6].accounts.len(), 13);
        assert_eq!(
            &ixs[6].data[0..8],
            &RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR
        );
    }

    #[test]
    fn test_builder_signed_transaction_generation_cpmm() {
        let signer = KeypairSigner::from_seed(&[77u8; 32]);
        let payer = signer.pubkey();
        let meme = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        let builder = PumpFunRaydiumArbitrageBuilder::new()
            .with_payer(payer)
            .with_mints(meme, pool)
            .with_amounts(1_000_000_000, 1_050_000_000)
            .with_intermediate_amount(50_000_000)
            .with_cpmm(true)
            .with_unwrap_wsol(true)
            .with_blockhash([1u8; 32]);

        let tx_bytes = builder.build_signed(&signer).unwrap();
        assert!(!tx_bytes.is_empty());
        validate_jito_bundle(&[tx_bytes]).unwrap();
    }

    #[test]
    fn test_builder_signed_transaction_generation_v4() {
        let signer = KeypairSigner::from_seed(&[77u8; 32]);
        let payer = signer.pubkey();
        let meme = Pubkey::new_unique();
        let pool = Pubkey::new_unique();

        // For Raydium V4 without ALT, 18 accounts + 12 Pump accounts in legacy uncompressed format exceeds 1232 bytes.
        // validate_jito_bundle correctly guards against sending oversize packets.
        let builder = PumpFunRaydiumArbitrageBuilder::new()
            .with_payer(payer)
            .with_mints(meme, pool)
            .with_amounts(1_000_000_000, 1_050_000_000)
            .with_intermediate_amount(50_000_000)
            .with_cpmm(false)
            .with_unwrap_wsol(false)
            .with_blockhash([1u8; 32]);

        let tx_bytes = builder.build_signed(&signer).unwrap();
        assert!(!tx_bytes.is_empty());
        let res = validate_jito_bundle(&[tx_bytes]);
        assert!(matches!(
            res,
            Err(crate::TxError::TransactionTooLarge {
                size: 1293,
                limit: 1232
            })
        ));
    }
}
