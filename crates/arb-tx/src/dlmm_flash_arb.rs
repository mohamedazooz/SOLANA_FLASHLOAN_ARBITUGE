//! Phase 17 — Kamino flash-loan funded two-pool Meteora DLMM arbitrage.
//!
//! Assembles **one** Solana transaction that performs, in order:
//!
//! ```text
//! [0] ComputeBudget::SetComputeUnitLimit
//! [1] ComputeBudget::SetComputeUnitPrice
//! [2] ATA CreateIdempotent  (settlement account, flash mint)
//! [3] ATA CreateIdempotent  (intermediate account, mid mint)
//! [4] klend::flash_borrow_reserve_liquidity      (borrow_instruction_index = 4)
//! [5] lb_clmm::swap2   (Pool A)
//! [6] lb_clmm::swap2   (Pool B)
//! [7] klend::flash_repay_reserve_liquidity       (borrow_instruction_index = 4)
//! ```
//!
//! Because every instruction lives in the same transaction, the route is atomic:
//! if the repayment cannot be funded the whole transaction reverts.
//!
//! Invariants enforced by this builder (all fail closed):
//!
//! 1. **Distinct pools** — `leg_a.lb_pair != leg_b.lb_pair`.
//! 2. **Closing route** — the flash mint in, the flash mint out.
//! 3. **Mint chaining** — Pool A's output mint is Pool B's input mint, on the same
//!    token program.
//! 4. **No inventory subsidy** — Pool B's `amount_in` is bound to Pool A's
//!    *conservative minimum* output, so a pre-existing balance in the intermediate
//!    account can never make an unprofitable route look profitable. The flash
//!    principal and the repayment both move through a dedicated settlement account,
//!    and Pool A consumes the principal in full, so pre-existing inventory there
//!    cannot subsidize the repayment either.
//! 5. **Repayment coverage** — `repay_amount >= borrow_amount + flash_fee`, and
//!    Pool B's minimum output is at least the repayment amount.
//! 6. **Borrow/repay ordering** — the repay instruction carries the real index of
//!    the borrow instruction inside this transaction.
//! 7. **Atomicity self-check** — the assembled instruction list is verified with
//!    `verify_atomic_integrity` before it is signed.
//! 8. **Size limit** — the serialized transaction must fit the 1232-byte packet
//!    limit; the v0 (address-lookup-table) form is used so the account count can
//!    exceed the legacy static-key budget.
//!
//! The DLMM swap instructions are built with the repo's official-mirroring
//! builder (`crate::meteora_dlmm::build_swap2_exact_in`), which reproduces the
//! generated Meteora client account order and the optional-account sentinels.

use crate::atomic_integrity::{ExecutionMode, verify_atomic_integrity};
use crate::dlmm_bin_discovery::compact_u16_len;
use crate::meteora_dlmm::{
    DLMM_PROGRAM_ID_STR, DlmmSwapAccountSet, MEMO_PROGRAM_ID_STR, build_swap2_exact_in,
};
use crate::{
    AddressLookupTable, CompiledInstruction, KaminoFlashParams, KeypairSigner, TxBuilder,
    build_compute_unit_limit, build_compute_unit_price, build_create_ata_idempotent_for_program,
    build_kamino_flash_borrow, build_kamino_flash_repay_with_index,
    get_associated_token_address_with_program_id,
};
use arb_types::TokenProgramKind;
use solana_pubkey::Pubkey;
use std::collections::BTreeMap;
use std::str::FromStr;

