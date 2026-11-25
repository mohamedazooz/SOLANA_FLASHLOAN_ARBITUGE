//! DEX protocol taxonomy and pool execution account shapes.

use super::pool::PoolKind;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

/// Supported Solana DEX Protocols.
///
/// Every variant is an independent adapter: its own program id, instruction
/// format, fee model and pricing curve. They are deliberately NOT treated as
/// interchangeable Uniswap V2/V3 clones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DexProtocol {
    /// Raydium constant-product AMM (legacy program, Serum/OpenBook accounts).
    RaydiumV4,
    /// Raydium CP-Swap (`CPMMoo8…`): constant product with an Anchor API.
    RaydiumCpmm,
    /// Raydium concentrated liquidity (Q64.64 ticks).
    RaydiumClmm,
    /// Orca Whirlpools (concentrated liquidity).
    OrcaWhirlpool,
    /// Meteora DLMM (discrete bin ladder with volatility-scaled fees).
    MeteoraDlmm,
    /// Meteora DAMM v2 (`cp_amm`, `cpamdp…`): configurable constant/concentrated AMM.
    MeteoraDammV2,
    /// Meteora Dynamic Bonding Curve (`dbcij…`): 16-point piecewise launchpad.
    MeteoraDbc,
    /// Raydium LaunchLab (`LanMV9…`): bonding curve launchpad.
    RaydiumLaunchLab,
    /// Pump.fun bonding curve constant-product AMM.
    PumpFun,
    /// PumpSwap post-graduation constant-product AMM.
    PumpSwap,
}

impl DexProtocol {
    /// Canonical protocol name used by the route monitor, web API and logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RaydiumV4 => "raydium_v4",
            Self::RaydiumCpmm => "raydium_cpmm",
            Self::RaydiumClmm => "raydium_clmm",
            Self::OrcaWhirlpool => "orca_whirlpool",
            Self::MeteoraDlmm => "meteora_dlmm",
            Self::MeteoraDammV2 => "meteora_damm_v2",
            Self::MeteoraDbc => "meteora_dbc",
            Self::RaydiumLaunchLab => "raydium_launchlab",
            Self::PumpFun => "pump_fun",
            Self::PumpSwap => "pump_swap",
        }
    }

    /// Pricing family. Determines which curve the quote model must use.
    ///
    /// Program ids live only in `arb-constants` (single source of truth); this
    /// crate deliberately carries no copy of them.
    pub const fn family(self) -> PoolKind {
        match self {
            Self::RaydiumV4
            | Self::RaydiumCpmm
            | Self::MeteoraDammV2
            | Self::MeteoraDbc
            | Self::RaydiumLaunchLab
            | Self::PumpFun
            | Self::PumpSwap => PoolKind::Cpmm,
            Self::RaydiumClmm | Self::OrcaWhirlpool => PoolKind::Clmm,
            Self::MeteoraDlmm => PoolKind::Dlmm,
        }
    }
}

/// Token program owning a given mint/token account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TokenProgramKind {
    #[default]
    SplToken,
    Token2022,
}

impl TokenProgramKind {
    pub fn program_id(self) -> solana_pubkey::Pubkey {
        use std::str::FromStr;
        match self {
            Self::SplToken => solana_pubkey::Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)
                .expect("valid SPL token program id"),
            Self::Token2022 => {
                solana_pubkey::Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID)
                    .expect("valid Token-2022 program id")
            }
        }
    }
}

/// Raydium AMM V4 swap instruction schema.
///
/// Raydium added `SwapBaseInV2` (opcode 16, 8 accounts, no OpenBook/Serum
/// dependencies) after the original `SwapBaseIn` (opcode 9, 18 accounts). The
/// V2 schema is the default for every pool that supports it. V1 remains an
/// explicit, opt-in compatibility path for older pools; it is never selected
/// silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaydiumV4SwapVersion {
    /// Canonical `SwapBaseInV2` — opcode 16, 8 accounts.
    #[default]
    V2,
    /// Legacy `SwapBaseIn` — opcode 9, 18 accounts (OpenBook/Serum required).
    V1,
}

impl RaydiumV4SwapVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::V1 => "v1",
        }
    }
}

/// PumpSwap instruction generation supported by a pool.
///
/// V2 is the canonical schema for current pools (17 accounts, `*_v2`
/// discriminators). V1 is the legacy schema (16 accounts, `buy`/`sell`
/// discriminators, requires the Pump Fees `fee_config` account). A pool is only
/// built with V1 when it is explicitly marked as such; V2 never silently
/// degrades to V1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PumpSwapPoolVersion {
    /// Current `buy_v2` / `sell_v2` / `buy_exact_quote_in_v2` schema.
    #[default]
    V2,
    /// Legacy `buy` / `sell` schema with `fee_config`.
    V1Legacy,
}

impl PumpSwapPoolVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::V1Legacy => "v1_legacy",
        }
    }
}

