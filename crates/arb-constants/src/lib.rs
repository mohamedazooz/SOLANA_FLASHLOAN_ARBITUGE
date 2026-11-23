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
/// SPL Memo Program ID.
pub const MEMO_PROGRAM_ID: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
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
/// USDT mint (SPL Token) on Mainnet.
pub const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
/// WIF mint on Solana.
pub const WIF_MINT: &str = "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm";
/// POPCAT mint on Solana.
pub const POPCAT_MINT: &str = "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr";
/// BONK mint on Solana.
pub const BONK_MINT: &str = "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263";
/// FARTCOIN mint on Solana.
pub const FARTCOIN_MINT: &str = "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump";
/// RAY mint on Solana.
pub const RAY_MINT: &str = "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R";
/// JUP mint on Solana.
pub const JUP_MINT: &str = "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN";
/// PYTH mint on Solana.
pub const PYTH_MINT: &str = "HZ1JovNiRvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3";
/// TRUMP mint on Solana.
pub const TRUMP_MINT: &str = "6p6xgHyF7AeE6TZkSmFsko444wqoP15icUSqi2jfGiPN";
/// BOME mint on Solana.
pub const BOME_MINT: &str = "ukHH6c7mMyiWCf1b9pnWe25TSpkDDt3H5pQZgZ74J82";
/// PENGU mint on Solana.
pub const PENGU_MINT: &str = "2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv";
/// FWOG mint on Solana.
pub const FWOG_MINT: &str = "A8C3xuqscfmyLrte3VmTqrAq8kgMASius9AFNANwpump";
/// MOODENG mint on Solana.
pub const MOODENG_MINT: &str = "ED5nyyWEzpPPiWimP8vYm7sD7TD3LAt3Q3gRTWHzPJBY";

/// Resolve the canonical WSOL mint Pubkey.
pub fn wsol_mint() -> Pubkey {
    Pubkey::from_str(WSOL_MINT).expect("Valid WSOL mint pubkey")
}

/// Resolve the canonical USDC mint Pubkey.
pub fn usdc_mint() -> Pubkey {
    Pubkey::from_str(USDC_MINT).expect("Valid USDC mint pubkey")
}

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
/// Conservative default Jito tip floor in lamports. The block-engine-enforced
/// minimum is 1,000 lamports (docs.jito.wtf); this runtime floor is 10x that to
/// avoid submitting bundles that can never clear a quiet auction.
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
/// Raydium AMM v4 Devnet Program ID.
pub const RAYDIUM_V4_DEVNET_PROGRAM_ID: &str = "C8npZJQ9Knqy6Q81ic4TqnFd3JX6A4qg1aXQY5sJX6eY";
/// Raydium AMM v4 Authority PDA.
pub const RAYDIUM_V4_AUTHORITY: &str = "5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1";
/// OpenBook (Serum v3 successor) DEX Program ID.
pub const OPENBOOK_PROGRAM_ID: &str = "srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX";
/// Raydium CP-Swap (CPMM) Program ID.
pub const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";
/// Raydium CLMM Program ID.
pub const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";
/// Orca Whirlpool Program ID.
pub const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";
/// Meteora DLMM (lb_clmm) Program ID.
pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

// ============================================================================
// CLMM sqrt-price official bounds (Q64.64)
// ============================================================================

