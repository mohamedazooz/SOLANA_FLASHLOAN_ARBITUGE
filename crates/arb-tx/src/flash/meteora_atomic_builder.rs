//! Meteora DLMM Atomic Arbitrage Builder (`MeteoraDlmmAtomicArbitrageBuilder`).
//!
//! Constructs a single atomic Solana transaction containing two Meteora DLMM swaps
//! executed in order in the same transaction block:
//! - Hop 1: Quote -> Base through Pool A (or Pool B)
//! - Hop 2: Base -> Quote through Pool B (or Pool A)
//!
//! Enforces:
//! 1. Strictly distinct pools: `pool_a.pubkey != pool_b.pubkey`
//! 2. Accurate real account metas (16 accounts + dynamic bin arrays in swap direction)
//! 3. Intermediate token isolation: Hop 2 input is bound to Hop 1 conservative minimum output
//!    so pre-existing wallet inventory never subsidizes an unprofitable second hop
//! 4. Strict final minimum output floor enforcing net profitability hurdle
//! 5. Pre-injected ComputeBudget instructions (limit + priority fee) on the legacy path
//! 6. Serialization with strict MTU verification: 1,232 bytes for legacy, 4,096 bytes for
//!    Solana Transaction v1 (SIMD-0385)
//!
//! Two wire formats are supported for the same route:
//!
//! * [`MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap`] /
//!   [`MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_with_traversal`] compile a
//!   legacy (unversioned) transaction behind a 1,232-byte packet limit. Compute budget is
//!   carried by `ComputeBudgetProgram` instructions.
//! * [`MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_v1`] compiles a Solana
//!   Transaction v1 (`0x81` discriminator, 4,096-byte limit, inline addresses, no ALT).
//!   Compute budget and priority fee move into the signed message config, so the route
//!   carries two fewer instructions and does not have to fit in 1,232 bytes.

use crate::error::TxError;
use crate::{
    AdaptiveTxCompiler, CompiledAdaptiveTransaction, CompiledInstruction, CompilerCapabilities,
    KeypairSigner, MAX_V1_ADDRESSES, MeteoraDlmmAccounts, SOLANA_PACKET_MTU_LIMIT,
    SOLANA_PACKET_V1_MTU, TransactionVersion, TxBuilder, V1ConfigParams, build_compute_unit_limit,
    build_compute_unit_price, build_create_ata_idempotent_for_program,
    build_meteora_dlmm_swap_instruction_with_bin_arrays, derive_meteora_bin_array_pda,
    get_associated_token_address_with_program_id, meteora_bin_array_indices,
};
use arb_constants::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
use arb_types::{PoolExecutionAccounts, PoolState, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Compute-unit limit used by the v1 route when the caller does not override it.
/// Two DLMM swaps plus two idempotent ATA creations, with headroom for a multi-array
/// traversal on each hop.
pub const METEORA_TWO_POOL_V1_DEFAULT_CU_LIMIT: u32 = 260_000;

/// Loaded-accounts data ceiling used by the v1 route when the caller does not override
/// it. 256 KiB comfortably covers two pools, their bin arrays, mints, vaults and oracles,
/// while still failing closed on a pathological traversal.
pub const METEORA_TWO_POOL_V1_DEFAULT_LOADED_DATA_LIMIT: u32 = 262_144;

/// Hard ceiling accepted for [`MeteoraTwoPoolV1Config::compute_unit_limit`]. Mirrors the
/// V1 runtime limit so a nonsensical limit is rejected before signing.
pub const METEORA_TWO_POOL_V1_MAX_CU_LIMIT: u32 = 1_400_000;

/// Native resource configuration for the Solana Transaction v1 two-pool Meteora route.
///
/// In v1 the compute-unit limit, the priority fee and the loaded-accounts data ceiling are
/// message header fields, not instructions: they are signed as part of the message and can
/// never be stripped or front-run by a wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraTwoPoolV1Config {
    /// Explicit compute-unit limit. v1 has no default: a zero limit is rejected.
    pub compute_unit_limit: u32,
    /// Total priority fee for the transaction, in lamports.
    pub priority_fee_lamports: u64,
    /// Maximum total account data bytes the transaction may load.
    pub loaded_accounts_data_size_limit: u32,
}

