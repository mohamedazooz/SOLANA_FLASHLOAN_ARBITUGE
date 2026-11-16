//! DEX State Deserialization, Binary Parsing, and Unified Quoting Engine.

pub mod dlmm_validation;
pub mod meteora_damm_v2;
pub mod meteora_dlmm;
pub mod orca_whirlpool;
pub mod pump_fun;
pub mod pump_swap;
pub mod raydium_clmm;
pub mod raydium_cpmm;
pub mod raydium_launchlab;
pub mod raydium_v4;
pub mod traits;

pub use dlmm_validation::{
    DlmmValidationError, DlmmValidationResult, ValidatedDlmmPool, canonical_mint_pair,
    derive_bin_array_pda, derive_bitmap_extension_pda, derive_event_authority, derive_oracle_pda,
    derive_reserve_pda, validate_bin_array_account, validate_bitmap_extension_account,
    validate_lb_pair_account, validate_pool_pair_compatibility,
};
pub use meteora_damm_v2::MeteoraDammV2Pool;
pub use meteora_dlmm::MeteoraDlmmPool;
pub use orca_whirlpool::OrcaWhirlpool;
pub use pump_fun::PumpFunBondingCurve;
pub use pump_swap::{PumpSwapFeeConfig, PumpSwapFeeTier, PumpSwapPool, PumpSwapPoolHeader};
pub use raydium_clmm::RaydiumClmmPool;
pub use raydium_cpmm::RaydiumCpmmPool;
pub use raydium_launchlab::RaydiumLaunchLabPool;
pub use raydium_v4::RaydiumV4Pool;
pub use traits::AmmPool;

/// Parsed Token-2022 TransferFeeConfig from a mint account's TLV region.
/// Values are raw token units; `epoch` and fee fields follow the on-chain POD layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferFeeSchedule {
    pub epoch: u64,
    pub maximum_fee: u64,
    pub basis_points: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferFeeConfig {
    pub older: TransferFeeSchedule,
    pub newer: TransferFeeSchedule,
}

/// Decode the Token-2022 TransferFeeConfig extension (extension type 1).
/// Returns None for legacy mints, absent extension, or malformed/unsupported layout.
pub fn decode_transfer_fee_config(mint_data: &[u8]) -> Option<TransferFeeConfig> {
    const TLV_START: usize = 166;
    const ACCOUNT_TYPE_MINT: u8 = 1;
    const TRANSFER_FEE_CONFIG_TYPE: u16 = 1;
    const CONFIG_LEN: usize = 108;

    if mint_data.len() == 82
        || mint_data.len() < TLV_START
        || mint_data.get(165).copied()? != ACCOUNT_TYPE_MINT
    {
        return None;
    }
    let mut offset = TLV_START;
    while offset.checked_add(4)? <= mint_data.len() {
        if mint_data[offset..].iter().all(|b| *b == 0) {
            return None;
        }
        let ty = u16::from_le_bytes([*mint_data.get(offset)?, *mint_data.get(offset + 1)?]);
        let len =
            u16::from_le_bytes([*mint_data.get(offset + 2)?, *mint_data.get(offset + 3)?]) as usize;
        let start = offset.checked_add(4)?;
        let end = start.checked_add(len)?;
        if end > mint_data.len() || end <= offset {
            return None;
        }
        if ty == TRANSFER_FEE_CONFIG_TYPE {
            if len != CONFIG_LEN {
                return None;
            }
            let value = &mint_data[start..end];
            let read_u64 = |i: usize| -> Option<u64> {
                Some(u64::from_le_bytes(value.get(i..i + 8)?.try_into().ok()?))
            };
            let read_u16 = |i: usize| -> Option<u16> {
                Some(u16::from_le_bytes(value.get(i..i + 2)?.try_into().ok()?))
            };
            let older = TransferFeeSchedule {
                epoch: read_u64(72)?,
                maximum_fee: read_u64(80)?,
                basis_points: read_u16(88)?,
            };
            let newer = TransferFeeSchedule {
                epoch: read_u64(90)?,
                maximum_fee: read_u64(98)?,
                basis_points: read_u16(106)?,
            };
            if older.basis_points > 10_000 || newer.basis_points > 10_000 {
                return None;
            }
            return Some(TransferFeeConfig { older, newer });
        }
        offset = end;
    }
    None
}

/// Calculate Token-2022's withheld transfer fee: ceil(amount*bps/10_000), capped.
pub fn calculate_transfer_fee(amount: u64, schedule: TransferFeeSchedule) -> Option<u64> {
    if schedule.basis_points > 10_000 {
        return None;
    }
    if amount == 0 || schedule.basis_points == 0 {
        return Some(0);
    }
    let numerator = (amount as u128).checked_mul(schedule.basis_points as u128)?;
    let fee = numerator.checked_add(9_999)? / 10_000;
    Some((fee.min(schedule.maximum_fee as u128)) as u64)
}

/// Select the active schedule using the Clock sysvar epoch.
pub fn active_transfer_fee(config: TransferFeeConfig, current_epoch: u64) -> TransferFeeSchedule {
    if current_epoch >= config.newer.epoch {
        config.newer
    } else {
        config.older
    }
}

