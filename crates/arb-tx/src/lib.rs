//! Transaction Assembly, Instruction Packing, and Atomic Bundle Builder.
//! Supports native DEX arbitrage and Kamino Flash Loans.

use anyhow::{Context, Result, bail};
use arb_types::{DexProtocol, ExecutionOpportunity, Lamports, SwapHop, TokenAmount};
use ed25519_dalek::{Signature, Signer, SigningKey};
use solana_pubkey::Pubkey;
use std::str::FromStr;

// All on-chain program IDs come from `arb-constants` (single source of truth).
// Re-exported so every existing `arb_tx::X` path keeps working unchanged.
pub use arb_constants::{
    ADDRESS_LOOKUP_TABLE_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID, BASE_PRIORITY_FEE_MICRO_LAMPORTS,
    COMPUTE_BUDGET_LIMIT_2HOP, COMPUTE_BUDGET_LIMIT_3HOP, COMPUTE_BUDGET_PROGRAM_ID, CU_BUFFER_PCT,
    DEFAULT_HOP_SLIPPAGE_BPS, DEFAULT_MIN_PROFIT_LAMPORTS, FLASH_BORROW_DISCRIMINATOR,
    FLASH_REPAY_DISCRIMINATOR, KAMINO_KLEND_PROGRAM_ID, KAMINO_MAIN_MARKET,
    KAMINO_MAIN_MARKET_AUTHORITY, KAMINO_USDC_RESERVE, KAMINO_WSOL_FEE_VAULT, KAMINO_WSOL_RESERVE,
    KAMINO_WSOL_SUPPLY_VAULT, MAX_COMPUTE_UNIT_LIMIT, METEORA_BINS_PER_BIN_ARRAY,
    METEORA_DLMM_PROGRAM_ID, METEORA_SOL_RESERVE, METEORA_SOL_USDC_BIN_STEP, METEORA_SOL_USDC_POOL,
    METEORA_SWAP2_DISCRIMINATOR, METEORA_USDC_RESERVE, ORCA_SOL_USDC_POOL, ORCA_SOL_VAULT,
    ORCA_USDC_VAULT, ORCA_WHIRLPOOL_PROGRAM_ID, PUMP_FUN_BONDING_CURVE_SEED,
    PUMP_FUN_BUY_DISCRIMINATOR, PUMP_FUN_EVENT_AUTHORITY_SEED, PUMP_FUN_FEE_RECIPIENT,
    PUMP_FUN_GLOBAL_SEED, PUMP_FUN_PROGRAM_ID, PUMP_FUN_SELL_DISCRIMINATOR,
    RAYDIUM_CLMM_OBSERVATION_SEED, RAYDIUM_CLMM_PROGRAM_ID, RAYDIUM_CLMM_SWAP_DISCRIMINATOR,
    RAYDIUM_CLMM_TICK_ARRAY_SEED, RAYDIUM_CPMM_AUTH_SEED, RAYDIUM_CPMM_OBSERVATION_SEED,
    RAYDIUM_CPMM_POOL_VAULT_SEED, RAYDIUM_CPMM_PROGRAM_ID, RAYDIUM_CPMM_SOL_USDC_POOL,
    RAYDIUM_CPMM_SOL_VAULT, RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR, RAYDIUM_CPMM_USDC_VAULT,
    RAYDIUM_SOL_USDC_POOL, RAYDIUM_SOL_VAULT, RAYDIUM_USDC_VAULT, RAYDIUM_V4_PROGRAM_ID,
    SOLANA_PACKET_MTU_LIMIT, SYSTEM_PROGRAM_ID, SYSVAR_INSTRUCTIONS_ID, SYSVAR_RENT_ID,
    TOKEN_PROGRAM_ID, USDC_MINT, WSOL_MINT, get_cu_limit_for_hops,
};

pub mod alt;
pub use alt::{
    AltManager, MAX_ALT_EXTEND_ADDRESSES, address_lookup_table_program_id, create_lookup_table,
    derive_lookup_table_address, extend_lookup_table,
};
pub mod error;
pub mod jupiter;
pub mod kamino;
pub mod template;

pub use error::TxError;
pub use jupiter::JupiterClient;
pub use kamino::{
    KaminoFlashAccounts, calculate_kamino_flash_fee, derive_lending_market_authority,
};
pub use template::PreWarmedTxTemplate;

/// Estimate the minimal compute units strictly required by the transaction
/// before safety buffer is applied, keeping instruction execution minimal and fast.
pub fn estimate_minimal_cu(has_flash_loan: bool, hops_count: usize) -> u32 {
    let base_overhead: u32 = 450; // Compute Budget + Tip
    let flash_overhead: u32 = if has_flash_loan { 28_000 } else { 0 }; // Kamino Borrow + Repay
    let swap_cu = (hops_count as u32).saturating_mul(35_000); // Minimal needed per DEX swap hop
    base_overhead
        .saturating_add(flash_overhead)
        .saturating_add(swap_cu)
}

/// Calculate dynamic compute unit limit with measured buffer (simulated via reserve depth proxy — upgrade path: real simulateTransaction).
pub fn calculate_dynamic_cu_limit(base_required_units: u32) -> u32 {
    // 15% safety buffer over measured base requirement.
    let buffer = base_required_units.saturating_mul(15) / 100;
    let total = base_required_units.saturating_add(buffer);
    total.min(MAX_COMPUTE_UNIT_LIMIT)
}

/// Calculate dynamic compute unit price in micro-lamports, dynamically elevated based on profit.
pub fn calculate_dynamic_cu_price(profit_lamports: u64, base_priority_fee: u64) -> u64 {
    let dynamic_elevation = (profit_lamports / 25_000).min(500_000);
    base_priority_fee.saturating_add(dynamic_elevation)
}

/// Calculate fee-deduplicated compute unit price.
/// In bundle-only mode (`bundle_only = true`), returns 0 to eliminate priority fee duplication
/// and concentrate 100% of priority budget into dynamic JitoTip.
pub fn calculate_fee_deduplicated_cu_price(
    profit_lamports: u64,
    base_priority_fee: u64,
    bundle_only: bool,
) -> u64 {
    if bundle_only {
        0
    } else {
        calculate_dynamic_cu_price(profit_lamports, base_priority_fee)
    }
}

/// Per-hop compute units by protocol. A CLMM crosses ticks and a DLMM walks
/// bins, so they cost materially more than a constant-product swap. Used to size
/// the CU limit from the REAL route instead of a flat per-hop guess.
pub fn per_hop_cu(protocol: arb_types::DexProtocol) -> u32 {
    use arb_types::DexProtocol::*;
    match protocol {
        RaydiumV4 | RaydiumCpmm => 35_000,
        PumpFun => 42_000,
        PumpSwap => 48_000,
        OrcaWhirlpool | RaydiumClmm => 85_000,
        MeteoraDlmm => 120_000,
    }
}

/// Estimate the CU a route actually needs: fixed overhead, flash-loan overhead,
/// per-hop cost (protocol-specific), and ephemeral ATA create/close. The caller
/// adds the safety buffer via `calculate_dynamic_cu_limit`.
pub fn estimate_route_cu(
    has_flash_loan: bool,
    hops: &[arb_types::DexProtocol],
    ephemeral_atas: usize,
) -> u32 {
    let base_overhead: u32 = 450; // ComputeBudget + tip
    let flash_overhead: u32 = if has_flash_loan { 28_000 } else { 0 }; // borrow + repay
    let swaps: u32 = hops.iter().map(|p| per_hop_cu(*p)).sum();
    let atas = (ephemeral_atas as u32).saturating_mul(20_000); // create + close each
    base_overhead
        .saturating_add(flash_overhead)
        .saturating_add(swaps)
        .saturating_add(atas)
}

/// Cap a compute-unit price so the total priority fee stays within
/// `max_share_bps` of `profit_lamports`.
///
/// The priority fee in lamports is `cu_price (µlamports/CU) * cu_limit / 1e6`.
/// Bounding it by a share of the edge stops a runaway priority bid from eating
/// the profit it was meant to capture.
pub fn cap_cu_price_by_profit(
    profit_lamports: u64,
    cu_limit: u32,
    desired_cu_price: u64,
    max_share_bps: u64,
) -> u64 {
    if cu_limit == 0 || max_share_bps == 0 {
        return 0;
    }
    let budget = profit_lamports.saturating_mul(max_share_bps) / 10_000;
    let max_price = budget.saturating_mul(1_000_000) / u64::from(cu_limit);
    desired_cu_price.min(max_price)
}

/// Metadata for accounts passed in Solana instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub fn new(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: true,
        }
    }

    pub fn new_readonly(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: false,
        }
    }
}

/// Generic compiled instruction ready for serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

/// Build Compute Budget Unit Limit instruction.
pub fn build_compute_unit_limit(units: u32) -> CompiledInstruction {
    let mut data = Vec::with_capacity(5);
    data.push(2); // SetComputeUnitLimit instruction tag
    data.extend_from_slice(&units.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
        accounts: Vec::new(),
        data,
    }
}

/// Build Compute Budget Unit Price (Priority Fee in micro-lamports) instruction.
pub fn build_compute_unit_price(micro_lamports: u64) -> CompiledInstruction {
    let mut data = Vec::with_capacity(9);
    data.push(3); // SetComputeUnitPrice instruction tag
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
        accounts: Vec::new(),
        data,
    }
}

/// Deprecated — use `arb_constants::JITO_TIP_ACCOUNTS` directly.
#[deprecated(since = "0.2.0", note = "use arb_constants::JITO_TIP_ACCOUNTS")]
pub use arb_constants::JITO_TIP_ACCOUNTS;

/// Get next Jito tip account using round-robin distribution (delegates to arb-constants).
pub fn get_next_jito_tip_account() -> Pubkey {
    arb_constants::jito_tip_account_round_robin()
}

/// Build native SOL transfer instruction for Jito Tip account.
pub fn build_jito_tip_instruction(
    payer: Pubkey,
    tip_account: Pubkey,
    lamports: Lamports,
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // SystemProgram::Transfer instruction index = 2
    data.extend_from_slice(&lamports.as_u64().to_le_bytes());

    CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(tip_account, false),
        ],
        data,
    }
}

/// Attach the Jito `jitodontfront` marker to an existing instruction.
///
/// A bundle containing a transaction that references the marker account is
/// rejected by the block engine unless that transaction sits at index 0, which
/// is what stops a searcher's own bundle from being sandwiched.
///
/// The marker is appended as a **read-only account of a real instruction**, not
/// as a standalone instruction: the marker account does not exist on chain, so
/// a transaction that named it as a program would fail to execute and take the
/// whole bundle down with it. Appending it to, say, the tip transfer costs one
/// extra account and no extra instruction.
pub fn attach_jitodontfront(instruction: &mut CompiledInstruction) {
    let marker = arb_constants::jitodontfront_marker();
    if instruction
        .accounts
        .iter()
        .any(|meta| meta.pubkey == marker)
    {
        return;
    }
    instruction
        .accounts
        .push(AccountMeta::new_readonly(marker, false));
}

/// Build a tip transfer that also carries the `jitodontfront` marker.
///
/// Convenience wrapper for the common case: the tip instruction is the last one
/// in the transaction and the only place the marker is needed.
pub fn build_marked_jito_tip_instruction(
    payer: Pubkey,
    tip_account: Pubkey,
    lamports: Lamports,
) -> CompiledInstruction {
    let mut instruction = build_jito_tip_instruction(payer, tip_account, lamports);
    attach_jitodontfront(&mut instruction);
    instruction
}

/// Kamino Flash Loan configuration parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaminoFlashParams {
    pub lending_market: Pubkey,
    pub reserve: Pubkey,
    pub reserve_source_liquidity: Pubkey,
    pub user_destination_liquidity: Pubkey,
    pub reserve_liquidity_fee_receiver: Pubkey,
}

impl Default for KaminoFlashParams {
    fn default() -> Self {
        Self {
            lending_market: Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            reserve: Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            reserve_source_liquidity: Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            user_destination_liquidity: Pubkey::default(),
            reserve_liquidity_fee_receiver: Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
        }
    }
}

impl KaminoFlashParams {
    pub fn for_user(user: Pubkey) -> Self {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let user_wsol_ata = get_associated_token_address(&user, &wsol);
        Self {
            lending_market: Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            reserve: Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            reserve_source_liquidity: Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            user_destination_liquidity: user_wsol_ata,
            reserve_liquidity_fee_receiver: Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
        }
    }

    /// Resolve full 12-account matrix for Kamino flash loan operations.
    pub fn to_flash_accounts(&self, user_authority: Pubkey) -> KaminoFlashAccounts {
        let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
        let (lma, _) = derive_lending_market_authority(&self.lending_market, &klend);
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let sysvar = Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let user_ata = if self.user_destination_liquidity == Pubkey::default()
            || self.user_destination_liquidity == user_authority
        {
            get_associated_token_address(&user_authority, &wsol)
        } else {
            self.user_destination_liquidity
        };

        KaminoFlashAccounts {
            user_authority,
            lending_market: self.lending_market,
            lending_market_authority: lma,
            reserve: self.reserve,
            reserve_liquidity_mint: wsol,
            reserve_supply_vault: self.reserve_source_liquidity,
            user_liquidity_ata: user_ata,
            reserve_fee_vault: self.reserve_liquidity_fee_receiver,
            referrer_token_state: None,
            referrer_account: None,
            sysvar_instructions: sysvar,
            token_program,
            klend_program_id: klend,
        }
    }
}

/// Build Kamino Flash Borrow instruction (12 accounts, 16-byte data payload).
pub fn build_kamino_flash_borrow(
    params: &KaminoFlashParams,
    user_authority: Pubkey,
    amount: u64,
) -> CompiledInstruction {
    let accounts = params.to_flash_accounts(user_authority);
    kamino::build_kamino_flash_borrow(&accounts, amount)
}

/// Build Kamino Flash Repay instruction with default borrow_instruction_index = 2 (12 accounts, 17-byte data payload).
pub fn build_kamino_flash_repay(
    params: &KaminoFlashParams,
    user_authority: Pubkey,
    amount: u64,
) -> CompiledInstruction {
    build_kamino_flash_repay_with_index(params, user_authority, amount, 2)
}

/// Build Kamino Flash Repay instruction with explicit borrow_instruction_index (12 accounts, 17-byte data payload).
pub fn build_kamino_flash_repay_with_index(
    params: &KaminoFlashParams,
    user_authority: Pubkey,
    amount: u64,
    borrow_instruction_index: u8,
) -> CompiledInstruction {
    let accounts = params.to_flash_accounts(user_authority);
    kamino::build_kamino_flash_repay(&accounts, amount, borrow_instruction_index)
}