impl Default for MeteoraTwoPoolV1Config {
    fn default() -> Self {
        Self {
            compute_unit_limit: METEORA_TWO_POOL_V1_DEFAULT_CU_LIMIT,
            priority_fee_lamports: 0,
            loaded_accounts_data_size_limit: METEORA_TWO_POOL_V1_DEFAULT_LOADED_DATA_LIMIT,
        }
    }
}

impl MeteoraTwoPoolV1Config {
    /// Validate the configuration and convert it into the compiler's parameter block.
    ///
    /// Every bound is enforced before signing: an out-of-range limit is a build error, not
    /// a transaction that fails at runtime.
    pub fn to_v1_config_params(&self) -> Result<V1ConfigParams, TxError> {
        if self.compute_unit_limit == 0
            || self.compute_unit_limit > METEORA_TWO_POOL_V1_MAX_CU_LIMIT
        {
            return Err(TxError::InvalidInput(format!(
                "MeteoraTwoPoolV1Config: compute_unit_limit must be in 1..={METEORA_TWO_POOL_V1_MAX_CU_LIMIT}, got {}",
                self.compute_unit_limit
            )));
        }
        if self.loaded_accounts_data_size_limit == 0 {
            return Err(TxError::InvalidInput(
                "MeteoraTwoPoolV1Config: loaded_accounts_data_size_limit must be non-zero"
                    .to_string(),
            ));
        }
        Ok(V1ConfigParams {
            priority_fee_lamports: Some(self.priority_fee_lamports),
            compute_unit_limit: Some(self.compute_unit_limit),
            loaded_accounts_data_size_limit: Some(self.loaded_accounts_data_size_limit),
            heap_size: None,
        })
    }
}

/// Result of a compiled atomic two-swap Meteora DLMM arbitrage transaction.
#[derive(Debug, Clone)]
pub struct MeteoraAtomicArbitrageTx {
    /// Wire format the transaction was compiled into.
    pub version: TransactionVersion,
    pub wire_bytes: Vec<u8>,
    pub tx_size: usize,
    pub amount_in: u64,
    pub min_hop1_out: u64,
    pub min_final_out: u64,
    pub instructions: Vec<CompiledInstruction>,
    pub hop1_pool: Pubkey,
    pub hop2_pool: Pubkey,
}

/// Assembled instruction plan shared by every wire-format path.
struct TwoSwapPlan {
    /// Idempotent ATA creations followed by the two DLMM swaps. Deliberately contains no
    /// `ComputeBudget` instructions: the legacy path prepends them, the v1 path must not
    /// carry them at all.
    instructions: Vec<CompiledInstruction>,
    amount_in: u64,
    min_hop1_out: u64,
    min_final_out: u64,
    hop1_pool: Pubkey,
    hop2_pool: Pubkey,
}

/// Count distinct addresses the transaction will reference, including the payer and every
/// program id. This matches the v1 address budget (64 inline addresses, no ALT).
fn count_distinct_addresses(
    payer: Pubkey,
    instructions: &[CompiledInstruction],
) -> std::collections::BTreeSet<Pubkey> {
    let mut set = std::collections::BTreeSet::new();
    set.insert(payer);
    for ix in instructions {
        set.insert(ix.program_id);
        for acc in &ix.accounts {
            set.insert(acc.pubkey);
        }
    }
    set
}

pub struct MeteoraDlmmAtomicArbitrageBuilder;

impl MeteoraDlmmAtomicArbitrageBuilder {
    fn token_program_pubkey(kind: TokenProgramKind) -> Pubkey {
        match kind {
            TokenProgramKind::SplToken => Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            TokenProgramKind::Token2022 => Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap(),
        }
    }

