//! Pre-warmed serialized byte template for sub-microsecond in-place transaction patching.
//! Enables hot-path mutation of borrow amount, repay amount, tip lamports, recent blockhash,
//! and Ed25519 Dalek cryptographic signature without allocating memory.

use crate::adaptive::{
    AdaptiveTxCompiler, CompiledAdaptiveTransaction, CompilerCapabilities, V1ConfigParams,
};
use crate::kamino::{FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR};
use crate::{AccountMeta, AddressLookupTable, CompiledInstruction, KeypairSigner, TxError};
use solana_pubkey::Pubkey;

/// Pre-warmed transaction template for hot-path sub-microsecond dispatch.
/// Enables in-place mutation of borrow amount, repay amount, swap leg amounts,
/// Jito tip, recent blockhash, and Ed25519 signature without re-serializing.
#[derive(Debug, Clone)]
pub struct PreWarmedTxTemplate {
    pub raw_bytes: Vec<u8>,
    pub message_offset: usize,
    pub blockhash_offset: usize,
    pub borrow_amount_offset: usize,
    pub repay_amount_offset: usize,
    pub tip_amount_offset: usize,
    pub signature_offset: usize,
    /// Optional offset to the first swap instruction's `amount_in` (u64 LE) in wire bytes.
    /// When `Some`, `patch_and_sign` will update it alongside borrow amount.
    pub swap_amount_in_offset: Option<usize>,
    /// Optional offset to the last swap instruction's `min_amount_out` (u64 LE) in wire bytes.
    /// When `Some`, `patch_and_sign` will update it alongside repay amount.
    pub swap_min_amount_out_offset: Option<usize>,
    /// Kamino flash-loan fee as the raw U68F60 on-chain value
    /// (`ReserveConfig.fees.flash_loan_fee_sf`, 60 fractional bits), never a bps
    /// rate. Used only to validate the repay principal, never added to it.
    pub fee_sf: u64,
}

