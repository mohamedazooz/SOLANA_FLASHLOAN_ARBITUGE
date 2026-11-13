//! Meteora DLMM account matrix, PDA derivation and swap instruction builders.

use crate::*;

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

/// Derive the Meteora DLMM `BinArrayBitmapExtension` PDA.
/// Official seeds from MeteoraAg/dlmm-sdk commons/src/pda.rs: `[b"bitmap", lb_pair]`.
pub fn derive_meteora_bin_array_bitmap_extension_pda(pool: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
    let seeds: &[&[u8]] = &[b"bitmap", pool.as_ref()];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Resolve the Meteora DLMM bin-array indices that a swap may touch, walking
/// in the swap direction from the active bin's array. Unbounded — the number
/// of arrays scales with `bins_traversed`, replacing the old fixed 2..=4 cap.
///
/// `swap_for_y == true` means selling X for Y (price moves down → lower array
/// indices); `swap_for_y == false` means selling Y for X (price moves up →
/// higher array indices).
pub fn meteora_bin_array_indices(
    active_bin_id: i32,
    swap_for_y: bool,
    bins_traversed: u32,
) -> Vec<i64> {
    let active_idx = bin_array_index_for_bin(active_bin_id);
    // Derive the array span from the first and last bin of the traversal, exactly
    // like `plan_bin_arrays`, so the legacy helper never under-provisions when
    // the walk starts near an array boundary.
    let step: i64 = if swap_for_y { -1 } else { 1 };
    let last_bin_id = active_bin_id
        .saturating_add((step as i32).saturating_mul((bins_traversed as i32).saturating_sub(1)));
    let last_idx = bin_array_index_for_bin(last_bin_id);
    let arrays_needed = (last_idx - active_idx).abs() + 1;
    let actual_increment: i64 = if last_idx >= active_idx { 1 } else { -1 };
    (0..arrays_needed)
        .map(|i| active_idx + actual_increment * i)
        .collect()
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
    // Resolve bin arrays dynamically in the swap direction, covering the bins the
    // swap is expected to traverse. No fixed 2..=4 ceiling — the count scales with
    // estimated price impact (bins_traversed). We default to covering active bin + 1-2
    // adjacent arrays in the trade direction (1-3 arrays total).
    // Estimate price impact in basis points from amount_in vs min_amount_out.
    let impact_bps: u32 = if amount_in > min_amount_out && amount_in > 0 {
        (((amount_in - min_amount_out) as u128 * 10_000) / amount_in as u128) as u32
    } else {
        0
    };
    let bin_step_bps = bin_step.max(1) as u32;
    let bins_traversed = (impact_bps / bin_step_bps).saturating_add(1).max(1);
    let bin_array_indices = meteora_bin_array_indices(active_bin_id, swap_for_y, bins_traversed);

    let mut bin_array_metas = Vec::with_capacity(bin_array_indices.len());
    for idx in bin_array_indices {
        let pda = derive_meteora_bin_array_pda(&accounts.lb_pair, idx);
        bin_array_metas.push(AccountMeta::new(pda, false));
    }

    // -------------------------------------------------------------------------
    // Account order — verified from dlmm::client::accounts::Swap2 / Swap2InstructionAccounts:
    //  0  lb_pair                          (writable)
    //  1  bin_array_bitmap_extension       (writable PDA; program_id placeholder only if unresolvable)
    //  2  reserve_x                        (writable)
    //  3  reserve_y                        (writable)
    //  4  user_token_in                    (writable)
    //  5  user_token_out                   (writable)
    //  6  token_x_mint                     (readonly)
    //  7  token_y_mint                     (readonly)
    //  8  oracle                           (writable)
    //  9  host_fee_in                      (optional; use program_id as placeholder)
    //  10 user                             (signer)
    //  11 token_x_program (SPL Token)      (readonly)
    //  12 token_y_program (SPL Token)      (readonly)
    //  13 memo_program                     (readonly)
    //  14 event_authority                  (readonly)
    //  15 program (self-reference)         (readonly)
    // Remaining:
    //  16..16+N bin_arrays                 (writable, dynamically sized, no fixed cap)
    // -------------------------------------------------------------------------
    let bitmap_extension_pda = derive_meteora_bin_array_bitmap_extension_pda(&accounts.lb_pair);
    let mut account_list = vec![
        AccountMeta::new(accounts.lb_pair, false),
        AccountMeta::new(bitmap_extension_pda, false),
        AccountMeta::new(accounts.reserve_x, false),
        AccountMeta::new(accounts.reserve_y, false),
        AccountMeta::new(user_token_in, false),
        AccountMeta::new(user_token_out, false),
        AccountMeta::new_readonly(accounts.token_x_mint, false),
        AccountMeta::new_readonly(accounts.token_y_mint, false),
        AccountMeta::new(accounts.oracle, false),
        AccountMeta::new_readonly(accounts.program_id, false), // host_fee_in placeholder
        AccountMeta::new_readonly(user_authority, true),       // user (signer)
        AccountMeta::new_readonly(token_program, false),       // token_x_program
        AccountMeta::new_readonly(token_program, false),       // token_y_program
        AccountMeta::new_readonly(memo_program, false),
        AccountMeta::new_readonly(accounts.event_authority, false),
        AccountMeta::new_readonly(accounts.program_id, false), // program self-reference
    ];
    account_list.extend(bin_array_metas);

    CompiledInstruction {
        program_id: accounts.program_id,
        accounts: account_list,
        data,
    }
}

/// Build a genuine Meteora DLMM `swap2` instruction using precomputed BinArray PDAs.
pub fn build_meteora_dlmm_swap_instruction_with_bin_arrays(
    accounts: &MeteoraDlmmAccounts,
    user_authority: Pubkey,
    user_token_in: Pubkey,
    user_token_out: Pubkey,
    token_x_program: Pubkey,
    token_y_program: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    bin_array_pdas: &[Pubkey],
    bitmap_extension: Option<Pubkey>,
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(28);
    data.extend_from_slice(&METEORA_SWAP2_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // RemainingAccountsInfo { slices: vec![] }

    let memo_program = Pubkey::from_str("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr").unwrap();

    let bitmap_ext_meta = if let Some(ext) = bitmap_extension {
        AccountMeta::new(ext, false)
    } else {
        AccountMeta::new_readonly(accounts.program_id, false) // Placeholder when absent
    };

    let mut account_list = vec![
        AccountMeta::new(accounts.lb_pair, false),
        bitmap_ext_meta,
        AccountMeta::new(accounts.reserve_x, false),
        AccountMeta::new(accounts.reserve_y, false),
        AccountMeta::new(user_token_in, false),
        AccountMeta::new(user_token_out, false),
        AccountMeta::new_readonly(accounts.token_x_mint, false),
        AccountMeta::new_readonly(accounts.token_y_mint, false),
        AccountMeta::new(accounts.oracle, false),
        AccountMeta::new_readonly(accounts.program_id, false), // host_fee_in placeholder
        AccountMeta::new_readonly(user_authority, true),       // user (signer)
        AccountMeta::new_readonly(token_x_program, false),
        AccountMeta::new_readonly(token_y_program, false),
        AccountMeta::new_readonly(memo_program, false),
        AccountMeta::new_readonly(accounts.event_authority, false),
        AccountMeta::new_readonly(accounts.program_id, false), // program self-reference
    ];
    for pda in bin_array_pdas {
        account_list.push(AccountMeta::new(*pda, false));
    }

    CompiledInstruction {
        program_id: accounts.program_id,
        accounts: account_list,
        data,
    }
}