/// Minimum `sqrt_price_x64` shared by Orca Whirlpools, Raydium CLMM, and Meteora
/// DAMM v2 (`1.0001^(-443636/2)` in Q64.64).
/// Source (verified 2026-11-19): orca-so/whirlpools `rust-sdk/core/src/constants/swap.rs`,
/// raydium-io/raydium-clmm `programs/amm/src/libraries/tick_math.rs`,
/// MeteoraAg/damm-v2 `programs/cp-amm/src/constants.rs` — identical in all three.
pub const CLMM_MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
/// Orca Whirlpools official `MAX_SQRT_PRICE` (orca_whirlpools_core 2.1.1, latest
/// crates.io release at verification time).
pub const ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64: u128 = 79_226_673_515_401_279_992_447_579_055;
/// Raydium CLMM official `MAX_SQRT_PRICE_X64` (TickArray-based program source).
pub const RAYDIUM_CLMM_MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;
/// Meteora DAMM v2 (cp-amm) official `MAX_SQRT_PRICE` — bit-identical to the
/// Raydium CLMM bound (shared lineage constants).
pub const METEORA_DAMM_V2_MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;
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
/// Raydium SOL-USDC OpenBook market address (verified: openserum.io / Solana StackExchange / Raydium SDK).
pub const RAYDIUM_SOL_USDC_OPENBOOK_MARKET: &str = "8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6";
/// Raydium SOL-USDC OpenBook amm open-orders account.
pub const RAYDIUM_SOL_USDC_OPEN_ORDERS: &str = "HmiHHzq4Fym9e1D4qzLS6LDDM3tNsCTBPDWHTLZ763jY";
/// Raydium SOL-USDC OpenBook target-orders account.
pub const RAYDIUM_SOL_USDC_TARGET_ORDERS: &str = "CZza3Ej4Mc58MnxWA385itCC9jCo3L1D7zc3LKy1bZMR";
/// Raydium SOL-USDC OpenBook bids account.
pub const RAYDIUM_SOL_USDC_SERUM_BIDS: &str = "5jWUncPNBMZJ3sTHKmMLszypVkoRK6bfEQMQUHweeQnh";
/// Raydium SOL-USDC OpenBook asks account.
pub const RAYDIUM_SOL_USDC_SERUM_ASKS: &str = "EaXdHx7x3mdGA38j5RSmKYSXMzAFzzUXCLNBEDXDn1d5";
/// Raydium SOL-USDC OpenBook event queue.
pub const RAYDIUM_SOL_USDC_SERUM_EVENT_QUEUE: &str = "8CvwxZ9Db6XbLD46NZwwmVDZZRDy7eydFcAGkXKh9axa";
/// Raydium SOL-USDC OpenBook base (SOL) vault.
pub const RAYDIUM_SOL_USDC_SERUM_COIN_VAULT: &str = "CKxTHwM9fPMRRvZmFnFoqKNd9pQR21c5Aq9bh5h9oghX";
/// Raydium SOL-USDC OpenBook quote (USDC) vault.
pub const RAYDIUM_SOL_USDC_SERUM_PC_VAULT: &str = "6A5NHCj1yF6urc9wZNe6Bcjj4LVszQNj5DwAWG97yzMu";
/// Raydium SOL-USDC OpenBook vault signer PDA.
pub const RAYDIUM_SOL_USDC_SERUM_VAULT_SIGNER: &str =
    "CTz5UMLQm2SRWHzQnU62Pi4yJqbNGjgRBHqqp6oDHfF7";
/// Raydium CPMM SOL-USDC pool AmmConfig account (0.25% trade fee tier; fee tiers are on-chain accounts — read rates from the account, never assume this tier).
///
/// Verified on-chain: owner == CPMM program, data length 236,
/// `trade_fee_rate` u64 LE @12 == 2500 (denominator 1_000_000 => 25 bps),
/// and it is the `amm_config` referenced by `RAYDIUM_CPMM_SOL_USDC_POOL` @8.
/// The previous constant `D4A2sqNA...` was fabricated — the account does not
/// exist on mainnet.
pub const RAYDIUM_CPMM_SOL_USDC_AMM_CONFIG: &str = "D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2";

// ============================================================================
// Orca Whirlpool — SOL-USDC Pool & Vaults (Mainnet)
// ============================================================================

/// Orca SOL-USDC pool address.
pub const ORCA_SOL_USDC_POOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";
/// Orca SOL vault address.
pub const ORCA_SOL_VAULT: &str = "EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9";
/// Orca USDC vault address.
pub const ORCA_USDC_VAULT: &str = "2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP";
/// Orca SOL-USDC 1 bps pool address.
pub const ORCA_SOL_USDC_1BPS_POOL: &str = "83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d";
/// Orca SOL-USDC 2 bps pool address.
pub const ORCA_SOL_USDC_2BPS_POOL: &str = "FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q";
/// Orca SOL-USDC 8 bps pool address.
pub const ORCA_SOL_USDC_8BPS_POOL: &str = "7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm";

