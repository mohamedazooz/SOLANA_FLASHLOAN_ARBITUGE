//! //! Flash-loan-funded strategy assemblers: Kamino funding, DLMM bin-array
//! planning and multi-leg atomic arbitrage compilation.

pub mod arb_executor;
pub mod compiler;
pub mod cross_dex_flash_arb;
pub mod dlmm_3leg_flash_arb;
pub mod dlmm_4leg_flash_arb;
pub mod dlmm_bin_discovery;
pub mod dlmm_flash_arb;
pub mod kamino;
pub mod kamino_flash;
pub mod meteora_atomic_builder;
pub mod provider;

pub use arb_executor::*;
pub use compiler::*;
pub use cross_dex_flash_arb::*;
pub use dlmm_3leg_flash_arb::*;
pub use dlmm_4leg_flash_arb::*;
pub use dlmm_bin_discovery::*;
pub use dlmm_flash_arb::*;
pub use kamino::{
    KAMINO_FRACTION_FRAC_BITS, KAMINO_FRACTION_ONE_BITS, KaminoFeeError, KaminoFlashAccounts,
    KaminoReadError, KaminoReserveCandidate, calculate_kamino_flash_fee,
    calculate_kamino_flash_fee_checked, decode_reserve_candidate, decode_reserve_fee_vault,
    decode_reserve_flash_fee_sf, decode_reserve_supply_vault, decode_supply_vault_liquidity,
    derive_lending_market_authority, select_kamino_flash_reserve,
};
pub use kamino_flash::*;
pub use meteora_atomic_builder::*;
pub use provider::*;

//experimental: gated — was merged unbuildable in PR #73 (depends on
//arb-engine upward + ambiguous AccountMeta under the flash glob re-exports).
//Repair behind this feature before promoting it; never compiled by default.
#[cfg(feature = "pump-v1-legacy-builder")]
pub mod pump_raydium_builder;
#[cfg(feature = "pump-v1-legacy-builder")]
pub use pump_raydium_builder::*;