    /// Extract `MeteoraDlmmAccounts` from `PoolState` and its execution accounts.
    pub fn extract_dlmm_accounts(pool: &PoolState) -> Result<MeteoraDlmmAccounts, TxError> {
        let Some(ref exec) = pool.execution_accounts else {
            return Err(TxError::MissingExecutionAccounts {
                pool_id: pool.pool_id.0,
                reason: "missing DLMM execution accounts".to_string(),
            });
        };

        match exec {
            PoolExecutionAccounts::MeteoraDlmm {
                reserve_x,
                reserve_y,
                ..
            } => Ok(MeteoraDlmmAccounts::for_pool(
                pool.pool_id.0,
                *reserve_x,
                *reserve_y,
                pool.base_mint.0,
                pool.quote_mint.0,
            )),
            _ => Err(TxError::MissingExecutionAccounts {
                pool_id: pool.pool_id.0,
                reason: "non-dlmm execution accounts for DLMM pool".to_string(),
            }),
        }
    }

    /// Build the ATA creations and the two DLMM swaps for a two-pool cycle.
    ///
    /// This is the single source of truth for the route: both the 1,232-byte legacy path
    /// and the 4,096-byte v1 path assemble their instructions here, so the routes can
    /// never drift apart.
    #[allow(clippy::too_many_arguments)]
    fn assemble_two_swap_plan(
        payer: &KeypairSigner,
        pool_a: &PoolState,
        pool_b: &PoolState,
        swap_a_first: bool,
        amount_in: u64,
        expected_hop1_out: u64,
        min_final_out: u64,
        slippage_bps: u64,
        hop1_bins_traversed: u32,
        hop2_bins_traversed: u32,
    ) -> Result<TwoSwapPlan, TxError> {
        // Invariant 1: Strictly distinct pools
        if pool_a.pool_id == pool_b.pool_id {
            return Err(TxError::InvalidInput(
                "MeteoraDlmmAtomicArbitrageBuilder: both pools must be distinct".to_string(),
            ));
        }

        // Invariant 2: Positive non-zero amounts
        if amount_in == 0 || expected_hop1_out == 0 || min_final_out == 0 {
            return Err(TxError::ZeroBorrowAmount);
        }

        let (first_pool, second_pool) = if swap_a_first {
            (pool_a, pool_b)
        } else {
            (pool_b, pool_a)
        };

        let first_accs = Self::extract_dlmm_accounts(first_pool)?;
        let second_accs = Self::extract_dlmm_accounts(second_pool)?;

        let payer_pubkey = payer.pubkey();

        // Intermediate token isolation:
        // Hop 1 in -> intermediate token.
        // Hop 2 intermediate token -> final out.
        let token_in = first_pool.quote_mint.0;
        let token_mid = first_pool.base_mint.0;
        let token_out = second_pool.quote_mint.0;

        let prog_in = Self::token_program_pubkey(first_pool.token_programs[1]);
        let prog_mid = Self::token_program_pubkey(first_pool.token_programs[0]);
        let prog_out = Self::token_program_pubkey(second_pool.token_programs[1]);

        let user_token_in =
            get_associated_token_address_with_program_id(&payer_pubkey, &token_in, &prog_in);
        let user_token_mid =
            get_associated_token_address_with_program_id(&payer_pubkey, &token_mid, &prog_mid);
        let user_token_out =
            get_associated_token_address_with_program_id(&payer_pubkey, &token_out, &prog_out);

        // Calculate conservative intermediate minimum output
        let slippage_factor = 10_000u64.saturating_sub(slippage_bps.min(10_000));
        let min_hop1_out = (expected_hop1_out as u128 * slippage_factor as u128 / 10_000) as u64;
        let hop1_min_out = min_hop1_out.max(1);

        // Hop 1 parameters
        let dlmm_first =
            first_pool
                .dlmm
                .as_ref()
                .ok_or_else(|| TxError::MissingExecutionAccounts {
                    pool_id: first_pool.pool_id.0,
                    reason: "missing dlmm state".into(),
                })?;
        let hop1_swap_for_y = first_pool.base_mint.0 == token_in;

        // Dynamic bin array resolution for Hop 1: derive the exact array span from
        // the quote's bins_crossed, in the swap direction. No fixed ceiling.
        let hop1_indices = meteora_bin_array_indices(
            dlmm_first.active_bin_id,
            hop1_swap_for_y,
            hop1_bins_traversed.max(1),
        );
        let hop1_bin_pdas: Vec<Pubkey> = hop1_indices
            .iter()
            .map(|&idx| derive_meteora_bin_array_pda(&first_accs.lb_pair, idx))
            .collect();

        let ix_hop1 = build_meteora_dlmm_swap_instruction_with_bin_arrays(
            &first_accs,
            payer_pubkey,
            user_token_in,
            user_token_mid,
            prog_mid, // token_x_program
            prog_in,  // token_y_program
            amount_in,
            hop1_min_out,
            &hop1_bin_pdas,
            None,
        );

        // Hop 2 parameters: input is exactly the guaranteed intermediate amount `hop1_min_out`
        let dlmm_second =
            second_pool
                .dlmm
                .as_ref()
                .ok_or_else(|| TxError::MissingExecutionAccounts {
                    pool_id: second_pool.pool_id.0,
                    reason: "missing dlmm state".into(),
                })?;
        let hop2_swap_for_y = second_pool.base_mint.0 == token_mid;

        let hop2_indices = meteora_bin_array_indices(
            dlmm_second.active_bin_id,
            hop2_swap_for_y,
            hop2_bins_traversed.max(1),
        );
        let hop2_bin_pdas: Vec<Pubkey> = hop2_indices
            .iter()
            .map(|&idx| derive_meteora_bin_array_pda(&second_accs.lb_pair, idx))
            .collect();

        let ix_hop2 = build_meteora_dlmm_swap_instruction_with_bin_arrays(
            &second_accs,
            payer_pubkey,
            user_token_mid,
            user_token_out,
            prog_mid, // token_x_program
            prog_out, // token_y_program
            hop1_min_out,
            min_final_out,
            &hop2_bin_pdas,
            None,
        );

        // ATA creation, token-program aware: a Token-2022 mint needs its own ATA program
        // owner, so the create must name the mint's token program rather than assuming
        // legacy SPL Token.
        let mut instructions = Vec::with_capacity(4);

        // Intermediate token account: receives hop 1 output and funds hop 2.
        instructions.push(build_create_ata_idempotent_for_program(
            payer_pubkey,
            payer_pubkey,
            token_mid,
            user_token_mid,
            prog_mid,
        ));

        // Input token account: hop 1 pulls `amount_in` from it.
        instructions.push(build_create_ata_idempotent_for_program(
            payer_pubkey,
            payer_pubkey,
            token_in,
            user_token_in,
            prog_in,
        ));

        // Output token account: normally identical to the input account because the cycle
        // closes back into the quote mint, in which case no extra instruction is needed.
        if user_token_out != user_token_in {
            instructions.push(build_create_ata_idempotent_for_program(
                payer_pubkey,
                payer_pubkey,
                token_out,
                user_token_out,
                prog_out,
            ));
        }

        instructions.push(ix_hop1);
        instructions.push(ix_hop2);

        Ok(TwoSwapPlan {
            instructions,
            amount_in,
            min_hop1_out: hop1_min_out,
            min_final_out,
            hop1_pool: first_pool.pool_id.0,
            hop2_pool: second_pool.pool_id.0,
        })
    }