/// CPMM decoder helper.
#[derive(Debug, Default, Clone)]
pub struct CpmmDecoder;

/// Conservative Token-2022 mint extension scanner.
///
/// Token-2022 mint extensions start after the 82-byte Mint base, the 83-byte
/// padding, and the account-type byte: TLV data therefore starts at offset 166.
/// This bot currently has no complete quote/transfer-hook model for Token-2022,
/// so any non-empty extension set is treated as unsupported. Malformed layouts
/// are also treated as unsupported (fail closed).
pub fn token2022_has_risky_extension(data: &[u8]) -> bool {
    const MINT_BASE_LEN: usize = 82;
    const ACCOUNT_TYPE_OFFSET: usize = 165;
    const TLV_START: usize = 166;
    const ACCOUNT_TYPE_MINT: u8 = 1;

    // Legacy SPL mint bytes have no Token-2022 extension trailer.
    if data.len() == MINT_BASE_LEN {
        return false;
    }

    // Anything between the base Mint and the canonical Token-2022 trailer is
    // malformed/ambiguous. Do not interpret it as an extension-free mint.
    if data.len() < TLV_START || data[ACCOUNT_TYPE_OFFSET] != ACCOUNT_TYPE_MINT {
        return true;
    }

    // No Token-2022 extensions are currently enabled for execution. Treat any
    // non-zero TLV bytes as unsupported; this also fails closed for malformed TLV.
    data[TLV_START..].iter().any(|byte| *byte != 0)
}

impl CpmmDecoder {
    pub fn new() -> Self {
        Self
    }
}

use arb_constants::{
    METEORA_DLMM_PROGRAM_ID, ORCA_WHIRLPOOL_PROGRAM_ID, PUMP_FUN_PROGRAM_ID, PUMP_SWAP_PROGRAM_ID,
    RAYDIUM_CLMM_PROGRAM_ID, RAYDIUM_CPMM_PROGRAM_ID, RAYDIUM_V4_PROGRAM_ID,
};
use arb_types::{DexProtocol, MintId, PoolId};
use solana_pubkey::Pubkey;

/// Map a program ID (account owner) to its DEX protocol, if known.
pub fn program_id_to_protocol(program_id: &Pubkey) -> Option<DexProtocol> {
    let id_str = program_id.to_string();
    match id_str.as_str() {
        RAYDIUM_V4_PROGRAM_ID => Some(DexProtocol::RaydiumV4),
        RAYDIUM_CPMM_PROGRAM_ID => Some(DexProtocol::RaydiumCpmm),
        RAYDIUM_CLMM_PROGRAM_ID => Some(DexProtocol::RaydiumClmm),
        ORCA_WHIRLPOOL_PROGRAM_ID => Some(DexProtocol::OrcaWhirlpool),
        METEORA_DLMM_PROGRAM_ID => Some(DexProtocol::MeteoraDlmm),
        PUMP_FUN_PROGRAM_ID => Some(DexProtocol::PumpFun),
        PUMP_SWAP_PROGRAM_ID => Some(DexProtocol::PumpSwap),
        _ => None,
    }
}