// ============================================================================
// Meteora DLMM — SOL-USDC Pool & Reserves (Mainnet)
// ============================================================================

/// Meteora SOL-USDC pool address (bin_step 4, highest TVL SOL-USDC DLMM).
pub const METEORA_SOL_USDC_POOL: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
/// Meteora SOL-USDC pool 2 (bin_step 10, deep liquidity).
pub const METEORA_SOL_USDC_POOL_2: &str = "BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh";
/// Meteora SOL-USDC pool 3 (bin_step 20).
pub const METEORA_SOL_USDC_POOL_3: &str = "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY";
/// Meteora SOL reserve vault address.
pub const METEORA_SOL_RESERVE: &str = "EYj9xKw6ZszwpyNibHY7JD5o3QgTVrSdcBp1fMJhrR9o";
/// Meteora USDC reserve vault address.
pub const METEORA_USDC_RESERVE: &str = "CoaxzEh8p5YyGLcj36Eo3cUThVJxeKCs7qvLAGDYwBcz";
/// Meteora DLMM bin step (basis points) for SOL-USDC pool.
pub const METEORA_SOL_USDC_BIN_STEP: u16 = 4;
/// Expected mainnet DLMM event authority PDA.
pub const METEORA_DLMM_EVENT_AUTHORITY: &str = "D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6";
/// Number of liquidity bins per `BinArray` account.
///
/// Meteora `lb_clmm::state::bin_array::BINS_PER_ARRAY` is 70 — NOT 64. The
/// BinArray PDA index is `floor(bin_id / 70)`, so this value drives which
/// on-chain BinArray accounts a `swap2` instruction references. A wrong divisor
/// silently targets the wrong BinArrays and the swap fails (or reads the wrong
/// bins). Keep it equal to the value the ingestion decoder uses.
pub const METEORA_BINS_PER_BIN_ARRAY: i64 = 70;
/// Maximum bins a single DLMM swap instruction may traverse.
///
/// `lb_clmm` 0.12.0 (mainnet 2026-05-21) reduced this from 280 to 260 because
/// the new limit-order logic consumes more CU (official DLMM changelog,
/// MeteoraAg/dlmm-sdk PR #273). A quote that crosses more bins than this
/// cannot be executed on-chain: the swap would abort with `TooManyBins`.
/// Quotes and bin-array plans must fail closed above this limit.
pub const METEORA_MAX_BINS_PER_SWAP: u32 = 260;
/// Anchor 8-byte discriminator for `swap2` = sha256("global:swap2")[..8].
pub const METEORA_SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

/// Half-width of the LbPair default bin-array bitmap window, in bin-array
/// indices. Official deployed value from the DLMM program IDL constant
/// `BIN_ARRAY_BITMAP_SIZE` (MeteoraAg/dlmm-sdk, ts-client/src/dlmm/idl/idl.json).
/// The LbPair's default bitmap covers bin-array indices
/// `[-(BIN_ARRAY_BITMAP_SIZE), BIN_ARRAY_BITMAP_SIZE - 1]` = `[-512, 511]`;
/// anything outside that window is tracked in the `BinArrayBitmapExtension`
/// account instead (official SDK rule `isOverflowDefaultBinArrayBitmap`,
/// ts-client/src/dlmm/helpers/binArray.ts).
pub const METEORA_BIN_ARRAY_BITMAP_SIZE: i64 = 512;

/// Inclusive default-bitmap window for bin-array indices.
pub const METEORA_DEFAULT_BITMAP_MIN_INDEX: i64 = -METEORA_BIN_ARRAY_BITMAP_SIZE;
pub const METEORA_DEFAULT_BITMAP_MAX_INDEX: i64 = METEORA_BIN_ARRAY_BITMAP_SIZE - 1;

