//! PumpSwap post-graduation AMM adapter.
//!
//! This module deliberately separates pool metadata decoding from vault balances and
//! fee configuration. Pool account bytes alone do not contain the live token balances
//! or the complete effective fee schedule. Callers must join both vault accounts and
//! the current Pump fee configuration before constructing a priceable pool.

use crate::traits::AmmPool;
use arb_math::cpmm_out_fee;
pub use arb_constants::{PUMP_SWAP_GLOBAL_CONFIG, PUMP_SWAP_POOL_DISCRIMINATOR, PUMP_SWAP_PROGRAM_ID};
use arb_types::{ArbError, DexProtocol, MintId, PoolId, Result};
use solana_pubkey::Pubkey;

/// Decoded metadata from the PumpSwap `Pool` account. Token amounts are deliberately
/// excluded: they must come from the two SPL token vault accounts at the same slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapPoolHeader {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub lp_mint: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub lp_supply: u64,
    pub coin_creator: Pubkey,
    pub is_mayhem_mode: bool,
    pub is_cashback_coin: bool,
    pub virtual_quote_reserves: i128,
    pub creator_fee_bps: u64,
    pub can_edit_creator_fee: bool,
    pub is_holder_reward: bool,
    /// Un-swept protocol fee bucket appended in the 2026-10 Pool layout.
    pub protocol_fees: u64,
    /// Un-swept creator fee bucket appended in the 2026-10 Pool layout.
    pub creator_fees: u64,
}

impl PumpSwapPoolHeader {
    /// Decode the stable prefix and optional appended fields. Missing appended fields
    /// are interpreted as protocol-defined zero/false values; truncated stable fields
    /// or a wrong discriminator are rejected.
    pub fn decode(data: &[u8]) -> Result<Self> {
        const MIN_LEN: usize = 245;
        if data.len() < MIN_LEN || data[..8] != PUMP_SWAP_POOL_DISCRIMINATOR {
            return Err(ArbError::DeserializationError(format!(
                "invalid PumpSwap Pool account: len={}, discriminator_ok={}",
                data.len(),
                data.get(..8) == Some(PUMP_SWAP_POOL_DISCRIMINATOR.as_slice())
            )));
        }
        for (start, end) in [(246usize, 261usize), (262, 271), (272, 279), (280, 287)] {
            if data.len() >= start && data.len() < end {
                return Err(ArbError::DeserializationError(format!(
                    "truncated PumpSwap Pool extension: account length {} falls inside {start}..{end}",
                    data.len()
                )));
            }
        }
        let read_key = |start: usize| -> Result<Pubkey> {
            let bytes: [u8; 32] = data.get(start..start + 32)
                .ok_or_else(|| ArbError::DeserializationError("truncated PumpSwap pubkey".into()))?
                .try_into().map_err(|_| ArbError::DeserializationError("invalid PumpSwap pubkey".into()))?;
            Ok(Pubkey::new_from_array(bytes))
        };
        let read_u64 = |start: usize| -> Result<u64> {
            let bytes: [u8; 8] = data.get(start..start + 8)
                .ok_or_else(|| ArbError::DeserializationError("truncated PumpSwap u64".into()))?
                .try_into().map_err(|_| ArbError::DeserializationError("invalid PumpSwap u64".into()))?;
            Ok(u64::from_le_bytes(bytes))
        };
        let read_bool = |offset: usize| -> Result<bool> {
            match data.get(offset).copied() {
                Some(0) => Ok(false),
                Some(1) => Ok(true),
                Some(_) => Err(ArbError::DeserializationError(format!("invalid PumpSwap bool at offset {offset}"))),
                None => Ok(false),
            }
        };
        let virtual_quote_reserves = if data.len() >= 261 {
            i128::from_le_bytes(
                data[245..261].try_into().map_err(|_| ArbError::DeserializationError("invalid virtual quote reserves".into()))?
            )
        } else { 0 };
        Ok(Self {
            pool_bump: data[8],
            index: u16::from_le_bytes([data[9], data[10]]),
            creator: read_key(11)?,
            base_mint: read_key(43)?,
            quote_mint: read_key(75)?,
            lp_mint: read_key(107)?,
            base_vault: read_key(139)?,
            quote_vault: read_key(171)?,
            lp_supply: read_u64(203)?,
            coin_creator: read_key(211)?,
            is_mayhem_mode: read_bool(243)?,
            is_cashback_coin: read_bool(244)?,
            virtual_quote_reserves,
            creator_fee_bps: if data.len() >= 269 { read_u64(261)? } else { 0 },
            can_edit_creator_fee: read_bool(269)?,
            is_holder_reward: read_bool(270)?,
            protocol_fees: if data.len() >= 279 { read_u64(271)? } else { 0 },
            creator_fees: if data.len() >= 287 { read_u64(279)? } else { 0 },
        })
    }

