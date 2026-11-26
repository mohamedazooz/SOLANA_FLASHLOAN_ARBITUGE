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
    /// Parse token mint account data conservatively.
    ///
    /// Token-2022 extension-bearing mints use an 83-byte padding region and
    /// AccountType byte after the 82-byte base Mint. Absolute TLV offset is
    /// therefore 166 (not 82). This lightweight type cannot represent epoch-
    /// selected transfer-fee schedules or dynamic TransferHook accounts, so
    /// every non-empty extension set is deliberately rejected for execution.
    /// The active quote path has its own fail-closed decoder and eligibility
    /// gate; do not treat this metadata parser as execution authorization.
    pub fn parse(mint: Pubkey, data: &[u8]) -> Self {
        let rejected = |is_token_2022: bool, detected_extensions: Vec<u16>, reason: String| Self {
            mint,
            is_token_2022,
            transfer_fee_config: TransferFeeConfig::ZERO,
            detected_extensions,
            is_execution_eligible: false,
            rejection_reason: Some(reason),
        };

        if data.len() < BASE_MINT_DATA_SIZE {
            return rejected(
                false,
                Vec::new(),
                format!("Account data length {} < base mint size 82", data.len()),
            );
        }

        // An 82-byte mint has no extension trailer. Whether it is owned by the
        // legacy Token Program or Token-2022 must be checked by the account
        // loader, because the owner is not available to this pure parser.
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

        const ACCOUNT_TYPE_MINT: u8 = 1;
        const TLV_START: usize = 166;
        if data.len() < TLV_START || data.get(165).copied() != Some(ACCOUNT_TYPE_MINT) {
            return rejected(
                true,
                Vec::new(),
                "Malformed Token-2022 mint: missing padding/account-type header".into(),
            );
        }

        let offset = TLV_START;
        let mut detected_extensions = Vec::new();
        if offset < data.len() {
            let remaining = &data[offset..];
            if !remaining.iter().all(|byte| *byte == 0) {
                let Some(header_end) = offset.checked_add(4) else {
                    return rejected(
                        true,
                        detected_extensions,
                        "Token-2022 TLV offset overflow".into(),
                    );
                };
                if header_end > data.len() {
                    return rejected(
                        true,
                        detected_extensions,
                        "Truncated Token-2022 TLV header".into(),
                    );
                }

                let ext_type = u16::from_le_bytes([data[offset], data[offset + 1]]);
                let ext_len = usize::from(u16::from_le_bytes([data[offset + 2], data[offset + 3]]));
                if !(ext_type == EXTENSION_UNINITIALIZED && ext_len == 0) {
                    let Some(payload_end) = header_end.checked_add(ext_len) else {
                        return rejected(
                            true,
                            detected_extensions,
                            "Token-2022 TLV length overflow".into(),
                        );
                    };
                    if payload_end > data.len() {
                        return rejected(
                            true,
                            detected_extensions,
                            format!("Token-2022 extension {ext_type} is truncated"),
                        );
                    }

                    detected_extensions.push(ext_type);
                    // Until current-epoch TransferFeeConfig and TransferHook extra
                    // account resolution are wired into BOTH quote and execution paths,
                    // every extension stays ineligible. Unknown extensions also fail
                    // closed; a permissive allow-list would become stale as SPL evolves.
                    return rejected(
                        true,
                        detected_extensions,
                        format!(
                            "Token-2022 extension {ext_type} is not enabled for execution; verified epoch-aware fees and transfer-hook account resolution are required"
                        ),
                    );
                }
            }
        }
        Self {
            mint,
            is_token_2022: true,
            transfer_fee_config: TransferFeeConfig::ZERO,
            detected_extensions,
            is_execution_eligible: true,
            rejection_reason: None,
        }
    }
}

/// Hop Transfer Fee Breakdown recording exact deductions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HopTransferFeeBreakdown {
    pub input_gross: u64,
    pub input_transfer_fee: u64,
    pub input_net_into_pool: u64,
    /// Gross tokens transferred from the pool vault before Token-2022 transfer fees.
    pub output_gross_from_pool: u64,
    /// Amount debited from invariant curve reserves; differs when a protocol fee is retained in-vault.
    pub output_curve_reserve_debit: u64,
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
    fn test_dangerous_extension_rejection_uses_official_mint_tlv_offset() {
        let mint = Pubkey::new_unique();
        // Extension-bearing Token-2022 mint layout:
        // 82-byte Mint + 83-byte padding + AccountType(Mint) + TLV.
        let mut data = vec![0u8; 166];
        data[165] = 1; // AccountType::Mint
        data.extend_from_slice(&10u16.to_le_bytes()); // PermanentDelegate
        data.extend_from_slice(&32u16.to_le_bytes());
        data.extend_from_slice(&[1u8; 32]);

        let semantics = TokenSemantics::parse(mint, &data);
        assert!(semantics.is_token_2022);
        assert_eq!(
            semantics.detected_extensions,
            vec![EXTENSION_PERMANENT_DELEGATE]
        );
        assert!(!semantics.is_execution_eligible);
        assert!(semantics.rejection_reason.is_some());
        assert_eq!(semantics.transfer_fee_config, TransferFeeConfig::ZERO);
    }

    #[test]
    fn test_token2022_malformed_tlv_fails_closed() {
        let mint = Pubkey::new_unique();
        let mut data = vec![0u8; 166];
        data[165] = 1;
        data.extend_from_slice(&EXTENSION_TRANSFER_FEE_CONFIG.to_le_bytes());
        data.extend_from_slice(&108u16.to_le_bytes());
        data.extend_from_slice(&[0u8; 12]); // truncated payload

        let semantics = TokenSemantics::parse(mint, &data);
        assert!(semantics.is_token_2022);
        assert!(!semantics.is_execution_eligible);
        assert!(semantics.rejection_reason.unwrap().contains("truncated"));
    }

    #[test]
    fn test_token2022_with_no_extensions_has_empty_tlv_area() {
        let mint = Pubkey::new_unique();
        let mut data = vec![0u8; 166];
        data[165] = 1;

        let semantics = TokenSemantics::parse(mint, &data);
        assert!(semantics.is_token_2022);
        assert!(semantics.is_execution_eligible);
        assert!(semantics.detected_extensions.is_empty());
    }
}