/// Official protocol rule: a bin-array index is tracked in the
/// `BinArrayBitmapExtension` (not the LbPair default bitmap) iff it falls
/// outside the default window `[-512, 511]`. A swap whose traversal touches any
/// such index requires the extension account in the `swap2` account list.
///
/// This is the ONLY correct condition. It is NOT `indices.len() > 1`: a
/// two-array traversal entirely inside the default window needs no extension,
/// and a single-array traversal at index 600 does.
#[inline]
pub fn meteora_bin_array_index_requires_bitmap_extension(bin_array_index: i64) -> bool {
    !(METEORA_DEFAULT_BITMAP_MIN_INDEX..=METEORA_DEFAULT_BITMAP_MAX_INDEX)
        .contains(&bin_array_index)
}

// ============================================================================
// Raydium CPMM — SOL-USDC Pool & Vaults (Mainnet)
// ============================================================================

/// Raydium CPMM SOL-USDC pool address (25 bps fee tier).
///
/// Verified on-chain (getAccountInfo, Alchemy mainnet, 2026 session):
/// - discriminator `f7ede3f5d7c3de46` == sha256("account:PoolState")[..8]
/// - owner == `RAYDIUM_CPMM_PROGRAM_ID` (CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C)
/// - data length 637 (matches the official PoolState layout)
/// - amm_config @8 == D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2
///
/// Cross-checked against the official Raydium API v3
/// (`api-v3.raydium.io/pools/info/ids?ids=7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny`):
/// mintA = WSOL, mintB = USDC, config D4FPEru..., tradeFeeRate 2500 (25 bps).
///
/// NOTE: the previous constant `79RbHpxo...` was fabricated — the account does
/// not exist on mainnet. This CPMM SOL/USDC pool is real but carries very small
/// liquidity (~0.5 SOL + ~37 USDC at verification time); most SOL/USDC depth
/// lives in the Raydium V4 pool `RAYDIUM_SOL_USDC_POOL`.
pub const RAYDIUM_CPMM_SOL_USDC_POOL: &str = "7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny";
/// Raydium CPMM SOL-USDC `token_0_vault` (WSOL, 9 decimals).
/// Verified on-chain: SPL token account, mint So1111..., owner == CPMM pool
/// authority PDA GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL.
pub const RAYDIUM_CPMM_SOL_VAULT: &str = "7VLUXrnSSDo9BfCa4NWaQs68g7ddDY1sdXBKW6Xswj9Y";
/// Raydium CPMM SOL-USDC `token_1_vault` (USDC, 6 decimals).
/// Verified on-chain: SPL token account, mint EPjFWdd5..., owner == CPMM pool
/// authority PDA GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL.
pub const RAYDIUM_CPMM_USDC_VAULT: &str = "3rzbbW5Q8MA7sCaowf28hNgACNPecdS2zceWy7Ptzua9";
/// Raydium CPMM SOL-USDC pool authority PDA (owner of both vault token accounts).
/// Verified on-chain: appears as the owner field of both vault token accounts.
pub const RAYDIUM_CPMM_SOL_USDC_POOL_AUTHORITY: &str =
    "GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL";
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
/// SPL Memo v2 Program ID (required by Raydium CLMM swap_v2) - alias to canonical `MEMO_PROGRAM_ID`.
pub const SPL_MEMO_V2_PROGRAM_ID: &str = MEMO_PROGRAM_ID;

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
// Meteora Dynamic Bonding Curve (DBC) — Program & Migration
// ============================================================================

/// Meteora Dynamic Bonding Curve (DBC) Program ID on Mainnet.
pub const METEORA_DBC_PROGRAM_ID: &str = "dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN";
/// Meteora DBC Migration Keeper Address.
pub const METEORA_DBC_MIGRATION_KEEPER: &str = "9rpVELNUXLoMjAcc19UCVLJ73xjXNoYKENvRMHcci2V2";
/// Anchor discriminator for `dynamic_bonding_curve::swap2`: sha256("global:swap2")[..8].
pub const METEORA_DBC_SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