    /// Build an atomic two-swap transaction across two distinct Meteora DLMM pools.
    ///
    /// This entry point does not have the quote's real bin traversal, so it covers
    /// one bin array ahead of the active array in the swap direction (2 arrays).
    /// Production callers that already have a quote should use
    /// [`Self::build_atomic_two_swap_with_traversal`] and pass the actual
    /// `bins_crossed` values, so the instruction carries exactly the arrays the
    /// swap will touch — with no execution ceiling.
    #[allow(clippy::too_many_arguments)]
    pub fn build_atomic_two_swap(
        payer: &KeypairSigner,
        pool_a: &PoolState,
        pool_b: &PoolState,
        swap_a_first: bool,
        amount_in: u64,
        expected_hop1_out: u64,
        min_final_out: u64,
        slippage_bps: u64,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        recent_blockhash: [u8; 32],
    ) -> Result<MeteoraAtomicArbitrageTx, TxError> {
        let default_span = u32::try_from(arb_constants::METEORA_BINS_PER_BIN_ARRAY).unwrap_or(70);
        Self::build_atomic_two_swap_with_traversal(
            payer,
            pool_a,
            pool_b,
            swap_a_first,
            amount_in,
            expected_hop1_out,
            min_final_out,
            slippage_bps,
            cu_limit,
            priority_fee_micro_lamports,
            recent_blockhash,
            default_span,
            default_span,
        )
    }