    /// Return the quote reserve PumpSwap prices against: raw vault balance plus
    /// signed `virtual_quote_reserves`. Un-swept fee buckets are excluded separately
    /// when calculating real payout liquidity.
    pub fn effective_quote_reserve(&self, raw_quote_vault_amount: u64) -> Result<u64> {
        let effective = (raw_quote_vault_amount as i128)
            .checked_add(self.virtual_quote_reserves)
            .ok_or(ArbError::Overflow)?;
        u64::try_from(effective).map_err(|_| ArbError::Overflow)
    }
}

/// Live fee components. PumpSwap canonical-pool creator fees are tiered by market cap;
/// non-canonical/custom pairs use different schedules. Never infer these from pool type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpSwapFeeConfig {
    /// Fee components normalized to PPM (on-chain FeeConfig stores whole BPS;
    /// one BPS is converted to 100 PPM for the common fixed-point quote interface).
    pub lp_fee_ppm: u32,
    pub protocol_fee_ppm: u32,
    pub creator_or_holder_fee_ppm: u32,
}

impl PumpSwapFeeConfig {
    pub fn total_ppm(self) -> Result<u32> {
        let total = self.lp_fee_ppm.checked_add(self.protocol_fee_ppm)
            .and_then(|v| v.checked_add(self.creator_or_holder_fee_ppm))
            .ok_or(ArbError::Overflow)?;
        if total > 1_000_000 {
            return Err(ArbError::InvalidInput("PumpSwap total fee exceeds 100%".into()));
        }
        Ok(total)
    }

    /// Construct from fee components decoded from the live Pump GlobalConfig/FeeConfig.
    /// The adapter intentionally does not embed a tier table: the on-chain FeeConfig
    /// is authoritative and can be upgraded independently of this binary.
    pub fn from_verified_ppm(
        lp_fee_ppm: u32,
        protocol_fee_ppm: u32,
        creator_or_holder_fee_ppm: u32,
    ) -> Result<Self> {
        let config = Self { lp_fee_ppm, protocol_fee_ppm, creator_or_holder_fee_ppm };
        config.total_ppm()?;
        Ok(config)
    }

    /// Select a fee tier from the live FeeConfig tier array. Thresholds and market cap
    /// must use the same raw quote-asset denomination. The tier array must be strictly
    /// increasing; no sorting or guessed fallback is performed.
    pub fn from_live_tiers(
        market_cap_quote_raw: u128,
        tiers: &[PumpSwapFeeTier],
    ) -> Result<Self> {
        if tiers.is_empty() {
            return Err(ArbError::InvalidInput("PumpSwap live FeeConfig has no tiers".into()));
        }
        for pair in tiers.windows(2) {
            if pair[0].market_cap_threshold_raw >= pair[1].market_cap_threshold_raw {
                return Err(ArbError::InvalidInput("PumpSwap FeeConfig tier thresholds are not strictly increasing".into()));
            }
        }
        let selected = tiers.iter().rev()
            .find(|tier| market_cap_quote_raw >= tier.market_cap_threshold_raw)
            .unwrap_or(&tiers[0]);
        Self::from_verified_ppm(
            selected.lp_fee_ppm,
            selected.protocol_fee_ppm,
            selected.creator_or_holder_fee_ppm,
        )
    }
}