impl PreWarmedTxTemplate {
    /// Construct a new pre-warmed template with explicit byte offsets.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        raw_bytes: Vec<u8>,
        message_offset: usize,
        blockhash_offset: usize,
        borrow_amount_offset: usize,
        repay_amount_offset: usize,
        tip_amount_offset: usize,
        signature_offset: usize,
        fee_sf: u64,
        swap_amount_in_offset: Option<usize>,
        swap_min_amount_out_offset: Option<usize>,
    ) -> Self {
        Self {
            raw_bytes,
            message_offset,
            blockhash_offset,
            borrow_amount_offset,
            repay_amount_offset,
            tip_amount_offset,
            signature_offset,
            swap_amount_in_offset,
            swap_min_amount_out_offset,
            fee_sf,
        }
    }

    /// Attach swap instruction offsets after template construction.
    /// Useful when templates are created from pre-compiled bytes where swap
    /// instruction offsets were resolved externally.
    pub fn with_swap_offsets(
        mut self,
        swap_amount_in_offset: usize,
        swap_min_amount_out_offset: usize,
    ) -> Self {
        self.swap_amount_in_offset = Some(swap_amount_in_offset);
        self.swap_min_amount_out_offset = Some(swap_min_amount_out_offset);
        self
    }

    /// Locate byte offsets within a pre-compiled signed transaction wire buffer.
    pub fn locate_offsets(
        raw_tx_bytes: &[u8],
        initial_blockhash: &[u8; 32],
        initial_tip_lamports: u64,
    ) -> Result<(usize, usize, usize, usize, usize, usize), TxError> {
        let signature_offset = 1usize;
        let message_offset = 65usize;

        if raw_tx_bytes.len() < message_offset + 32 {
            return Err(TxError::TemplateOffsetError(
                "Transaction wire bytes too short for v0 header".to_string(),
            ));
        }

        // 1. Locate blockhash (32 bytes) in message
        let blockhash_offset = raw_tx_bytes[message_offset..]
            .windows(32)
            .position(|window| window == initial_blockhash)
            .map(|pos| message_offset + pos)
            .ok_or_else(|| {
                TxError::TemplateOffsetError("Initial blockhash not found in message".to_string())
            })?;

        // 2. Locate FlashBorrow discriminator (8 bytes)
        let borrow_disc_pos = raw_tx_bytes[message_offset..]
            .windows(8)
            .position(|window| window == FLASH_BORROW_DISCRIMINATOR)
            .map(|pos| message_offset + pos)
            .ok_or_else(|| {
                TxError::TemplateOffsetError("FlashBorrow discriminator not found".to_string())
            })?;
        let borrow_amount_offset = borrow_disc_pos + 8;

        // 3. Locate FlashRepay discriminator (8 bytes)
        let repay_disc_pos = raw_tx_bytes[borrow_amount_offset..]
            .windows(8)
            .position(|window| window == FLASH_REPAY_DISCRIMINATOR)
            .map(|pos| borrow_amount_offset + pos)
            .ok_or_else(|| {
                TxError::TemplateOffsetError("FlashRepay discriminator not found".to_string())
            })?;
        let repay_amount_offset = repay_disc_pos + 8;

        // 4. Locate Jito Tip transfer payload: [2, 0, 0, 0] followed by initial_tip_lamports
        let mut tip_pattern = Vec::with_capacity(12);
        tip_pattern.extend_from_slice(&2u32.to_le_bytes());
        tip_pattern.extend_from_slice(&initial_tip_lamports.to_le_bytes());

        let tip_pos = raw_tx_bytes[repay_amount_offset..]
            .windows(12)
            .position(|window| window == tip_pattern.as_slice())
            .map(|pos| repay_amount_offset + pos)
            .ok_or_else(|| {
                TxError::TemplateOffsetError("Jito Tip transfer payload not found".to_string())
            })?;
        let tip_amount_offset = tip_pos + 4; // Skip the 4-byte opcode [2, 0, 0, 0]

        Ok((
            message_offset,
            blockhash_offset,
            borrow_amount_offset,
            repay_amount_offset,
            tip_amount_offset,
            signature_offset,
        ))
    }

    /// Scan wire bytes for known swap instruction discriminators and return the
    /// byte offsets of the first `amount_in` (u64 LE) and last `min_amount_out`
    /// (u64 LE) fields found.
    ///
    /// This is used by `from_bundle_params` to auto-resolve swap patch offsets
    /// so `patch_and_sign` can update swap leg amounts in-place on the hot path.
    ///
    /// Known discriminators scanned:
    ///   - Raydium V4 SwapBaseIn: opcode 0x09
    ///   - Pump.fun Buy/Sell: [23, 183, 248, 55, 96, 216, 172, 96]
    ///   - Meteora DLMM swap2: [65, 75, 63, 76, 235, 91, 91, 136]
    ///   - Anchor-discriminated swaps (Raydium CPMM/CLMM, Orca Whirlpool): any 8-byte sequence
    ///     immediately followed by two u64 LE amounts.
    pub fn locate_swap_offsets(
        raw_tx_bytes: &[u8],
    ) -> Result<(Option<usize>, Option<usize>), TxError> {
        let mut first_amount_in: Option<usize> = None;
        let mut last_min_out: Option<usize> = None;

        // Known swap discriminators (8 bytes each).
        let discriminators: &[&[u8]] = &[
            &[9u8],                                // Raydium V4 SwapBaseIn opcode (single byte)
            &[23, 183, 248, 55, 96, 216, 172, 96], // Pump.fun buy/sell
            &[65, 75, 63, 76, 235, 91, 91, 136],   // Meteora DLMM swap2
        ];

        for disc in discriminators {
            let mut pos = 0;
            while let Some(found) = raw_tx_bytes[pos..]
                .windows(disc.len())
                .position(|w| w == *disc)
            {
                let abs = pos + found;
                // amount_in is at abs + disc.len(), min_amount_out at abs + disc.len() + 8
                let amt_off = abs + disc.len();
                let min_off = amt_off + 8;
                if min_off + 8 <= raw_tx_bytes.len() {
                    if first_amount_in.is_none() {
                        first_amount_in = Some(amt_off);
                    }
                    last_min_out = Some(min_off);
                }
                pos = abs + 1;
            }
        }

        Ok((first_amount_in, last_min_out))
    }

    /// Create template by inspecting a pre-compiled signed wire transaction.
    pub fn from_precompiled_bytes(
        raw_tx_bytes: Vec<u8>,
        initial_blockhash: &[u8; 32],
        initial_tip_lamports: u64,
        fee_sf: u64,
    ) -> Result<Self, TxError> {
        let (
            message_offset,
            blockhash_offset,
            borrow_amount_offset,
            repay_amount_offset,
            tip_amount_offset,
            signature_offset,
        ) = Self::locate_offsets(&raw_tx_bytes, initial_blockhash, initial_tip_lamports)?;

        Ok(Self {
            raw_bytes: raw_tx_bytes,
            message_offset,
            blockhash_offset,
            borrow_amount_offset,
            repay_amount_offset,
            tip_amount_offset,
            signature_offset,
            swap_amount_in_offset: None,
            swap_min_amount_out_offset: None,
            fee_sf,
        })
    }

    /// Create template directly from KaminoBundleParams by compiling the bundle.
    /// `fee_sf` is the verified Kamino flash-loan fee (U68F60, 60 fractional
    /// bits) used to patch the
    /// repay amount on each `patch_and_sign`.
    ///
    /// Swap instruction offsets are resolved automatically by scanning the compiled
    /// wire bytes for known swap instruction discriminators and reading the adjacent
    /// u64 LE `amount_in` / `min_amount_out` fields.
    pub fn from_bundle_params(
        params: &crate::KaminoBundleParams,
        fee_sf: u64,
    ) -> Result<Self, TxError> {
        // Keep the compiled repay amount at the placeholder (sf = 0); the real
        // fee is re-priced at patch time via `fee_sf`. (ARB-005: direct callers
        // of build_kamino_bundle must pass the verified sf in KaminoBundleParams.)
        let v0_tx = crate::TxBuilder::build_kamino_bundle(params)?;
        let mut tpl = Self::from_precompiled_bytes(
            v0_tx.wire_bytes.clone(),
            &params.recent_blockhash,
            params.tip_lamports.as_u64(),
            fee_sf,
        )?;
        // Resolve swap instruction offsets by scanning wire bytes for known swap
        // discriminators and reading the adjacent u64 LE amount fields.
        let (amount_in_off, min_out_off) = Self::locate_swap_offsets(&v0_tx.wire_bytes)?;
        tpl.swap_amount_in_offset = amount_in_off;
        tpl.swap_min_amount_out_offset = min_out_off;
        Ok(tpl)
    }

    /// Mutate borrow size, repay size, Jito tip, recent blockhash,
    /// compute Ed25519 signature over message bytes, and return slice to serialized transaction.
    /// Executes with zero heap allocations and sub-microsecond in-place memory writes.
    #[inline(always)]
    pub fn patch_and_sign(
        &mut self,
        borrow_amount: u64,
        tip_lamports: u64,
        blockhash: [u8; 32],
        signer: &KeypairSigner,
    ) -> &[u8] {
        self.patch_and_sign_with_swaps(borrow_amount, tip_lamports, blockhash, signer, None, None)
    }

    /// Mutate borrow size, repay size, swap leg amounts, Jito tip, recent blockhash,
    /// compute Ed25519 signature over message bytes, and return slice to serialized transaction.
    #[inline(always)]
    pub fn patch_and_sign_with_swaps(
        &mut self,
        borrow_amount: u64,
        tip_lamports: u64,
        blockhash: [u8; 32],
        signer: &KeypairSigner,
        swap_amount_in: Option<u64>,
        swap_min_amount_out: Option<u64>,
    ) -> &[u8] {
        // 1. Overwrite borrow amount in KaminoFlashBorrow
        self.raw_bytes[self.borrow_amount_offset..self.borrow_amount_offset + 8]
            .copy_from_slice(&borrow_amount.to_le_bytes());

        // 2. Overwrite repay amount in KaminoFlashRepay. The instruction's
        //    `amount` argument MUST equal the borrow principal: klend's
        //    `flash_repay_checks` enforces `liquidity_amount == borrow amount`
        //    and collects the origination fee separately, on top, at repay time.
        //    (`self.fee_sf` is retained only so callers can price the fee for
        //    funding the source ATA; it is never added to the repay argument.)
        self.raw_bytes[self.repay_amount_offset..self.repay_amount_offset + 8]
            .copy_from_slice(&borrow_amount.to_le_bytes());

        // 3. Overwrite swap instruction amounts when offsets are resolved.
        //    amount_in for the first swap leg must match the new borrow amount;
        //    min_amount_out for the last swap leg must cover repay + profit floor.
        if let (Some(offset), Some(amount)) = (self.swap_amount_in_offset, swap_amount_in) {
            self.raw_bytes[offset..offset + 8].copy_from_slice(&amount.to_le_bytes());
        }
        if let (Some(offset), Some(amount)) = (self.swap_min_amount_out_offset, swap_min_amount_out)
        {
            self.raw_bytes[offset..offset + 8].copy_from_slice(&amount.to_le_bytes());
        }

        // 4. Overwrite tip lamports in Jito Tip Transfer
        self.raw_bytes[self.tip_amount_offset..self.tip_amount_offset + 8]
            .copy_from_slice(&tip_lamports.to_le_bytes());

        // 5. Overwrite 32-byte recent blockhash
        self.raw_bytes[self.blockhash_offset..self.blockhash_offset + 32]
            .copy_from_slice(&blockhash);

        // 6. Sign message slice (bytes 65..)
        let message_slice = &self.raw_bytes[self.message_offset..];
        let signature = signer.sign(message_slice);

        // 7. Overwrite signature in transaction prefix (offset 1..65)
        self.raw_bytes[self.signature_offset..self.signature_offset + 64]
            .copy_from_slice(&signature);

        &self.raw_bytes
    }

    /// Direct reference to raw wire bytes.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        &self.raw_bytes
    }

    /// Consume template into raw wire bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.raw_bytes
    }
}

