//! DEX State Deserialization, Binary Parsing, and Unified Quoting Engine.

pub mod orca_whirlpool;
pub mod pump_fun;
pub mod pump_swap;
pub mod raydium_v4;
pub mod traits;

pub use orca_whirlpool::OrcaWhirlpool;
pub use pump_fun::PumpFunBondingCurve;
pub use pump_swap::{PumpSwapFeeConfig, PumpSwapFeeTier, PumpSwapPool, PumpSwapPoolHeader};
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

    if mint_data.len() == 82 || mint_data.len() < TLV_START
        || mint_data.get(165).copied()? != ACCOUNT_TYPE_MINT {
        return None;
    }
    let mut offset = TLV_START;
    while offset.checked_add(4)? <= mint_data.len() {
        if mint_data[offset..].iter().all(|b| *b == 0) { return None; }
        let ty = u16::from_le_bytes([*mint_data.get(offset)?, *mint_data.get(offset + 1)?]);
        let len = u16::from_le_bytes([*mint_data.get(offset + 2)?, *mint_data.get(offset + 3)?]) as usize;
        let start = offset.checked_add(4)?;
        let end = start.checked_add(len)?;
        if end > mint_data.len() || end <= offset { return None; }
        if ty == TRANSFER_FEE_CONFIG_TYPE {
            if len != CONFIG_LEN { return None; }
            let value = &mint_data[start..end];
            let read_u64 = |i: usize| -> Option<u64> { Some(u64::from_le_bytes(value.get(i..i+8)?.try_into().ok()?)) };
            let read_u16 = |i: usize| -> Option<u16> { Some(u16::from_le_bytes(value.get(i..i+2)?.try_into().ok()?)) };
            let older = TransferFeeSchedule { epoch: read_u64(72)?, maximum_fee: read_u64(80)?, basis_points: read_u16(88)? };
            let newer = TransferFeeSchedule { epoch: read_u64(90)?, maximum_fee: read_u64(98)?, basis_points: read_u16(106)? };
            if older.basis_points > 10_000 || newer.basis_points > 10_000 { return None; }
            return Some(TransferFeeConfig { older, newer });
        }
        offset = end;
    }
    None
}

/// Calculate Token-2022's withheld transfer fee: ceil(amount*bps/10_000), capped.
pub fn calculate_transfer_fee(amount: u64, schedule: TransferFeeSchedule) -> Option<u64> {
    if schedule.basis_points > 10_000 { return None; }
    if amount == 0 || schedule.basis_points == 0 { return Some(0); }
    let numerator = (amount as u128).checked_mul(schedule.basis_points as u128)?;
    let fee = numerator.checked_add(9_999)? / 10_000;
    Some((fee.min(schedule.maximum_fee as u128)) as u64)
}

/// Select the active schedule using the Clock sysvar epoch.
pub fn active_transfer_fee(config: TransferFeeConfig, current_epoch: u64) -> TransferFeeSchedule {
    if current_epoch >= config.newer.epoch { config.newer } else { config.older }
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

    let mut offset = TLV_START;
    while offset < data.len() {
        // Remaining all-zero bytes are padding; a partial non-zero TLV is invalid.
        if data[offset..].iter().all(|byte| *byte == 0) {
            return false;
        }
        if data.len() - offset < 4 {
            return true;
        }

        let extension_type = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let extension_len = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
        if extension_type == 0 {
            // Uninitialized marker is only valid if the rest is zero padding.
            return !data[offset..].iter().all(|byte| *byte == 0);
        }

        let Some(value_start) = offset.checked_add(4) else {
            return true;
        };
        let Some(next) = value_start.checked_add(extension_len) else {
            return true;
        };
        if next > data.len() || next <= offset {
            return true;
        }

        // No Token-2022 extensions are currently enabled for execution. This
        // deliberately includes known and future extension IDs.
        return true;
    }
    false
}

impl CpmmDecoder {
    pub fn new() -> Self {
        Self
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
        let schedule = TransferFeeSchedule { epoch: 0, maximum_fee: 7, basis_points: 150 };
        assert_eq!(calculate_transfer_fee(1, schedule), Some(1));
        assert_eq!(calculate_transfer_fee(100, schedule), Some(2));
        assert_eq!(calculate_transfer_fee(10_000, schedule), Some(7));
    }

    #[test]
    fn active_transfer_fee_uses_epoch_boundary() {
        let config = TransferFeeConfig {
            older: TransferFeeSchedule { epoch: 0, maximum_fee: 10, basis_points: 100 },
            newer: TransferFeeSchedule { epoch: 12, maximum_fee: 20, basis_points: 200 },
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