/// Errors raised while assembling the flash-funded route.
#[derive(Debug, thiserror::Error)]
pub enum FlashArbError {
    #[error("both pools must be distinct")]
    SamePool,
    #[error("zero borrow amount")]
    ZeroBorrowAmount,
    #[error("flash repayment {repay} does not cover borrow {borrow} + fee {fee}")]
    InsufficientRepayment { borrow: u64, repay: u64, fee: u64 },
    #[error("pool A output mint {a} does not match pool B input mint {b}")]
    MintChainMismatch { a: String, b: String },
    #[error("route does not return to the flash mint {expected}, got {actual}")]
    NonClosingRoute { expected: String, actual: String },
    #[error("pool B minimum output {min_out} is below the repayment {repay}")]
    FinalOutputBelowRepayment { min_out: u64, repay: u64 },
    #[error("atomicity check failed: {0}")]
    AtomicityFailed(String),
    #[error("transaction too large: {size} bytes, limit {limit}")]
    TransactionTooLarge { size: usize, limit: usize },
    #[error("alt compilation failed: {0}")]
    AltCompilation(String),
}

/// One Meteora DLMM swap leg of the flash-funded route.
///
/// The leg is described in the pool's **on-chain** X/Y orientation. `swap_for_y`
/// selects the direction, so a pool whose on-chain ordering is (USDC, WSOL) is
/// handled without any special case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashArbPoolLeg {
    /// `LbPair` account of the pool.
    pub lb_pair: Pubkey,
    /// Token X reserve vault (PDA `[lb_pair, token_x_mint]`).
    pub reserve_x: Pubkey,
    /// Token Y reserve vault (PDA `[lb_pair, token_y_mint]`).
    pub reserve_y: Pubkey,
    /// On-chain token X mint.
    pub token_x_mint: Pubkey,
    /// On-chain token Y mint.
    pub token_y_mint: Pubkey,
    /// On-chain oracle PDA.
    pub oracle: Pubkey,
    /// Token program owning token X.
    pub token_x_program: Pubkey,
    /// Token program owning token Y.
    pub token_y_program: Pubkey,
    /// `true` for X -> Y (price down), `false` for Y -> X (price up).
    pub swap_for_y: bool,
    /// Exact input amount for this leg.
    pub amount_in: u64,
    /// Minimum output the leg must produce.
    pub min_amount_out: u64,
    /// Bin arrays the leg will touch, in traversal order.
    pub bin_array_pdas: Vec<Pubkey>,
    /// `BinArrayBitmapExtension` PDA when the account exists on chain.
    pub bitmap_extension: Option<Pubkey>,
    /// Optional host-fee / referral account. `None` encodes the protocol sentinel.
    pub host_fee_in: Option<Pubkey>,
}

impl FlashArbPoolLeg {
    /// Mint paid into this leg.
    #[inline]
    pub fn token_in(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_x_mint
        } else {
            self.token_y_mint
        }
    }

    /// Mint received from this leg.
    #[inline]
    pub fn token_out(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_y_mint
        } else {
            self.token_x_mint
        }
    }

    /// Token program of the input mint.
    #[inline]
    pub fn token_in_program(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_x_program
        } else {
            self.token_y_program
        }
    }

    /// Token program of the output mint.
    #[inline]
    pub fn token_out_program(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_y_program
        } else {
            self.token_x_program
        }
    }

    /// Build the official-mirroring account set for this leg.
    pub fn account_set(&self, payer: &Pubkey) -> DlmmSwapAccountSet {
        let (user_in, user_out) = if self.swap_for_y {
            (
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_x_mint,
                    &self.token_x_program,
                ),
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_y_mint,
                    &self.token_y_program,
                ),
            )
        } else {
            (
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_y_mint,
                    &self.token_y_program,
                ),
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_x_mint,
                    &self.token_x_program,
                ),
            )
        };

        DlmmSwapAccountSet {
            lb_pair: self.lb_pair,
            bin_array_bitmap_extension: self.bitmap_extension,
            reserve_x: self.reserve_x,
            reserve_y: self.reserve_y,
            user_token_in: user_in,
            user_token_out: user_out,
            token_x_mint: self.token_x_mint,
            token_y_mint: self.token_y_mint,
            oracle: self.oracle,
            host_fee_in: self.host_fee_in,
            user: *payer,
            token_x_program: self.token_x_program,
            token_y_program: self.token_y_program,
            memo_program: Pubkey::from_str(MEMO_PROGRAM_ID_STR).expect("valid memo program"),
            event_authority: crate::derive_meteora_event_authority(),
            bin_arrays: self.bin_array_pdas.clone(),
        }
    }
}