/// Meteora DLMM Pro Program ID on Mainnet.
pub const METEORA_DLMM_PRO_PROGRAM_ID: &str = "dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n";
/// Anchor discriminator for `dlmm_pro::swap`: sha256("global:swap")[..8].
pub const METEORA_DLMM_PRO_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
/// Anchor discriminator for DLMM Pro TickArray account.
pub const METEORA_DLMM_PRO_TICK_ARRAY_DISCRIMINATOR: [u8; 8] = [69, 97, 189, 190, 110, 7, 66, 187];

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
/// USDC main-market reserve supply vault. Read live from the reserve account
/// at `KAMINO_RESERVE_SUPPLY_VAULT_OFFSET` on 2026-11-17 (public mainnet RPC);
/// the same offset read on the WSOL reserve reproduced the verified
/// `KAMINO_WSOL_SUPPLY_VAULT` constant, cross-checking the layout.
pub const KAMINO_USDC_SUPPLY_VAULT: &str = "Bgq7trRgVMeq33yt235zM2onQ4bRDBsY5EWiTetF4qw6";
/// USDC main-market reserve fee vault (same live verification as above).
pub const KAMINO_USDC_FEE_VAULT: &str = "BbDUrk1bVtSixgQsPLBJFZEF7mwGstnD5joA1WzYvYFX";

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
// Overflow-Safe Financial Arithmetic Helpers (F-05)
// ============================================================================

/// Validate a basis-points slippage parameter.
///
/// `true` only for `0 <= slippage_bps <= 10_000`. Callers that carry a
/// stricter ceiling (e.g. `HOP_SLIPPAGE_MAX_BPS`) enforce it on top.
#[inline]
pub fn is_valid_slippage_bps(slippage_bps: u64) -> bool {
    slippage_bps <= 10_000
}

/// Slippage amount: the size of the discount applied to `amount` for a given
/// tolerance, i.e. `floor(amount * slippage_bps / 10_000)`.
///
/// Wide (u128) intermediate multiplication prevents the overflow that the
/// naive `amount * slippage_bps` in u64 hits for large meme-token raw amounts
/// (e.g. `u64::MAX` raw of a 6-decimal token at 100 bps). The result is
/// always `<= amount`, so it always fits back into `u64`.
#[inline]
pub fn slippage_amount_bps(amount: u64, slippage_bps: u64) -> u64 {
    debug_assert!(is_valid_slippage_bps(slippage_bps), "slippage bps > 10_000");
    let bps = slippage_bps.min(10_000) as u128;
    ((amount as u128 * bps) / 10_000u128) as u64
}

/// Slippage floor: the guaranteed minimum output after applying the
/// tolerance, i.e. `floor(amount * (10_000 - slippage_bps) / 10_000)`.
///
/// Wide (u128) intermediates. The result is always `<= amount`, so the
/// `as u64` conversion is lossless. CRITICAL: computing this as
/// `amount.saturating_mul(10_000 - slippage_bps) / 10_000` silently
/// collapses the floor to `u64::MAX / 10_000` (~4 orders of magnitude too
/// small) for large amounts — a weakened safety floor, not a correct one.
#[inline]
pub fn slippage_floor_bps(amount: u64, slippage_bps: u64) -> u64 {
    debug_assert!(is_valid_slippage_bps(slippage_bps), "slippage bps > 10_000");
    let bps = slippage_bps.min(10_000) as u128;
    ((amount as u128 * (10_000u128 - bps)) / 10_000u128) as u64
}

// ============================================================================
// Compute Budget Limits
// ============================================================================