    /// Build an atomic two-swap legacy transaction using the quote's real bin traversal.
    ///
    /// `hop1_bins_traversed` / `hop2_bins_traversed` come from the DLMM quote
    /// (`bins_crossed`). The bin-array set is derived from those counts, so a swap
    /// that crosses many arrays gets many arrays, and a shallow swap gets one.
    /// There is deliberately no upper bound here; the caller's transaction-size
    /// check is the only limit, and it fails closed.
    #[allow(clippy::too_many_arguments)]
    pub fn build_atomic_two_swap_with_traversal(
        payer: &KeypairSigner,
        pool_a: &PoolState,
        pool_b: &PoolState,
        swap_a_first: bool,
        amount_in: u64,
        expected_hop1_out: u64,
        min_final_out: u64,
        slippage_bps: u64,
        cu_limit: u32,
        priority_fee_micro_lamports: u64,
        recent_blockhash: [u8; 32],
        hop1_bins_traversed: u32,
        hop2_bins_traversed: u32,
    ) -> Result<MeteoraAtomicArbitrageTx, TxError> {
        let plan = Self::assemble_two_swap_plan(
            payer,
            pool_a,
            pool_b,
            swap_a_first,
            amount_in,
            expected_hop1_out,
            min_final_out,
            slippage_bps,
            hop1_bins_traversed,
            hop2_bins_traversed,
        )?;

        // Legacy instruction assembly:
        // [0] ComputeUnitLimit
        // [1] ComputeUnitPrice
        // [2..] ATA creations + Hop 1 + Hop 2
        let mut instructions = Vec::with_capacity(plan.instructions.len() + 2);
        instructions.push(build_compute_unit_limit(cu_limit));
        instructions.push(build_compute_unit_price(priority_fee_micro_lamports));
        instructions.extend(plan.instructions);

        // Compile and sign transaction using standard wire format (self-contained, no external ALT required)
        let wire_bytes =
            TxBuilder::build_signed_transaction(payer, &instructions, recent_blockhash)?;

        let tx_size = wire_bytes.len();
        // Strict MTU enforcement
        if tx_size > SOLANA_PACKET_MTU_LIMIT {
            return Err(TxError::TransactionTooLarge {
                size: tx_size,
                limit: SOLANA_PACKET_MTU_LIMIT,
            });
        }

        Ok(MeteoraAtomicArbitrageTx {
            version: TransactionVersion::Legacy,
            wire_bytes,
            tx_size,
            amount_in: plan.amount_in,
            min_hop1_out: plan.min_hop1_out,
            min_final_out: plan.min_final_out,
            instructions,
            hop1_pool: plan.hop1_pool,
            hop2_pool: plan.hop2_pool,
        })
    }