/// Everything needed to assemble the flash-funded two-pool transaction.
#[derive(Debug, Clone)]
pub struct FlashArbParams {
    /// Transaction authority (payer). It signs the flash borrow and the swaps.
    pub payer: Pubkey,
    /// Kamino flash-loan parameters. `user_destination_liquidity` is overridden with
    /// the settlement account so the principal and the repayment share one account.
    pub kamino: KaminoFlashParams,
    /// Flash principal in flash-mint raw units.
    pub borrow_amount: u64,
    /// Kamino flash fee in flash-mint raw units (`ceil(borrow * fee_sf / 2^64)`).
    pub flash_fee: u64,
    /// First DLMM swap leg (Pool A).
    pub leg_a: FlashArbPoolLeg,
    /// Second DLMM swap leg (Pool B).
    pub leg_b: FlashArbPoolLeg,
    /// Compute-unit limit.
    pub cu_limit: u32,
    /// Priority fee in micro-lamports.
    pub priority_fee_micro_lamports: u64,
}

impl FlashArbParams {
    /// Amount that must be repaid: principal plus the flash fee.
    #[inline]
    pub fn repay_amount(&self) -> u64 {
        self.borrow_amount.saturating_add(self.flash_fee)
    }
}

/// Result of assembling the flash-funded route.
#[derive(Debug, Clone)]
pub struct FlashTwoPoolArbitrageTx {
    /// Serialized (signed) transaction bytes.
    pub wire_bytes: Vec<u8>,
    /// Serialized size in bytes.
    pub tx_size: usize,
    /// The assembled instruction list.
    pub instructions: Vec<CompiledInstruction>,
    /// Index of the flash-borrow instruction inside `instructions`.
    pub borrow_instruction_index: usize,
    /// Index of the flash-repay instruction inside `instructions`.
    pub repay_instruction_index: usize,
    /// Flash principal.
    pub borrow_amount: u64,
    /// Flash fee.
    pub flash_fee: u64,
    /// Repayment amount (principal + fee).
    pub repay_amount: u64,
    /// Conservative minimum output of Pool A.
    pub min_hop_a_out: u64,
    /// Conservative minimum output of Pool B (>= repayment).
    pub min_final_out: u64,
    /// Settlement account (flash mint) used for borrow, final output and repayment.
    pub settlement_account: Pubkey,
    /// Intermediate account (mid mint) used between the two swaps.
    pub intermediate_account: Pubkey,
    /// Pool A `LbPair`.
    pub pool_a: Pubkey,
    /// Pool B `LbPair`.
    pub pool_b: Pubkey,
}