/// Derive Associated Token Address (ATA) deterministically for a wallet and mint.
pub fn get_associated_token_address(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let seeds = &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()];
    let (ata, _) = Pubkey::find_program_address(seeds, &ata_program);
    ata
}

/// Build idempotent ATA creation instruction (CreateIdempotent opcode 1).
/// Safely initializes intermediate token account if missing, does not fail if it already exists.
pub fn build_create_ata_idempotent(
    payer: Pubkey,
    wallet_owner: Pubkey,
    token_mint: Pubkey,
    associated_token_account: Pubkey,
) -> CompiledInstruction {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

    CompiledInstruction {
        program_id: ata_program,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(associated_token_account, false),
            AccountMeta::new_readonly(wallet_owner, false),
            AccountMeta::new_readonly(token_mint, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1], // CreateIdempotent opcode
    }
}

/// Build SPL Token close_account instruction (opcode 9) to reclaim 100% of the rent deposit back to payer.
pub fn build_close_token_account(
    account_to_close: Pubkey,
    destination_rent_receiver: Pubkey,
    owner_authority: Pubkey,
) -> CompiledInstruction {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

    CompiledInstruction {
        program_id: token_program,
        accounts: vec![
            AccountMeta::new(account_to_close, false),
            AccountMeta::new(destination_rent_receiver, false),
            AccountMeta::new_readonly(owner_authority, true),
        ],
        data: vec![9], // CloseAccount opcode
    }
}

/// Build SPL Token sync_native instruction (opcode 17).
pub fn build_sync_native(account: Pubkey) -> CompiledInstruction {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    CompiledInstruction {
        program_id: token_program,
        accounts: vec![AccountMeta::new(account, false)],
        data: vec![17], // SyncNative opcode
    }
}

/// Build standard System Program transfer instruction.
pub fn build_system_transfer(from: Pubkey, to: Pubkey, lamports: u64) -> CompiledInstruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // SystemProgram::Transfer instruction index = 2
    data.extend_from_slice(&lamports.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
        data,
    }
}

/// Complete 18-Account Matrix for Raydium v4 SwapBaseIn instruction
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumV4Accounts {
    pub token_program: Pubkey,
    pub amm_pool: Pubkey,
    pub amm_authority: Pubkey,
    pub amm_open_orders: Pubkey,
    pub amm_target_orders: Pubkey,
    pub pool_coin_vault: Pubkey,
    pub pool_pc_vault: Pubkey,
    pub serum_program: Pubkey,
    pub serum_market: Pubkey,
    pub serum_bids: Pubkey,
    pub serum_asks: Pubkey,
    pub serum_event_queue: Pubkey,
    pub serum_coin_vault: Pubkey,
    pub serum_pc_vault: Pubkey,
    pub serum_vault_signer: Pubkey,
}

impl RaydiumV4Accounts {
    pub fn mainnet_sol_usdc() -> Self {
        Self {
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            amm_pool: Pubkey::from_str(RAYDIUM_SOL_USDC_POOL).unwrap(),
            amm_authority: Pubkey::from_str("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1")
                .unwrap(),
            amm_open_orders: Pubkey::from_str("HmiHHzq4Fym9e1D4qzLS6LDDM3tNsCTBPDWHTLZ763jY")
                .unwrap(),
            amm_target_orders: Pubkey::from_str("CZza3Ej4Mc58MnxWA385itCC9jCo3L1D7zc3LKy1bZMR")
                .unwrap(),
            pool_coin_vault: Pubkey::from_str(RAYDIUM_SOL_VAULT).unwrap(),
            pool_pc_vault: Pubkey::from_str(RAYDIUM_USDC_VAULT).unwrap(),
            serum_program: Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX").unwrap(),
            serum_market: Pubkey::from_str("8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6").unwrap(),
            serum_bids: Pubkey::from_str("5jWUncPNBMZJ3sTHKmMLszypVkoRK6bfEQMQUHweeQnh").unwrap(),
            serum_asks: Pubkey::from_str("EaXdHx7x3mdGA38j5RSmKYSXMzAFzzUXCLNBEDXDn1d5").unwrap(),
            serum_event_queue: Pubkey::from_str("8CvwxZ9Db6XbLD46NZwwmVDZZRDy7eydFcAGkXKh9axa")
                .unwrap(),
            serum_coin_vault: Pubkey::from_str("CKxTHwM9fPMRRvZmFnFoqKNd9pQR21c5Aq9bh5h9oghX")
                .unwrap(),
            serum_pc_vault: Pubkey::from_str("6A5NHCj1yF6urc9wZNe6Bcjj4LVszQNj5DwAWG97yzMu")
                .unwrap(),
            serum_vault_signer: Pubkey::from_str("CTz5UMLQm2SRWHzQnU62Pi4yJqbNGjgRBHqqp6oDHfF7")
                .unwrap(),
        }
    }
}

/// Build genuine Raydium v4 Swap Instruction with the complete 18-account matrix.
pub fn build_raydium_v4_swap_instruction_18(
    accounts: &RaydiumV4Accounts,
    user_authority: Pubkey,
    user_source_token: Pubkey,
    user_destination_token: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;
    let mut data = Vec::with_capacity(17);
    data.push(9); // SwapBaseIn opcode
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new(accounts.amm_pool, false),
            AccountMeta::new_readonly(accounts.amm_authority, false),
            AccountMeta::new(accounts.amm_open_orders, false),
            AccountMeta::new(accounts.amm_target_orders, false),
            AccountMeta::new(accounts.pool_coin_vault, false),
            AccountMeta::new(accounts.pool_pc_vault, false),
            AccountMeta::new_readonly(accounts.serum_program, false),
            AccountMeta::new(accounts.serum_market, false),
            AccountMeta::new(accounts.serum_bids, false),
            AccountMeta::new(accounts.serum_asks, false),
            AccountMeta::new(accounts.serum_event_queue, false),
            AccountMeta::new(accounts.serum_coin_vault, false),
            AccountMeta::new(accounts.serum_pc_vault, false),
            AccountMeta::new_readonly(accounts.serum_vault_signer, false),
            AccountMeta::new(user_source_token, false),
            AccountMeta::new(user_destination_token, false),
            AccountMeta::new_readonly(user_authority, true),
        ],
        data,
    })
}

/// Complete 11-Account Matrix for Orca Whirlpool Swap instruction
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaWhirlpoolAccounts {
    pub token_program: Pubkey,
    pub whirlpool: Pubkey,
    pub token_vault_a: Pubkey,
    pub token_vault_b: Pubkey,
    pub tick_array_0: Pubkey,
    pub tick_array_1: Pubkey,
    pub tick_array_2: Pubkey,
    pub oracle: Pubkey,
}

impl OrcaWhirlpoolAccounts {
    pub fn for_whirlpool(
        whirlpool: Pubkey,
        token_vault_a: Pubkey,
        token_vault_b: Pubkey,
        tick_spacing: u16,
        tick_current_index: i32,
        a_to_b: bool,
    ) -> Self {
        let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
        let spacing = (tick_spacing as i32).max(1);
        let ticks_in_array = 88 * spacing;
        let start_tick = tick_current_index.div_euclid(ticks_in_array) * ticks_in_array;

        let derive_ta = |start: i32| {
            let start_str = start.to_string();
            let seeds: &[&[u8]] = &[b"tick_array", whirlpool.as_ref(), start_str.as_bytes()];
            Pubkey::find_program_address(seeds, &program_id).0
        };

        let ta0 = derive_ta(start_tick);
        let (ta1, ta2) = if a_to_b {
            (
                derive_ta(start_tick - ticks_in_array),
                derive_ta(start_tick - 2 * ticks_in_array),
            )
        } else {
            (
                derive_ta(start_tick + ticks_in_array),
                derive_ta(start_tick + 2 * ticks_in_array),
            )
        };

        let (oracle, _) =
            Pubkey::find_program_address(&[b"oracle", whirlpool.as_ref()], &program_id);

        Self {
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            whirlpool,
            token_vault_a,
            token_vault_b,
            tick_array_0: ta0,
            tick_array_1: ta1,
            tick_array_2: ta2,
            oracle,
        }
    }

    pub fn mainnet_sol_usdc() -> Self {
        Self::for_whirlpool(
            Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap(),
            Pubkey::from_str(ORCA_SOL_VAULT).unwrap(),
            Pubkey::from_str(ORCA_USDC_VAULT).unwrap(),
            4,
            -23621,
            true,
        )
    }
}

/// Build genuine Orca Whirlpool Swap Instruction with the complete 11-account matrix.
pub fn build_orca_whirlpool_swap_instruction_11(
    accounts: &OrcaWhirlpoolAccounts,
    user_authority: Pubkey,
    token_owner_account_a: Pubkey,
    token_owner_account_b: Pubkey,
    amount_in: u64,
    other_amount_threshold: u64,
    a_to_b: bool,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;
    let mut data = Vec::with_capacity(34);
    // Anchor discriminator for `swap`
    data.extend_from_slice(&[248, 198, 158, 145, 225, 117, 135, 200]);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    // Slippage-derived sqrt_price_limit: no hardcoded full-range values.
    // When a_to_b (price drops), the limit is the price at which we receive exactly
    // other_amount_threshold of token B for amount_in of token A.
    // When b_to_a (price rises), the limit is the price at which we pay exactly
    // amount_in of token B for other_amount_threshold of token A.
    const Q64: u128 = 1u128 << 64;
    let sqrt_price_limit = if amount_in > 0 && other_amount_threshold > 0 {
        if a_to_b {
            // price = other_amount_threshold / amount_in (B per A)
            (u128::from(other_amount_threshold) * Q64) / u128::from(amount_in)
        } else {
            // price = amount_in / other_amount_threshold (A per B) -> invert for B per A
            (u128::from(amount_in) * Q64) / u128::from(other_amount_threshold)
        }
    } else if a_to_b {
        // No slippage protection possible; use the absolute minimum price.
        4_295_048_016u128
    } else {
        79_226_673_515_401_499_993_453_864_427u128
    };
    data.extend_from_slice(&sqrt_price_limit.to_le_bytes());
    data.push(1); // amount_specified_is_input = true
    data.push(a_to_b as u8);

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new_readonly(user_authority, true),
            AccountMeta::new(accounts.whirlpool, false),
            AccountMeta::new(token_owner_account_a, false),
            AccountMeta::new(accounts.token_vault_a, false),
            AccountMeta::new(token_owner_account_b, false),
            AccountMeta::new(accounts.token_vault_b, false),
            AccountMeta::new(accounts.tick_array_0, false),
            AccountMeta::new(accounts.tick_array_1, false),
            AccountMeta::new(accounts.tick_array_2, false),
            AccountMeta::new_readonly(accounts.oracle, false),
        ],
        data,
    })
}