/// Compute budget limit for 2-hop arbitrage (initial fallback before first measured CU).
pub const COMPUTE_BUDGET_LIMIT_2HOP: u32 = SOLANA_RUNTIME_MAX_COMPUTE_UNITS;
/// Compute budget limit for 3-hop arbitrage (initial fallback before first measured CU).
pub const COMPUTE_BUDGET_LIMIT_3HOP: u32 = SOLANA_RUNTIME_MAX_COMPUTE_UNITS;
/// Solana runtime protocol consensus maximum compute units per transaction.
pub const SOLANA_RUNTIME_MAX_COMPUTE_UNITS: u32 = 1_400_000;
/// Application-level ceiling for arbitrage execution.
/// Set to the Solana runtime protocol consensus maximum so complex multi-DEX
/// flash routes are never clamped below their measured requirement. Priority
/// fee exposure stays bounded through `cap_cu_price_by_profit` (profit-share
/// cap), not through a CU ceiling.
pub const APPLICATION_SAFETY_CU_LIMIT: u32 = SOLANA_RUNTIME_MAX_COMPUTE_UNITS;
/// Maximum compute unit limit enforced by application policy (delegates to APPLICATION_SAFETY_CU_LIMIT).
pub const MAX_COMPUTE_UNIT_LIMIT: u32 = APPLICATION_SAFETY_CU_LIMIT;
/// Compute unit buffer percentage.
pub const CU_BUFFER_PCT: u32 = 15;
/// Solana runtime legacy v0 packet MTU limit (IPv6 UDP / Jito).
pub const SOLANA_V0_PACKET_MTU_LIMIT: usize = 1232;
/// Solana runtime v1 packet MTU limit (SIMD-0296 & SIMD-0385 QUIC 4096 bytes).
pub const SOLANA_V1_PACKET_MTU_LIMIT: usize = 4096;
/// Default application packet MTU limit for legacy v0 transactions.
pub const SOLANA_PACKET_MTU_LIMIT: usize = SOLANA_V0_PACKET_MTU_LIMIT;
/// Transaction version discriminator for legacy v0 (0x80).
pub const TRANSACTION_VERSION_V0_PREFIX: u8 = 0x80;
/// Transaction version discriminator for SIMD-0385 v1 (0x81).
pub const TRANSACTION_VERSION_V1_PREFIX: u8 = 0x81;
/// Standard Solana consensus network base signature fee (5,000 lamports).
pub const SOLANA_BASE_FEE_LAMPORTS: u64 = 5_000;
/// Base priority fee in micro-lamports.
pub const BASE_PRIORITY_FEE_MICRO_LAMPORTS: u64 = 1_000;
/// Flash-arbitrage fast-path priority fee in micro-lamports.
///
/// Deliberately higher than [`BASE_PRIORITY_FEE_MICRO_LAMPORTS`]: flash routes
/// compete for the same slot as the pools they arbitrage. This is the single
/// source of truth for every flash-pipeline default; encode paths still clamp
/// the resulting price against the profit share via `cap_cu_price_by_profit`.
pub const FLASH_ARB_PRIORITY_FEE_MICRO_LAMPORTS: u64 = 10_000;
/// Default baseline priority fee maximum share of gross profit (1,000 bps = 10%).
pub const DEFAULT_PRIORITY_FEE_BASE_SHARE_BPS: u64 = 1_000;
/// Default maximum priority fee share cap under peak competition (2,000 bps = 20%).
pub const DEFAULT_PRIORITY_FEE_MAX_SHARE_BPS: u64 = 2_000;

/// Agave runtime maximum instruction invocation stack depth (`MAX_INSTRUCTION_STACK_DEPTH`):
/// the deepest chain of nested CPI frames a transaction may build. The executor-CPI
/// flash layout (arb-executor -> DEX program -> token program) must stay inside this
/// budget; enforced by `arb_tx::flash::arb_executor::validate_executor_cpi_stack_depth`.
/// Source: anza-xyz/agave `program-runtime/src/execution_budget.rs` (verified 2026-11-17).
pub const MAX_INSTRUCTION_STACK_DEPTH: usize = 5;
/// SIMD-0268 raises the runtime stack depth to 9 when the feature is active on the
/// cluster. Unknown until the fork advertises it; the conservative default above applies.
pub const MAX_INSTRUCTION_STACK_DEPTH_SIMD_0268: usize = 9;
/// Agave default per-transaction loaded-accounts data size limit, 64 MiB
/// (`MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES`), unless a
/// `set_loaded_accounts_data_size_limit` compute-budget instruction lowers it.
/// Enforced fail-closed by the simulation guard
/// `arb_sim::fork_sim::apply_runtime_limit_guards`.
/// Source: anza-xyz/agave `program-runtime/src/execution_budget.rs` (verified 2026-11-17).
pub const MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES: u64 = 64 * 1024 * 1024;