/// PumpSwap swap direction intent — whether the hop spends an exact input
/// amount or requires an exact output amount.
///
/// `ExactIn` is the canonical default (spend exactly `amount_in`, receive at
/// least `min_amount_out`). `ExactOutBase` requires exactly `min_amount_out`
/// base tokens and spends at most `amount_in` quote tokens.
///
/// Sell + ExactOutBase is not supported by PumpSwap and must fail closed — the
/// program only offers `buy`/`sell` (exact-in) and `buy_exact_base_out_v2`
/// (buy exact-out) discriminators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PumpSwapTradeIntent {
    /// Spend exact input, receive minimum output (canonical path).
    #[default]
    ExactIn,
    /// Receive exact base output, spend at most the given quote input.
    ExactOutBase,
}

/// Verified per-pool on-chain execution accounts resolved during ingestion/decoding.
/// Eliminates hardcoded SOL-USDC account fallbacks and invalid PDA guesses in `arb-tx`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolExecutionAccounts {
    RaydiumV4 {
        amm_authority: Pubkey,
        open_orders: Pubkey,
        target_orders: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        serum_program: Pubkey,
        serum_market: Pubkey,
        serum_bids: Pubkey,
        serum_asks: Pubkey,
        serum_event_queue: Pubkey,
        serum_coin_vault: Pubkey,
        serum_pc_vault: Pubkey,
        serum_vault_signer: Pubkey,
        /// Which instruction schema this pool must use. Defaults to V2.
        #[serde(default)]
        swap_version: RaydiumV4SwapVersion,
    },
    RaydiumCpmm {
        amm_config: Pubkey,
        authority: Pubkey,
        vault_0: Pubkey,
        vault_1: Pubkey,
        observation_state: Pubkey,
    },
    RaydiumClmm {
        amm_config: Pubkey,
        vault_0: Pubkey,
        vault_1: Pubkey,
        observation_state: Pubkey,
        tick_array_pdas: Vec<Pubkey>,
        bitmap_extension: Option<Pubkey>,
        /// On-chain `token_mint_0` (@73..105). Defines swap direction:
        /// `zero_for_one = (mint_in == pool_mint_0)`. Raydium does NOT enforce
        /// `mint_0 < mint_1` on live pools (probe-verified), so lexicographic
        /// ordering is not a valid direction rule. `#[serde(default)]` keeps
        /// previously persisted routes deserializable; default pubkey is
        /// rejected fail-closed at construction time.
        #[serde(default)]
        pool_mint_0: Pubkey,
    },
    OrcaWhirlpool {
        vault_a: Pubkey,
        vault_b: Pubkey,
        oracle: Pubkey,
        tick_array_pdas: Vec<Pubkey>,
        /// On-chain `token_mint_a` (@101..133). Defines swap direction:
        /// `a_to_b = (mint_in == pool_mint_a)`. Orca does NOT enforce
        /// `mint_a < mint_b` on live pools (probe-verified). `#[serde(default)]`
        /// keeps previously persisted routes deserializable; default pubkey is
        /// rejected fail-closed at construction time.
        #[serde(default)]
        pool_mint_a: Pubkey,
    },
    MeteoraDlmm {
        reserve_x: Pubkey,
        reserve_y: Pubkey,
        oracle: Pubkey,
        bin_array_pdas: Vec<Pubkey>,
        bitmap_extension: Option<Pubkey>,
        /// On-chain LbPair `token_x_mint` (@88..120) and `token_y_mint`
        /// (@120..152). The DLMM program validates these accounts against the
        /// pair (official SDK always passes `pool.tokenX/tokenY` in the fixed
        /// x/y slots, never `inToken/outToken`), so passing the hop's
        /// `mint_in` as `token_x_mint` fails for any Y->X direction.
        /// `#[serde(default)]` keeps previously persisted routes
        /// deserializable; default pubkeys are rejected fail-closed at build
        /// time.
        #[serde(default)]
        pool_token_x: Pubkey,
        #[serde(default)]
        pool_token_y: Pubkey,
    },
    MeteoraDammV2 {
        config: Pubkey,
        authority: Pubkey,
        vault_a: Pubkey,
        vault_b: Pubkey,
        event_authority: Pubkey,
    },
    MeteoraDbc {
        config: Pubkey,
        authority: Pubkey,
        vault_base: Pubkey,
        vault_quote: Pubkey,
        event_authority: Pubkey,
    },
    RaydiumLaunchLab {
        authority: Pubkey,
        global_config: Pubkey,
        platform_config: Pubkey,
        base_vault: Pubkey,
        quote_vault: Pubkey,
        event_authority: Pubkey,
    },
    PumpFun {
        creator: Pubkey,
        fee_recipient: Pubkey,
    },
    PumpSwap {
        global_config: Pubkey,
        pool_base_vault: Pubkey,
        pool_quote_vault: Pubkey,
        protocol_fee_recipient: Pubkey,
        protocol_fee_recipient_ata: Pubkey,
        coin_creator_vault_authority: Pubkey,
        coin_creator_vault_ata: Pubkey,
        fee_config: Pubkey,
        /// Which PumpSwap instruction generation the pool supports.
        #[serde(default)]
        pool_version: PumpSwapPoolVersion,
    },
}