/// A decoded row from the on-chain Pump FeeConfig. All values are already normalized
/// to PPM by the protocol decoder; no display-percentage parsing is allowed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpSwapFeeTier {
    pub market_cap_threshold_raw: u128,
    pub lp_fee_ppm: u32,
    pub protocol_fee_ppm: u32,
    pub creator_or_holder_fee_ppm: u32,
}

/// Quote-ready PumpSwap pool. `base_reserve` is the base vault amount; `quote_reserve`
/// is the effective quote reserve (raw quote vault + virtual quote reserve).
#[derive(Debug, Clone)]
pub struct PumpSwapPool {
    pub pool_id: PoolId,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    pub base_reserve: u64,
    pub quote_reserve: u64,
    /// Spendable quote vault liquidity after excluding un-swept fee buckets.
    pub real_quote_reserve: u64,
    /// Verified mint decimals. Zero is a valid decimal count, not an unknown sentinel.
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub fee: PumpSwapFeeConfig,
    /// PumpSwap GlobalConfig disable flags captured with the joined state.
    /// Bit 3 disables buys; bit 4 disables sells.
    pub disable_flags: u8,
    pub header: PumpSwapPoolHeader,
}

impl PumpSwapPool {
    pub fn new_with_decimals(
        pool_id: PoolId,
        header: PumpSwapPoolHeader,
        base_vault_amount: u64,
        raw_quote_vault_amount: u64,
        fee: PumpSwapFeeConfig,
        base_decimals: u8,
        quote_decimals: u8,
    ) -> Result<Self> {
        Self::new_with_state(
            pool_id, header, base_vault_amount, raw_quote_vault_amount, fee,
            base_decimals, quote_decimals, 0,
        )
    }

    /// Construct a quote-ready pool from a coherent account join, preserving the
    /// direction-specific GlobalConfig disable flags.
    pub fn new_with_state(
        pool_id: PoolId,
        header: PumpSwapPoolHeader,
        base_vault_amount: u64,
        raw_quote_vault_amount: u64,
        fee: PumpSwapFeeConfig,
        base_decimals: u8,
        quote_decimals: u8,
        disable_flags: u8,
    ) -> Result<Self> {
        let quote_reserve = header.effective_quote_reserve(raw_quote_vault_amount)?;
        let fee_buckets = header.protocol_fees.checked_add(header.creator_fees).ok_or(ArbError::Overflow)?;
        let real_quote_reserve = raw_quote_vault_amount.checked_sub(fee_buckets)
            .ok_or_else(|| ArbError::InvalidInput("PumpSwap fee buckets exceed quote vault".into()))?;
        fee.total_ppm()?;
        if base_vault_amount == 0 || quote_reserve == 0 {
            return Err(ArbError::InvalidInput("PumpSwap pool has zero effective liquidity".into()));
        }
        Ok(Self {
            pool_id,
            base_mint: MintId(header.base_mint),
            quote_mint: MintId(header.quote_mint),
            base_reserve: base_vault_amount,
            quote_reserve,
            real_quote_reserve,
            base_decimals,
            quote_decimals,
            fee,
            disable_flags,
            header,
        })
    }

    pub fn update_vault_balances(&mut self, base: u64, raw_quote: u64) -> Result<()> {
        let quote = self.header.effective_quote_reserve(raw_quote)?;
        let fee_buckets = self.header.protocol_fees.checked_add(self.header.creator_fees).ok_or(ArbError::Overflow)?;
        let real_quote = raw_quote.checked_sub(fee_buckets)
            .ok_or_else(|| ArbError::InvalidInput("PumpSwap fee buckets exceed quote vault".into()))?;
        if base == 0 || quote == 0 {
            return Err(ArbError::InvalidInput("PumpSwap vault update has zero effective liquidity".into()));
        }
        self.base_reserve = base;
        self.quote_reserve = quote;
        self.real_quote_reserve = real_quote;
        Ok(())
    }