/// Freshness tolerance (slots) for accepting a Kamino reserve/mint snapshot read
/// from either the live stream store or the fallback store during flash-liquidity
/// evaluation (`supervisor/execution.rs`). ~1000 slots covers normal stream jitter
/// without letting truly stale snapshots into quote math.
pub const KAMINO_ZAP_FRESHNESS_TOLERANCE_SLOTS: u64 = 1_000;
/// Maximum age (slots) for the Kamino supply-vault balance used to size flash
/// liquidity. The vault is reconciled every ~30 seconds; 150 slots allow one
/// missed refresh while preventing a long-stale balance from inflating borrow
/// capacity.
pub const KAMINO_SUPPLY_VAULT_MAX_AGE_SLOTS: u64 = 150;
/// Freshness bound (slots) for a raw RPC snapshot of the Kamino fallback sources:
/// only snapshots read this recently may substitute for an unavailable stream state.
pub const KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS: u64 = 1_500;

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

// ============================================================================
// Local Network Binding Defaults
// ============================================================================

/// Default local binding address for the Web API and dashboard.
pub const DEFAULT_WEB_BIND_ADDR: &str = "127.0.0.1:8080";
/// Default local binding address for Prometheus metrics scraping.
pub const DEFAULT_METRICS_BIND_ADDR: &str = "127.0.0.1:9100";
/// Default local binding address for the Administrative control server.
pub const DEFAULT_ADMIN_BIND_ADDR: &str = "127.0.0.1:9101";

/// Build-time placeholder only; not verified as deployed on any Solana cluster.
pub const ARB_EXECUTOR_PROGRAM_ID: &str = "ArbExec1111111111111111111111111111111111111";
/// Default mainnet Address Lookup Table key.
pub const DEFAULT_MAINNET_ALT_KEY: &str = "ALT1111111111111111111111111111111111111111";

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
    fn test_clmm_sqrt_price_bounds_match_official_program_values() {
        assert_eq!(CLMM_MIN_SQRT_PRICE_X64, 4_295_048_016);
        assert_eq!(
            ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64,
            79_226_673_515_401_279_992_447_579_055
        );
        assert_eq!(
            RAYDIUM_CLMM_MAX_SQRT_PRICE_X64,
            79_226_673_521_066_979_257_578_248_091
        );
        assert_eq!(
            METEORA_DAMM_V2_MAX_SQRT_PRICE_X64,
            79_226_673_521_066_979_257_578_248_091
        );
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
        assert!(Pubkey::from_str(MEMO_PROGRAM_ID).is_ok());
        assert!(Pubkey::from_str(TRUMP_MINT).is_ok());
        assert!(Pubkey::from_str(BOME_MINT).is_ok());
        assert!(Pubkey::from_str(PENGU_MINT).is_ok());
        assert!(Pubkey::from_str(FWOG_MINT).is_ok());
        assert!(Pubkey::from_str(MOODENG_MINT).is_ok());
        assert!(Pubkey::from_str(ORCA_SOL_USDC_1BPS_POOL).is_ok());
        assert!(Pubkey::from_str(ORCA_SOL_USDC_2BPS_POOL).is_ok());
        assert!(Pubkey::from_str(ORCA_SOL_USDC_8BPS_POOL).is_ok());
        assert!(Pubkey::from_str(METEORA_SOL_USDC_POOL_2).is_ok());
        assert!(Pubkey::from_str(METEORA_SOL_USDC_POOL_3).is_ok());
        assert!(Pubkey::from_str(RAYDIUM_V4_DEVNET_PROGRAM_ID).is_ok());
        assert!(Pubkey::from_str(ARB_EXECUTOR_PROGRAM_ID).is_ok());
        assert!(Pubkey::from_str(DEFAULT_MAINNET_ALT_KEY).is_ok());
    }
}

#[cfg(test)]
mod slippage_arithmetic_tests {
    use super::*;

    /// F-05: `u64::MAX` amounts must not overflow the `u64 * bps`
    /// multiplication. The u128 intermediate keeps the exact quotient.
    #[test]
    fn slippage_helpers_are_overflow_safe_at_u64_max() {
        for bps in [0u64, 1, 10, 100, 1_000, 10_000] {
            let amount = u64::MAX;
            // Exact math in u128 as the reference.
            let exp_amount = (amount as u128 * bps as u128 / 10_000u128) as u64;
            let exp_floor = (amount as u128 * (10_000u128 - bps as u128) / 10_000u128) as u64;
            assert_eq!(slippage_amount_bps(amount, bps), exp_amount);
            assert_eq!(slippage_floor_bps(amount, bps), exp_floor);
        }
    }

