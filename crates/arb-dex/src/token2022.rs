//! SPL Token-2022 Extension Parsing, Fee Computation, and Extension Security Gate.
//!
//! Enforces:
//! - Exact transfer fee calculation matching SPL Token-2022 program specification:
//!   `fee = min(ceil(amount * fee_bps / 10,000), max_fee)`
//! - Strict extension validation: rejects unverified TransferHook, PermanentDelegate,
//!   NonTransferable, DefaultAccountState (Frozen), and MintCloseAuthority.
//! - Sequential multi-hop net calculation:
//!   Input sent -> Input Transfer Fee -> Pool Net Received -> Swap Math -> Output Gross -> Output Transfer Fee -> Final Hop Net.

use solana_pubkey::Pubkey;

/// Extension type IDs defined by SPL Token-2022.
pub const EXTENSION_UNINITIALIZED: u16 = 0;
pub const EXTENSION_TRANSFER_FEE_CONFIG: u16 = 1;
pub const EXTENSION_TRANSFER_FEE_AMOUNT: u16 = 2;
pub const EXTENSION_MINT_CLOSE_AUTHORITY: u16 = 3;
pub const EXTENSION_CONFIDENTIAL_TRANSFER_MINT: u16 = 4;
pub const EXTENSION_DEFAULT_ACCOUNT_STATE: u16 = 5;
pub const EXTENSION_IMMUTABLE_OWNER: u16 = 6;
pub const EXTENSION_NON_TRANSFERABLE: u16 = 7;
pub const EXTENSION_INTEREST_BEARING_MINT: u16 = 8;
pub const EXTENSION_PERMANENT_DELEGATE: u16 = 10;
pub const EXTENSION_TRANSFER_HOOK: u16 = 12;

/// Standard SPL Token mint data size (without extensions).
pub const BASE_MINT_DATA_SIZE: usize = 82;

/// Dangerous extensions that make atomic flash-loan arbitrage execution unsafe or non-deterministic.
pub const DANGEROUS_EXTENSIONS: &[u16] = &[
    EXTENSION_MINT_CLOSE_AUTHORITY,
    EXTENSION_DEFAULT_ACCOUNT_STATE,
    EXTENSION_NON_TRANSFERABLE,
    EXTENSION_PERMANENT_DELEGATE,
    EXTENSION_TRANSFER_HOOK, // Unless explicitly verified whitelisted hook program
];

/// Parsed TransferFeeConfig representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferFeeConfig {
    pub transfer_fee_basis_points: u16,
    pub maximum_fee: u64,
}

impl TransferFeeConfig {
    pub const ZERO: Self = Self {
        transfer_fee_basis_points: 0,
        maximum_fee: 0,
    };

    /// Calculate exact transfer fee for a given gross amount matching SPL Token-2022 on-chain math:
    /// `fee = min(ceil((amount * fee_bps) / 10,000), max_fee)`
    pub fn calculate_fee(&self, amount: u64) -> u64 {
        if self.transfer_fee_basis_points == 0 || amount == 0 {
            return 0;
        }

        let num = (amount as u128).saturating_mul(self.transfer_fee_basis_points as u128);
        let denom = 10_000u128;
        // Ceiling division: (num + denom - 1) / denom
        let raw_fee = (num.saturating_add(denom - 1)) / denom;
        let fee_u64 = u64::try_from(raw_fee).unwrap_or(amount);

        fee_u64.min(self.maximum_fee)
    }

    /// Calculate net amount received by destination after transfer fee deduction:
    pub fn calculate_net_received(&self, amount: u64) -> u64 {
        let fee = self.calculate_fee(amount);
        amount.saturating_sub(fee)
    }

    /// Calculate gross input amount required so that the recipient receives exactly `target_net`:
    pub fn calculate_gross_required(&self, target_net: u64) -> Option<u64> {
        if self.transfer_fee_basis_points == 0 {
            return Some(target_net);
        }
        if self.transfer_fee_basis_points >= 10_000 {
            return None; // 100% fee, impossible to deliver net
        }

        // target_net <= gross - min(ceil(gross * bps / 10_000), max_fee)
        // If capped by max_fee: gross = target_net + max_fee
        let gross_if_capped = target_net.checked_add(self.maximum_fee)?;
        let fee_at_capped = self.calculate_fee(gross_if_capped);
        if fee_at_capped == self.maximum_fee {
            return Some(gross_if_capped);
        }

        // Otherwise solve: target_net = gross - ceil(gross * bps / 10000)
        // gross approx target_net / (1 - bps / 10000)
        let num = (target_net as u128).checked_mul(10_000)?;
        let denom = 10_000u128.saturating_sub(self.transfer_fee_basis_points as u128);
        if denom == 0 {
            return None;
        }
        let mut gross = u64::try_from(num.div_ceil(denom)).ok()?;

        // Verify and refine if off by 1 due to ceiling rounding
        while self.calculate_net_received(gross) < target_net {
            gross = gross.checked_add(1)?;
        }
        Some(gross)
    }
}

/// Token semantics validation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSemantics {
    pub mint: Pubkey,
    pub is_token_2022: bool,
    pub transfer_fee_config: TransferFeeConfig,
    pub detected_extensions: Vec<u16>,
    pub is_execution_eligible: bool,
    pub rejection_reason: Option<String>,
}