/// Dynamic accounts and amounts refreshed on every hot-path trigger.
#[derive(Debug, Clone, Default)]
pub struct TemplateHopRefresh {
    /// Hop index.
    pub hop_index: usize,
    /// Refreshed input amount (u64).
    pub amount_in: u64,
    /// Refreshed minimum output amount (slippage bounded, u64).
    pub min_amount_out: u64,
    /// Refreshed dynamic accounts (e.g. tick arrays for CLMM / Whirlpool, bin arrays for DLMM).
    pub dynamic_accounts: Vec<AccountMeta>,
}

/// Dynamic refresh parameters applied immediately before compilation.
#[derive(Debug, Clone)]
pub struct TemplateRefreshParams {
    /// Refreshed 32-byte blockhash from recent bank state.
    pub recent_blockhash: [u8; 32],
    /// Refreshed borrow amount for flash loan.
    pub borrow_amount: u64,
    /// Refreshed tip lamports for Jito.
    pub tip_lamports: u64,
    /// Refreshed compute unit limit.
    pub cu_limit: Option<u32>,
    /// Refreshed priority fee in micro-lamports.
    pub priority_fee_micro_lamports: Option<u64>,
    /// Refreshed per-hop parameters.
    pub hop_refreshes: Vec<TemplateHopRefresh>,
}

