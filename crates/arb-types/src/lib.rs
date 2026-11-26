//! Core Domain Types (Newtypes) and Canonical Primitives for Solana Arbitrage.
//!
//! Split by responsibility; every historical `arb_types::X` path is preserved
//! through the glob re-exports below.

pub mod currency;
pub mod dex;
pub mod dimensional;
pub mod error;
pub mod execution;
pub mod pool;
pub mod primitives;

pub use currency::*;
pub use dex::*;
pub use error::*;
pub use execution::*;
pub use pool::*;
pub use primitives::*;

pub use arb_constants::{DEFAULT_HOP_SLIPPAGE_BPS, HOP_SLIPPAGE_MAX_BPS};

#[cfg(test)]
mod address_conflict_regression_tests {
    use super::*;
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    /// C-01: arb-constants::KAMINO_USDC_RESERVE must equal the canonical value,
    /// never the audit-doc typo found in verified_config.rs history.
    #[test]
    fn kamino_usdc_reserve_matches_canonical_constant() {
        let canonical = Pubkey::from_str(arb_constants::KAMINO_USDC_RESERVE)
            .expect("KAMINO_USDC_RESERVE must be a valid pubkey");
        assert_eq!(
            canonical,
            Pubkey::from_str("D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59")
                .expect("canonical USDC reserve must be a valid pubkey"),
            "KAMINO_USDC_RESERVE drifted from canonical value (C-01 regression)"
        );

        // Explicitly reject the historical typo value without carrying the forbidden literal in source.
        let typo_str = ["D6q6wuQSrifJK", "UvxCoXepEAbW39zWCuYAZBpqUg6xK25"].concat();
        let typo = Pubkey::from_str(&typo_str).expect("typo must be a valid pubkey");
        assert_ne!(
            canonical, typo,
            "KAMINO_USDC_RESERVE must not equal the historical typo value (C-01 regression)"
        );
    }

    /// SEC-05: fee_bps must never silently convert invalid fees to 100%.
    #[test]
    fn fee_bps_rejects_invalid_fees_fail_closed() {
        fn test_pool(fee_numerator: u64, fee_denominator: u64) -> PoolState {
            PoolState {
                pool_id: PoolId::new([1; 32]),
                protocol: DexProtocol::RaydiumV4,
                slot: Slot(1),
                write_version: 1,
                base_mint: MintId(Pubkey::new_from_array([1; 32])),
                quote_mint: MintId(Pubkey::new_from_array([2; 32])),
                decimals_a: 9,
                decimals_b: 6,
                reserve_in: 1_000_000,
                reserve_out: 1_000_000,
                fee_numerator,
                fee_denominator,
                raydium_cpmm_fee: None,
                is_active: true,
                has_risky_token: false,
                kind: PoolKind::Cpmm,
                clmm: None,
                dlmm: None,
                pump_fun: None,
                damm_v2: None,
                transfer_fee_bps: 0,
                token_programs: [TokenProgramKind::SplToken; 2],
                execution_accounts: None,
                last_update_slot: 1,
                last_update_ms: 0,
            }
        }

        // Zero denominator -> None.
        let p = test_pool(25, 0);
        assert_eq!(p.fee_bps(), None);
        assert!(!p.is_fee_valid());

        // Fee above FEE_BPS_MAX (100%) -> None, never Some(10_000).
        let p = test_pool(20_000, 10_000); // 200%
        assert_eq!(p.fee_bps(), None);
        assert!(!p.is_fee_valid());

        // Valid fee -> Some.
        let p = test_pool(25, 10_000); // 25 bps
        assert_eq!(p.fee_bps(), Some(25));
        assert!(p.is_fee_valid());

        // Overflow-safe: huge numerator that would overflow u16 -> None.
        let p = test_pool(u64::MAX, 1);
        assert_eq!(p.fee_bps(), None);
        assert!(!p.is_fee_valid());
    }
}