/// Derive the Meteora DLMM reserve (token vault) PDA for a given pool and token mint.
/// Official seeds from MeteoraAg/dlmm-sdk commons/src/pda.rs: `[lb_pair, token_mint]`
/// (no "reserve" prefix — verified against the canonical Rust SDK source).
pub fn derive_meteora_reserve_pda(pool: &Pubkey, mint: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[pool.as_ref(), mint.as_ref()];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Derive a Meteora DLMM BinArray PDA for a given pool and bin-array index.
/// Official seeds from MeteoraAg/dlmm-sdk commons/src/pda.rs:
/// `[b"bin_array", lb_pair, bin_array_index_le_i64]` — index is i64, not i32.
pub fn derive_meteora_bin_array_pda(pool: &Pubkey, bin_array_index: i64) -> Pubkey {
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
    let idx_bytes = bin_array_index.to_le_bytes();
    let seeds: &[&[u8]] = &[b"bin_array", pool.as_ref(), &idx_bytes];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Derive the standard Anchor `__event_authority` PDA for the DLMM program.
/// Official seeds from MeteoraAg/dlmm-sdk commons/src/pda.rs: `[b"__event_authority"]`.
pub fn derive_meteora_event_authority() -> Pubkey {
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[b"__event_authority"];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Derive the Meteora DLMM oracle PDA for a given pool.
/// Official seeds from MeteoraAg/dlmm-sdk commons/src/pda.rs: `[b"oracle", lb_pair]`.
pub fn derive_meteora_oracle_pda(pool: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[b"oracle", pool.as_ref()];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Compute the bin-array index (i64) that contains a given DLMM bin_id.
/// `METEORA_BINS_PER_BIN_ARRAY = 70` bins per array; uses floor division
/// toward negative infinity so negative bin IDs map correctly.
pub fn bin_array_index_for_bin(bin_id: i32) -> i64 {
    (bin_id as i64).div_euclid(METEORA_BINS_PER_BIN_ARRAY)
}

/// Complete account matrix for a Meteora DLMM `swap2` instruction.
///
/// Account order mirrors `dlmm::client::accounts::Swap2` from the official
/// MeteoraAg/dlmm-sdk (cli/src/instructions/swap_exact_in.rs), verified
/// against on-chain live transactions.
/// reserve_x/reserve_y are stored fields on the LbPair account —
/// for the canonical SOL-USDC pool they are the verified live addresses
/// fetched from the Meteora REST API. For arbitrary pools they are decoded
/// from the LbPair account data by the ingestion layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraDlmmAccounts {
    pub lb_pair: Pubkey,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub oracle: Pubkey,
    pub event_authority: Pubkey,
    pub program_id: Pubkey,
}

impl MeteoraDlmmAccounts {
    /// Canonical mainnet SOL-USDC DLMM pool.
    /// reserve_x / reserve_y are the verified live on-chain token vault
    /// addresses fetched directly from the Meteora REST API — not derived PDAs.
    pub fn mainnet_sol_usdc() -> Self {
        let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
        let pool = Pubkey::from_str(METEORA_SOL_USDC_POOL).unwrap();
        Self {
            lb_pair: pool,
            reserve_x: Pubkey::from_str(METEORA_SOL_RESERVE).unwrap(),
            reserve_y: Pubkey::from_str(METEORA_USDC_RESERVE).unwrap(),
            token_x_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
            token_y_mint: Pubkey::from_str(USDC_MINT).unwrap(),
            oracle: derive_meteora_oracle_pda(&pool),
            event_authority: derive_meteora_event_authority(),
            program_id,
        }
    }

    /// Build accounts for an arbitrary live pool where reserve_x/reserve_y
    /// have been decoded live from the LbPair account by the ingestion layer.
    pub fn for_pool(
        pool: Pubkey,
        reserve_x: Pubkey,
        reserve_y: Pubkey,
        token_x_mint: Pubkey,
        token_y_mint: Pubkey,
    ) -> Self {
        let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
        Self {
            lb_pair: pool,
            reserve_x,
            reserve_y,
            token_x_mint,
            token_y_mint,
            oracle: derive_meteora_oracle_pda(&pool),
            event_authority: derive_meteora_event_authority(),
            program_id,
        }
    }
}

/// Build a genuine Meteora DLMM `swap2` instruction with the complete real account
/// matrix (lb_pair, reserve vaults, user token accounts, mints, oracle, signer,
/// token program, Anchor event-authority + program) and the swap-route bin arrays.
///
/// `active_bin_id` and `bin_step` are LIVE values decoded from the on-chain LbPair
/// account by the ingestion layer — they drive the correct bin-array selection so
/// the instruction targets the active liquidity region. No hardcoded reserve
/// values are used: the on-chain program reads live vault balances at execution.
///
/// NOTE: for byte-exact, fully-account-resolved execution the supervisor routes
/// Meteora hops through the Jupiter v6 `swap-instructions` API (live). This native
/// builder produces a structurally correct DLMM instruction against the REAL
/// Meteora program (never the Raydium V4 dummy) and is used by the synchronous
/// assemble paths.
pub fn build_meteora_dlmm_swap_instruction(
    accounts: &MeteoraDlmmAccounts,
    user_authority: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    active_bin_id: i32,
    bin_step: u16,
    swap_for_y: bool,
) -> CompiledInstruction {
    // -------------------------------------------------------------------------
    // Data layout for `swap2` (from dlmm::client::args::Swap2::data()):
    //   [0..8]   discriminator = sha256("global:swap2")[..8]
    //            Verified: [65, 75, 63, 76, 235, 91, 91, 136]
    //   [8..16]  amount_in: u64 (le)
    //   [16..24] min_amount_out: u64 (le)
    //   [24..28] remaining_accounts_info.slices.len(): u32 = 0 (le)
    //            Empty slices = no Token-2022 transfer-hook accounts;
    //            bin arrays are passed as remaining accounts directly.
    // -------------------------------------------------------------------------
    let mut data = Vec::with_capacity(28);
    data.extend_from_slice(&METEORA_SWAP2_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // RemainingAccountsInfo { slices: vec![] }

    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let memo_program = Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr").unwrap();

    // Bin array PDAs — i64 index, per official SDK pda.rs.
    // Dynamically calculate the number of contiguous BinArray PDAs to append (up to 4).
    // Large price movements or volatile tokens can span across multiple bin arrays (70 bins each).
    let active_idx = bin_array_index_for_bin(active_bin_id);
    let bin_array_count: usize = {
        let step = u64::from(bin_step.max(1));
        let est_bins = if amount_in > 0 && min_amount_out > 0 {
            let price_diff_bps = if swap_for_y {
                // If selling X for Y, price impact drops output relative to input
                amount_in
                    .saturating_sub(min_amount_out)
                    .saturating_mul(10_000)
                    / amount_in
            } else {
                min_amount_out
                    .saturating_sub(amount_in)
                    .saturating_mul(10_000)
                    / min_amount_out
            };
            price_diff_bps / step
        } else {
            0
        };

        if est_bins >= 140 {
            4
        } else if est_bins >= 70 || bin_step <= 5 {
            3
        } else {
            2
        }
    };

    let mut bin_array_metas = Vec::with_capacity(bin_array_count);
    for i in 0..bin_array_count {
        let offset = i as i64;
        let idx = if swap_for_y {
            active_idx - offset
        } else {
            active_idx + offset
        };
        let pda = derive_meteora_bin_array_pda(&accounts.lb_pair, idx);
        bin_array_metas.push(AccountMeta::new(pda, false));
    }

    // -------------------------------------------------------------------------
    // Account order — verified from dlmm::client::accounts::Swap2 and
    // cli/src/instructions/swap_exact_in.rs in MeteoraAg/dlmm-sdk:
    //  0  lb_pair                          (writable)
    //  1  bin_array_bitmap_extension       (optional; use program_id as placeholder)
    //  2  reserve_x                        (writable)
    //  3  reserve_y                        (writable)
    //  4  token_x_mint                     (readonly)
    //  5  token_y_mint                     (readonly)
    //  6  token_x_program (SPL Token)      (readonly)
    //  7  token_y_program (SPL Token)      (readonly)
    //  8  user                             (readonly signer)
    //  9  user_token_in                    (writable)
    //  10 user_token_out                   (writable)
    //  11 oracle                           (readonly)
    //  12 host_fee_in                      (optional; use program_id as placeholder)
    //  13 event_authority                  (readonly)
    //  14 program (self-reference)         (readonly)
    //  15 memo_program                     (readonly)
    // Remaining:
    //  16..16+N bin_arrays                 (writable, dynamically 2..=4 contiguous arrays)
    // -------------------------------------------------------------------------
    let mut account_list = vec![
        AccountMeta::new(accounts.lb_pair, false),
        AccountMeta::new_readonly(accounts.program_id, false), // bin_array_bitmap_extension placeholder
        AccountMeta::new(accounts.reserve_x, false),
        AccountMeta::new(accounts.reserve_y, false),
        AccountMeta::new_readonly(accounts.token_x_mint, false),
        AccountMeta::new_readonly(accounts.token_y_mint, false),
        AccountMeta::new_readonly(token_program, false), // token_x_program
        AccountMeta::new_readonly(token_program, false), // token_y_program
        AccountMeta::new_readonly(user_authority, true), // user (signer)
        AccountMeta::new(user_token_in, false),
        AccountMeta::new(user_token_out, false),
        AccountMeta::new_readonly(accounts.oracle, false),
        AccountMeta::new_readonly(accounts.program_id, false), // host_fee_in placeholder
        AccountMeta::new_readonly(accounts.event_authority, false),
        AccountMeta::new_readonly(accounts.program_id, false), // program self-reference
        AccountMeta::new_readonly(memo_program, false),
    ];
    account_list.extend(bin_array_metas);

    CompiledInstruction {
        program_id: accounts.program_id,
        accounts: account_list,
        data,
    }
}

/// Complete 13-Account Matrix for Raydium CP-Swap (CPMM) `swap_base_input` instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmAccounts {
    pub payer: Pubkey,
    pub authority: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub input_token_program: Pubkey,
    pub output_token_program: Pubkey,
    pub input_token_mint: Pubkey,
    pub output_token_mint: Pubkey,
    pub observation_state: Pubkey,
}

impl RaydiumCpmmAccounts {
    /// Canonical mainnet SOL-USDC CPMM pool.
    pub fn mainnet_sol_usdc(
        user_authority: Pubkey,
        user_input_token: Pubkey,
        user_output_token: Pubkey,
    ) -> Self {
        let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
        let pool = Pubkey::from_str(RAYDIUM_CPMM_SOL_USDC_POOL).unwrap();
        let (authority, _) = Pubkey::find_program_address(&[RAYDIUM_CPMM_AUTH_SEED], &program_id);
        let (observation_state, _) = Pubkey::find_program_address(
            &[RAYDIUM_CPMM_OBSERVATION_SEED, pool.as_ref()],
            &program_id,
        );
        let amm_config = Pubkey::from_str("D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ").unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

        Self {
            payer: user_authority,
            authority,
            amm_config,
            pool_state: pool,
            input_token_account: user_input_token,
            output_token_account: user_output_token,
            input_vault: Pubkey::from_str(RAYDIUM_CPMM_SOL_VAULT).unwrap(),
            output_vault: Pubkey::from_str(RAYDIUM_CPMM_USDC_VAULT).unwrap(),
            input_token_program: token_program,
            output_token_program: token_program,
            input_token_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
            output_token_mint: Pubkey::from_str(USDC_MINT).unwrap(),
            observation_state,
        }
    }

    /// Construct accounts for an arbitrary pool by deriving PDAs.
    #[allow(clippy::too_many_arguments)]
    pub fn for_pool(
        pool: Pubkey,
        payer: Pubkey,
        input_token_account: Pubkey,
        output_token_account: Pubkey,
        input_token_mint: Pubkey,
        output_token_mint: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        amm_config: Pubkey,
    ) -> Self {
        let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
        let (authority, _) = Pubkey::find_program_address(&[RAYDIUM_CPMM_AUTH_SEED], &program_id);
        let (observation_state, _) = Pubkey::find_program_address(
            &[RAYDIUM_CPMM_OBSERVATION_SEED, pool.as_ref()],
            &program_id,
        );
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

        Self {
            payer,
            authority,
            amm_config,
            pool_state: pool,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            input_token_program: token_program,
            output_token_program: token_program,
            input_token_mint,
            output_token_mint,
            observation_state,
        }
    }
}

/// Derive Raydium CPMM pool vault PDA for a given pool state and token mint.
/// Seeds: `[b"pool_vault", pool_state.as_ref(), mint.as_ref()]`
pub fn derive_raydium_cpmm_vault_pda(pool_state: &Pubkey, mint: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[
        RAYDIUM_CPMM_POOL_VAULT_SEED,
        pool_state.as_ref(),
        mint.as_ref(),
    ];
    Pubkey::find_program_address(seeds, &program_id).0
}

/// Derive Raydium CPMM authority PDA.
pub fn derive_raydium_cpmm_authority() -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[RAYDIUM_CPMM_AUTH_SEED], &program_id).0
}

/// Derive Raydium CPMM observation state PDA.
pub fn derive_raydium_cpmm_observation_pda(pool_state: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(
        &[RAYDIUM_CPMM_OBSERVATION_SEED, pool_state.as_ref()],
        &program_id,
    )
    .0
}

/// Build genuine Raydium CPMM `swap_base_input` instruction with the official 13-account matrix.
pub fn build_raydium_cpmm_swap_instruction(
    accounts: &RaydiumCpmmAccounts,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(accounts.payer, true),
            AccountMeta::new_readonly(accounts.authority, false),
            AccountMeta::new_readonly(accounts.amm_config, false),
            AccountMeta::new(accounts.pool_state, false),
            AccountMeta::new(accounts.input_token_account, false),
            AccountMeta::new(accounts.output_token_account, false),
            AccountMeta::new(accounts.input_vault, false),
            AccountMeta::new(accounts.output_vault, false),
            AccountMeta::new_readonly(accounts.input_token_program, false),
            AccountMeta::new_readonly(accounts.output_token_program, false),
            AccountMeta::new_readonly(accounts.input_token_mint, false),
            AccountMeta::new_readonly(accounts.output_token_mint, false),
            AccountMeta::new(accounts.observation_state, false),
        ],
        data,
    })
}

/// Complete Account Matrix for Raydium CLMM `swap` instruction (with 60-ticks/array tick arrays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumClmmAccounts {
    pub payer: Pubkey,
    pub amm_config: Pubkey,
    pub pool_state: Pubkey,
    pub input_token_account: Pubkey,
    pub output_token_account: Pubkey,
    pub input_vault: Pubkey,
    pub output_vault: Pubkey,
    pub observation_state: Pubkey,
    pub token_program: Pubkey,
    pub tick_array_0: Pubkey,
    pub tick_array_1: Pubkey,
    pub tick_array_2: Pubkey,
}

impl RaydiumClmmAccounts {
    #[allow(clippy::too_many_arguments)]
    pub fn for_pool(
        pool_state: Pubkey,
        payer: Pubkey,
        input_token_account: Pubkey,
        output_token_account: Pubkey,
        input_vault: Pubkey,
        output_vault: Pubkey,
        amm_config: Pubkey,
        tick_spacing: u16,
        tick_current: i32,
        zero_for_one: bool,
    ) -> Self {
        let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let (observation_state, _) = Pubkey::find_program_address(
            &[RAYDIUM_CLMM_OBSERVATION_SEED, pool_state.as_ref()],
            &program_id,
        );

        let spacing = (tick_spacing as i32).max(1);
        let ticks_in_array = 60 * spacing;
        let start_tick = tick_current.div_euclid(ticks_in_array) * ticks_in_array;

        let derive_ta = |start: i32| {
            let start_bytes = start.to_be_bytes();
            let seeds: &[&[u8]] = &[
                RAYDIUM_CLMM_TICK_ARRAY_SEED,
                pool_state.as_ref(),
                &start_bytes,
            ];
            Pubkey::find_program_address(seeds, &program_id).0
        };

        let ta0 = derive_ta(start_tick);
        let (ta1, ta2) = if zero_for_one {
            (
                derive_ta(start_tick - ticks_in_array),
                derive_ta(start_tick - 2 * ticks_in_array),
            )
        } else {
            (
                derive_ta(start_tick + ticks_in_array),
                derive_ta(start_tick + 2 * ticks_in_array),
            )
        };

        Self {
            payer,
            amm_config,
            pool_state,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            observation_state,
            token_program,
            tick_array_0: ta0,
            tick_array_1: ta1,
            tick_array_2: ta2,
        }
    }
}

/// Derive Raydium CLMM tick array PDA.
pub fn derive_raydium_clmm_tick_array_pda(pool_state: &Pubkey, start_index: i32) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    let start_bytes = start_index.to_be_bytes();
    let seeds: &[&[u8]] = &[
        RAYDIUM_CLMM_TICK_ARRAY_SEED,
        pool_state.as_ref(),
        &start_bytes,
    ];
    Pubkey::find_program_address(seeds, &program_id).0
}

/// Derive Raydium CLMM observation PDA.
pub fn derive_raydium_clmm_observation_pda(pool_state: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(
        &[RAYDIUM_CLMM_OBSERVATION_SEED, pool_state.as_ref()],
        &program_id,
    )
    .0
}

/// Derive Raydium CLMM vault PDA.
pub fn derive_raydium_clmm_vault_pda(pool_state: &Pubkey, mint: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[b"pool_vault", pool_state.as_ref(), mint.as_ref()];
    Pubkey::find_program_address(seeds, &program_id).0
}

/// Build genuine Raydium CLMM `swap` instruction.
pub fn build_raydium_clmm_swap_instruction(
    accounts: &RaydiumClmmAccounts,
    amount_in: u64,
    other_amount_threshold: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&other_amount_threshold.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(accounts.payer, true),
            AccountMeta::new_readonly(accounts.amm_config, false),
            AccountMeta::new(accounts.pool_state, false),
            AccountMeta::new(accounts.input_token_account, false),
            AccountMeta::new(accounts.output_token_account, false),
            AccountMeta::new(accounts.input_vault, false),
            AccountMeta::new(accounts.output_vault, false),
            AccountMeta::new(accounts.observation_state, false),
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new(accounts.tick_array_0, false),
            AccountMeta::new(accounts.tick_array_1, false),
            AccountMeta::new(accounts.tick_array_2, false),
        ],
        data,
    })
}

