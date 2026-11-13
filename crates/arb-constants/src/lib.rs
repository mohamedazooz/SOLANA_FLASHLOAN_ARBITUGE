//! Centralized Constants for Solana Arbitrage Bot.
//!
//! Single source of truth for all on-chain addresses, program IDs,
//! mint addresses, and protocol constants used across the workspace.
//! Import this crate instead of duplicating literals.

use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};

// ============================================================================
// Token Programs & System IDs
// ============================================================================

/// SPL Token Program ID.
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// Token-2022 Program ID.
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
/// Pump Fees program ID.
pub const PUMP_FEES_PROGRAM_ID: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";
/// Anchor discriminator for Pump Fees FeeConfig accounts.
pub const PUMP_FEES_FEE_CONFIG_DISCRIMINATOR: [u8; 8] = [143, 52, 146, 187, 219, 123, 76, 155];
/// Anchor discriminator for PumpSwap GlobalConfig accounts.
pub const PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];
/// Associated Token (ATA) Program ID.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
/// Address Lookup Table Program ID.
pub const ADDRESS_LOOKUP_TABLE_PROGRAM_ID: &str = "AddressLookupTab1e1111111111111111111111111";
/// Sysvar Instructions account.
pub const SYSVAR_INSTRUCTIONS_ID: &str = "Sysvar1nstructions1111111111111111111111111";
/// Sysvar Rent account.
pub const SYSVAR_RENT_ID: &str = "SysvarRent111111111111111111111111111111111";
/// System Program (zero address).
pub const SYSTEM_PROGRAM_ID: [u8; 32] = [0; 32];
/// Compute Budget Program ID (`ComputeBudget111111111111111111111111111111`).
pub const COMPUTE_BUDGET_PROGRAM_ID: [u8; 32] = [
    3, 6, 70, 111, 229, 33, 23, 50, 255, 236, 173, 186, 114, 195, 155, 231, 188, 140, 229, 187,
    197, 247, 18, 107, 44, 67, 155, 58, 64, 0, 0, 0,
];

// ============================================================================
// Token Mints
// ============================================================================

/// WSOL (Wrapped SOL) mint address.
pub const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
/// Token-2022 native SOL mint.
pub const TOKEN_2022_NATIVE_MINT: &str = "9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP";
/// USDC mint address on Solana.
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

// ============================================================================
// Jito Tip Accounts (Mainnet)
// ============================================================================

/// Official online-verified Jito Tip Accounts on Solana Mainnet.
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

/// Default minimum net profit (retained edge) that justifies execution, in lamports.
pub const DEFAULT_MIN_PROFIT_LAMPORTS: u64 = 50_000;
/// Default Jito tip floor enforced by the block engine, in lamports.
pub const DEFAULT_JITO_TIP_FLOOR_LAMPORTS: u64 = 10_000;
/// Default Jito tip share applied to a route's gross edge (70% per SEEEVV.MD).
pub const DEFAULT_JITO_TIP_SHARE_BPS: u64 = 7_000;
/// Default maximum Jito tip share as a fraction of the available edge (85% per SEEEVV.MD).
pub const DEFAULT_JITO_MAX_TIP_SHARE_BPS: u64 = 8_500;
/// Default keep-share used by the adaptive bidder when no operator override is set.
pub const DEFAULT_KEEP_SHARE: f64 = 0.85;
/// Default hard cap on the tip as a fraction of the edge (85% per SEEEVV.MD).
pub const DEFAULT_MAX_TIP_SHARE: f64 = 0.85;
/// Default minimum retained edge after tip, in lamports.
pub const DEFAULT_MIN_KEEP_LAMPORTS: u64 = DEFAULT_MIN_PROFIT_LAMPORTS;

/// Resolve a round-robin Jito tip account Pubkey by index.
pub fn jito_tip_account_at(idx: usize) -> Pubkey {
    Pubkey::from_str(JITO_TIP_ACCOUNTS[idx % JITO_TIP_ACCOUNTS.len()])
        .expect("Valid Jito tip pubkey")
}