impl TokenSemantics {
    /// Parse token mint account data and validate extensions.
    pub fn parse(mint: Pubkey, data: &[u8]) -> Self {
        if data.len() < BASE_MINT_DATA_SIZE {
            return Self {
                mint,
                is_token_2022: false,
                transfer_fee_config: TransferFeeConfig::ZERO,
                detected_extensions: Vec::new(),
                is_execution_eligible: false,
                rejection_reason: Some(format!(
                    "Account data length {} < base mint size 82",
                    data.len()
                )),
            };
        }

        // Standard legacy SPL Token mint is exactly 82 bytes
        if data.len() == BASE_MINT_DATA_SIZE {
            return Self {
                mint,
                is_token_2022: false,
                transfer_fee_config: TransferFeeConfig::ZERO,
                detected_extensions: Vec::new(),
                is_execution_eligible: true,
                rejection_reason: None,
            };
        }

        // Token-2022 with extension data
        let mut offset = BASE_MINT_DATA_SIZE;
        let mut detected_extensions = Vec::new();
        let mut transfer_fee_config = TransferFeeConfig::ZERO;
        let mut rejection_reason = None;

        while offset + 4 <= data.len() {
            let ext_type = u16::from_le_bytes([data[offset], data[offset + 1]]);
            let ext_len = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as usize;
            let payload_start = offset + 4;
            let payload_end = payload_start + ext_len;

            if payload_end > data.len() {
                break;
            }

            detected_extensions.push(ext_type);

            if DANGEROUS_EXTENSIONS.contains(&ext_type) {
                rejection_reason = Some(format!(
                    "Mint carries dangerous/unsupported Token-2022 extension id: {ext_type}"
                ));
            }

            if ext_type == EXTENSION_TRANSFER_FEE_CONFIG {
                // TransferFeeConfig layout in SPL Token-2022:
                // [0..32]: transfer_fee_config_authority (Pubkey)
                // [32..64]: withdraw_withheld_authority (Pubkey)
                // [64..72]: withheld_amount (u64)
                // [72..74]: older_transfer_fee epoch (u16)
                // [74..82]: older_transfer_fee maximum_fee (u64)
                // [82..84]: older_transfer_fee transfer_fee_basis_points (u16)
                // [84..86]: newer_transfer_fee epoch (u16)
                // [86..94]: newer_transfer_fee maximum_fee (u64)
                // [94..96]: newer_transfer_fee transfer_fee_basis_points (u16)
                let p = &data[payload_start..payload_end];
                if p.len() >= 96 {
                    let max_fee = u64::from_le_bytes(p[86..94].try_into().unwrap_or([0; 8]));
                    let fee_bps = u16::from_le_bytes(p[94..96].try_into().unwrap_or([0; 2]));
                    transfer_fee_config = TransferFeeConfig {
                        transfer_fee_basis_points: fee_bps,
                        maximum_fee: max_fee,
                    };
                }
            }

            // Move to next extension aligned to 4 bytes
            let next_offset = (payload_end + 3) & !3;
            if next_offset <= offset || next_offset > data.len() {
                break;
            }
            offset = next_offset;

            if ext_type == EXTENSION_UNINITIALIZED && ext_len == 0 {
                break;
            }
        }

        let is_eligible = rejection_reason.is_none();
        Self {
            mint,
            is_token_2022: true,
            transfer_fee_config,
            detected_extensions,
            is_execution_eligible: is_eligible,
            rejection_reason,
        }
    }
}

/// Hop Transfer Fee Breakdown recording exact deductions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HopTransferFeeBreakdown {
    pub input_gross: u64,
    pub input_transfer_fee: u64,
    pub input_net_into_pool: u64,
    pub output_gross_from_pool: u64,
    pub output_transfer_fee: u64,
    pub output_net_to_next_hop: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_fee_calculation_ceiling() {
        let config = TransferFeeConfig {
            transfer_fee_basis_points: 25, // 0.25%
            maximum_fee: 10_000,
        };

        // 10,000 * 25 / 10,000 = 25
        assert_eq!(config.calculate_fee(10_000), 25);
        assert_eq!(config.calculate_net_received(10_000), 9975);

        // Ceiling rounding: 1 token * 25 / 10000 = 0.0025 -> rounds up to 1
        assert_eq!(config.calculate_fee(1), 1);
        assert_eq!(config.calculate_net_received(1), 0);

        // Maximum fee cap:
        let large_amount = 100_000_000;
        assert_eq!(config.calculate_fee(large_amount), 10_000);
    }

    #[test]
    fn test_dangerous_extension_rejection() {
        let mint = Pubkey::new_unique();
        let mut data = vec![0u8; 82]; // Base mint
        // Add Extension type 10 (PermanentDelegate)
        data.extend_from_slice(&10u16.to_le_bytes()); // ext_type = 10
        data.extend_from_slice(&32u16.to_le_bytes()); // ext_len = 32
        data.extend_from_slice(&[1u8; 32]); // payload

        let semantics = TokenSemantics::parse(mint, &data);
        assert!(semantics.is_token_2022);
        assert!(!semantics.is_execution_eligible);
        assert!(semantics.rejection_reason.is_some());
    }
}