/// Complete 12-Account Matrix for Pump.fun Buy and Sell instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpFunAccounts {
    pub global: Pubkey,
    pub fee_recipient: Pubkey,
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub associated_bonding_curve: Pubkey,
    pub associated_user: Pubkey,
    pub user: Pubkey,
    pub system_program: Pubkey,
    pub token_program: Pubkey,
    pub rent: Pubkey,
    pub event_authority: Pubkey,
    pub program: Pubkey,
}

impl PumpFunAccounts {
    pub fn for_mint(mint: Pubkey, user: Pubkey) -> Self {
        let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
        let fee_recipient = Pubkey::from_str(PUMP_FUN_FEE_RECIPIENT).unwrap();
        let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
        let rent = Pubkey::from_str(SYSVAR_RENT_ID).unwrap();

        let (global, _) = Pubkey::find_program_address(&[PUMP_FUN_GLOBAL_SEED], &program);
        let (bonding_curve, _) =
            Pubkey::find_program_address(&[PUMP_FUN_BONDING_CURVE_SEED, mint.as_ref()], &program);
        let (event_authority, _) =
            Pubkey::find_program_address(&[PUMP_FUN_EVENT_AUTHORITY_SEED], &program);

        let associated_bonding_curve = get_associated_token_address(&bonding_curve, &mint);
        let associated_user = get_associated_token_address(&user, &mint);

        Self {
            global,
            fee_recipient,
            mint,
            bonding_curve,
            associated_bonding_curve,
            associated_user,
            user,
            system_program,
            token_program,
            rent,
            event_authority,
            program,
        }
    }
}

/// Derive Pump.fun global PDA.
pub fn derive_pump_fun_global_pda() -> Pubkey {
    let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[PUMP_FUN_GLOBAL_SEED], &program).0
}

/// Derive Pump.fun bonding curve PDA for a token mint.
pub fn derive_pump_fun_bonding_curve_pda(mint: &Pubkey) -> Pubkey {
    let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[PUMP_FUN_BONDING_CURVE_SEED, mint.as_ref()], &program).0
}

/// Derive Pump.fun event authority PDA.
pub fn derive_pump_fun_event_authority() -> Pubkey {
    let program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
    Pubkey::find_program_address(&[PUMP_FUN_EVENT_AUTHORITY_SEED], &program).0
}

/// Build genuine Pump.fun swap instruction (Buy or Sell) with the official 12-account matrix.
pub fn build_pump_fun_swap_instruction(
    accounts: &PumpFunAccounts,
    amount_in: u64,
    min_amount_out: u64,
    is_buy: bool,
) -> Result<CompiledInstruction, TxError> {
    let mut data = Vec::with_capacity(24);
    if is_buy {
        data.extend_from_slice(&PUMP_FUN_BUY_DISCRIMINATOR);
        data.extend_from_slice(&min_amount_out.to_le_bytes()); // amount of tokens to buy
        data.extend_from_slice(&amount_in.to_le_bytes()); // max_sol_cost
    } else {
        data.extend_from_slice(&PUMP_FUN_SELL_DISCRIMINATOR);
        data.extend_from_slice(&amount_in.to_le_bytes()); // amount of tokens to sell
        data.extend_from_slice(&min_amount_out.to_le_bytes()); // min_sol_output
    }

    Ok(CompiledInstruction {
        program_id: accounts.program,
        accounts: vec![
            AccountMeta::new_readonly(accounts.global, false),
            AccountMeta::new(accounts.fee_recipient, false),
            AccountMeta::new_readonly(accounts.mint, false),
            AccountMeta::new(accounts.bonding_curve, false),
            AccountMeta::new(accounts.associated_bonding_curve, false),
            AccountMeta::new(accounts.associated_user, false),
            AccountMeta::new(accounts.user, true),
            AccountMeta::new_readonly(accounts.system_program, false),
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new_readonly(accounts.rent, false),
            AccountMeta::new_readonly(accounts.event_authority, false),
            AccountMeta::new_readonly(accounts.program, false),
        ],
        data,
    })
}

/// Build genuine DEX swap instruction based on protocol hop with complete accounts matrix.
pub fn build_swap_hop_instruction(
    hop: &SwapHop,
    user_authority: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    match hop.protocol {
        DexProtocol::PumpSwap => Err(TxError::UnsupportedProtocol("PumpSwap transaction adapter is not enabled; shadow-only".into())),
        DexProtocol::RaydiumV4 => {
            let mut accounts = RaydiumV4Accounts::mainnet_sol_usdc();
            if hop.pool_id.0 != Pubkey::default() && hop.pool_id.0 != accounts.amm_pool {
                accounts.amm_pool = hop.pool_id.0;
            }
            build_raydium_v4_swap_instruction_18(
                &accounts,
                user_authority,
                get_associated_token_address(&user_authority, &hop.mint_in.0),
                get_associated_token_address(&user_authority, &hop.mint_out.0),
                amount_in,
                min_amount_out,
            )
        }
        DexProtocol::OrcaWhirlpool => {
            let a_to_b = hop.mint_in.0 == Pubkey::from_str(WSOL_MINT).unwrap_or(hop.mint_in.0);
            let pool_pk = if hop.pool_id.0 != Pubkey::default() {
                hop.pool_id.0
            } else {
                Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()
            };
            let is_mainnet = pool_pk == Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap();
            let accounts = if is_mainnet {
                OrcaWhirlpoolAccounts::mainnet_sol_usdc()
            } else {
                let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
                let (token_vault_a, _) = Pubkey::find_program_address(
                    &[b"token_vault_a", pool_pk.as_ref()],
                    &program_id,
                );
                let (token_vault_b, _) = Pubkey::find_program_address(
                    &[b"token_vault_b", pool_pk.as_ref()],
                    &program_id,
                );
                OrcaWhirlpoolAccounts::for_whirlpool(
                    pool_pk,
                    token_vault_a,
                    token_vault_b,
                    64,
                    0,
                    a_to_b,
                )
            };
            let (token_owner_a, token_owner_b) = if a_to_b {
                (
                    get_associated_token_address(&user_authority, &hop.mint_in.0),
                    get_associated_token_address(&user_authority, &hop.mint_out.0),
                )
            } else {
                (
                    get_associated_token_address(&user_authority, &hop.mint_out.0),
                    get_associated_token_address(&user_authority, &hop.mint_in.0),
                )
            };
            build_orca_whirlpool_swap_instruction_11(
                &accounts,
                user_authority,
                token_owner_a,
                token_owner_b,
                amount_in,
                min_amount_out,
                a_to_b,
            )
        }
        DexProtocol::MeteoraDlmm => {
            let mainnet_pool = Pubkey::from_str(METEORA_SOL_USDC_POOL).unwrap();
            let accounts = if hop.pool_id.0 == mainnet_pool {
                MeteoraDlmmAccounts::mainnet_sol_usdc()
            } else {
                let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
                let (reserve_x, _) = Pubkey::find_program_address(
                    &[b"reserve_x", hop.pool_id.0.as_ref()],
                    &program_id,
                );
                let (reserve_y, _) = Pubkey::find_program_address(
                    &[b"reserve_y", hop.pool_id.0.as_ref()],
                    &program_id,
                );
                let (oracle, _) =
                    Pubkey::find_program_address(&[b"oracle", hop.pool_id.0.as_ref()], &program_id);
                let (event_authority, _) =
                    Pubkey::find_program_address(&[b"__event_authority"], &program_id);
                MeteoraDlmmAccounts {
                    lb_pair: hop.pool_id.0,
                    reserve_x,
                    reserve_y,
                    token_x_mint: hop.mint_in.0,
                    token_y_mint: hop.mint_out.0,
                    oracle,
                    event_authority,
                    program_id,
                }
            };
            let mint_in = hop.mint_in.0;
            let mint_out = hop.mint_out.0;
            let swap_for_y = mint_in.to_bytes() < mint_out.to_bytes();
            let active_bin_id = hop.active_bin_id.unwrap_or(0);
            Ok(build_meteora_dlmm_swap_instruction(
                &accounts,
                user_authority,
                get_associated_token_address(&user_authority, &hop.mint_in.0),
                get_associated_token_address(&user_authority, &hop.mint_out.0),
                amount_in,
                min_amount_out,
                active_bin_id,
                METEORA_SOL_USDC_BIN_STEP,
                swap_for_y,
            ))
        }
        DexProtocol::RaydiumCpmm => {
            let mainnet_pool = Pubkey::from_str(RAYDIUM_CPMM_SOL_USDC_POOL).unwrap();
            let user_token_in = get_associated_token_address(&user_authority, &hop.mint_in.0);
            let user_token_out = get_associated_token_address(&user_authority, &hop.mint_out.0);

            let accounts = if hop.pool_id.0 == mainnet_pool {
                RaydiumCpmmAccounts::mainnet_sol_usdc(user_authority, user_token_in, user_token_out)
            } else {
                let pool = hop.pool_id.0;
                let input_vault = derive_raydium_cpmm_vault_pda(&pool, &hop.mint_in.0);
                let output_vault = derive_raydium_cpmm_vault_pda(&pool, &hop.mint_out.0);
                let amm_config =
                    Pubkey::from_str("D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ").unwrap();
                RaydiumCpmmAccounts::for_pool(
                    pool,
                    user_authority,
                    user_token_in,
                    user_token_out,
                    hop.mint_in.0,
                    hop.mint_out.0,
                    input_vault,
                    output_vault,
                    amm_config,
                )
            };
            build_raydium_cpmm_swap_instruction(&accounts, amount_in, min_amount_out)
        }
        DexProtocol::RaydiumClmm => {
            let pool = hop.pool_id.0;
            let user_token_in = get_associated_token_address(&user_authority, &hop.mint_in.0);
            let user_token_out = get_associated_token_address(&user_authority, &hop.mint_out.0);
            let input_vault = derive_raydium_clmm_vault_pda(&pool, &hop.mint_in.0);
            let output_vault = derive_raydium_clmm_vault_pda(&pool, &hop.mint_out.0);
            let amm_config =
                Pubkey::from_str("D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ").unwrap();
            let zero_for_one = hop.mint_in.0.to_bytes() < hop.mint_out.0.to_bytes();
            let tick_current = hop.active_bin_id.unwrap_or(0);
            let tick_spacing = 60; // standard Raydium CLMM spacing
            let accounts = RaydiumClmmAccounts::for_pool(
                pool,
                user_authority,
                user_token_in,
                user_token_out,
                input_vault,
                output_vault,
                amm_config,
                tick_spacing,
                tick_current,
                zero_for_one,
            );
            const Q64: u128 = 1u128 << 64;
            let sqrt_price_limit = if amount_in > 0 && min_amount_out > 0 {
                if zero_for_one {
                    (u128::from(min_amount_out) * Q64) / u128::from(amount_in)
                } else {
                    (u128::from(amount_in) * Q64) / u128::from(min_amount_out)
                }
            } else if zero_for_one {
                4_295_048_016u128
            } else {
                79_226_673_515_401_499_993_453_864_427u128
            };
            build_raydium_clmm_swap_instruction(
                &accounts,
                amount_in,
                min_amount_out,
                sqrt_price_limit,
                true,
            )
        }
        DexProtocol::PumpFun => Err(TxError::UnsupportedProtocol(
            "Pump.fun live adapter disabled: current buy_v2/v3 account matrix and fee mode must be verified before signing".into(),
        )),
    }
}

/// In-memory Keypair Signer securely decoded from Base58 with real Ed25519 signing.
#[derive(Clone)]
pub struct KeypairSigner {
    pubkey: Pubkey,
    signing_key: SigningKey,
}

impl std::fmt::Debug for KeypairSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeypairSigner")
            .field("pubkey", &self.pubkey)
            .field("signing_key", &"[REDACTED_SECURE]")
            .finish()
    }
}

impl KeypairSigner {
    pub fn from_base58(secret_b58: &str) -> Result<Self> {
        let decoded = bs58::decode(secret_b58.trim())
            .into_vec()
            .context("Failed to decode base58 secret key")?;

        if decoded.len() != 64 {
            bail!(
                "Invalid Solana private key length: expected 64 bytes, got {}",
                decoded.len()
            );
        }

        let seed: [u8; 32] = decoded[0..32]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid secret seed"))?;
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes: [u8; 32] = decoded[32..64]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid pubkey"))?;
        let pubkey = Pubkey::new_from_array(pubkey_bytes);

        Ok(Self {
            pubkey,
            signing_key,
        })
    }

    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let pubkey = Pubkey::new_from_array(pubkey_bytes);
        Self {
            pubkey,
            signing_key,
        }
    }

    pub fn from_bytes(bytes: &[u8; 64]) -> Result<Self> {
        let seed: [u8; 32] = bytes[0..32]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid secret seed"))?;
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes: [u8; 32] = bytes[32..64]
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid pubkey"))?;
        let pubkey = Pubkey::new_from_array(pubkey_bytes);

        Ok(Self {
            pubkey,
            signing_key,
        })
    }

    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let trimmed = content.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let bytes: Vec<u8> = serde_json::from_str(trimmed)?;
            if bytes.len() != 64 {
                bail!(
                    "Invalid keypair JSON length: expected 64, got {}",
                    bytes.len()
                );
            }
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            Self::from_bytes(&arr)
        } else {
            Self::from_base58(trimmed)
        }
    }

    pub fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    /// Sign payload using real cryptographic Ed25519 signature algorithm
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let sig: Signature = self.signing_key.sign(message);
        sig.to_bytes()
    }

    /// Verifying key for signature verification
    pub fn verifying_key(&self) -> ed25519_dalek::VerifyingKey {
        self.signing_key.verifying_key()
    }
}

/// Address Lookup Table structure for Solana v0 versioned transactions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressLookupTable {
    pub key: Pubkey,
    pub addresses: Vec<Pubkey>,
}

impl AddressLookupTable {
    pub fn new(key: Pubkey, addresses: Vec<Pubkey>) -> Self {
        Self { key, addresses }
    }
}

/// Parameters for assembling and signing an atomic Kamino flash loan bundle.
#[derive(Debug, Clone)]
pub struct KaminoBundleParams<'a> {
    pub payer: Pubkey,
    pub signer: &'a KeypairSigner,
    pub tip_account: Pubkey,
    pub tip_lamports: Lamports,
    pub priority_fee_micro_lamports: u64,
    pub opportunity: &'a ExecutionOpportunity,
    pub kamino_params: &'a KaminoFlashParams,
    pub recent_blockhash: [u8; 32],
    pub lookup_tables: &'a [AddressLookupTable],
}