static ROUND_ROBIN_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Round-robin Jito tip account for load distribution across all 8 tip accounts.
pub fn jito_tip_account_round_robin() -> Pubkey {
    let idx = ROUND_ROBIN_COUNTER.fetch_add(1, Ordering::Relaxed);
    jito_tip_account_at(idx)
}

/// Deterministically resolve a Jito tip account for a validator leader identity.
///
/// Hashing the validator identity across the 8 verified Jito tip accounts guarantees
/// that transactions targeting the same leader consistently use the identical tip account,
/// preventing cache bust during retries while distributing tip volume uniformly across accounts.
pub fn jito_tip_account_for_leader(leader: &Pubkey) -> Pubkey {
    let bytes = leader.to_bytes();
    let hash = bytes
        .iter()
        .fold(0usize, |acc, &b| acc.wrapping_add(b as usize));
    jito_tip_account_at(hash)
}

/// Resolve the appropriate Jito tip account for a target slot, mapping to the leader
/// identity when known, or falling back to deterministic slot-based indexing.
pub fn jito_tip_account_for_slot(slot: u64, leader: Option<&Pubkey>) -> Pubkey {
    match leader {
        Some(leader_pk) => jito_tip_account_for_leader(leader_pk),
        None => jito_tip_account_at(slot as usize),
    }
}

// ============================================================================
// Jito Sandwich-Protection Marker
// ============================================================================

/// `jitodontfront` marker account, pre-decoded to raw bytes.
///
/// A bundle containing a transaction that references this account is rejected by
/// the block engine unless that transaction sits at index 0 of the bundle. The
/// account does not need to exist on chain and must be marked read-only.
///
/// Stored as bytes, matching the `SYSTEM_PROGRAM_ID` / `COMPUTE_BUDGET_PROGRAM_ID`
/// convention: decoding at the use site would make a compile-time constant fail
/// at runtime.
pub const JITODONTFRONT: [u8; 32] = [
    10, 241, 195, 67, 33, 136, 202, 58, 99, 81, 53, 161, 58, 24, 149, 26, 206, 189, 41, 230, 172,
    45, 174, 103, 255, 219, 6, 215, 64, 0, 0, 0,
];

/// The `jitodontfront` marker as a `Pubkey`.
pub fn jitodontfront_marker() -> Pubkey {
    Pubkey::new_from_array(JITODONTFRONT)
}

// ============================================================================
// DEX Program IDs (Mainnet)
// ============================================================================

/// Raydium AMM v4 Program ID.
pub const RAYDIUM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
/// Raydium AMM v4 Authority PDA.
pub const RAYDIUM_V4_AUTHORITY: &str = "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1";
/// Raydium CP-Swap (CPMM) Program ID.
pub const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
/// Raydium CLMM Program ID.
pub const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
/// Orca Whirlpool Program ID.
pub const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
/// Meteora DLMM (lb_clmm) Program ID.
pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";
/// Pump.fun Bonding Curve Program ID.
pub const PUMP_FUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
/// PumpSwap post-graduation AMM Program ID.
pub const PUMP_SWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
/// PumpSwap GlobalConfig PDA (derived from ["global_config"]).
pub const PUMP_SWAP_GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";
/// Anchor discriminator for the PumpSwap Pool account.
pub const PUMP_SWAP_POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
/// Pump.fun Fee Recipient.
pub const PUMP_FUN_FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
/// Pump.fun 8-byte account discriminator for BondingCurve (`sha256("account:BondingCurve")[..8]`).
pub const PUMP_FUN_BONDING_CURVE_DISCRIMINATOR: [u8; 8] = [23, 183, 248, 55, 96, 216, 172, 96];
/// Pump.fun buy instruction 8-byte discriminator.
pub const PUMP_FUN_BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
/// Pump.fun sell instruction 8-byte discriminator.
pub const PUMP_FUN_SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
/// Pump.fun global account PDA seed.
pub const PUMP_FUN_GLOBAL_SEED: &[u8] = b"global";
/// Pump.fun bonding curve account PDA seed.
pub const PUMP_FUN_BONDING_CURVE_SEED: &[u8] = b"bonding-curve";
/// Pump.fun event authority account PDA seed.
pub const PUMP_FUN_EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";