    pub fn quote_exact_in(&self, amount_in: u64, base_to_quote: bool) -> Result<u64> {
        // `base_to_quote` is a sell; `quote_to_base` is a buy. Do not disable an
        // otherwise usable pool just because only one direction is disabled.
        let disabled_bit = if base_to_quote { 4 } else { 3 };
        if self.disable_flags & (1 << disabled_bit) != 0 {
            return Err(ArbError::InvalidInput(if base_to_quote {
                "PumpSwap sells are disabled by GlobalConfig".into()
            } else {
                "PumpSwap buys are disabled by GlobalConfig".into()
            }));
        }
        if amount_in == 0 {
            return Err(ArbError::InvalidInput("zero PumpSwap input".into()));
        }
        let scale = 10_000u128;
        let fee_bps = [
            self.fee.lp_fee_ppm,
            self.fee.protocol_fee_ppm,
            if self.header.coin_creator == Pubkey::default() { 0 } else { self.fee.creator_or_holder_fee_ppm },
        ];
        if fee_bps.iter().any(|rate| rate % 100 != 0) {
            return Err(ArbError::InvalidInput("PumpSwap fee PPM is not an integer BPS multiple".into()));
        }
        let fee_bps = fee_bps.map(|rate| (rate / 100) as u64);
        let fee_amount = |gross: u128, bps: u64| -> Result<u128> {
            gross.checked_mul(bps as u128)
                .ok_or(ArbError::Overflow)
                .map(|v| v.div_ceil(scale))
        };
        let out = if base_to_quote {
            let gross = cpmm_out_fee(
                amount_in as u128,
                self.base_reserve as u128,
                self.quote_reserve as u128,
                scale,
                scale,
            ).ok_or_else(|| ArbError::InvalidInput("PumpSwap gross sell quote failed".into()))?;
            let lp = fee_amount(gross, fee_bps[0])?;
            let protocol = fee_amount(gross, fee_bps[1])?;
            let creator = fee_amount(gross, fee_bps[2])?;
            let without_lp = gross.checked_sub(lp).ok_or(ArbError::Overflow)?;
            if (self.real_quote_reserve as u128) < without_lp {
                return Err(ArbError::InvalidInput("PumpSwap insufficient real quote reserves".into()));
            }
            let out = without_lp
                .checked_sub(protocol).and_then(|v| v.checked_sub(creator))
                .ok_or_else(|| ArbError::InvalidInput("PumpSwap fees exceed sell output".into()))?;
            u64::try_from(out).map_err(|_| ArbError::Overflow)?
        } else {
            let input = amount_in as u128;
            let total_bps = fee_bps.iter().try_fold(0u128, |sum, rate| {
                sum.checked_add(*rate as u128).ok_or(ArbError::Overflow)
            })?;
            let mut net = input.checked_mul(scale).ok_or(ArbError::Overflow)?
                / scale.checked_add(total_bps).ok_or(ArbError::Overflow)?;
            if net == 0 {
                return Err(ArbError::InvalidInput("PumpSwap buy input is dust".into()));
            }
            let [fee_lp, fee_protocol, fee_creator] = fee_bps.map(|rate| fee_amount(net, rate));
            let fees = [fee_lp?, fee_protocol?, fee_creator?];
            let cost = net.checked_add(fees.iter().sum::<u128>()).ok_or(ArbError::Overflow)?;
            if cost > input {
                net = net.checked_sub(cost - input)
                    .filter(|value| *value > 0)
                    .ok_or_else(|| ArbError::InvalidInput("PumpSwap exact-in fees consume input".into()))?;
            }
            let swap_input = net.checked_sub(1)
                .filter(|value| *value > 0)
                .ok_or_else(|| ArbError::InvalidInput("PumpSwap buy input is dust".into()))?;
            let out = cpmm_out_fee(
                swap_input,
                self.quote_reserve as u128,
                self.base_reserve as u128,
                scale,
                scale,
            ).ok_or_else(|| ArbError::InvalidInput("PumpSwap net buy quote failed".into()))?;
            u64::try_from(out).map_err(|_| ArbError::Overflow)?
        };
        let reserve_out = if base_to_quote { self.real_quote_reserve } else { self.base_reserve };
        if out == 0 || out >= reserve_out {
            return Err(ArbError::InvalidInput("PumpSwap insufficient output liquidity".into()));
        }
        Ok(out)
    }
}