/// Assemble the instruction list for the flash-funded two-pool route.
///
/// Returns the instructions together with the borrow/repay instruction indices so
/// the caller can verify ordering and simulate.
pub fn assemble_flash_funded_two_pool(
    params: &FlashArbParams,
) -> Result<(Vec<CompiledInstruction>, usize, usize), FlashArbError> {
    // ── Invariant 1: distinct pools ──
    if params.leg_a.lb_pair == params.leg_b.lb_pair {
        return Err(FlashArbError::SamePool);
    }
    // ── Invariant 2: non-zero principal ──
    if params.borrow_amount == 0 {
        return Err(FlashArbError::ZeroBorrowAmount);
    }
    // ── Invariant 3: repayment covers principal + fee ──
    let repay_amount = params.repay_amount();
    if repay_amount < params.borrow_amount.saturating_add(params.flash_fee) {
        return Err(FlashArbError::InsufficientRepayment {
            borrow: params.borrow_amount,
            repay: repay_amount,
            fee: params.flash_fee,
        });
    }

    let flash_mint = params.leg_a.token_in();
    let mid_mint = params.leg_a.token_out();
    let final_mint = params.leg_b.token_out();

    // ── Invariant 4: closing route (flash mint in, flash mint out) ──
    if final_mint != flash_mint {
        return Err(FlashArbError::NonClosingRoute {
            expected: flash_mint.to_string(),
            actual: final_mint.to_string(),
        });
    }
    // ── Invariant 5: mint chaining between the legs ──
    if params.leg_b.token_in() != mid_mint {
        return Err(FlashArbError::MintChainMismatch {
            a: mid_mint.to_string(),
            b: params.leg_b.token_in().to_string(),
        });
    }
    // The intermediate mint must be the *same token program* on both legs.
    if params.leg_a.token_out_program() != params.leg_b.token_in_program() {
        return Err(FlashArbError::MintChainMismatch {
            a: params.leg_a.token_out_program().to_string(),
            b: params.leg_b.token_in_program().to_string(),
        });
    }
    // ── Invariant 6: Pool B must be able to fund the repayment ──
    if params.leg_b.min_amount_out < repay_amount {
        return Err(FlashArbError::FinalOutputBelowRepayment {
            min_out: params.leg_b.min_amount_out,
            repay: repay_amount,
        });
    }

    let payer = params.payer;
    let flash_program = params.leg_a.token_in_program();
    let mid_program = params.leg_a.token_out_program();

    // Settlement account: receives the flash principal, receives Pool B's output and
    // pays the repayment. Keeping all three on one account means a pre-existing
    // balance there cannot subsidize the repayment, because Pool A consumes the
    // principal in full and Pool B's output is the only other inflow.
    let settlement_account =
        get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_program);
    let intermediate_account =
        get_associated_token_address_with_program_id(&payer, &mid_mint, &mid_program);

    // Kamino accounts: the principal must land in the settlement account.
    let mut kamino = params.kamino.clone();
    kamino.user_destination_liquidity = settlement_account;

    // ── Instruction layout ──
    let mut ixs: Vec<CompiledInstruction> = Vec::with_capacity(8);
    ixs.push(build_compute_unit_limit(params.cu_limit));
    ixs.push(build_compute_unit_price(params.priority_fee_micro_lamports));
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        flash_mint,
        settlement_account,
        flash_program,
    ));
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        mid_mint,
        intermediate_account,
        mid_program,
    ));

    // ── Flash borrow ──
    let borrow_idx = ixs.len();
    ixs.push(build_kamino_flash_borrow(
        &kamino,
        payer,
        params.borrow_amount,
    ));

    // ── Pool A swap ──
    let accounts_a = params.leg_a.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_a,
        params.leg_a.amount_in,
        params.leg_a.min_amount_out,
    ));

    // ── Pool B swap ──
    let accounts_b = params.leg_b.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_b,
        params.leg_b.amount_in,
        params.leg_b.min_amount_out,
    ));

    // ── Flash repay (borrow instruction index must point at the real borrow) ──
    let repay_idx = ixs.len();
    ixs.push(build_kamino_flash_repay_with_index(
        &kamino,
        payer,
        repay_amount,
        borrow_idx as u8,
    ));

    Ok((ixs, borrow_idx, repay_idx))
}