// ============================================================================
// Raydium V4 — SOL-USDC Pool & Vaults (Mainnet)
// ============================================================================

/// Raydium SOL-USDC pool address.
pub const RAYDIUM_SOL_USDC_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
/// Raydium SOL vault address.
pub const RAYDIUM_SOL_VAULT: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
/// Raydium USDC vault address.
pub const RAYDIUM_USDC_VAULT: &str = "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz";

// ============================================================================
// Orca Whirlpool — SOL-USDC Pool & Vaults (Mainnet)
// ============================================================================

/// Orca SOL-USDC pool address.
pub const ORCA_SOL_USDC_POOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";
/// Orca SOL vault address.
pub const ORCA_SOL_VAULT: &str = "EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9";
/// Orca USDC vault address.
pub const ORCA_USDC_VAULT: &str = "2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP";

// ============================================================================
// Meteora DLMM — SOL-USDC Pool & Reserves (Mainnet)
// ============================================================================

/// Meteora SOL-USDC pool address (bin_step 4, highest TVL SOL-USDC DLMM).
pub const METEORA_SOL_USDC_POOL: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
/// Meteora SOL reserve vault address.
pub const METEORA_SOL_RESERVE: &str = "EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o";
/// Meteora USDC reserve vault address.
pub const METEORA_USDC_RESERVE: &str = "CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz";
/// Meteora DLMM bin step (basis points) for SOL-USDC pool.
pub const METEORA_SOL_USDC_BIN_STEP: u16 = 4;
/// Number of liquidity bins per `BinArray` account.
///
/// Meteora `lb_clmm::state::bin_array::BINS_PER_ARRAY` is 70 — NOT 64. The
/// BinArray PDA index is `floor(bin_id / 70)`, so this value drives which
/// on-chain BinArray accounts a `swap2` instruction references. A wrong divisor
/// silently targets the wrong BinArrays and the swap fails (or reads the wrong
/// bins). Keep it equal to the value the ingestion decoder uses.
pub const METEORA_BINS_PER_BIN_ARRAY: i64 = 70;
/// Anchor 8-byte discriminator for `swap2` = sha256("global:swap2")[..8].
pub const METEORA_SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

// ============================================================================
// Raydium CPMM — SOL-USDC Pool & Vaults (Mainnet)
// ============================================================================

/// Raydium CPMM SOL-USDC pool address (1 bps fee tier).
pub const RAYDIUM_CPMM_SOL_USDC_POOL: &str = "79RbHpxoYSB3EJnN8aGLouuRrDJVGWLzxM3r1qVKsKJ7";
/// Raydium CPMM SOL vault address.
pub const RAYDIUM_CPMM_SOL_VAULT: &str = "DJgKsuDtTVvT1VqjC1vHTu6LHjq5Z6KRX1jaLKZJ3VwW";
/// Raydium CPMM USDC vault address.
pub const RAYDIUM_CPMM_USDC_VAULT: &str = "HYbZV1vHH8rDvn3a2uPtR8nCnBkL5xXwK7yP1uJ9nC2L";
/// Anchor discriminator for Raydium CPMM swap_base_input instruction.
pub const RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR: [u8; 8] =
    [143, 190, 90, 218, 196, 30, 51, 222];
/// Raydium CPMM vault and LP mint authority seed.
pub const RAYDIUM_CPMM_AUTH_SEED: &[u8] = b"vault_and_lp_mint_auth_seed";
/// Raydium CPMM pool vault seed.
pub const RAYDIUM_CPMM_POOL_VAULT_SEED: &[u8] = b"pool_vault";
/// Raydium CPMM observation state seed.
pub const RAYDIUM_CPMM_OBSERVATION_SEED: &[u8] = b"observation";

// ============================================================================
// Raydium CLMM — Discriminators & Seeds
// ============================================================================