/// Decode raw account bytes into an `AmmPool` instance for a given protocol and pool address.
pub fn decode_amm_pool(
    protocol: DexProtocol,
    pool_id: PoolId,
    data: &[u8],
) -> Option<Box<dyn AmmPool>> {
    let mut pool: Box<dyn AmmPool> = match protocol {
        DexProtocol::RaydiumV4 => Box::new(RaydiumV4Pool::new(
            pool_id,
            MintId::default(),
            MintId::default(),
            Pubkey::default(),
            Pubkey::default(),
            Pubkey::default(),
        )),
        DexProtocol::RaydiumCpmm => {
            if data.len() < raydium_cpmm::RAYDIUM_CPMM_POOL_MIN_SIZE {
                return None;
            }
            Box::new(RaydiumCpmmPool::from_account(pool_id, data).ok()?)
        }
        DexProtocol::RaydiumClmm => {
            if data.len() < raydium_clmm::RAYDIUM_CLMM_POOL_MIN_SIZE {
                return None;
            }
            Box::new(RaydiumClmmPool::from_account(pool_id, data).ok()?)
        }
        DexProtocol::OrcaWhirlpool => Box::new(OrcaWhirlpool::new(
            pool_id,
            MintId::default(),
            MintId::default(),
            Pubkey::default(),
            Pubkey::default(),
            64,
            3000,
        )),
        DexProtocol::MeteoraDlmm => {
            if data.len() < meteora_dlmm::METEORA_LB_PAIR_MIN_SIZE {
                return None;
            }
            Box::new(MeteoraDlmmPool::from_account(pool_id, data).ok()?)
        }
        DexProtocol::PumpFun => {
            if data.len() < pump_fun::PUMP_FUN_BONDING_CURVE_SIZE {
                return None;
            }
            Box::new(PumpFunBondingCurve::new(
                pool_id,
                MintId::default(),
                MintId::default(),
            ))
        }
        _ => return None,
    };

    if pool.update_from_account(data).is_ok() {
        Some(pool)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{MintId, PoolId};
    use solana_pubkey::Pubkey;

    #[test]
    fn raydium_v4_basic_quote() {
        let mut pool = RaydiumV4Pool::new(
            PoolId::new([1; 32]),
            MintId::new([2; 32]),
            MintId::new([3; 32]),
            Pubkey::new_from_array([4; 32]),
            Pubkey::new_from_array([5; 32]),
            Pubkey::new_from_array([6; 32]),
        );
        pool.update_reserves(1_000_000, 2_000_000);
        pool.fee_numerator = 25;
        pool.fee_denominator = 10_000;
        pool.status = 6;

        let quote = pool.quote(10_000, true).unwrap();
        assert!(quote > 0);
        assert!(quote < 20_000);
    }

    #[test]
    fn token2022_scanner_accepts_legacy_mint_and_rejects_malformed_layouts() {
        assert!(!token2022_has_risky_extension(&[0u8; 82]));
        assert!(token2022_has_risky_extension(&[0u8; 165]));
        let mut wrong_type = vec![0u8; 166];
        wrong_type[165] = 2; // Token account, not mint
        assert!(token2022_has_risky_extension(&wrong_type));
    }

    #[test]
    fn token2022_scanner_accepts_extensionless_mint_and_rejects_any_extension() {
        let mut extensionless = vec![0u8; 166];
        extensionless[165] = 1; // Mint account type
        assert!(!token2022_has_risky_extension(&extensionless));

        let mut with_extension = extensionless;
        with_extension.extend_from_slice(&[1, 0, 1, 0, 7]); // type=1, len=1, value
        assert!(token2022_has_risky_extension(&with_extension));
    }

    #[test]
    fn token2022_scanner_rejects_truncated_tlv() {
        let mut truncated = vec![0u8; 166];
        truncated[165] = 1;
        truncated.extend_from_slice(&[1, 0, 8, 0, 7]); // claims 8 bytes, only 1 present
        assert!(token2022_has_risky_extension(&truncated));
    }

    #[test]
    fn orca_quote_rejects_missing_tick_arrays() {
        let mut pool = OrcaWhirlpool::new(
            PoolId::new([1; 32]),
            MintId::new([2; 32]),
            MintId::new([3; 32]),
            Pubkey::new_from_array([4; 32]),
            Pubkey::new_from_array([5; 32]),
            64,
            3000,
        );
        pool.liquidity = 1_000_000;
        pool.sqrt_price_x64 = 1u128 << 64;
        assert!(pool.quote(1_000, true).is_err());
    }

    #[test]
    fn orca_whirlpool_basic() {
        let pool = OrcaWhirlpool::new(
            PoolId::new([1; 32]),
            MintId::new([2; 32]),
            MintId::new([3; 32]),
            Pubkey::new_from_array([4; 32]),
            Pubkey::new_from_array([5; 32]),
            64,
            3000, // 0.3%
        );
        assert_eq!(pool.fee_bps(), Some(30));
    }

    #[test]
    fn transfer_fee_rounds_up_and_caps() {
        let schedule = TransferFeeSchedule {
            epoch: 0,
            maximum_fee: 7,
            basis_points: 150,
        };
        assert_eq!(calculate_transfer_fee(1, schedule), Some(1));
        assert_eq!(calculate_transfer_fee(100, schedule), Some(2));
        assert_eq!(calculate_transfer_fee(10_000, schedule), Some(7));
    }

    #[test]
    fn active_transfer_fee_uses_epoch_boundary() {
        let config = TransferFeeConfig {
            older: TransferFeeSchedule {
                epoch: 0,
                maximum_fee: 10,
                basis_points: 100,
            },
            newer: TransferFeeSchedule {
                epoch: 12,
                maximum_fee: 20,
                basis_points: 200,
            },
        };
        assert_eq!(active_transfer_fee(config, 11).basis_points, 100);
        assert_eq!(active_transfer_fee(config, 12).basis_points, 200);
    }

    #[test]
    fn transfer_fee_config_decoder_reads_onchain_layout() {
        let mut data = vec![0u8; 166 + 4 + 108];
        data[165] = 1;
        data[166..168].copy_from_slice(&1u16.to_le_bytes());
        data[168..170].copy_from_slice(&108u16.to_le_bytes());
        let v = &mut data[170..];
        v[72..80].copy_from_slice(&3u64.to_le_bytes());
        v[80..88].copy_from_slice(&50u64.to_le_bytes());
        v[88..90].copy_from_slice(&100u16.to_le_bytes());
        v[90..98].copy_from_slice(&5u64.to_le_bytes());
        v[98..106].copy_from_slice(&75u64.to_le_bytes());
        v[106..108].copy_from_slice(&200u16.to_le_bytes());
        let cfg = decode_transfer_fee_config(&data).unwrap();
        assert_eq!(cfg.older.epoch, 3);
        assert_eq!(cfg.newer.maximum_fee, 75);
        assert_eq!(cfg.newer.basis_points, 200);
    }
}
