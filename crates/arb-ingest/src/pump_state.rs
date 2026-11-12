//! Coherent PumpSwap account-state joiner.
//!
//! A pool is not quote-ready from its Pool header alone. This module joins the Pool,
//! both token vaults, both mints and the Pump Fees FeeConfig account from the Yellowstone
//! account journal. Pool and vaults must share the same slot; mint/config snapshots may
//! be older because they are configuration state, but may never be from a future slot.
//! FeeConfig bytes are returned raw by the joiner and decoded by the bounded Pump Fees
//! account decoder below; unknown account extensions are rejected rather than guessed.

use crate::{AccountSnapshot, AccountSnapshotStore};
use arb_constants::{
    ASSOCIATED_TOKEN_PROGRAM_ID, KAMINO_WSOL_RESERVE, PUMP_FEES_FEE_CONFIG_DISCRIMINATOR,
    PUMP_FEES_PROGRAM_ID, PUMP_FUN_PROGRAM_ID, PUMP_SWAP_GLOBAL_CONFIG,
    PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR, PUMP_SWAP_PROGRAM_ID, TOKEN_2022_NATIVE_MINT,
    TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID, USDC_MINT, WSOL_MINT,
};
use arb_dex::{PumpSwapFeeConfig, PumpSwapPool, PumpSwapPoolHeader};
use arb_types::{ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Result, Slot};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const TOKEN_ACCOUNT_BASE_LEN: usize = 165;
const MINT_BASE_LEN: usize = 82;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpFeeComponentsBps {
    /// On-chain basis points; quote adapter normalizes to PPM by multiplying by 100.
    pub lp: u64,
    pub protocol: u64,
    pub creator: u64,
}

impl PumpFeeComponentsBps {
    fn is_zero(self) -> bool {
        self.lp == 0 && self.protocol == 0 && self.creator == 0
    }

    fn validate(self) -> Result<Self> {
        let total = self
            .lp
            .checked_add(self.protocol)
            .and_then(|v| v.checked_add(self.creator))
            .ok_or(ArbError::Overflow)?;
        if total > 10_000 {
            return Err(ArbError::InvalidInput(
                "Pump FeeConfig component sum exceeds 100% (BPS)".into(),
            ));
        }
        Ok(self)
    }