/// Anchor discriminator for Raydium CLMM swap instruction: sha256("global:swap")[..8].
pub const RAYDIUM_CLMM_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
/// Raydium CLMM tick array seed.
pub const RAYDIUM_CLMM_TICK_ARRAY_SEED: &[u8] = b"tick_array";
/// Raydium CLMM observation state seed.
pub const RAYDIUM_CLMM_OBSERVATION_SEED: &[u8] = b"observation";
/// Anchor discriminator for Raydium CLMM `swap_v2`: sha256("global:swap_v2")[..8].
pub const RAYDIUM_CLMM_SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];
/// SPL Memo v2 Program ID (required by Raydium CLMM swap_v2).
pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

// ============================================================================
// Meteora DAMM v2 (cp_amm) — Program, Authority & Discriminators
// ============================================================================

/// Meteora DAMM v2 (`cp_amm`) Program ID.
pub const METEORA_DAMM_V2_PROGRAM_ID: &str = "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG";
/// Meteora DAMM v2 Pool Authority PDA.
pub const METEORA_DAMM_V2_POOL_AUTHORITY: &str = "HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC";
/// Anchor discriminator for `cp_amm::swap`: sha256("global:swap")[..8].
pub const METEORA_DAMM_V2_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
/// Anchor discriminator for `cp_amm::swap2`: sha256("global:swap2")[..8].
pub const METEORA_DAMM_V2_SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

// ============================================================================
// Raydium LaunchLab — Program & Discriminators
// ============================================================================

/// Raydium LaunchLab Program ID.
pub const RAYDIUM_LAUNCHLAB_PROGRAM_ID: &str = "LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj";
/// Anchor discriminator for `buy_exact_in`: sha256("global:buy_exact_in")[..8].
pub const RAYDIUM_LAUNCHLAB_BUY_EXACT_IN_DISCRIMINATOR: [u8; 8] =
    [250, 234, 13, 123, 213, 156, 19, 236];
/// Anchor discriminator for `sell_exact_in`: sha256("global:sell_exact_in")[..8].
pub const RAYDIUM_LAUNCHLAB_SELL_EXACT_IN_DISCRIMINATOR: [u8; 8] =
    [149, 39, 222, 155, 211, 124, 152, 26];

// ============================================================================
// PumpSwap — Instruction Discriminators
// ============================================================================

/// Anchor discriminator for PumpSwap `buy`: sha256("global:buy")[..8].
pub const PUMP_SWAP_BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
/// Anchor discriminator for PumpSwap `sell`: sha256("global:sell")[..8].
pub const PUMP_SWAP_SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
/// Anchor discriminator for PumpSwap `buy_v2`: sha256("global:buy_v2")[..8].
pub const PUMP_SWAP_BUY_V2_DISCRIMINATOR: [u8; 8] = [184, 23, 238, 97, 103, 197, 211, 61];
/// Anchor discriminator for PumpSwap `buy_exact_quote_in_v2`: sha256("global:buy_exact_quote_in_v2")[..8].
pub const PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR: [u8; 8] =
    [194, 171, 28, 70, 104, 77, 91, 47];
/// Anchor discriminator for PumpSwap `sell_v2`: sha256("global:sell_v2")[..8].
pub const PUMP_SWAP_SELL_V2_DISCRIMINATOR: [u8; 8] = [93, 246, 130, 60, 231, 233, 64, 178];

// ============================================================================
// Proprietary AMMs (Routed via Jupiter Fallback)
// ============================================================================

/// HumidiFi Swap Program ID.
pub const HUMIDIFI_PROGRAM_ID: &str = "9H6tua7jkLhdm3w8BvgpTn5LZNU7g4ZynDmCiNN3q6Rp";
/// SolFi V2 Program ID.
pub const SOLFI_PROGRAM_ID: &str = "SoLFiHG9TfgtdUXUjWAxi3LtvYuFyDLVhBWxdMZxyCe";
/// ZeroFi Program ID.
pub const ZEROFI_PROGRAM_ID: &str = "ZERor4xhbUycZ6gb9ntrhqscUcZmAbQDjEAtCf4hbZY";
/// All proprietary AMMs routed through Jupiter fallback.
pub const PROP_AMM_PROGRAM_IDS: [&str; 3] =
    [HUMIDIFI_PROGRAM_ID, SOLFI_PROGRAM_ID, ZEROFI_PROGRAM_ID];

// ============================================================================
// Kamino Lend (klend) — Program & Main Market (Mainnet)
// ============================================================================