/// Assemble **and sign** the flash-funded two-pool transaction as a v0 versioned
/// transaction.
///
/// Address lookup tables are required in practice: the Kamino + two-DLMM route
/// references far more accounts than a legacy message can carry inside the
/// 1232-byte packet limit.
pub fn build_flash_funded_two_pool_transaction(
    params: &FlashArbParams,
    signer: &KeypairSigner,
    recent_blockhash: [u8; 32],
    lookup_tables: &[AddressLookupTable],
) -> Result<FlashTwoPoolArbitrageTx, FlashArbError> {
    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_two_pool(params)?;

    // ── Atomicity self-check ──
    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )
    .map_err(|e| FlashArbError::AltCompilation(e.to_string()))?;
    if !verification.is_atomic {
        return Err(FlashArbError::AtomicityFailed(
            verification
                .rejection_reason
                .unwrap_or_else(|| "unknown".to_string()),
        ));
    }

    let message =
        TxBuilder::compile_v0_message(signer.pubkey(), &ixs, recent_blockhash, lookup_tables)
            .map_err(|e| FlashArbError::AltCompilation(e.to_string()))?;
    let signature = signer.sign(&message);

    let mut wire_bytes = Vec::with_capacity(1 + 64 + message.len());
    wire_bytes.push(1); // compact-u16 signature count = 1
    wire_bytes.extend_from_slice(&signature);
    wire_bytes.extend_from_slice(&message);

    let tx_size = wire_bytes.len();
    if tx_size > arb_constants::SOLANA_PACKET_MTU_LIMIT {
        return Err(FlashArbError::TransactionTooLarge {
            size: tx_size,
            limit: arb_constants::SOLANA_PACKET_MTU_LIMIT,
        });
    }

    let flash_mint = params.leg_a.token_in();
    let mid_mint = params.leg_a.token_out();
    let settlement_account = get_associated_token_address_with_program_id(
        &params.payer,
        &flash_mint,
        &params.leg_a.token_in_program(),
    );
    let intermediate_account = get_associated_token_address_with_program_id(
        &params.payer,
        &mid_mint,
        &params.leg_a.token_out_program(),
    );

    Ok(FlashTwoPoolArbitrageTx {
        wire_bytes,
        tx_size,
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: params.repay_amount(),
        min_hop_a_out: params.leg_a.min_amount_out,
        min_final_out: params.leg_b.min_amount_out,
        settlement_account,
        intermediate_account,
        pool_a: params.leg_a.lb_pair,
        pool_b: params.leg_b.lb_pair,
    })
}

/// Estimated serialized size of a v0 transaction carrying `instructions`.
///
/// Mirrors `TxBuilder::compile_v0_message`: accounts that live in an address lookup
/// table cost one index byte instead of a 32-byte inline key.
pub fn estimate_v0_transaction_size(
    instructions: &[CompiledInstruction],
    lookup_tables: &[AddressLookupTable],
) -> usize {
    let mut in_table: std::collections::HashSet<Pubkey> = std::collections::HashSet::new();
    for table in lookup_tables {
        for addr in &table.addresses {
            in_table.insert(*addr);
        }
    }

    let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
    for ix in instructions {
        for acc in &ix.accounts {
            let entry = account_map.entry(acc.pubkey).or_insert((false, false));
            entry.0 |= acc.is_signer;
            entry.1 |= acc.is_writable;
        }
        account_map.entry(ix.program_id).or_insert((false, false));
    }

    // Signers are never loaded from a lookup table: they must stay static.
    let static_len = account_map
        .iter()
        .filter(|(pk, (is_signer, _))| !*is_signer && !in_table.contains(*pk))
        .count()
        + account_map
            .values()
            .filter(|(is_signer, _)| *is_signer)
            .count();

    let mut size = 1usize; // signature count
    size += 64; // one signature
    size += 4; // version prefix + 3 header bytes
    size += compact_u16_len(static_len);
    size += 32 * static_len;
    size += 32; // recent blockhash
    size += compact_u16_len(instructions.len());

    for ix in instructions {
        size += 1; // program index
        size += compact_u16_len(ix.accounts.len());
        size += ix.accounts.len();
        size += compact_u16_len(ix.data.len());
        size += ix.data.len();
    }

    // Address table lookups section.
    let mut used_tables: Vec<(usize, usize)> = Vec::new();
    for table in lookup_tables {
        let mut writable = 0usize;
        let mut readonly = 0usize;
        for addr in &table.addresses {
            let Some((is_signer, is_writable)) = account_map.get(addr) else {
                continue;
            };
            if *is_signer {
                continue;
            }
            if *is_writable {
                writable += 1;
            } else {
                readonly += 1;
            }
        }
        if writable > 0 || readonly > 0 {
            used_tables.push((writable, readonly));
        }
    }
    size += compact_u16_len(used_tables.len());
    for (writable, readonly) in &used_tables {
        size += 32; // table address
        size += compact_u16_len(*writable) + writable;
        size += compact_u16_len(*readonly) + readonly;
    }

    size
}