    /// Build the same two-pool Meteora route as a Solana Transaction v1 (SIMD-0385).
    ///
    /// Why v1 for this route:
    ///
    /// * The 4,096-byte packet limit removes the 1,232-byte bin-array ceiling, so a deep
    ///   traversal that touches four or more arrays per hop still fits.
    /// * The compute-unit limit, the priority fee and the loaded-accounts data ceiling are
    ///   signed message fields rather than `ComputeBudgetProgram` instructions, so they
    ///   cannot be stripped or replaced by a wrapping program.
    /// * Addresses are inlined (64 max, no ALT), so a stale or missing lookup table can
    ///   never turn a valid route into an unloadable transaction.
    ///
    /// The trade-off is reach: v1 is only accepted by senders and relays that support it.
    /// Jito bundle ingress still caps at 1,232 bytes, so a Jito-bound caller must keep
    /// using [`Self::build_atomic_two_swap_with_traversal`].
    ///
    /// Fails closed on: identical pools, zero amounts, a v1 config outside its bounds, a
    /// route that needs more than 64 distinct addresses, and any wire payload above
    /// 4,096 bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn build_atomic_two_swap_v1(
        payer: &KeypairSigner,
        pool_a: &PoolState,
        pool_b: &PoolState,
        swap_a_first: bool,
        amount_in: u64,
        expected_hop1_out: u64,
        min_final_out: u64,
        slippage_bps: u64,
        v1_config: MeteoraTwoPoolV1Config,
        recent_blockhash: [u8; 32],
        hop1_bins_traversed: u32,
        hop2_bins_traversed: u32,
    ) -> Result<MeteoraAtomicArbitrageTx, TxError> {
        let config = v1_config.to_v1_config_params()?;

        let plan = Self::assemble_two_swap_plan(
            payer,
            pool_a,
            pool_b,
            swap_a_first,
            amount_in,
            expected_hop1_out,
            min_final_out,
            slippage_bps,
            hop1_bins_traversed,
            hop2_bins_traversed,
        )?;

        // v1 inlines every address, so the 64-address budget is a hard execution ceiling.
        // Check it here to name the offending route instead of surfacing a compiler error.
        let addresses = count_distinct_addresses(payer.pubkey(), &plan.instructions);
        if addresses.len() > MAX_V1_ADDRESSES {
            return Err(TxError::InvalidInput(format!(
                "MeteoraTwoPoolV1Builder: route needs {} distinct addresses, v1 allows {MAX_V1_ADDRESSES}",
                addresses.len()
            )));
        }

        let compiled = AdaptiveTxCompiler::compile_adaptive(
            payer,
            &plan.instructions,
            recent_blockhash,
            &[], // v1 inlines addresses: no lookup tables by design
            Some(config),
            CompilerCapabilities::v1_capable(),
            Some(TransactionVersion::V1),
        )?;

        // The adaptive compiler is allowed to fall back, so never trust a v1 build
        // implicitly: assert the version it actually produced.
        if compiled.version != TransactionVersion::V1 {
            return Err(TxError::Serialization(format!(
                "MeteoraTwoPoolV1Builder: compiler produced {:?}, expected V1",
                compiled.version
            )));
        }
        let parsed = CompiledAdaptiveTransaction::parse_version_from_wire(&compiled.wire_bytes)?;
        if parsed != TransactionVersion::V1 {
            return Err(TxError::Serialization(format!(
                "MeteoraTwoPoolV1Builder: wire bytes carry {parsed:?}, expected V1"
            )));
        }

        let tx_size = compiled.wire_bytes.len();
        if tx_size > SOLANA_PACKET_V1_MTU {
            return Err(TxError::TransactionTooLarge {
                size: tx_size,
                limit: SOLANA_PACKET_V1_MTU,
            });
        }

        Ok(MeteoraAtomicArbitrageTx {
            version: TransactionVersion::V1,
            wire_bytes: compiled.wire_bytes,
            tx_size,
            amount_in: plan.amount_in,
            min_hop1_out: plan.min_hop1_out,
            min_final_out: plan.min_final_out,
            instructions: plan.instructions,
            hop1_pool: plan.hop1_pool,
            hop2_pool: plan.hop2_pool,
        })
    }
}
