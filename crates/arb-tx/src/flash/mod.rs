//! //! Flash-loan-funded strategy assemblers: Kamino funding, DLMM bin-array
//! planning and multi-leg atomic arbitrage compilation.

pub mod compiler;
pub mod cross_dex_flash_arb;
pub mod dlmm_3leg_flash_arb;
pub mod dlmm_4leg_flash_arb;
pub mod dlmm_bin_discovery;
pub mod dlmm_flash_arb;
pub mod kamino;
pub mod kamino_flash;
pub mod meteora_atomic_builder;

pub use compiler::*;
pub use cross_dex_flash_arb::*;
pub use dlmm_3leg_flash_arb::*;
pub use dlmm_4leg_flash_arb::*;
pub use dlmm_bin_discovery::*;
pub use dlmm_flash_arb::*;
pub use kamino::{
    KAMINO_FRACTION_FRAC_BITS, KAMINO_FRACTION_ONE_BITS, KaminoFeeError, KaminoFlashAccounts,
    KaminoReadError, calculate_kamino_flash_fee, calculate_kamino_flash_fee_checked,
    decode_reserve_flash_fee_sf, decode_supply_vault_liquidity, derive_lending_market_authority,
};
pub use kamino_flash::*;
pub use meteora_atomic_builder::*;