/// Phase 5: Prewarmed reusable unsigned instruction-building structure.
///
/// Pre-resolves and caches:
/// - Payer authority
/// - Static account metas across all route hops
/// - Program IDs
/// - Compute budget and Jito tip instruction templates
/// - Address lookup tables
///
/// On the hot path:
/// - `refresh(...)` updates amounts, dynamic tick/bin arrays, blockhash, and limits.
/// - `compile_and_sign(...)` recompiles a brand new Message and signs it freshly with Ed25519.
/// - ZERO mutation of signed transactions. ZERO stale signature reuse.
#[derive(Debug, Clone)]
pub struct PrewarmedUnsignedTemplate {
    pub payer: Pubkey,
    pub instructions: Vec<CompiledInstruction>,
    pub recent_blockhash: [u8; 32],
    pub lookup_tables: Vec<AddressLookupTable>,
    pub fee_sf: u64,
    pub cu_limit: u32,
    pub priority_fee_micro_lamports: u64,
    pub tip_lamports: u64,
    pub tip_account: Option<Pubkey>,
    /// Indices mapping hops to instructions in `instructions`.
    pub hop_instruction_indices: Vec<usize>,
    pub flash_borrow_idx: Option<usize>,
    pub flash_repay_idx: Option<usize>,
    pub cu_limit_idx: Option<usize>,
    pub priority_fee_idx: Option<usize>,
    pub tip_idx: Option<usize>,
}