/// Kamino Lend (klend) Program ID.
pub const KAMINO_KLEND_PROGRAM_ID: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
/// Kamino Main Market address.
pub const KAMINO_MAIN_MARKET: &str = "7u3HeHxYDLhnCoErrtycNokbQYbWGzLs6JSDqGAv5PfF";
/// Kamino Main Market Authority PDA (derived on-chain).
pub const KAMINO_MAIN_MARKET_AUTHORITY: &str = "9DrvZvyWh1HuAoZxvYWMvkf2XCzryCpGgHqrMjyDWpmo";
/// Kamino WSOL Reserve address.
pub const KAMINO_WSOL_RESERVE: &str = "d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q";
/// Kamino WSOL Supply Vault address.
pub const KAMINO_WSOL_SUPPLY_VAULT: &str = "GafNuUXj9rxGLn4y79dPu6MHSuPWeJR6UtTWuexpGh3U";
/// Kamino WSOL Fee Vault address.
pub const KAMINO_WSOL_FEE_VAULT: &str = "3JNof8s453bwG5UqiXBLJc77NRQXezYYEBbk3fqnoKph";
/// Kamino USDC Reserve address.
pub const KAMINO_USDC_RESERVE: &str = "D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59";

/// 8-byte Anchor discriminator: sha256("global:flash_borrow_reserve_liquidity")[0..8]
pub const FLASH_BORROW_DISCRIMINATOR: [u8; 8] = [135, 231, 52, 167, 7, 52, 212, 193];
/// 8-byte Anchor discriminator: sha256("global:flash_repay_reserve_liquidity")[0..8]
pub const FLASH_REPAY_DISCRIMINATOR: [u8; 8] = [185, 117, 0, 203, 96, 245, 180, 186];
/// Seeds for deriving lending market authority PDA: `[b"lma", lending_market.as_ref()]`.
pub const LENDING_MARKET_AUTH_SEED: &[u8] = b"lma";
// The Kamino flash-loan fee is NOT a constant: it is read on-chain from each
// reserve's `ReserveConfig.fees.flash_loan_fee_sf` (a U68F60 `Fraction`, i.e.
// 60 fractional bits — *not* Q64.64) via Yellowstone account snapshots decoded
// by `arb_tx::kamino::decode_reserve_flash_fee_sf`. Verified main-market WSOL
// reserve (`d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q`) on 2026-11-11 carries
// `sf = 11_529_215_046_068` (== 1e-5 == 0.1 bps → 10,000 lamports per 1 SOL).
// There is deliberately no default fee constant — a hardcoded 0/5 bps (or the
// old Q64.64 625-lamport figure) misprices quotes.

/// Per-hop slippage tolerance used when deriving on-chain `min_amount_out`.
/// Small enough to avoid compounding (n-1)*s across hops; the final profit
/// floor is the real safety net.
pub const DEFAULT_HOP_SLIPPAGE_BPS: u64 = 10;

/// Hard safety ceiling on the per-hop slippage floor (10%). At 1000 bps the
/// `min_amount_out` still guarantees 90% of the quoted output; beyond this the
/// floor no longer protects against adverse selection. Operators may lower
/// the configured value per strategy but can never raise it past this cap.
pub const HOP_SLIPPAGE_MAX_BPS: u64 = 1_000;

// ============================================================================
// Compute Budget Limits
// ============================================================================

/// Compute budget limit for 2-hop arbitrage.
pub const COMPUTE_BUDGET_LIMIT_2HOP: u32 = 260_000;
/// Compute budget limit for 3-hop arbitrage.
pub const COMPUTE_BUDGET_LIMIT_3HOP: u32 = 320_000;
/// Maximum compute unit limit.
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = 350_000;
/// Compute unit buffer percentage.
pub const CU_BUFFER_PCT: u32 = 15;
/// Solana packet MTU limit.
pub const SOLANA_PACKET_MTU_LIMIT: usize = 1232;
/// Base priority fee in micro-lamports.
pub const BASE_PRIORITY_FEE_MICRO_LAMPORTS: u64 = 1_000;