    fn to_ppm(self) -> Result<(u32, u32, u32)> {
        self.validate()?;
        Ok((
            u32::try_from(self.lp.checked_mul(100).ok_or(ArbError::Overflow)?)
                .map_err(|_| ArbError::Overflow)?,
            u32::try_from(self.protocol.checked_mul(100).ok_or(ArbError::Overflow)?)
                .map_err(|_| ArbError::Overflow)?,
            u32::try_from(self.creator.checked_mul(100).ok_or(ArbError::Overflow)?)
                .map_err(|_| ArbError::Overflow)?,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpFeeTier {
    pub market_cap_threshold_raw: u128,
    pub fees: PumpFeeComponentsBps,
}

/// IDL/Borsh-decoded Pump Fees FeeConfig. The account is program-specific: Pump and
/// PumpSwap have distinct PDAs even though both are owned by the same fee program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpFeesFeeConfig {
    pub bump: u8,
    pub flat_fees: PumpFeeComponentsBps,
    pub fee_tiers: Vec<PumpFeeTier>,
    pub stable_fee_tiers: Vec<PumpFeeTier>,
    pub exotic_flat_fees: Option<PumpFeeComponentsBps>,
}

impl PumpFeesFeeConfig {
    /// Decode the bounded Borsh prefix. Supported account layouts are the legacy
    /// single-ladder form and current standard + stable + optional exotic-fee form.
    /// Unknown non-zero trailing bytes are rejected rather than reinterpreted.
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 65 || data.get(..8) != Some(PUMP_FEES_FEE_CONFIG_DISCRIMINATOR.as_slice()) {
            return Err(ArbError::DeserializationError(
                "invalid Pump Fees FeeConfig header".into(),
            ));
        }
        let bump = data[8];
        let flat_fees = read_fee_components(data, 41)?;
        let mut cursor = 65usize;
        let fee_tiers = read_fee_tiers(data, &mut cursor)?;
        let remaining = data.len().saturating_sub(cursor);
        let stable_fee_tiers = if remaining >= 4 {
            read_fee_tiers(data, &mut cursor)?
        } else {
            if data[cursor..].iter().any(|byte| *byte != 0) {
                return Err(ArbError::DeserializationError(
                    "unexpected Pump FeeConfig tail".into(),
                ));
            }
            Vec::new()
        };
        let remaining = data.len().saturating_sub(cursor);
        let exotic_flat_fees = if remaining >= 24 {
            let fees = read_fee_components(data, cursor)?;
            cursor += 24;
            Some(fees)
        } else {
            None
        };
        if data[cursor..].iter().any(|byte| *byte != 0) {
            return Err(ArbError::DeserializationError(
                "unknown non-zero Pump FeeConfig extension".into(),
            ));
        }
        flat_fees.validate()?;
        if fee_tiers.is_empty() {
            return Err(ArbError::InvalidInput(
                "Pump FeeConfig standard tier ladder is empty".into(),
            ));
        }
        validate_fee_tiers(&fee_tiers)?;
        if !stable_fee_tiers.is_empty() {
            validate_fee_tiers(&stable_fee_tiers)?;
        }
        if let Some(fees) = exotic_flat_fees {
            fees.validate()?;
        }
        Ok(Self {
            bump,
            flat_fees,
            fee_tiers,
            stable_fee_tiers,
            exotic_flat_fees,
        })
    }

    fn select_for_pool(
        &self,
        canonical: bool,
        quote_mint: &Pubkey,
        market_cap_quote_raw: u128,
    ) -> Result<PumpFeeComponentsBps> {
        if !canonical {
            return self.flat_fees.validate();
        }
        let wsol = parse_program_id(WSOL_MINT)?;
        let token_2022_native = parse_program_id(TOKEN_2022_NATIVE_MINT)?;
        let usdc = parse_program_id(USDC_MINT)?;
        if *quote_mint == Pubkey::default()
            || *quote_mint == wsol
            || *quote_mint == token_2022_native
        {
            select_fee_tier(&self.fee_tiers, market_cap_quote_raw)
        } else if *quote_mint == usdc {
            select_fee_tier(&self.stable_fee_tiers, market_cap_quote_raw)
        } else {
            self.exotic_flat_fees
                .filter(|fees| !fees.is_zero())
                .unwrap_or(self.flat_fees)
                .validate()
        }
    }
}

fn read_fee_components(data: &[u8], offset: usize) -> Result<PumpFeeComponentsBps> {
    let read = |at: usize| -> Result<u64> {
        let bytes: [u8; 8] = data
            .get(at..at + 8)
            .ok_or_else(|| ArbError::DeserializationError("truncated Pump fee component".into()))?
            .try_into()
            .map_err(|_| ArbError::DeserializationError("invalid Pump fee component".into()))?;
        Ok(u64::from_le_bytes(bytes))
    };
    Ok(PumpFeeComponentsBps {
        lp: read(offset)?,
        protocol: read(offset + 8)?,
        creator: read(offset + 16)?,
    })
}

fn read_fee_tiers(data: &[u8], cursor: &mut usize) -> Result<Vec<PumpFeeTier>> {
    let len_bytes: [u8; 4] = data
        .get(*cursor..*cursor + 4)
        .ok_or_else(|| {
            ArbError::DeserializationError("truncated Pump FeeConfig tier vector length".into())
        })?
        .try_into()
        .map_err(|_| {
            ArbError::DeserializationError("invalid Pump FeeConfig tier vector length".into())
        })?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    *cursor += 4;
    let bytes_needed = len.checked_mul(40).ok_or(ArbError::Overflow)?;
    let end = (*cursor)
        .checked_add(bytes_needed)
        .ok_or(ArbError::Overflow)?;
    if end > data.len() || len > 128 {
        return Err(ArbError::DeserializationError(
            "Pump FeeConfig tier vector exceeds account bounds".into(),
        ));
    }
    let mut tiers = Vec::with_capacity(len);
    for _ in 0..len {
        let threshold =
            u128::from_le_bytes(data[*cursor..*cursor + 16].try_into().map_err(|_| {
                ArbError::DeserializationError("invalid Pump fee tier threshold".into())
            })?);
        let fees = read_fee_components(data, *cursor + 16)?.validate()?;
        tiers.push(PumpFeeTier {
            market_cap_threshold_raw: threshold,
            fees,
        });
        *cursor += 40;
    }
    Ok(tiers)
}

fn validate_fee_tiers(tiers: &[PumpFeeTier]) -> Result<()> {
    for pair in tiers.windows(2) {
        if pair[0].market_cap_threshold_raw >= pair[1].market_cap_threshold_raw {
            return Err(ArbError::InvalidInput(
                "Pump FeeConfig thresholds are not strictly increasing".into(),
            ));
        }
    }
    Ok(())
}

fn select_fee_tier(
    tiers: &[PumpFeeTier],
    market_cap_quote_raw: u128,
) -> Result<PumpFeeComponentsBps> {
    if tiers.is_empty() {
        return Err(ArbError::InvalidInput(
            "Pump FeeConfig tier ladder is empty".into(),
        ));
    }
    Ok(tiers
        .iter()
        .rev()
        .find(|tier| market_cap_quote_raw >= tier.market_cap_threshold_raw)
        .unwrap_or(&tiers[0])
        .fees)
}

#[derive(Debug, Clone)]
pub struct PumpSwapJoinedState {
    pub pool_key: Pubkey,
    pub slot: u64,
    pub header: PumpSwapPoolHeader,
    pub base_vault_amount: u64,
    pub quote_vault_amount: u64,
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub base_mint_supply: u64,
    /// Raw, owner- and PDA-verified Pump Fees FeeConfig snapshot.
    /// A dedicated IDL decoder must turn this into `PumpSwapFeeConfig`.
    pub fee_config_account: AccountSnapshot,
    pub fee_config_bump: u8,
    /// PumpSwap GlobalConfig snapshot used to gate disabled buy/sell directions.
    pub global_config_account: AccountSnapshot,
}

impl PumpSwapJoinedState {
    /// Promote the joined account state to a quote pool only after the caller has
    /// decoded the same FeeConfig snapshot into normalized PPM components.
    pub fn into_pool(self, fee: PumpSwapFeeConfig) -> Result<PumpSwapPool> {
        let global = decode_pump_swap_global_config(&self.global_config_account.data)?;
        PumpSwapPool::new_with_state(
            PoolId(self.pool_key),
            self.header,
            self.base_vault_amount,
            self.quote_vault_amount,
            fee,
            self.base_decimals,
            self.quote_decimals,
            global.disable_flags,
        )
    }

    /// Whether at least one swap direction remains enabled.
    pub fn trading_enabled(&self) -> Result<bool> {
        let config = decode_pump_swap_global_config(&self.global_config_account.data)?;
        Ok(config.disable_flags & ((1 << 3) | (1 << 4)) != ((1 << 3) | (1 << 4)))
    }

    pub fn trading_enabled_for(&self, is_buy: bool) -> Result<bool> {
        let config = decode_pump_swap_global_config(&self.global_config_account.data)?;
        let bit = if is_buy { 3 } else { 4 };
        Ok(config.disable_flags & (1 << bit) == 0)
    }

    /// Select the live Pump fee schedule for this exact pool state. Canonical pools
    /// are identified by `pool.creator == PDA(pump, ["pool-authority", base_mint])`;
    /// `coin_creator` is deliberately not used for canonical-pool classification.
    pub fn select_fee(&self, config: &PumpFeesFeeConfig) -> Result<PumpSwapFeeConfig> {
        let pump_program = parse_program_id(PUMP_FUN_PROGRAM_ID)?;
        let (canonical_creator, _) = Pubkey::find_program_address(
            &[b"pool-authority", self.header.base_mint.as_ref()],
            &pump_program,
        );
        let canonical = self.header.creator == canonical_creator;
        let effective_quote = self
            .header
            .effective_quote_reserve(self.quote_vault_amount)?;
        if self.base_vault_amount == 0 {
            return Err(ArbError::InvalidInput(
                "PumpSwap base reserve is zero".into(),
            ));
        }
        let market_cap = (self.base_mint_supply as u128)
            .checked_mul(effective_quote as u128)
            .ok_or(ArbError::Overflow)?
            / self.base_vault_amount as u128;
        let mut fees = config.select_for_pool(canonical, &self.header.quote_mint, market_cap)?;
        let global_config = decode_pump_swap_global_config(&self.global_config_account.data)?;
        let wsol = parse_program_id(WSOL_MINT)?;
        let token_2022_native = parse_program_id(TOKEN_2022_NATIVE_MINT)?;
        let usdc = parse_program_id(USDC_MINT)?;
        let is_custom_quote = self.header.quote_mint != wsol
            && self.header.quote_mint != token_2022_native
            && self.header.quote_mint != usdc;
        if is_custom_quote
            && global_config.creator_fee_configurable
            && self.header.creator_fee_bps > 0
        {
            if global_config.max_configurable_creator_fee_bps == 0
                || self.header.creator_fee_bps > global_config.max_configurable_creator_fee_bps
            {
                return Err(ArbError::InvalidInput(
                    "PumpSwap pool creator fee exceeds GlobalConfig configurable cap".into(),
                ));
            }
            fees.creator = self.header.creator_fee_bps;
        }
        if self.header.coin_creator == Pubkey::default() {
            fees.creator = 0;
        }
        let (lp, protocol, creator) = fees.to_ppm()?;
        PumpSwapFeeConfig::from_verified_ppm(lp, protocol, creator)
    }
}

/// Join PumpSwap Pool + vault + mint + FeeConfig snapshots.
///
/// Safety invariants:
/// - Pool account owner is PumpSwap program.
/// - Pool and both vaults are from the exact same slot.
/// - Vault token mints and authorities match the decoded Pool header.
/// - Vaults and mint accounts are owned by the correct SPL Token program.
/// - Frozen token accounts and uninitialized mints are rejected.
/// - FeeConfig address is the canonical PDA for PumpSwap under Pump Fees program.
/// - FeeConfig owner is Pump Fees program and its snapshot is not newer than the pool.
pub fn join_pump_swap_state(
    store: &AccountSnapshotStore,
    pool_key: Pubkey,
    fee_config_key: Pubkey,
) -> Result<PumpSwapJoinedState> {
    let pump_swap_program = parse_program_id(PUMP_SWAP_PROGRAM_ID)?;
    let pump_fees_program = parse_program_id(PUMP_FEES_PROGRAM_ID)?;
    let token_program = parse_program_id(TOKEN_PROGRAM_ID)?;
    let token_2022_program = parse_program_id(TOKEN_2022_PROGRAM_ID)?;

    let pool = required_snapshot(store, &pool_key, "PumpSwap pool")?;
    require_owner(&pool, &pump_swap_program, "PumpSwap pool")?;
    let header = PumpSwapPoolHeader::decode(&pool.data)?;
    let index_seed = header.index.to_le_bytes();
    let (expected_pool_key, expected_pool_bump) = Pubkey::find_program_address(
        &[
            b"pool",
            &index_seed,
            header.creator.as_ref(),
            header.base_mint.as_ref(),
            header.quote_mint.as_ref(),
        ],
        &pump_swap_program,
    );
    if expected_pool_key != pool_key || expected_pool_bump != header.pool_bump {
        return Err(ArbError::InvalidInput(
            "PumpSwap Pool key/bump does not match canonical PDA seeds".into(),
        ));
    }

    let (expected_global_config, _) =
        Pubkey::find_program_address(&[b"global_config"], &pump_swap_program);
    let configured_global_config = parse_program_id(PUMP_SWAP_GLOBAL_CONFIG)?;
    if expected_global_config != configured_global_config {
        return Err(ArbError::InvalidInput(
            "configured PumpSwap GlobalConfig does not match canonical PDA".into(),
        ));
    }
    let global_config =
        required_snapshot(store, &configured_global_config, "PumpSwap GlobalConfig")?;
    require_owner(&global_config, &pump_swap_program, "PumpSwap GlobalConfig")?;
    decode_pump_swap_disable_flags(&global_config.data)?;
    if global_config.slot > pool.slot {
        return Err(ArbError::InvalidInput(
            "PumpSwap GlobalConfig snapshot is newer than Pool snapshot".into(),
        ));
    }

    let (expected_fee_config, expected_fee_config_bump) = Pubkey::find_program_address(
        &[b"fee_config", pump_swap_program.as_ref()],
        &pump_fees_program,
    );
    if fee_config_key != expected_fee_config {
        return Err(ArbError::InvalidInput(
            "PumpSwap FeeConfig key is not the canonical Pump Fees PDA".into(),
        ));
    }
    let fee_config = required_snapshot(store, &fee_config_key, "Pump Fees FeeConfig")?;
    require_owner(&fee_config, &pump_fees_program, "Pump Fees FeeConfig")?;
    if fee_config.data.len() < 65
        || fee_config.data.get(..8) != Some(PUMP_FEES_FEE_CONFIG_DISCRIMINATOR.as_slice())
    {
        return Err(ArbError::DeserializationError(
            "Pump Fees FeeConfig discriminator/layout mismatch".into(),
        ));
    }
    if fee_config.slot > pool.slot {
        return Err(ArbError::InvalidInput(
            "PumpSwap FeeConfig snapshot is newer than Pool snapshot".into(),
        ));
    }

    let base_vault = required_snapshot(store, &header.base_vault, "PumpSwap base vault")?;
    let quote_vault = required_snapshot(store, &header.quote_vault, "PumpSwap quote vault")?;
    if base_vault.slot != pool.slot || quote_vault.slot != pool.slot {
        return Err(ArbError::InvalidInput(format!(
            "PumpSwap pool/vault slot mismatch: pool={}, base_vault={}, quote_vault={}",
            pool.slot, base_vault.slot, quote_vault.slot
        )));
    }

    let (base_amount, base_token_program) = decode_token_account(
        &base_vault,
        &header.base_mint,
        &pool_key,
        &token_program,
        &token_2022_program,
        "base vault",
    )?;
    let (quote_amount, quote_token_program) = decode_token_account(
        &quote_vault,
        &header.quote_mint,
        &pool_key,
        &token_program,
        &token_2022_program,
        "quote vault",
    )?;
    let associated_token_program = parse_program_id(ASSOCIATED_TOKEN_PROGRAM_ID)?;
    let expected_base_vault = associated_token_address(
        &pool_key,
        &header.base_mint,
        &base_token_program,
        &associated_token_program,
    );
    let expected_quote_vault = associated_token_address(
        &pool_key,
        &header.quote_mint,
        &quote_token_program,
        &associated_token_program,
    );
    if header.base_vault != expected_base_vault || header.quote_vault != expected_quote_vault {
        return Err(ArbError::InvalidInput(
            "PumpSwap Pool vault keys are not the canonical pool ATAs".into(),
        ));
    }

    let base_mint = required_snapshot(store, &header.base_mint, "PumpSwap base mint")?;
    let quote_mint = required_snapshot(store, &header.quote_mint, "PumpSwap quote mint")?;
    if base_mint.slot > pool.slot || quote_mint.slot > pool.slot {
        return Err(ArbError::InvalidInput(
            "PumpSwap mint snapshot is newer than Pool snapshot".into(),
        ));
    }
    let (base_decimals, base_mint_supply) =
        decode_mint(&base_mint, &base_token_program, "base mint")?;
    let (quote_decimals, _) = decode_mint(&quote_mint, &quote_token_program, "quote mint")?;
    let base_has_unsupported_t22 = base_token_program == token_2022_program
        && (base_mint.data.len() > MINT_BASE_LEN
            || arb_dex::token2022_has_risky_extension(&base_mint.data));
    let quote_has_unsupported_t22 = quote_token_program == token_2022_program
        && (quote_mint.data.len() > MINT_BASE_LEN
            || arb_dex::token2022_has_risky_extension(&quote_mint.data));
    if base_has_unsupported_t22 || quote_has_unsupported_t22 {
        return Err(ArbError::InvalidInput(
            "PumpSwap Token-2022 mint extensions are unsupported by the current quote model".into(),
        ));
    }

    Ok(PumpSwapJoinedState {
        pool_key,
        slot: pool.slot,
        header,
        base_vault_amount: base_amount,
        quote_vault_amount: quote_amount,
        base_decimals,
        quote_decimals,
        base_mint_supply,
        fee_config_account: fee_config,
        fee_config_bump: expected_fee_config_bump,
        global_config_account: global_config,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PumpSwapRefreshReport {
    pub pool_accounts_seen: usize,
    pub dependencies_tracked: usize,
    pub pools_joined: usize,
    pub pools_promoted: usize,
    pub pools_rejected: usize,
}

impl crate::IngestClient {
    /// Discover PumpSwap Pool accounts from the owner-filtered Yellowstone journal,
    /// subscribe to their vault/mint/FeeConfig dependencies, and promote only coherent
    /// snapshots into the shared market cache. No HTTP/RPC fallback is used.
    pub fn refresh_pump_swap_states(&self, max_state_age_slots: u64) -> PumpSwapRefreshReport {
        let Ok(pump_swap_program) = parse_program_id(PUMP_SWAP_PROGRAM_ID) else {
            return PumpSwapRefreshReport::default();
        };
        let Ok(pump_fees_program) = parse_program_id(PUMP_FEES_PROGRAM_ID) else {
            return PumpSwapRefreshReport::default();
        };
        let fee_config_key = Pubkey::find_program_address(
            &[b"fee_config", pump_swap_program.as_ref()],
            &pump_fees_program,
        )
        .0;
        let Ok(global_config_key) = parse_program_id(PUMP_SWAP_GLOBAL_CONFIG) else {
            return PumpSwapRefreshReport::default();
        };
        let store = self.account_store();
        let pool_snapshots = store.snapshots_by_owner(&pump_swap_program);
        let mut report = PumpSwapRefreshReport {
            pool_accounts_seen: pool_snapshots.len(),
            ..PumpSwapRefreshReport::default()
        };
        let Ok(kamino_reserve_key) = parse_program_id(KAMINO_WSOL_RESERVE) else {
            return PumpSwapRefreshReport::default();
        };
        let Ok(wsol_mint_key) = parse_program_id(WSOL_MINT) else {
            return PumpSwapRefreshReport::default();
        };
        let mut dependencies = Vec::with_capacity(pool_snapshots.len().saturating_mul(5) + 4);
        dependencies.extend([
            fee_config_key,
            global_config_key,
            kamino_reserve_key,
            wsol_mint_key,
        ]);
        let mut decoded = Vec::with_capacity(pool_snapshots.len());
        for snapshot in &pool_snapshots {
            let Ok(header) = PumpSwapPoolHeader::decode(&snapshot.data) else {
                continue;
            };
            dependencies.extend([
                snapshot.pubkey,
                header.base_vault,
                header.quote_vault,
                header.base_mint,
                header.quote_mint,
            ]);
            decoded.push((snapshot.clone(), header));
        }
        dependencies.sort_unstable();
        dependencies.dedup();
        report.dependencies_tracked = dependencies.len();
        if self.set_tracked_accounts(&dependencies).is_err() {
            report.pools_rejected = decoded.len();
            return report;
        }

        let current_slot = self.current_slot();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0)
            .min(u64::MAX as u128) as u64;

        for (pool_snapshot, _header) in decoded {
            if current_slot > 0
                && current_slot.saturating_sub(pool_snapshot.slot) > max_state_age_slots
            {
                report.pools_rejected += 1;
                continue;
            }
            let joined = match join_pump_swap_state(&store, pool_snapshot.pubkey, fee_config_key) {
                Ok(state) => state,
                Err(_) => {
                    report.pools_rejected += 1;
                    continue;
                }
            };
            report.pools_joined += 1;
            // Mayhem and holder-reward pools have valid direct quote math when their
            // live fee components are decoded. Cashback affects instruction-version
            // compatibility, not the quote curve; the transaction builder must choose
            // a supported instruction variant before execution.
            if !joined.trading_enabled().unwrap_or(false) {
                report.pools_rejected += 1;
                continue;
            }
            // Fee/global configuration may be older than pool state, but must remain
            // within the same configured freshness window. A stale fee schedule can
            // otherwise produce a false-positive edge even when vaults are fresh.
            if current_slot > 0
                && (current_slot.saturating_sub(joined.fee_config_account.slot)
                    > max_state_age_slots
                    || current_slot.saturating_sub(joined.global_config_account.slot)
                        > max_state_age_slots)
            {
                report.pools_rejected += 1;
                continue;
            }
            let fee_config = match PumpFeesFeeConfig::decode(&joined.fee_config_account.data) {
                Ok(config) if config.bump == joined.fee_config_bump => config,
                _ => {
                    report.pools_rejected += 1;
                    continue;
                }
            };
            let fee = match joined.select_fee(&fee_config) {
                Ok(fee) => fee,
                Err(_) => {
                    report.pools_rejected += 1;
                    continue;
                }
            };
            let pool = match joined.clone().into_pool(fee) {
                Ok(pool) => pool,
                Err(_) => {
                    report.pools_rejected += 1;
                    continue;
                }
            };
            // The graph is a candidate detector and currently stores integer BPS.
            // Round the PPM fee UP to the next BPS so candidate discovery is conservative;
            // exact execution quotes must call PumpSwapPool::quote_exact_in.
            let total_ppm = match pool.fee.total_ppm() {
                Ok(value) => value as u64,
                Err(_) => {
                    report.pools_rejected += 1;
                    continue;
                }
            };
            let conservative_fee_bps = total_ppm.saturating_add(99) / 100;
            if conservative_fee_bps > 10_000 {
                report.pools_rejected += 1;
                continue;
            }
            let state = PoolState {
                pool_id: PoolId(pool_snapshot.pubkey),
                protocol: DexProtocol::PumpSwap,
                slot: Slot(joined.slot),
                write_version: pool_snapshot.write_version,
                base_mint: MintId(joined.header.base_mint),
                quote_mint: MintId(joined.header.quote_mint),
                decimals_a: joined.base_decimals,
                decimals_b: joined.quote_decimals,
                reserve_in: pool.base_reserve as u128,
                reserve_out: pool.quote_reserve as u128,
                fee_numerator: conservative_fee_bps,
                fee_denominator: 10_000,
                is_active: true,
                has_risky_token: false,
                kind: PoolKind::Cpmm,
                clmm: None,
                dlmm: None,
                pump_fun: None,
                token_programs: Default::default(),
                execution_accounts: None,
                transfer_fee_bps: 0,
                last_update_slot: joined.slot,
                last_update_ms: now_ms,
            };
            match self.apply_pool_update(state) {
                Ok(true) => report.pools_promoted += 1,
                Ok(false) => {}
                Err(_) => report.pools_rejected += 1,
            }
        }
        report
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PumpSwapGlobalConfigState {
    pub disable_flags: u8,
    pub creator_fee_configurable: bool,
    pub max_configurable_creator_fee_bps: u64,
}

/// PumpSwap GlobalConfig Borsh layout. Historical 940-byte accounts end at
/// `boost_enabled`; current 949-byte accounts append `creator_fee_configurable`
/// at byte 940 and `max_configurable_creator_fee_bps` at bytes 941..949.
pub fn decode_pump_swap_global_config(data: &[u8]) -> Result<PumpSwapGlobalConfigState> {
    if data.len() < 940
        || data.get(..8) != Some(PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR.as_slice())
        || (data.len() > 940 && data.len() < 949)
    {
        return Err(ArbError::DeserializationError(
            "invalid or truncated PumpSwap GlobalConfig".into(),
        ));
    }
    let (creator_fee_configurable, max_configurable_creator_fee_bps) = if data.len() >= 949 {
        let configurable = match data[940] {
            0 => false,
            1 => true,
            _ => {
                return Err(ArbError::DeserializationError(
                    "invalid creator_fee_configurable flag".into(),
                ));
            }
        };
        let max_fee = u64::from_le_bytes(data[941..949].try_into().map_err(|_| {
            ArbError::DeserializationError("truncated max_configurable_creator_fee_bps".into())
        })?);
        (configurable, max_fee)
    } else {
        (false, 0)
    };
    Ok(PumpSwapGlobalConfigState {
        disable_flags: data[56],
        creator_fee_configurable,
        max_configurable_creator_fee_bps,
    })
}

pub fn decode_pump_swap_disable_flags(data: &[u8]) -> Result<u8> {
    Ok(decode_pump_swap_global_config(data)?.disable_flags)
}

fn associated_token_address(
    authority: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
    associated_token_program: &Pubkey,
) -> Pubkey {
    Pubkey::find_program_address(
        &[authority.as_ref(), token_program.as_ref(), mint.as_ref()],
        associated_token_program,
    )
    .0
}

fn required_snapshot(
    store: &AccountSnapshotStore,
    key: &Pubkey,
    label: &str,
) -> Result<AccountSnapshot> {
    store.get(key).ok_or_else(|| {
        ArbError::InvalidInput(format!("missing Yellowstone snapshot for {label}: {key}"))
    })
}

fn require_owner(snapshot: &AccountSnapshot, expected: &Pubkey, label: &str) -> Result<()> {
    if snapshot.owner != *expected {
        return Err(ArbError::InvalidInput(format!(
            "{label} owner mismatch: got {}, expected {}",
            snapshot.owner, expected
        )));
    }
    Ok(())
}

fn decode_token_account(
    snapshot: &AccountSnapshot,
    expected_mint: &Pubkey,
    expected_authority: &Pubkey,
    token_program: &Pubkey,
    token_2022_program: &Pubkey,
    label: &str,
) -> Result<(u64, Pubkey)> {
    if snapshot.data.len() < TOKEN_ACCOUNT_BASE_LEN {
        return Err(ArbError::DeserializationError(format!(
            "{label} token account too short: {}",
            snapshot.data.len()
        )));
    }
    let program = snapshot.owner;
    if program != *token_program && program != *token_2022_program {
        return Err(ArbError::InvalidInput(format!(
            "{label} has unsupported token program owner {program}"
        )));
    }
    let mint = read_pubkey(&snapshot.data, 0, label)?;
    let authority = read_pubkey(&snapshot.data, 32, label)?;
    if mint != *expected_mint || authority != *expected_authority {
        return Err(ArbError::InvalidInput(format!(
            "{label} mint/authority mismatch"
        )));
    }
    // SPL AccountState: 0=uninitialized, 1=initialized, 2=frozen.
    if snapshot.data[108] != 1 {
        return Err(ArbError::InvalidInput(format!(
            "{label} is not initialized and unfrozen (state={})",
            snapshot.data[108]
        )));
    }
    let amount = u64::from_le_bytes(
        snapshot.data[64..72]
            .try_into()
            .map_err(|_| ArbError::DeserializationError(format!("invalid {label} amount")))?,
    );
    Ok((amount, program))
}

fn decode_mint(
    snapshot: &AccountSnapshot,
    expected_token_program: &Pubkey,
    label: &str,
) -> Result<(u8, u64)> {
    if snapshot.owner != *expected_token_program {
        return Err(ArbError::InvalidInput(format!(
            "{label} token program differs from its vault token program"
        )));
    }
    if snapshot.data.len() < MINT_BASE_LEN {
        return Err(ArbError::DeserializationError(format!(
            "{label} account too short: {}",
            snapshot.data.len()
        )));
    }
    if snapshot.data[45] != 1 {
        return Err(ArbError::InvalidInput(format!(
            "{label} is not initialized"
        )));
    }
    let supply = u64::from_le_bytes(
        snapshot.data[36..44]
            .try_into()
            .map_err(|_| ArbError::DeserializationError(format!("invalid {label} supply")))?,
    );
    Ok((snapshot.data[44], supply))
}

fn read_pubkey(data: &[u8], offset: usize, label: &str) -> Result<Pubkey> {
    let bytes: [u8; 32] = data
        .get(offset..offset + 32)
        .ok_or_else(|| ArbError::DeserializationError(format!("truncated {label} pubkey")))?
        .try_into()
        .map_err(|_| ArbError::DeserializationError(format!("invalid {label} pubkey")))?;
    Ok(Pubkey::new_from_array(bytes))
}

fn parse_program_id(value: &str) -> Result<Pubkey> {
    Pubkey::from_str(value)
        .map_err(|_| ArbError::InvalidInput(format!("invalid configured program id: {value}")))
}

#[cfg(test)]
#[path = "pump_state_tests.rs"]
mod tests;