/// Versioned transaction container matching Solana v0 wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedTransaction {
    pub signatures: Vec<[u8; 64]>,
    pub message: Vec<u8>,
    pub wire_bytes: Vec<u8>,
}

impl VersionedTransaction {
    pub fn into_bytes(self) -> Vec<u8> {
        self.wire_bytes
    }
}

impl AsRef<[u8]> for VersionedTransaction {
    fn as_ref(&self) -> &[u8] {
        &self.wire_bytes
    }
}

/// Builder for constructing atomic arbitrage transactions and private bundles.
#[derive(Debug, Default, Clone)]
pub struct TxBuilder {
    instructions: Vec<CompiledInstruction>,
}

impl TxBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an instruction to the transaction.
    pub fn add_instruction(&mut self, ix: CompiledInstruction) {
        self.instructions.push(ix);
    }

    /// Assemble strictly the atomic 7-instruction (2-hop) or 8-instruction (3-hop) bundle.
    /// Excludes ephemeral ATA creation from the hot path.
    ///
    /// Instruction Layout:
    /// [0] SetComputeUnitLimit (260,000 for 2-hop / 320,000 for 3-hop)
    /// [1] SetComputeUnitPrice (priority fee micro-lamports)
    /// [2] Kamino FlashBorrow (amount: d*)
    /// [3] Swap Leg 1
    /// [4] Swap Leg 2 (and 3 if 3-hop)
    /// [5] (or [6]) Kamino FlashRepay (amount: d* + fee, borrow_instruction_index: 2)
    /// [6] (or [7]) Jito Tip Transfer
    pub fn assemble_atomic_bundle(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        priority_fee_micro_lamports: u64,
        opportunity: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
        min_profit_raw: TokenAmount, // Profit floor in base-mint raw units
    ) -> Result<Vec<CompiledInstruction>, TxError> {
        let hops_count = opportunity.hops.len();
        if !(2..=3).contains(&hops_count) {
            return Err(TxError::InvalidHopCount(hops_count));
        }

        let d_star = opportunity.size_in.as_u64();
        if d_star == 0 {
            return Err(TxError::ZeroBorrowAmount);
        }
        // P0 #4: ensure the route returns to the borrowed mint.
        if let (Some(first), Some(last)) = (opportunity.hops.first(), opportunity.hops.last()) {
            if first.mint_in != last.mint_out {
                return Err(TxError::NonClosingRoute {
                    in_mint: first.mint_in.0.to_string(),
                    out_mint: last.mint_out.0.to_string(),
                });
            }
        }

        let mut ixs = Vec::with_capacity(if hops_count == 2 { 7 } else { 8 });

        // [0] SetComputeUnitLimit: 260_000 (2-hop) or 320_000 (3-hop)
        let cu_limit = get_cu_limit_for_hops(hops_count);
        ixs.push(build_compute_unit_limit(cu_limit));

        // [1] SetComputeUnitPrice
        ixs.push(build_compute_unit_price(priority_fee_micro_lamports));

        // [2] Kamino FlashBorrow (placed at index 2)
        let borrow_idx = ixs.len();
        let accounts = kamino_params.to_flash_accounts(payer);
        ixs.push(kamino::build_kamino_flash_borrow(&accounts, d_star));

        // [3..] Swap Legs
        let slippage_bps: u64 = DEFAULT_HOP_SLIPPAGE_BPS;
        let fee = calculate_kamino_flash_fee(d_star, flash_loan_fee_sf);
        let tip_raw = tip_amount.as_u64();
        let legs = Self::derive_swap_legs(
            opportunity,
            fee,
            tip_raw,
            min_profit_raw.as_u64(),
            slippage_bps,
        )?;
        for (hop, (amount_in, min_out)) in opportunity.hops.iter().zip(legs.iter()) {
            ixs.push(build_swap_hop_instruction(
                hop, payer, *amount_in, *min_out,
            )?);
        }

        // [5] (or [6]) Kamino FlashRepay (borrow_instruction_index = 2)
        let repay_amount = d_star.saturating_add(fee);
        ixs.push(kamino::build_kamino_flash_repay(
            &accounts,
            repay_amount,
            borrow_idx as u8,
        ));

        // [6] (or [7]) Jito Tip Transfer
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Full end-to-end builder producing a signed Solana v0 Versioned Transaction.
    pub fn build_kamino_bundle(
        params: &KaminoBundleParams,
    ) -> Result<VersionedTransaction, TxError> {
        // min_profit_raw from opportunity (base-mint raw units)
        let min_profit_raw = TokenAmount(params.opportunity.net_profit.as_u64());
        let ixs = Self::assemble_atomic_bundle(
            params.payer,
            params.tip_account,
            params.tip_lamports,
            params.priority_fee_micro_lamports,
            params.opportunity,
            params.kamino_params,
            0, // fee_sf: KaminoBundleParams does not carry the verified fee; callers must re-price
            min_profit_raw,
        )?;

        let message = Self::compile_v0_message(
            params.signer.pubkey(),
            &ixs,
            params.recent_blockhash,
            params.lookup_tables,
        )
        .map_err(|e| TxError::AltCompilationError(e.to_string()))?;

        let signature = params.signer.sign(&message);

        let mut wire_bytes = Vec::with_capacity(1 + 64 + message.len());
        Self::encode_compact_u16(1, &mut wire_bytes);
        wire_bytes.extend_from_slice(&signature);
        wire_bytes.extend_from_slice(&message);

        if wire_bytes.len() > SOLANA_PACKET_MTU_LIMIT {
            return Err(TxError::TransactionTooLarge {
                size: wire_bytes.len(),
                limit: SOLANA_PACKET_MTU_LIMIT,
            });
        }

        Ok(VersionedTransaction {
            signatures: vec![signature],
            message,
            wire_bytes,
        })
    }

    /// Assemble full native arbitrage execution instructions from an Opportunity.
    /// ComputeBudget instructions are strictly injected BEFORE swap instructions.
    pub fn assemble_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        opportunity: &ExecutionOpportunity,
        min_profit_raw: TokenAmount, // Profit floor in base-mint raw units
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();

        if let (Some(first), Some(last)) = (opportunity.hops.first(), opportunity.hops.last()) {
            if first.mint_in != last.mint_out {
                return Err(TxError::NonClosingRoute {
                    in_mint: first.mint_in.0.to_string(),
                    out_mint: last.mint_out.0.to_string(),
                }
                .into());
            }
            if first.mint_in.0 != Pubkey::from_str(WSOL_MINT).unwrap() {
                return Err(TxError::NonWsolFlashLoan(first.mint_in.0.to_string()).into());
            }
        }

        // 1. Compute Budget (Dynamically bounded <= 200,000 CU with 15% buffer, elevated priority fee)
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        let dynamic_cu_price = calculate_dynamic_cu_price(
            opportunity.gross_profit.as_u64(),
            priority_fee_micro_lamports,
        );
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(dynamic_cu_price));

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        // Identify any intermediate tokens (from 40 trending tokens) needing ephemeral ATAs
        let mut intermediate_atas = Vec::new();
        for hop in &opportunity.hops {
            for mint in [hop.mint_in.0, hop.mint_out.0] {
                if mint != wsol
                    && mint != usdc
                    && !intermediate_atas.iter().any(|(m, _)| *m == mint)
                {
                    let ata = get_associated_token_address(&payer, &mint);
                    intermediate_atas.push((mint, ata));
                }
            }
        }

        // Ephemeral ATA Creation (Idempotent: safe if already exists, creates if missing)
        for (mint, ata) in &intermediate_atas {
            ixs.push(build_create_ata_idempotent(payer, payer, *mint, *ata));
        }

        // 2. Arbitrage Swap Hops (Injected after Compute Budget and ATA creation)
        let slippage_bps: u64 = DEFAULT_HOP_SLIPPAGE_BPS;
        let tip_raw = tip_amount.as_u64();
        let legs = Self::derive_swap_legs(
            opportunity,
            0,
            tip_raw,
            min_profit_raw.as_u64(),
            slippage_bps,
        )?;

        let user_wsol_ata = get_associated_token_address(&payer, &wsol);
        let first_is_pump_buy = opportunity
            .hops
            .first()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);
        let last_is_pump_sell = opportunity
            .hops
            .last()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);

        // If Hop 1 is Raydium/Meteora (needs WSOL) but last is Pump.fun (yields native SOL):
        // Wrap amount_in of native SOL to WSOL ATA before Hop 1
        if !first_is_pump_buy && last_is_pump_sell {
            ixs.push(build_create_ata_idempotent(
                payer,
                payer,
                wsol,
                user_wsol_ata,
            ));
            if let Some(first_leg) = legs.first() {
                ixs.push(build_system_transfer(payer, user_wsol_ata, first_leg.0));
                ixs.push(build_sync_native(user_wsol_ata));
            }
        }

        for (hop, (amount_in, min_out)) in opportunity.hops.iter().zip(legs.iter()) {
            ixs.push(build_swap_hop_instruction(
                hop, payer, *amount_in, *min_out,
            )?);
        }

        // If last hop outputted WSOL (e.g. Pump.fun Buy -> Raydium Sell):
        // Unwrap WSOL back to native SOL for the payer
        if first_is_pump_buy && !last_is_pump_sell {
            ixs.push(build_close_token_account(user_wsol_ata, payer, payer));
        }

        // 3. Close Ephemeral ATAs to reclaim 100% of the rent deposit (0.00203928 SOL) directly to payer
        for (_, ata) in &intermediate_atas {
            ixs.push(build_close_token_account(*ata, payer, payer));
        }

        // 4. Jito Tip at the end of the atomic transaction
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Derive `(amount_in, min_amount_out)` per hop.
    ///
    /// B1/B2 fixes:
    /// - `amount_in` for hop `i` is `hop_inputs[i]` — the previous hop's *expected*
    ///   output, never the discounted `min_out` and never the input amount.
    /// - `min_amount_out` discounts `expected_out` by slippage tolerance only,
    ///   applied once per hop (never deducted twice).
    /// - The final hop's `min_out` is raised to the profit floor
    ///   (`borrow + flash_fee + min_profit`).
    ///
    /// `hop_inputs` must be `[borrow, out_1, ..., out_n]` with length
    /// `hops.len() + 1`. There is no legacy fallback: intermediate inputs must
    /// come from the previous hop's actual expected output.
    pub fn derive_swap_legs(
        opportunity: &ExecutionOpportunity,
        flash_fee_raw: u64,
        tip_raw: u64,
        min_profit_raw: u64,
        slippage_bps: u64,
    ) -> Result<Vec<(u64, u64)>, TxError> {
        if opportunity.hops.is_empty() {
            return Ok(Vec::new());
        }

        let inputs = opportunity.hop_inputs.as_slice();
        if inputs.len() != opportunity.hops.len() + 1 {
            return Err(TxError::MissingHopInputs);
        }

        let borrow = inputs[0];
        // Enforce full on-chain solvent floor: borrow + flash_fee + jito_tip + net_retained_profit
        let floor = borrow
            .checked_add(flash_fee_raw)
            .and_then(|v| v.checked_add(tip_raw))
            .and_then(|v| v.checked_add(min_profit_raw))
            .ok_or_else(|| TxError::InvalidInput("profit floor arithmetic overflow".into()))?;

        let mut legs = Vec::with_capacity(opportunity.hops.len());
        for (i, _hop) in opportunity.hops.iter().enumerate() {
            let amount_in = inputs[i];
            let expected_out = inputs[i + 1];
            if amount_in > 0 && expected_out == 0 {
                return Err(TxError::InvalidInput(format!("hop {i} quotes zero output")));
            }

            // Slippage applied once per hop, never deducted twice.
            let min_out_slippage = expected_out
                .checked_sub((expected_out * slippage_bps) / 10_000)
                .ok_or_else(|| {
                    TxError::InvalidInput(format!("hop {i} slippage arithmetic overflow"))
                })?;

            let is_last = i + 1 == opportunity.hops.len();
            let min_out = if is_last {
                if expected_out < floor {
                    return Err(TxError::ProfitFloorNotMet {
                        expected: expected_out,
                        floor,
                    });
                }
                min_out_slippage.max(floor)
            } else {
                min_out_slippage
            };

            legs.push((amount_in, min_out));
        }
        Ok(legs)
    }

    /// Assemble atomic Kamino Flash Loan arbitrage transaction.
    /// Order: ComputeBudget -> Ephemeral ATAs -> FlashBorrow -> Swaps -> FlashRepay -> Close ATAs (Rent Reclaim) -> Jito Tip.
    pub fn assemble_flash_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        opportunity: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
        min_profit_raw: TokenAmount, // Profit floor in base-mint raw units
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();

        // 1. Compute Budget Limit & Dynamic Priority Fee (Injected BEFORE Swaps & Borrow)
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        let dynamic_cu_price = calculate_dynamic_cu_price(
            opportunity.gross_profit.as_u64(),
            priority_fee_micro_lamports,
        );
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(dynamic_cu_price));

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let user_wsol_ata = get_associated_token_address(&payer, &wsol);
        let mut resolved_params = kamino_params.clone();
        if resolved_params.user_destination_liquidity == Pubkey::default()
            || resolved_params.user_destination_liquidity == wsol
        {
            resolved_params.user_destination_liquidity = user_wsol_ata;
        }

        // 2. Ensure user's WSOL ATA exists idempotently for Flash Loan receipt
        ixs.push(build_create_ata_idempotent(
            payer,
            payer,
            wsol,
            user_wsol_ata,
        ));

        // Identify any intermediate tokens needing ephemeral ATAs
        let mut intermediate_atas = Vec::new();
        for hop in &opportunity.hops {
            for mint in [hop.mint_in.0, hop.mint_out.0] {
                if mint != wsol
                    && mint != usdc
                    && !intermediate_atas.iter().any(|(m, _)| *m == mint)
                {
                    let ata = get_associated_token_address(&payer, &mint);
                    intermediate_atas.push((mint, ata));
                }
            }
        }

        // Ephemeral ATA Creation (Idempotent: zero failure if already exists)
        for (mint, ata) in &intermediate_atas {
            ixs.push(build_create_ata_idempotent(payer, payer, *mint, *ata));
        }

        // 3. Kamino Flash Borrow
        let borrow_idx = ixs.len();
        let borrow_amount = opportunity.size_in.as_u64();
        if let (Some(first), Some(last)) = (opportunity.hops.first(), opportunity.hops.last()) {
            if first.mint_in != last.mint_out {
                return Err(TxError::NonClosingRoute {
                    in_mint: first.mint_in.0.to_string(),
                    out_mint: last.mint_out.0.to_string(),
                }
                .into());
            }
            if first.mint_in.0 != Pubkey::from_str(WSOL_MINT).unwrap() {
                return Err(TxError::NonWsolFlashLoan(first.mint_in.0.to_string()).into());
            }
        }
        ixs.push(build_kamino_flash_borrow(
            &resolved_params,
            payer,
            borrow_amount,
        ));

        // 4. Sequential Genuine Swap Hops (Injected before Repay)
        let slippage_bps: u64 = DEFAULT_HOP_SLIPPAGE_BPS;
        let fee = calculate_kamino_flash_fee(borrow_amount, flash_loan_fee_sf);
        let tip_raw = tip_amount.as_u64();
        let legs = Self::derive_swap_legs(
            opportunity,
            fee,
            tip_raw,
            min_profit_raw.as_u64(),
            slippage_bps,
        )?;

        let first_is_pump_buy = opportunity
            .hops
            .first()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);
        let last_is_pump_sell = opportunity
            .hops
            .last()
            .map(|h| h.protocol == DexProtocol::PumpFun)
            .unwrap_or(false);

        // If Hop 1 is Pump.fun Buy: unwrap borrowed WSOL into native SOL so Pump.fun can spend it
        if first_is_pump_buy {
            ixs.push(build_close_token_account(user_wsol_ata, payer, payer));
        }

        for (i, (hop, (amount_in, min_out))) in opportunity.hops.iter().zip(legs.iter()).enumerate()
        {
            // If Hop 1 unwrapped user_wsol_ata and Hop 2 outputs WSOL (Raydium/Meteora):
            // Re-create user_wsol_ata so Hop 2 has a valid destination
            if first_is_pump_buy && i == 1 {
                ixs.push(build_create_ata_idempotent(
                    payer,
                    payer,
                    wsol,
                    user_wsol_ata,
                ));
            }
            ixs.push(build_swap_hop_instruction(
                hop, payer, *amount_in, *min_out,
            )?);
        }

        let repay_amount = borrow_amount.saturating_add(fee);

        // If last hop was Pump.fun Sell: user received native SOL, wrap it back into WSOL for Kamino repay
        if last_is_pump_sell {
            ixs.push(build_create_ata_idempotent(
                payer,
                payer,
                wsol,
                user_wsol_ata,
            ));
            ixs.push(build_system_transfer(payer, user_wsol_ata, repay_amount));
            ixs.push(build_sync_native(user_wsol_ata));
        }

        // 5. Kamino Flash Repay (repay amount + fee)
        ixs.push(build_kamino_flash_repay_with_index(
            &resolved_params,
            payer,
            repay_amount,
            borrow_idx as u8,
        ));

        // 5. Ephemeral ATA Close & 100% Rent Reclaim directly to payer
        for (_, ata) in &intermediate_atas {
            ixs.push(build_close_token_account(*ata, payer, payer));
        }

        // 6. Jito Tip at the end
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Assemble atomic Kamino Flash Loan arbitrage using Jupiter v6 Swap Instructions.
    /// Order: ComputeBudget -> FlashBorrow -> Jupiter Swaps (all accounts resolved) -> FlashRepay -> Jito Tip.
    #[allow(clippy::too_many_arguments)]
    pub fn assemble_jupiter_flash_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        borrow_amount: u64,
        kamino_params: &KaminoFlashParams,
        jupiter_swap_instructions: Vec<CompiledInstruction>,
        flash_loan_fee_sf: u64,
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();

        // 1. Compute Budget Limit & Dynamic Priority Fee
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(priority_fee_micro_lamports));

        // 2. Kamino Flash Borrow
        let borrow_idx = ixs.len();
        ixs.push(build_kamino_flash_borrow(
            kamino_params,
            payer,
            borrow_amount,
        ));

        // 3. Jupiter Swap Instructions (All accounts resolved by Jupiter API)
        for ix in jupiter_swap_instructions {
            ixs.push(ix);
        }

        // 4. Kamino Flash Repay
        let fee = calculate_kamino_flash_fee(borrow_amount, flash_loan_fee_sf);
        let repay_amount = borrow_amount.saturating_add(fee);
        ixs.push(build_kamino_flash_repay_with_index(
            kamino_params,
            payer,
            repay_amount,
            borrow_idx as u8,
        ));

        // 5. Jito Tip at the end
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Assemble Kamino flash loan with split DEX swap legs.
    /// Order: ComputeBudget -> FlashBorrow -> Split Swaps -> Return Swap -> FlashRepay -> Jito Tip
    #[allow(clippy::too_many_arguments)]
    pub fn assemble_split_flash_arbitrage(
        payer: Pubkey,
        tip_account: Pubkey,
        tip_amount: Lamports,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        total_loan: u64,
        legs: &[CompiledInstruction],
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
    ) -> Result<Vec<CompiledInstruction>> {
        let mut ixs = Vec::new();
        let effective_cu = calculate_dynamic_cu_limit(cu_limit);
        ixs.push(build_compute_unit_limit(effective_cu));
        ixs.push(build_compute_unit_price(priority_fee_micro_lamports));

        // 1. Kamino Flash Borrow
        let borrow_idx = ixs.len();
        ixs.push(build_kamino_flash_borrow(kamino_params, payer, total_loan));

        // 2. Parallel / Split DEX Legs
        for leg in legs {
            ixs.push(leg.clone());
        }

        // 3. Kamino Flash Repay
        let fee = calculate_kamino_flash_fee(total_loan, flash_loan_fee_sf);
        let repay_amount = total_loan.saturating_add(fee);
        ixs.push(build_kamino_flash_repay_with_index(
            kamino_params,
            payer,
            repay_amount,
            borrow_idx as u8,
        ));

        // 4. Standalone Jito Tip
        if tip_amount.as_u64() > 0 {
            ixs.push(build_marked_jito_tip_instruction(
                payer,
                tip_account,
                tip_amount,
            ));
        }

        Ok(ixs)
    }

    /// Compact-u16 encoder according to Solana binary serialization spec.
    pub fn encode_compact_u16(mut val: u16, out: &mut Vec<u8>) {
        loop {
            let mut elem = (val & 0x7f) as u8;
            val >>= 7;
            if val == 0 {
                out.push(elem);
                break;
            } else {
                elem |= 0x80;
                out.push(elem);
            }
        }
    }

    /// Compile a list of instructions into a standard Solana legacy Message byte buffer.
    pub fn compile_message(
        payer: Pubkey,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        use std::collections::BTreeMap;

        let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
        // Payer is always account 0: signer and writable
        account_map.insert(payer, (true, true));

        for ix in instructions {
            for acc in &ix.accounts {
                let entry = account_map.entry(acc.pubkey).or_insert((false, false));
                entry.0 |= acc.is_signer;
                entry.1 |= acc.is_writable;
            }
            // Program IDs are non-signer, readonly
            account_map.entry(ix.program_id).or_insert((false, false));
        }

        let mut writable_signers = Vec::new();
        let mut readonly_signers = Vec::new();
        let mut writable_non_signers = Vec::new();
        let mut readonly_non_signers = Vec::new();

        writable_signers.push(payer);

        for (pubkey, (is_signer, is_writable)) in account_map {
            if pubkey == payer {
                continue;
            }
            if is_signer {
                if is_writable {
                    writable_signers.push(pubkey);
                } else {
                    readonly_signers.push(pubkey);
                }
            } else if is_writable {
                writable_non_signers.push(pubkey);
            } else {
                readonly_non_signers.push(pubkey);
            }
        }

        let num_required_signatures = (writable_signers.len() + readonly_signers.len()) as u8;
        let num_readonly_signed_accounts = readonly_signers.len() as u8;
        let num_readonly_unsigned_accounts = readonly_non_signers.len() as u8;

        let mut all_accounts = Vec::new();
        all_accounts.extend(writable_signers);
        all_accounts.extend(readonly_signers);
        all_accounts.extend(writable_non_signers);
        all_accounts.extend(readonly_non_signers);

        let key_to_index: std::collections::HashMap<Pubkey, u8> = all_accounts
            .iter()
            .enumerate()
            .map(|(idx, &pk)| (pk, idx as u8))
            .collect();

        let mut message = Vec::new();
        // Message Header
        message.push(num_required_signatures);
        message.push(num_readonly_signed_accounts);
        message.push(num_readonly_unsigned_accounts);

        // Account keys array
        Self::encode_compact_u16(all_accounts.len() as u16, &mut message);
        for pk in &all_accounts {
            message.extend_from_slice(pk.as_ref());
        }

        // Recent blockhash
        message.extend_from_slice(&recent_blockhash);

        // Compiled instructions
        Self::encode_compact_u16(instructions.len() as u16, &mut message);
        for ix in instructions {
            let prog_idx = *key_to_index.get(&ix.program_id).ok_or_else(|| {
                TxError::InstructionError(format!(
                    "program id not in account map: {}",
                    ix.program_id
                ))
            })?;
            message.push(prog_idx);

            Self::encode_compact_u16(ix.accounts.len() as u16, &mut message);
            for acc in &ix.accounts {
                let acc_idx = key_to_index.get(&acc.pubkey).ok_or_else(|| {
                    TxError::InstructionError(format!("account not in account map: {}", acc.pubkey))
                })?;
                message.push(*acc_idx);
            }

            Self::encode_compact_u16(ix.data.len() as u16, &mut message);
            message.extend_from_slice(&ix.data);
        }

        Ok(message)
    }

    /// Build a standalone Jito tip transaction for 2-transaction MEV bundles.
    pub fn build_jito_tip_transaction(
        signer: &KeypairSigner,
        tip_account: Pubkey,
        tip_amount: Lamports,
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        let ix = build_jito_tip_instruction(signer.pubkey(), tip_account, tip_amount);
        Self::build_signed_transaction(signer, &[ix], recent_blockhash)
    }

    /// Construct and sign a fully-formed Solana Wire Format transaction ready for RPC / Jito submission.
    pub fn build_signed_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
    ) -> Result<Vec<u8>, TxError> {
        let message = Self::compile_message(signer.pubkey(), instructions, recent_blockhash)?;
        let signature = signer.sign(&message);

        let mut tx = Vec::with_capacity(1 + 64 + message.len());
        Self::encode_compact_u16(1, &mut tx); // 1 signature
        tx.extend_from_slice(&signature);
        tx.extend_from_slice(&message);
        Ok(tx)
    }

    /// Compile a list of instructions and Address Lookup Tables into a Solana v0 Message byte buffer.
    pub fn compile_v0_message(
        payer: Pubkey,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        lookup_tables: &[AddressLookupTable],
    ) -> Result<Vec<u8>> {
        use std::collections::{BTreeMap, HashMap};

        // Track signer/writable requirements for all referenced accounts
        let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
        // Payer is always writable signer
        account_map.insert(payer, (true, true));

        for ix in instructions {
            for acc in &ix.accounts {
                let entry = account_map.entry(acc.pubkey).or_insert((false, false));
                entry.0 |= acc.is_signer;
                entry.1 |= acc.is_writable;
            }
            account_map.entry(ix.program_id).or_insert((false, false));
        }

        struct TableLookupEntry {
            table_key: Pubkey,
            writable_indices: Vec<u8>,
            readonly_indices: Vec<u8>,
        }

        let mut table_entries: Vec<TableLookupEntry> = lookup_tables
            .iter()
            .map(|t| TableLookupEntry {
                table_key: t.key,
                writable_indices: Vec::new(),
                readonly_indices: Vec::new(),
            })
            .collect();

        let mut loaded_writable_keys = Vec::new();
        let mut loaded_readonly_keys = Vec::new();

        let mut static_writable_signers = Vec::new();
        let mut static_readonly_signers = Vec::new();
        let mut static_writable_non_signers = Vec::new();
        let mut static_readonly_non_signers = Vec::new();

        static_writable_signers.push(payer);

        for (pubkey, (is_signer, is_writable)) in account_map {
            if pubkey == payer {
                continue;
            }

            if is_signer {
                if is_writable {
                    static_writable_signers.push(pubkey);
                } else {
                    static_readonly_signers.push(pubkey);
                }
                continue;
            }

            // Check if pubkey exists in any lookup table
            let mut found_in_table = false;
            for (t_idx, table) in lookup_tables.iter().enumerate() {
                if let Some(pos) = table.addresses.iter().position(|&a| a == pubkey) {
                    if pos > 255 {
                        continue;
                    }
                    found_in_table = true;
                    if is_writable {
                        table_entries[t_idx].writable_indices.push(pos as u8);
                        loaded_writable_keys.push(pubkey);
                    } else {
                        table_entries[t_idx].readonly_indices.push(pos as u8);
                        loaded_readonly_keys.push(pubkey);
                    }
                    break;
                }
            }

            if !found_in_table {
                if is_writable {
                    static_writable_non_signers.push(pubkey);
                } else {
                    static_readonly_non_signers.push(pubkey);
                }
            }
        }

        let num_required_signatures =
            (static_writable_signers.len() + static_readonly_signers.len()) as u8;
        let num_readonly_signed_accounts = static_readonly_signers.len() as u8;
        let num_readonly_unsigned_accounts = static_readonly_non_signers.len() as u8;

        let mut static_account_keys = Vec::new();
        static_account_keys.extend(static_writable_signers);
        static_account_keys.extend(static_readonly_signers);
        static_account_keys.extend(static_writable_non_signers);
        static_account_keys.extend(static_readonly_non_signers);

        let mut key_to_index: HashMap<Pubkey, u8> = HashMap::new();
        for (i, pk) in static_account_keys.iter().enumerate() {
            key_to_index.insert(*pk, i as u8);
        }
        let static_len = static_account_keys.len();
        for (i, pk) in loaded_writable_keys.iter().enumerate() {
            key_to_index.insert(*pk, (static_len + i) as u8);
        }
        let writable_offset = static_len + loaded_writable_keys.len();
        for (i, pk) in loaded_readonly_keys.iter().enumerate() {
            key_to_index.insert(*pk, (writable_offset + i) as u8);
        }

        let mut message = vec![
            0x80, // Versioned Message prefix: 0x80 (version 0)
            num_required_signatures,
            num_readonly_signed_accounts,
            num_readonly_unsigned_accounts,
        ];

        // Static account keys
        Self::encode_compact_u16(static_account_keys.len() as u16, &mut message);
        for pk in &static_account_keys {
            message.extend_from_slice(pk.as_ref());
        }

        // Recent blockhash
        message.extend_from_slice(&recent_blockhash);

        // Instructions
        Self::encode_compact_u16(instructions.len() as u16, &mut message);
        for ix in instructions {
            let prog_idx = *key_to_index
                .get(&ix.program_id)
                .ok_or_else(|| anyhow::anyhow!("Program ID {} not in key map", ix.program_id))?;
            message.push(prog_idx);

            Self::encode_compact_u16(ix.accounts.len() as u16, &mut message);
            for acc in &ix.accounts {
                let acc_idx = *key_to_index
                    .get(&acc.pubkey)
                    .ok_or_else(|| anyhow::anyhow!("Account {} not in key map", acc.pubkey))?;
                message.push(acc_idx);
            }

            Self::encode_compact_u16(ix.data.len() as u16, &mut message);
            message.extend_from_slice(&ix.data);
        }

        // Address table lookups
        let active_table_entries: Vec<&TableLookupEntry> = table_entries
            .iter()
            .filter(|t| !t.writable_indices.is_empty() || !t.readonly_indices.is_empty())
            .collect();

        Self::encode_compact_u16(active_table_entries.len() as u16, &mut message);
        for t in active_table_entries {
            message.extend_from_slice(t.table_key.as_ref());
            Self::encode_compact_u16(t.writable_indices.len() as u16, &mut message);
            message.extend_from_slice(&t.writable_indices);
            Self::encode_compact_u16(t.readonly_indices.len() as u16, &mut message);
            message.extend_from_slice(&t.readonly_indices);
        }

        Ok(message)
    }

    /// Construct and sign a Solana v0 Versioned Transaction with Address Lookup Tables.
    pub fn build_signed_v0_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        lookup_tables: &[AddressLookupTable],
    ) -> Result<Vec<u8>> {
        let message = Self::compile_v0_message(
            signer.pubkey(),
            instructions,
            recent_blockhash,
            lookup_tables,
        )?;
        let signature = signer.sign(&message);

        let mut tx = Vec::with_capacity(1 + 64 + message.len());
        Self::encode_compact_u16(1, &mut tx); // 1 signature
        tx.extend_from_slice(&signature);
        tx.extend_from_slice(&message);
        Ok(tx)
    }

    /// Create a standard Address Lookup Table containing well-known static program, pool,
    /// vault, and sysvar pubkeys for Raydium, Orca, Kamino, and SPL Token programs on Solana Mainnet.
    pub fn create_default_mainnet_alt() -> AddressLookupTable {
        let alt_key = Pubkey::from_str("ALT1111111111111111111111111111111111111111").unwrap();
        let addresses = vec![
            Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap(),
            Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
            Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap(),
            Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
            Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap(),
            Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap(),
            Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap(),
            Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap(),
            Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap(),
            Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap(),
            Pubkey::from_str(RAYDIUM_SOL_USDC_POOL).unwrap(),
            Pubkey::from_str(RAYDIUM_SOL_VAULT).unwrap(),
            Pubkey::from_str(RAYDIUM_USDC_VAULT).unwrap(),
            Pubkey::from_str("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1").unwrap(), // Raydium Authority
            Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX").unwrap(), // OpenBook / Serum Program
            Pubkey::from_str("8BnEgHoWFysVcuFFX7QBpDmvu3xei5qGE9KitUfqWQH9").unwrap(), // SOL-USDC Serum Market
            Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap(),
            Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap(),
            Pubkey::from_str(ORCA_SOL_VAULT).unwrap(),
            Pubkey::from_str(ORCA_USDC_VAULT).unwrap(),
            Pubkey::from_str(WSOL_MINT).unwrap(),
            Pubkey::from_str(USDC_MINT).unwrap(),
            Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap(),
        ];
        AddressLookupTable::new(alt_key, addresses)
    }

    /// Sign a pre-compiled versioned (v0) transaction (e.g. from Jupiter v6 API).
    pub fn sign_versioned_transaction(
        signer: &KeypairSigner,
        mut tx_bytes: Vec<u8>,
    ) -> Result<Vec<u8>> {
        if tx_bytes.len() < 66 {
            bail!("Transaction bytes too short for versioned transaction");
        }
        let msg_bytes = &tx_bytes[65..];
        let signature = signer.sign(msg_bytes);
        tx_bytes[1..65].copy_from_slice(&signature);
        Ok(tx_bytes)
    }

    /// Encode instructions into a serialized byte representation for simulation or broadcast.
    pub fn serialize_instructions(instructions: &[CompiledInstruction]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(instructions.len() as u16).to_le_bytes());
        for ix in instructions {
            bytes.extend_from_slice(ix.program_id.as_ref());
            bytes.extend_from_slice(&(ix.accounts.len() as u16).to_le_bytes());
            for acc in &ix.accounts {
                bytes.extend_from_slice(acc.pubkey.as_ref());
                bytes.push(acc.is_signer as u8);
                bytes.push(acc.is_writable as u8);
            }
            bytes.extend_from_slice(&(ix.data.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&ix.data);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{Bps, DexProtocol, MintId, PoolId, RouteId, Slot, SwapHop, TokenAmount};
    use ed25519_dalek::Verifier;

    #[test]
    fn cu_model_is_protocol_aware_and_route_sized() {
        // CLMM/DLMM cost more than a CPMM swap.
        assert!(per_hop_cu(DexProtocol::OrcaWhirlpool) > per_hop_cu(DexProtocol::RaydiumV4));
        assert!(per_hop_cu(DexProtocol::MeteoraDlmm) > per_hop_cu(DexProtocol::OrcaWhirlpool));
        // A 2-hop flash route with one ephemeral ATA is sized above the flat guess.
        let cu = estimate_route_cu(true, &[DexProtocol::RaydiumV4, DexProtocol::MeteoraDlmm], 1);
        assert!(cu >= 28_000 + 35_000 + 120_000);
        // The dynamic limit adds the 15% buffer and respects the ceiling.
        let lim = calculate_dynamic_cu_limit(cu);
        assert_eq!(lim, cu + cu * 15 / 100);
        assert!(calculate_dynamic_cu_limit(u32::MAX) <= MAX_COMPUTE_UNIT_LIMIT);
    }

    #[test]
    fn priority_fee_is_capped_by_profit_share() {
        // 1 SOL profit, 200k CU limit, 20% cap => budget 200_000_000 lamports.
        // max cu_price = 200_000_000 * 1e6 / 200_000 = 1_000_000_000 µlamports.
        assert_eq!(
            cap_cu_price_by_profit(1_000_000_000, 200_000, 5_000_000_000, 2_000),
            1_000_000_000
        );
        // A modest desired price below the cap is untouched.
        assert_eq!(
            cap_cu_price_by_profit(1_000_000_000, 200_000, 500, 2_000),
            500
        );
        // Zero profit / zero CU => zero price.
        assert_eq!(cap_cu_price_by_profit(0, 200_000, 5_000_000_000, 2_000), 0);
        assert_eq!(
            cap_cu_price_by_profit(1_000_000, 0, 5_000_000_000, 2_000),
            0
        );
    }

    #[test]
    fn compute_budget_instructions() {
        let ix1 = build_compute_unit_limit(180_000);
        assert_eq!(ix1.data[0], 2);
        assert_eq!(&ix1.data[1..5], &180_000u32.to_le_bytes());

        // Test dynamic CU limit with 15% buffer
        let dynamic_cu = calculate_dynamic_cu_limit(100_000);
        assert_eq!(dynamic_cu, 115_000); // 100k + 15%

        let capped_cu = calculate_dynamic_cu_limit(400_000);
        assert_eq!(capped_cu, MAX_COMPUTE_UNIT_LIMIT); // Capped at MAX_COMPUTE_UNIT_LIMIT (350k)

        // Verify 260k and 320k constants and helpers
        assert_eq!(COMPUTE_BUDGET_LIMIT_2HOP, 260_000);
        assert_eq!(COMPUTE_BUDGET_LIMIT_3HOP, 320_000);
        assert_eq!(get_cu_limit_for_hops(2), 260_000);
        assert_eq!(get_cu_limit_for_hops(3), 320_000);
    }

    #[test]
    fn tip_instruction_builder() {
        let payer = Pubkey::new_from_array([1; 32]);
        let tip_acc = Pubkey::new_from_array([2; 32]);
        let ix = build_jito_tip_instruction(payer, tip_acc, Lamports(50_000));
        assert_eq!(ix.accounts.len(), 2);
        assert!(ix.accounts[0].is_signer);
    }

    #[test]
    fn kamino_flash_assembly() {
        let payer = Pubkey::new_from_array([1; 32]);
        let tip_acc = Pubkey::new_from_array([2; 32]);
        let params = KaminoFlashParams::default();

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        // 1. Route between permanent base assets (WSOL <-> USDC)
        let opp_base = ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(50_000_000_000), // 50 SOL
            expected_out: TokenAmount(50_500_000_000),
            hop_inputs: vec![50_000_000_000, 50_250_000_000, 50_500_000_000],
            gross_profit: Lamports(500_000_000),
            net_profit: Lamports(450_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId::new([10; 32]),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(usdc),
                    fee_bps: 25,
                    active_bin_id: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                    protocol: DexProtocol::OrcaWhirlpool,
                    mint_in: MintId(usdc),
                    mint_out: MintId(wsol),
                    fee_bps: 5,
                    active_bin_id: None,
                },
            ],
        };

        let ixs_base = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_acc,
            Lamports(10_000),
            160_000,
            1000,
            &opp_base,
            &params,
            0,                   // verified sf: 0 in this fixture
            TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
        )
        .unwrap();
        // Base route: CU limit + CU price + WSOL ATA Create + Kamino Borrow + 2 Swaps + Kamino Repay + Jito Tip = 8 instructions
        assert_eq!(ixs_base.len(), 8);
        assert_eq!(&ixs_base[0].data[1..5], &184_000u32.to_le_bytes()); // 160k + 15% buffer = 184k
        assert_eq!(ixs_base[3].accounts[0].pubkey, payer); // Borrow user authority (signer)
        assert_eq!(ixs_base[3].accounts[2].pubkey, params.lending_market); // Borrow lending market
        assert_eq!(ixs_base[6].accounts[0].pubkey, payer); // Repay user authority (signer)
        assert_eq!(ixs_base[6].accounts[2].pubkey, params.lending_market); // Repay lending market

        // 2. Route with intermediate trending token (Ephemeral ATA Create -> Swaps -> Close & Rent Reclaim)
        let intermediate_token = Pubkey::new_from_array([88; 32]);
        let opp_intermediate = ExecutionOpportunity {
            route_id: RouteId(2),
            slot: Slot(100),
            size_in: TokenAmount(50_000_000_000),
            expected_out: TokenAmount(50_500_000_000),
            hop_inputs: vec![50_000_000_000, 50_250_000_000, 50_500_000_000],
            gross_profit: Lamports(500_000_000),
            net_profit: Lamports(450_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId::new([10; 32]),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(intermediate_token),
                    fee_bps: 25,
                    active_bin_id: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                    protocol: DexProtocol::OrcaWhirlpool,
                    mint_in: MintId(intermediate_token),
                    mint_out: MintId(wsol),
                    fee_bps: 5,
                    active_bin_id: None,
                },
            ],
        };

        let ixs_inter = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_acc,
            Lamports(10_000),
            160_000,
            1000,
            &opp_intermediate,
            &params,
            0,                   // verified sf: 0 in this fixture
            TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
        )
        .unwrap();
        // Base (8) + 1 Ephemeral ATA Create + 1 Ephemeral ATA Close = 10 instructions
        assert_eq!(ixs_inter.len(), 10);
        assert_eq!(
            ixs_inter[2].program_id,
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        ); // CreateIdempotent (WSOL)
        assert_eq!(
            ixs_inter[3].program_id,
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        ); // CreateIdempotent (Intermediate)
        assert_eq!(
            ixs_inter[8].program_id,
            Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
        ); // CloseAccount (Rent Reclaimed to Payer)
        assert_eq!(ixs_inter[8].data, vec![9]); // CloseAccount opcode
    }

    #[test]
    fn keypair_signer_decoding_and_cryptographic_verification() {
        let seed = [9u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let mut keypair_bytes = [0u8; 64];
        keypair_bytes[..32].copy_from_slice(&seed);
        keypair_bytes[32..].copy_from_slice(&pubkey_bytes);

        let b58_str = bs58::encode(&keypair_bytes).into_string();
        let signer = KeypairSigner::from_base58(&b58_str).unwrap();
        assert_eq!(signer.pubkey(), Pubkey::new_from_array(pubkey_bytes));

        let message = b"solana_mev_private_bundle_payload";
        let sig_bytes = signer.sign(message);
        assert_eq!(sig_bytes.len(), 64);

        // Genuine cryptographic Ed25519 verification
        let verifying_key = signer.verifying_key();
        let parsed_sig = Signature::from_bytes(&sig_bytes);
        assert!(verifying_key.verify(message, &parsed_sig).is_ok());
    }

    #[test]
    fn test_build_signed_transaction_wire_format() {
        let seed = [7u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let mut keypair_bytes = [0u8; 64];
        keypair_bytes[..32].copy_from_slice(&seed);
        keypair_bytes[32..].copy_from_slice(&pubkey_bytes);

        let signer = KeypairSigner::from_bytes(&keypair_bytes).unwrap();
        let payer = signer.pubkey();
        let tip_acc = Pubkey::new_from_array([3; 32]);

        let cu_min = estimate_minimal_cu(true, 2);
        assert!(cu_min > 90_000 && cu_min < 120_000);
        let cu_limit = calculate_dynamic_cu_limit(cu_min);
        assert!(cu_limit <= 200_000);

        let ix1 = build_compute_unit_limit(cu_limit);
        let ix2 = build_jito_tip_instruction(payer, tip_acc, Lamports(10_000));
        let blockhash = [42u8; 32];

        let tx_bytes =
            TxBuilder::build_signed_transaction(&signer, &[ix1, ix2], blockhash).unwrap();
        assert!(!tx_bytes.is_empty());
        assert_eq!(tx_bytes[0], 1); // 1 signature

        let sig_slice = &tx_bytes[1..65];
        let message_slice = &tx_bytes[65..];

        // Verify cryptographic signature matches message
        let parsed_sig = Signature::from_bytes(sig_slice.try_into().unwrap());
        assert!(
            signer
                .verifying_key()
                .verify(message_slice, &parsed_sig)
                .is_ok()
        );
    }

    #[test]
    fn test_raydium_cpmm_swap_layout_and_accounts() {
        let user = Pubkey::new_unique();
        let user_in = Pubkey::new_unique();
        let user_out = Pubkey::new_unique();
        let accounts = RaydiumCpmmAccounts::mainnet_sol_usdc(user, user_in, user_out);

        assert_eq!(accounts.payer, user);
        assert_eq!(
            accounts.pool_state,
            Pubkey::from_str(RAYDIUM_CPMM_SOL_USDC_POOL).unwrap()
        );
        assert_eq!(
            accounts.input_vault,
            Pubkey::from_str(RAYDIUM_CPMM_SOL_VAULT).unwrap()
        );
        assert_eq!(
            accounts.output_vault,
            Pubkey::from_str(RAYDIUM_CPMM_USDC_VAULT).unwrap()
        );

        let amount_in = 1_000_000_000u64;
        let min_amount_out = 150_000_000u64;
        let ix = build_raydium_cpmm_swap_instruction(&accounts, amount_in, min_amount_out).unwrap();

        assert_eq!(
            ix.program_id,
            Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap()
        );
        assert_eq!(ix.accounts.len(), 13);

        // Account 0: payer (signer, writable)
        assert_eq!(ix.accounts[0].pubkey, user);
        assert!(ix.accounts[0].is_signer);
        assert!(ix.accounts[0].is_writable);

        // Account 1: authority (readonly)
        assert_eq!(ix.accounts[1].pubkey, derive_raydium_cpmm_authority());
        assert!(!ix.accounts[1].is_signer);
        assert!(!ix.accounts[1].is_writable);

        // Account 3: pool_state (writable)
        assert_eq!(ix.accounts[3].pubkey, accounts.pool_state);
        assert!(ix.accounts[3].is_writable);

        // Account 4 & 5: token accounts
        assert_eq!(ix.accounts[4].pubkey, user_in);
        assert_eq!(ix.accounts[5].pubkey, user_out);

        // Account 6 & 7: vaults
        assert_eq!(ix.accounts[6].pubkey, accounts.input_vault);
        assert_eq!(ix.accounts[7].pubkey, accounts.output_vault);

        // Account 12: observation_state (writable)
        assert_eq!(ix.accounts[12].pubkey, accounts.observation_state);
        assert!(ix.accounts[12].is_writable);

        // Data payload
        assert_eq!(ix.data.len(), 24);
        assert_eq!(&ix.data[0..8], &RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR);
        assert_eq!(&ix.data[8..16], &amount_in.to_le_bytes());
        assert_eq!(&ix.data[16..24], &min_amount_out.to_le_bytes());
    }

    #[test]
    fn test_raydium_clmm_swap_layout_and_accounts() {
        let pool = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let user_in = Pubkey::new_unique();
        let user_out = Pubkey::new_unique();
        let vault_in = Pubkey::new_unique();
        let vault_out = Pubkey::new_unique();
        let amm_config = Pubkey::new_unique();

        let accounts = RaydiumClmmAccounts::for_pool(
            pool, payer, user_in, user_out, vault_in, vault_out, amm_config, 60, 1200, true,
        );

        assert_eq!(accounts.payer, payer);
        assert_eq!(accounts.pool_state, pool);
        assert_ne!(accounts.tick_array_0, accounts.tick_array_1);
        assert_ne!(accounts.tick_array_1, accounts.tick_array_2);

        let amount_in = 500_000_000u64;
        let other_threshold = 40_000_000u64;
        let sqrt_price_limit = 1_000_000_000u128;
        let ix = build_raydium_clmm_swap_instruction(
            &accounts,
            amount_in,
            other_threshold,
            sqrt_price_limit,
            true,
        )
        .unwrap();

        assert_eq!(
            ix.program_id,
            Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap()
        );
        assert_eq!(ix.accounts.len(), 12);

        // Account 0: payer
        assert_eq!(ix.accounts[0].pubkey, payer);
        assert!(ix.accounts[0].is_signer);

        // Account 2: pool_state
        assert_eq!(ix.accounts[2].pubkey, pool);
        assert!(ix.accounts[2].is_writable);

        // Tick arrays: accounts 9, 10, 11
        assert_eq!(ix.accounts[9].pubkey, accounts.tick_array_0);
        assert_eq!(ix.accounts[10].pubkey, accounts.tick_array_1);
        assert_eq!(ix.accounts[11].pubkey, accounts.tick_array_2);
        assert!(ix.accounts[9].is_writable);
        assert!(ix.accounts[10].is_writable);
        assert!(ix.accounts[11].is_writable);

        // Data payload
        assert_eq!(ix.data.len(), 41);
        assert_eq!(&ix.data[0..8], &RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
        assert_eq!(&ix.data[8..16], &amount_in.to_le_bytes());
        assert_eq!(&ix.data[16..24], &other_threshold.to_le_bytes());
        assert_eq!(&ix.data[24..40], &sqrt_price_limit.to_le_bytes());
        assert_eq!(ix.data[40], 1); // is_base_input = true
    }

    #[test]
    fn test_pump_fun_buy_and_sell_layout_and_accounts() {
        let mint = Pubkey::new_unique();
        let user = Pubkey::new_unique();
        let accounts = PumpFunAccounts::for_mint(mint, user);

        assert_eq!(accounts.mint, mint);
        assert_eq!(accounts.user, user);
        assert_eq!(accounts.global, derive_pump_fun_global_pda());
        assert_eq!(
            accounts.bonding_curve,
            derive_pump_fun_bonding_curve_pda(&mint)
        );
        assert_eq!(accounts.event_authority, derive_pump_fun_event_authority());

        // Test BUY
        let sol_amount_in = 2_000_000_000u64; // max SOL to spend
        let min_tokens_out = 50_000_000_000u64; // tokens to receive
        let buy_ix =
            build_pump_fun_swap_instruction(&accounts, sol_amount_in, min_tokens_out, true)
                .unwrap();

        assert_eq!(
            buy_ix.program_id,
            Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap()
        );
        assert_eq!(buy_ix.accounts.len(), 12);
        assert_eq!(buy_ix.accounts[0].pubkey, accounts.global);
        assert_eq!(buy_ix.accounts[1].pubkey, accounts.fee_recipient);
        assert!(buy_ix.accounts[1].is_writable);
        assert_eq!(buy_ix.accounts[2].pubkey, mint);
        assert_eq!(buy_ix.accounts[3].pubkey, accounts.bonding_curve);
        assert!(buy_ix.accounts[3].is_writable);
        assert_eq!(buy_ix.accounts[4].pubkey, accounts.associated_bonding_curve);
        assert!(buy_ix.accounts[4].is_writable);
        assert_eq!(buy_ix.accounts[5].pubkey, accounts.associated_user);
        assert!(buy_ix.accounts[5].is_writable);
        assert_eq!(buy_ix.accounts[6].pubkey, user);
        assert!(buy_ix.accounts[6].is_signer);
        assert!(buy_ix.accounts[6].is_writable);
        assert_eq!(buy_ix.accounts[10].pubkey, accounts.event_authority);

        // Buy data layout: [BUY_DISC(8), min_tokens_out(8), sol_amount_in(8)]
        assert_eq!(buy_ix.data.len(), 24);
        assert_eq!(&buy_ix.data[0..8], &PUMP_FUN_BUY_DISCRIMINATOR);
        assert_eq!(&buy_ix.data[8..16], &min_tokens_out.to_le_bytes());
        assert_eq!(&buy_ix.data[16..24], &sol_amount_in.to_le_bytes());

        // Test SELL
        let tokens_in = 50_000_000_000u64;
        let min_sol_out = 1_800_000_000u64;
        let sell_ix =
            build_pump_fun_swap_instruction(&accounts, tokens_in, min_sol_out, false).unwrap();

        assert_eq!(sell_ix.data.len(), 24);
        assert_eq!(&sell_ix.data[0..8], &PUMP_FUN_SELL_DISCRIMINATOR);
        assert_eq!(&sell_ix.data[8..16], &tokens_in.to_le_bytes());
        assert_eq!(&sell_ix.data[16..24], &min_sol_out.to_le_bytes());
    }

    #[test]
    fn test_meteora_multi_bin_array_expansion() {
        let accounts = MeteoraDlmmAccounts::mainnet_sol_usdc();
        let user = Pubkey::new_unique();
        let user_in = Pubkey::new_unique();
        let user_out = Pubkey::new_unique();

        // 1. Moderate swap with bin_step 4: produces 3 contiguous bin arrays (since bin_step <= 5)
        let ix_normal = build_meteora_dlmm_swap_instruction(
            &accounts, user, user_in, user_out, 1_000_000, 990_000, 0, 4, true,
        );
        // 16 base accounts + 3 bin arrays = 19 accounts
        assert_eq!(ix_normal.accounts.len(), 19);

        // 2. Large price swing: 1_000_000 -> 100_000 (9000 bps price difference / 4 = 2250 bins > 140)
        // Must expand to 4 contiguous bin arrays!
        let ix_large_swing = build_meteora_dlmm_swap_instruction(
            &accounts, user, user_in, user_out, 1_000_000, 100_000, 0, 4, true,
        );
        // 16 base accounts + 4 bin arrays = 20 accounts
        assert_eq!(ix_large_swing.accounts.len(), 20);

        // Verify that all 4 bin arrays are distinct and contiguous
        let ba0 = ix_large_swing.accounts[16].pubkey;
        let ba1 = ix_large_swing.accounts[17].pubkey;
        let ba2 = ix_large_swing.accounts[18].pubkey;
        let ba3 = ix_large_swing.accounts[19].pubkey;
        assert_ne!(ba0, ba1);
        assert_ne!(ba1, ba2);
        assert_ne!(ba2, ba3);
    }

    #[test]
    fn test_build_swap_hop_instruction_for_all_protocols() {
        let user = Pubkey::new_unique();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let meme_token = Pubkey::new_unique();

        let protocols = [
            (DexProtocol::RaydiumV4, wsol, usdc),
            (DexProtocol::OrcaWhirlpool, wsol, usdc),
            (DexProtocol::MeteoraDlmm, wsol, usdc),
            (DexProtocol::RaydiumCpmm, wsol, usdc),
            (DexProtocol::RaydiumClmm, wsol, usdc),
            (DexProtocol::PumpFun, wsol, meme_token),
        ];

        for (proto, mint_in, mint_out) in protocols {
            let hop = SwapHop {
                pool_id: arb_types::PoolId(Pubkey::new_unique()),
                protocol: proto,
                mint_in: arb_types::MintId(mint_in),
                mint_out: arb_types::MintId(mint_out),
                fee_bps: 25,
                active_bin_id: Some(0),
            };

            let res = build_swap_hop_instruction(&hop, user, 1_000_000_000, 100_000_000);
            assert!(
                res.is_ok(),
                "build_swap_hop_instruction failed for {:?}: {:?}",
                proto,
                res.err()
            );
            let ix = res.unwrap();
            assert!(!ix.accounts.is_empty());
            assert!(!ix.data.is_empty());
        }
    }

    #[test]
    fn test_final_hop_onchain_balance_assertion_guarantee() {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        let opp = ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000), // 1 SOL borrow
            expected_out: TokenAmount(1_050_000_000), // 1.05 SOL expected
            hop_inputs: vec![1_000_000_000, 200_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(30_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(usdc),
                    fee_bps: 25,
                    active_bin_id: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::OrcaWhirlpool,
                    mint_in: MintId(usdc),
                    mint_out: MintId(wsol),
                    fee_bps: 30,
                    active_bin_id: None,
                },
            ],
        };

        let flash_fee = 1_000_000; // 0.001 SOL
        let jito_tip = 7_000_000; // 0.007 SOL (70% profit share tip)
        let min_profit = 10_000_000; // 0.01 SOL
        let slippage_bps = 50; // 0.5%

        let legs = TxBuilder::derive_swap_legs(&opp, flash_fee, jito_tip, min_profit, slippage_bps)
            .unwrap();
        assert_eq!(legs.len(), 2);

        // Required floor = borrow + flash_fee + jito_tip + min_profit = 1_018_000_000
        let floor = 1_000_000_000 + flash_fee + jito_tip + min_profit;
        let final_min_out = legs[1].1;
        assert!(
            final_min_out >= floor,
            "Final hop min_out {final_min_out} must enforce profit floor {floor}"
        );

        // If expected_out cannot meet the floor, derive_swap_legs must reject pre-flight
        let failing_opp = ExecutionOpportunity {
            expected_out: TokenAmount(1_015_000_000), // below floor of 1_018_000_000
            hop_inputs: vec![1_000_000_000, 200_000_000, 1_015_000_000],
            ..opp
        };
        let err = TxBuilder::derive_swap_legs(
            &failing_opp,
            flash_fee,
            jito_tip,
            min_profit,
            slippage_bps,
        );
        assert!(matches!(err, Err(TxError::ProfitFloorNotMet { .. })));
    }

    #[test]
    fn test_pump_fun_flash_arbitrage_assembly_with_wsol_bridge() {
        let payer = Pubkey::new_unique();
        let tip_account = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let meme = Pubkey::new_unique();
        let kamino_params = KaminoFlashParams::default();

        // 1. Route: Pump.fun Buy -> Raydium Sell
        let opp1 = ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000), // 1 SOL borrow
            expected_out: TokenAmount(1_050_000_000),
            hop_inputs: vec![1_000_000_000, 35_000_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(30_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::PumpFun,
                    mint_in: MintId(wsol),
                    mint_out: MintId(meme),
                    fee_bps: 100,
                    active_bin_id: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(meme),
                    mint_out: MintId(wsol),
                    fee_bps: 25,
                    active_bin_id: None,
                },
            ],
        };

        let ixs1 = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_account,
            Lamports(1_000_000),
            300_000,
            1_000,
            &opp1,
            &kamino_params,
            6250,
            TokenAmount(1_000_000),
        )
        .expect("assemble_flash_arbitrage should succeed for Pump.fun -> Raydium route");

        let pump_program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap();
        let klend_program = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
        assert!(ixs1.iter().any(|ix| ix.program_id == pump_program));
        assert!(ixs1.iter().any(|ix| ix.program_id == klend_program));

        // 2. Route: Raydium Buy -> Pump.fun Sell
        let opp2 = ExecutionOpportunity {
            route_id: RouteId(2),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000),
            expected_out: TokenAmount(1_050_000_000),
            hop_inputs: vec![1_000_000_000, 35_000_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(30_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(meme),
                    fee_bps: 25,
                    active_bin_id: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::PumpFun,
                    mint_in: MintId(meme),
                    mint_out: MintId(wsol),
                    fee_bps: 100,
                    active_bin_id: None,
                },
            ],
        };

        let ixs2 = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_account,
            Lamports(1_000_000),
            300_000,
            1_000,
            &opp2,
            &kamino_params,
            6250,
            TokenAmount(1_000_000),
        )
        .expect("assemble_flash_arbitrage should succeed for Raydium -> Pump.fun route");

        assert!(ixs2.iter().any(|ix| ix.program_id == pump_program));
        assert!(ixs2.iter().any(|ix| ix.program_id == klend_program));
    }
}