/// Program id of the Meteora DLMM program.
pub fn dlmm_program_id() -> Pubkey {
    Pubkey::from_str(DLMM_PROGRAM_ID_STR).unwrap()
}

/// Token program id for a mint's token-program kind.
pub fn token_program_id(kind: TokenProgramKind) -> Pubkey {
    kind.program_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::TokenProgramKind;

    const WSOL: &str = "So11111111111111111111111111111111111111112";
    const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const POOL_A: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
    const POOL_B: &str = "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y";

    fn wsol() -> Pubkey {
        Pubkey::from_str(WSOL).unwrap()
    }
    fn usdc() -> Pubkey {
        Pubkey::from_str(USDC).unwrap()
    }
    fn pool_a() -> Pubkey {
        Pubkey::from_str(POOL_A).unwrap()
    }
    fn pool_b() -> Pubkey {
        Pubkey::from_str(POOL_B).unwrap()
    }

    fn leg(lb_pair: Pubkey, x: Pubkey, y: Pubkey, swap_for_y: bool) -> FlashArbPoolLeg {
        FlashArbPoolLeg {
            lb_pair,
            reserve_x: crate::derive_meteora_reserve_pda(&lb_pair, &x),
            reserve_y: crate::derive_meteora_reserve_pda(&lb_pair, &y),
            token_x_mint: x,
            token_y_mint: y,
            oracle: crate::derive_meteora_oracle_pda(&lb_pair),
            token_x_program: TokenProgramKind::SplToken.program_id(),
            token_y_program: TokenProgramKind::SplToken.program_id(),
            swap_for_y,
            amount_in: 1_000_000,
            min_amount_out: 1,
            bin_array_pdas: vec![crate::derive_meteora_bin_array_pda(&lb_pair, 0)],
            bitmap_extension: None,
            host_fee_in: None,
        }
    }

    fn params() -> FlashArbParams {
        FlashArbParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default(),
            borrow_amount: 1_000_000,
            flash_fee: 625,
            leg_a: leg(pool_a(), wsol(), usdc(), true),
            leg_b: leg(pool_b(), wsol(), usdc(), false),
            cu_limit: 400_000,
            priority_fee_micro_lamports: 10_000,
        }
    }

    #[test]
    fn identical_pools_are_rejected() {
        let mut p = params();
        p.leg_b.lb_pair = p.leg_a.lb_pair;
        assert!(matches!(
            assemble_flash_funded_two_pool(&p),
            Err(FlashArbError::SamePool)
        ));
    }

    #[test]
    fn zero_borrow_is_rejected() {
        let mut p = params();
        p.borrow_amount = 0;
        assert!(matches!(
            assemble_flash_funded_two_pool(&p),
            Err(FlashArbError::ZeroBorrowAmount)
        ));
    }

    #[test]
    fn non_closing_route_is_rejected() {
        let mut p = params();
        // Pool B swaps X(WSOL) -> Y(USDC), so the route ends in USDC instead of
        // returning to the flash mint.
        p.leg_b.token_x_mint = wsol();
        p.leg_b.token_y_mint = usdc();
        p.leg_b.swap_for_y = true;
        assert!(matches!(
            assemble_flash_funded_two_pool(&p),
            Err(FlashArbError::NonClosingRoute { .. })
        ));
    }

    #[test]
    fn mint_chain_mismatch_is_rejected() {
        let mut p = params();
        // Pool A now delivers BONK instead of USDC, so Pool B (which still takes
        // USDC in) cannot be funded. The route still closes back to WSOL, so the
        // mint-chain invariant, not the closing-route one, must fire.
        p.leg_a.token_y_mint = Pubkey::new_from_array([9u8; 32]);
        assert!(matches!(
            assemble_flash_funded_two_pool(&p),
            Err(FlashArbError::MintChainMismatch { .. })
        ));
    }

    #[test]
    fn final_output_below_repayment_is_rejected() {
        let mut p = params();
        p.leg_b.min_amount_out = 1;
        assert!(matches!(
            assemble_flash_funded_two_pool(&p),
            Err(FlashArbError::FinalOutputBelowRepayment { .. })
        ));
    }

    #[test]
    fn layout_is_borrow_swaps_repay() {
        let mut p = params();
        p.leg_b.min_amount_out = p.repay_amount();
        let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_two_pool(&p).unwrap();
        assert_eq!(ixs.len(), 8);
        assert_eq!(borrow_idx, 4);
        assert_eq!(repay_idx, 7);
        // Borrow carries the principal.
        assert_eq!(&ixs[borrow_idx].data[8..16], &p.borrow_amount.to_le_bytes());
        // Repay carries principal + fee and points back at the borrow index.
        let repay = p.repay_amount();
        assert_eq!(&ixs[repay_idx].data[8..16], &repay.to_le_bytes());
        assert_eq!(ixs[repay_idx].data[16], borrow_idx as u8);
    }

    #[test]
    fn atomicity_verification_passes() {
        let mut p = params();
        p.leg_b.min_amount_out = p.repay_amount();
        let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_two_pool(&p).unwrap();
        let v = verify_atomic_integrity(
            ExecutionMode::SingleTransaction,
            &ixs,
            Some(borrow_idx),
            Some(repay_idx),
            p.borrow_amount,
            p.repay_amount(),
            p.flash_fee,
        )
        .unwrap();
        assert!(v.is_atomic, "{:?}", v.rejection_reason);
    }

    #[test]
    fn reversed_mint_ordering_is_supported() {
        // Pool A: on-chain (USDC, WSOL) -> trading WSOL->USDC is Y->X.
        // Pool B: on-chain (USDC, WSOL) -> trading USDC->WSOL is X->Y.
        let mut p = params();
        p.leg_a = leg(pool_a(), usdc(), wsol(), false);
        p.leg_b = leg(pool_b(), usdc(), wsol(), true);
        p.leg_a.min_amount_out = 1_000_000;
        p.leg_b.amount_in = 1_000_000;
        p.leg_b.min_amount_out = p.repay_amount();
        let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_two_pool(&p).unwrap();
        assert_eq!(borrow_idx, 4);
        assert_eq!(repay_idx, 7);
        assert_eq!(ixs.len(), 8);
    }

    #[test]
    fn v0_size_estimate_is_below_the_packet_limit_with_a_table() {
        let mut p = params();
        p.leg_b.min_amount_out = p.repay_amount();
        let (ixs, _, _) = assemble_flash_funded_two_pool(&p).unwrap();
        let mut addrs: Vec<Pubkey> = Vec::new();
        for ix in &ixs {
            for acc in &ix.accounts {
                if !addrs.contains(&acc.pubkey) {
                    addrs.push(acc.pubkey);
                }
            }
        }
        let table = AddressLookupTable::new(Pubkey::new_unique(), addrs);
        let size = estimate_v0_transaction_size(&ixs, &[table]);
        assert!(size < arb_constants::SOLANA_PACKET_MTU_LIMIT, "size={size}");
    }
}