    /// F-05 regression: the old `amount.saturating_mul(10_000 - bps) / 10_000`
    /// collapsed the floor to ~u64::MAX/10_000 for large amounts. The helper
    /// must never do that: floor == amount * (1e4 - bps) / 1e4 exactly.
    #[test]
    fn slippage_floor_does_not_saturating_collapse() {
        let amount = u64::MAX;
        let bps = 100u64; // 1 %
        let collapsed = amount.saturating_mul(10_000 - bps) / 10_000; // broken legacy path
        let correct = slippage_floor_bps(amount, bps);
        // The legacy path saturates the product to u64::MAX first, so the
        // collapsed floor is exactly u64::MAX / 10_000 (~18.4e15 lamports)
        // regardless of bps; the corrected value is ~amount * 0.99.
        assert_eq!(collapsed, u64::MAX / 10_000);
        assert!(
            correct > collapsed,
            "the corrected floor ({correct}) must exceed the saturating \
             collapse ({collapsed}) by orders of magnitude"
        );
        assert_eq!(correct, (amount as u128 * 9_900u128 / 10_000u128) as u64);
    }

    /// Round-trip invariants: both helpers stay within `amount`, and the
    /// floor is never weaker than `amount - margin` (the sold-side floor
    /// users derive with floor rounding may be at most 1 ulp below).
    #[test]
    fn slippage_invariants_hold() {
        for amount in [0u64, 1, 9_999, 10_000, 123_456, u64::MAX / 2] {
            for bps in [0u64, 1, 35, 999, 10_000] {
                let margin = slippage_amount_bps(amount, bps);
                let floor = slippage_floor_bps(amount, bps);
                assert!(margin <= amount);
                assert!(floor <= amount);
                let residual_ok = amount.checked_sub(margin).is_some_and(|diff| floor <= diff);
                assert!(
                    residual_ok,
                    "floor ({floor}) must never exceed amount - margin \
                     (amount={amount}, bps={bps})"
                );
                // Zero slippage is the identity; 100 % slippage floors to zero.
                if bps == 0 {
                    assert_eq!(floor, amount);
                }
                if bps == 10_000 {
                    assert_eq!(floor, 0);
                }
            }
        }
    }

    /// Zero edges and rounding boundaries.
    #[test]
    fn slippage_zero_edges_and_rounding() {
        assert_eq!(slippage_amount_bps(0, 100), 0);
        assert_eq!(slippage_floor_bps(0, 100), 0);
        // Sub-1-lamport discounts floor to zero (floor semantics).
        assert_eq!(
            slippage_amount_bps(9_999, 100),
            99,
            "9999*100/10000 = 99.99 -> 99"
        );
        assert_eq!(
            slippage_floor_bps(9_999, 100),
            9_899,
            "9999*9900/10000 = 9899.01 floors to 9899 (floor semantics)"
        );
        assert_eq!(slippage_amount_bps(10_000, 100), 100);
        assert_eq!(slippage_floor_bps(10_000, 100), 9_900);
    }

    /// Validation contract: the helper accepts the full 0..=10_000 range and
    /// nothing above.
    #[test]
    fn is_valid_slippage_bps_contract() {
        assert!(is_valid_slippage_bps(0));
        assert!(is_valid_slippage_bps(10));
        assert!(is_valid_slippage_bps(1_000));
        assert!(is_valid_slippage_bps(10_000));
        assert!(!is_valid_slippage_bps(10_001));
        assert!(!is_valid_slippage_bps(u64::MAX));
    }

    /// The strategy ceiling constant is stricter than the protocol ceiling.
    #[test]
    fn hop_slippage_max_is_stricter_than_protocol_max() {
        assert!(HOP_SLIPPAGE_MAX_BPS <= 10_000);
    }
}