impl AmmPool for PumpSwapPool {
    fn pool_id(&self) -> PoolId { self.pool_id }
    fn protocol(&self) -> DexProtocol { DexProtocol::PumpSwap }
    fn base_mint(&self) -> MintId { self.base_mint }
    fn quote_mint(&self) -> MintId { self.quote_mint }
    fn reserves(&self) -> (u128, u128) { (self.base_reserve as u128, self.quote_reserve as u128) }
    fn fee_bps(&self) -> Option<u16> {
        let ppm = self.fee.total_ppm().ok()?;
        if ppm % 100 != 0 { return None; }
        u16::try_from(ppm / 100).ok()
    }
    fn decimals(&self) -> (u8, u8) { (self.base_decimals, self.quote_decimals) }
    fn update_from_account(&mut self, _data: &[u8]) -> Result<()> {
        Err(ArbError::InvalidInput("PumpSwap requires a coherent Pool + two vault account update".into()))
    }
    fn quote(&self, amount_in: u64, a_to_b: bool) -> Result<u64> { self.quote_exact_in(amount_in, a_to_b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_header() -> PumpSwapPoolHeader {
        PumpSwapPoolHeader {
            pool_bump: 1, index: 0,
            creator: Pubkey::new_unique(), base_mint: Pubkey::new_unique(),
            quote_mint: Pubkey::new_unique(), lp_mint: Pubkey::new_unique(),
            base_vault: Pubkey::new_unique(), quote_vault: Pubkey::new_unique(),
            lp_supply: 1_000, coin_creator: Pubkey::new_unique(),
            is_mayhem_mode: false, is_cashback_coin: false,
            virtual_quote_reserves: 100, creator_fee_bps: 0, can_edit_creator_fee: false, is_holder_reward: false,
            protocol_fees: 0, creator_fees: 0,
        }
    }

    #[test]
    fn effective_quote_reserve_includes_signed_virtual_adjustment() {
        assert_eq!(fixture_header().effective_quote_reserve(1_000).unwrap(), 1_100);
        let mut header = fixture_header();
        header.virtual_quote_reserves = -100;
        assert_eq!(header.effective_quote_reserve(1_000).unwrap(), 900);
        assert!(header.effective_quote_reserve(50).is_err());
    }

    #[test]
    fn quote_uses_live_fee_components_and_direction() {
        let pool = PumpSwapPool::new_with_decimals(
            PoolId(Pubkey::new_unique()), fixture_header(), 10_000, 20_000,
            PumpSwapFeeConfig { lp_fee_ppm: 2_000, protocol_fee_ppm: 500, creator_or_holder_fee_ppm: 10_000 }, 6, 9,
        ).unwrap();
        let sell_out = pool.quote_exact_in(100, true).unwrap();
        let buy_out = pool.quote_exact_in(100, false).unwrap();
        assert_eq!(sell_out, 195);
        assert_eq!(buy_out, 47);
    }

    #[test]
    fn direction_flags_disable_only_the_configured_side() {
        let pool = PumpSwapPool::new_with_state(
            PoolId(Pubkey::new_unique()), fixture_header(), 10_000, 20_000,
            PumpSwapFeeConfig { lp_fee_ppm: 2_000, protocol_fee_ppm: 500, creator_or_holder_fee_ppm: 0 },
            6, 9, 1 << 3,
        ).unwrap();
        assert!(pool.quote_exact_in(100, false).is_err(), "buy disabled");
        assert!(pool.quote_exact_in(100, true).is_ok(), "sell remains enabled");
    }

    #[test]
    fn mayhem_and_holder_reward_modes_are_quoteable_when_fees_are_verified() {
        let mut header = fixture_header();
        header.is_mayhem_mode = true;
        header.is_holder_reward = true;
        let pool = PumpSwapPool::new_with_decimals(
            PoolId(Pubkey::new_unique()), header, 10_000, 20_000,
            PumpSwapFeeConfig { lp_fee_ppm: 2_000, protocol_fee_ppm: 500, creator_or_holder_fee_ppm: 10_000 },
            6, 9,
        ).unwrap();
        assert!(pool.quote_exact_in(100, false).is_ok());
        assert!(pool.quote_exact_in(100, true).is_ok());
    }

    #[test]
    fn excludes_un_swept_fee_buckets_from_sellable_quote_liquidity() {
        let mut header = fixture_header();
        header.protocol_fees = 250;
        header.creator_fees = 200;
        let pool = PumpSwapPool::new_with_decimals(
            PoolId(Pubkey::new_unique()),
            header,
            1_000,
            500,
            PumpSwapFeeConfig { lp_fee_ppm: 20_000, protocol_fee_ppm: 5_000, creator_or_holder_fee_ppm: 0 },
            6,
            9,
        ).unwrap();
        assert_eq!(pool.real_quote_reserve, 50);
        assert!(pool.quote_exact_in(100, true).is_err());
    }

    #[test]
    fn rejects_invalid_fee_and_zero_liquidity() {
        assert!(PumpSwapFeeConfig { lp_fee_ppm: 900_000, protocol_fee_ppm: 200_000, creator_or_holder_fee_ppm: 0 }.total_ppm().is_err());
        assert!(PumpSwapPool::new_with_decimals(PoolId(Pubkey::new_unique()), fixture_header(), 0, 1_000,
            PumpSwapFeeConfig { lp_fee_ppm: 2_000, protocol_fee_ppm: 500, creator_or_holder_fee_ppm: 0 }, 6, 9).is_err());
    }

    #[test]
    fn canonical_fee_tiers_preserve_bps_when_normalized_to_ppm() {
        let tiers = [
            PumpSwapFeeTier { market_cap_threshold_raw: 0, lp_fee_ppm: 2_000, protocol_fee_ppm: 9_300, creator_or_holder_fee_ppm: 3_000 },
            PumpSwapFeeTier { market_cap_threshold_raw: 1_000, lp_fee_ppm: 2_000, protocol_fee_ppm: 500, creator_or_holder_fee_ppm: 9_500 },
        ];
        assert_eq!(PumpSwapFeeConfig::from_live_tiers(999, &tiers).unwrap().total_ppm().unwrap(), 14_300);
        assert_eq!(PumpSwapFeeConfig::from_live_tiers(1_000, &tiers).unwrap().total_ppm().unwrap(), 12_000);
        assert!(PumpSwapFeeConfig::from_live_tiers(1_000, &[]).is_err());
    }

    #[test]
    fn decodes_pool_account_layout_and_older_short_layout() {
        let mut data = vec![0u8; 287];
        data[..8].copy_from_slice(&PUMP_SWAP_POOL_DISCRIMINATOR);
        data[8] = 7;
        data[9..11].copy_from_slice(&3u16.to_le_bytes());
        for (offset, key) in [(11, Pubkey::new_unique()), (43, Pubkey::new_unique()),
            (75, Pubkey::new_unique()), (107, Pubkey::new_unique()),
            (139, Pubkey::new_unique()), (171, Pubkey::new_unique()), (211, Pubkey::new_unique())] {
            data[offset..offset+32].copy_from_slice(key.as_ref());
        }
        data[203..211].copy_from_slice(&55u64.to_le_bytes());
        data[245..261].copy_from_slice(&123i128.to_le_bytes());
        data[261..269].copy_from_slice(&30u64.to_le_bytes());
        data[269] = 1;
        data[270] = 1;
        data[271..279].copy_from_slice(&7u64.to_le_bytes());
        data[279..287].copy_from_slice(&9u64.to_le_bytes());
        let header = PumpSwapPoolHeader::decode(&data).unwrap();
        assert_eq!(header.index, 3);
        assert_eq!(header.virtual_quote_reserves, 123);
        assert_eq!(header.creator_fee_bps, 30);
        assert!(header.can_edit_creator_fee);
        assert!(header.is_holder_reward);
        assert_eq!((header.protocol_fees, header.creator_fees), (7, 9));

        let old = data[..245].to_vec();
        let old_header = PumpSwapPoolHeader::decode(&old).unwrap();
        assert_eq!(old_header.virtual_quote_reserves, 0);
        assert_eq!(old_header.creator_fee_bps, 0);
        assert!(!old_header.can_edit_creator_fee);
        assert!(!old_header.is_holder_reward);
    }
}
