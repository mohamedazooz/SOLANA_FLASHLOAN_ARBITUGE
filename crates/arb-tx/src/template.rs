//! Pre-warmed serialized byte template for sub-microsecond in-place transaction patching.
//! Enables hot-path mutation of borrow amount, repay amount, tip lamports, recent blockhash,
//! and Ed25519 Dalek cryptographic signature without allocating memory.

use crate::kamino::{
    FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR, calculate_kamino_flash_fee,
};
use crate::{KeypairSigner, TxError};

/// Pre-warmed transaction template for hot-path sub-microsecond dispatch.
#[derive(Debug, Clone)]
pub struct PreWarmedTxTemplate {
    pub raw_bytes: Vec<u8>,
    pub message_offset: usize,
    pub blockhash_offset: usize,
    pub borrow_amount_offset: usize,
    pub repay_amount_offset: usize,
    pub tip_amount_offset: usize,
    pub signature_offset: usize,
    /// Kamino flash-loan fee as the raw Q64.64 on-chain value
    /// (`ReserveConfig.fees.flash_loan_fee_sf`), never a bps rate.
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
    ) -> Self {
        Self {
            raw_bytes,
            message_offset,
            blockhash_offset,
            borrow_amount_offset,
            repay_amount_offset,
            tip_amount_offset,
            signature_offset,
            fee_sf,
        }
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
            fee_sf,
        })
    }

    /// Create template directly from KaminoBundleParams by compiling the bundle.
    /// `fee_sf` is the verified Kamino flash-loan fee (Q64.64) used to patch the
    /// repay amount on each `patch_and_sign`.
    pub fn from_bundle_params(
        params: &crate::KaminoBundleParams,
        fee_sf: u64,
    ) -> Result<Self, TxError> {
        let v0_tx = crate::TxBuilder::build_kamino_bundle(params)?;
        Self::from_precompiled_bytes(
            v0_tx.wire_bytes,
            &params.recent_blockhash,
            params.tip_lamports.as_u64(),
            fee_sf,
        )
    }

    /// Mutate borrow size, repay size, Jito tip, recent blockhash in-place,
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
        // 1. Overwrite borrow amount in KaminoFlashBorrow
        self.raw_bytes[self.borrow_amount_offset..self.borrow_amount_offset + 8]
            .copy_from_slice(&borrow_amount.to_le_bytes());

        // 2. Overwrite repay amount in KaminoFlashRepay (amount + fee)
        let fee = calculate_kamino_flash_fee(borrow_amount, self.fee_sf);
        let repay_amount = borrow_amount.saturating_add(fee);
        self.raw_bytes[self.repay_amount_offset..self.repay_amount_offset + 8]
            .copy_from_slice(&repay_amount.to_le_bytes());

        // 3. Overwrite tip lamports in Jito Tip Transfer
        self.raw_bytes[self.tip_amount_offset..self.tip_amount_offset + 8]
            .copy_from_slice(&tip_lamports.to_le_bytes());

        // 4. Overwrite 32-byte recent blockhash
        self.raw_bytes[self.blockhash_offset..self.blockhash_offset + 32]
            .copy_from_slice(&blockhash);

        // 5. Sign message slice (bytes 65..)
        let message_slice = &self.raw_bytes[self.message_offset..];
        let signature = signer.sign(message_slice);

        // 6. Overwrite signature in transaction prefix (offset 1..65)
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