/// Return the compute budget limit for a given hop count.
#[inline]
pub fn get_cu_limit_for_hops(hops_count: usize) -> u32 {
    if hops_count <= 2 {
        COMPUTE_BUDGET_LIMIT_2HOP
    } else {
        COMPUTE_BUDGET_LIMIT_3HOP
    }
}
/// Pump.fun bonding-curve graduation threshold in lamports (protocol constant; verify against deployed program upgrades).
pub const PUMP_FUN_GRADUATION_LAMPORTS: u64 = 85_000_000_000;

// ============================================================================
// Endpoint Role Constants & Pacing Defaults
// ============================================================================

pub const DEFAULT_WS_COMMITMENT: &str = "processed";
pub const DEFAULT_WS_ENCODING: &str = "base64";
pub const DEFAULT_RPC_MAX_CONCURRENT_REQUESTS: usize = 1;
pub const DEFAULT_RPC_MIN_INTERVAL_MS: u64 = 200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitodontfront_marker_has_the_protected_prefix() {
        // The block engine only honours the marker when the account's base58
        // encoding starts with `jitodontfront`; a single wrong byte silently
        // disables the whole protection.
        let marker = jitodontfront_marker();
        let encoded = marker.to_string();
        assert!(
            encoded.starts_with("jitodontfront"),
            "marker does not carry the protected prefix: {encoded}"
        );
        assert_eq!(encoded, "jitodontfront111111111111111111111111111111");
    }

    #[test]
    fn tip_accounts_are_all_distinct_and_valid() {
        let mut seen = std::collections::HashSet::new();
        for idx in 0..JITO_TIP_ACCOUNTS.len() {
            let key = jito_tip_account_at(idx);
            assert_ne!(key, Pubkey::new_from_array([0; 32]));
            assert!(seen.insert(key), "duplicate tip account at index {idx}");
        }
        assert_eq!(seen.len(), JITO_TIP_ACCOUNTS.len());

        // Leader deterministic mapping
        let leader_a = Pubkey::new_from_array([1; 32]);
        let leader_b = Pubkey::new_from_array([2; 32]);
        let tip_a = jito_tip_account_for_leader(&leader_a);
        let tip_b = jito_tip_account_for_leader(&leader_b);
        assert!(seen.contains(&tip_a));
        assert!(seen.contains(&tip_b));
        // Idempotency: same leader always produces same tip account
        assert_eq!(tip_a, jito_tip_account_for_leader(&leader_a));

        // Slot deterministic mapping
        let slot_tip_known = jito_tip_account_for_slot(12345, Some(&leader_a));
        assert_eq!(slot_tip_known, tip_a);
        let slot_tip_unknown = jito_tip_account_for_slot(12345, None);
        assert!(seen.contains(&slot_tip_unknown));
    }

    #[test]
    fn test_new_constants_are_valid() {
        assert_eq!(
            RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR,
            [143, 190, 90, 218, 196, 30, 51, 222]
        );
        assert_eq!(RAYDIUM_CPMM_AUTH_SEED, b"vault_and_lp_mint_auth_seed");
        assert_eq!(
            RAYDIUM_CLMM_SWAP_DISCRIMINATOR,
            [248, 198, 158, 145, 225, 117, 135, 200]
        );
        assert_eq!(
            PUMP_FUN_BUY_DISCRIMINATOR,
            [102, 6, 61, 18, 1, 218, 235, 234]
        );
        assert_eq!(
            PUMP_FUN_SELL_DISCRIMINATOR,
            [51, 230, 133, 164, 1, 127, 131, 173]
        );
        assert_eq!(PUMP_FUN_GLOBAL_SEED, b"global");
        assert_eq!(PUMP_FUN_BONDING_CURVE_SEED, b"bonding-curve");
        assert_eq!(PUMP_FUN_EVENT_AUTHORITY_SEED, b"__event_authority");
        assert!(Pubkey::from_str(ADDRESS_LOOKUP_TABLE_PROGRAM_ID).is_ok());
        assert!(Pubkey::from_str(SYSVAR_RENT_ID).is_ok());
        assert!(Pubkey::from_str(PUMP_FUN_FEE_RECIPIENT).is_ok());
    }
}