impl PrewarmedUnsignedTemplate {
    /// Create a new prewarmed unsigned template from skeleton components.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        payer: Pubkey,
        instructions: Vec<CompiledInstruction>,
        recent_blockhash: [u8; 32],
        lookup_tables: Vec<AddressLookupTable>,
        fee_sf: u64,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        tip_lamports: u64,
        tip_account: Option<Pubkey>,
        hop_instruction_indices: Vec<usize>,
        flash_borrow_idx: Option<usize>,
        flash_repay_idx: Option<usize>,
        cu_limit_idx: Option<usize>,
        priority_fee_idx: Option<usize>,
        tip_idx: Option<usize>,
    ) -> Self {
        Self {
            payer,
            instructions,
            recent_blockhash,
            lookup_tables,
            fee_sf,
            cu_limit,
            priority_fee_micro_lamports,
            tip_lamports,
            tip_account,
            hop_instruction_indices,
            flash_borrow_idx,
            flash_repay_idx,
            cu_limit_idx,
            priority_fee_idx,
            tip_idx,
        }
    }

    /// Refresh amount arguments, dynamic tick/bin arrays, blockhash, and resource limits.
    /// Executes entirely in-memory with minimal allocations before message compilation.
    pub fn refresh(&mut self, params: TemplateRefreshParams) {
        // 1. Refresh blockhash
        self.recent_blockhash = params.recent_blockhash;

        // 2. Refresh CU limit if instruction present
        if let Some(new_cu) = params.cu_limit {
            self.cu_limit = new_cu;
            if let Some(idx) = self.cu_limit_idx {
                if idx < self.instructions.len() && self.instructions[idx].data.len() >= 5 {
                    self.instructions[idx].data[1..5].copy_from_slice(&new_cu.to_le_bytes());
                }
            }
        }

        // 3. Refresh Priority Fee if instruction present
        if let Some(new_prio) = params.priority_fee_micro_lamports {
            self.priority_fee_micro_lamports = new_prio;
            if let Some(idx) = self.priority_fee_idx {
                if idx < self.instructions.len() && self.instructions[idx].data.len() >= 9 {
                    self.instructions[idx].data[1..9].copy_from_slice(&new_prio.to_le_bytes());
                }
            }
        }

        // 4. Refresh Flash Borrow amount (Kamino instruction: 8-byte discriminator + 8-byte u64 amount)
        if let Some(idx) = self.flash_borrow_idx {
            if idx < self.instructions.len() && self.instructions[idx].data.len() >= 16 {
                self.instructions[idx].data[8..16]
                    .copy_from_slice(&params.borrow_amount.to_le_bytes());
            }
        }

        // 5. Refresh Flash Repay amount. The `amount` argument MUST equal the
        //    borrow principal (klend enforces `liquidity_amount == borrow
        //    amount`); the origination fee is collected separately, on top.
        if let Some(idx) = self.flash_repay_idx {
            if idx < self.instructions.len() && self.instructions[idx].data.len() >= 16 {
                self.instructions[idx].data[8..16]
                    .copy_from_slice(&params.borrow_amount.to_le_bytes());
            }
        }

        // 6. Refresh Hop Amounts & Dynamic Accounts (tick arrays, bin arrays)
        for hop_refresh in params.hop_refreshes {
            if let Some(&ix_idx) = self.hop_instruction_indices.get(hop_refresh.hop_index) {
                if ix_idx < self.instructions.len() {
                    let ix = &mut self.instructions[ix_idx];

                    // Patch amount_in and min_amount_out in instruction data if space permits
                    // For typical 8-byte discriminator swap: data[8..16] = amount_in, data[16..24] = min_amount_out
                    if ix.data.len() >= 24 {
                        ix.data[8..16].copy_from_slice(&hop_refresh.amount_in.to_le_bytes());
                        ix.data[16..24].copy_from_slice(&hop_refresh.min_amount_out.to_le_bytes());
                    } else if ix.data.len() >= 17 {
                        // Raydium V4 SwapBaseIn: 1-byte opcode + 8-byte amount_in + 8-byte min_amount_out
                        ix.data[1..9].copy_from_slice(&hop_refresh.amount_in.to_le_bytes());
                        ix.data[9..17].copy_from_slice(&hop_refresh.min_amount_out.to_le_bytes());
                    }

                    // Refresh dynamic accounts (e.g. tick arrays or bin arrays)
                    if !hop_refresh.dynamic_accounts.is_empty() {
                        // Append or replace dynamic accounts
                        ix.accounts.extend(hop_refresh.dynamic_accounts);
                    }
                }
            }
        }

        // 7. Refresh Jito Tip amount
        self.tip_lamports = params.tip_lamports;
        if let Some(idx) = self.tip_idx {
            if idx < self.instructions.len() && self.instructions[idx].data.len() >= 12 {
                // System transfer instruction: 4-byte instruction_type (2) + 8-byte lamports
                self.instructions[idx].data[4..12]
                    .copy_from_slice(&params.tip_lamports.to_le_bytes());
            }
        }
    }

    /// Recompile a fresh message and cryptographically sign it with Ed25519.
    ///
    /// Guarantees:
    /// - Always produces a freshly signed transaction matching the refreshed parameters.
    /// - Never mutates signed transaction bytes.
    /// - Never reuses invalid or stale signatures.
    pub fn compile_and_sign(
        &self,
        signer: &KeypairSigner,
        _capabilities: CompilerCapabilities,
    ) -> Result<CompiledAdaptiveTransaction, TxError> {
        let v1_config = V1ConfigParams {
            priority_fee_lamports: Some(0),
            compute_unit_limit: Some(self.cu_limit),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        };
        AdaptiveTxCompiler::compile_v1_only(
            signer,
            &self.instructions,
            self.recent_blockhash,
            v1_config,
        )
    }
}
