//! Live reserve hydration for [`MultiDexRegistry`]: batched vault fetches,
//! mint-owner verification and per-pool `PoolState` construction.

use crate::fetcher::CONSECUTIVE_RPC_FAILURES;
use arb_types::{DexProtocol, PoolState, Slot, TokenProgramKind};
use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::sync::atomic::Ordering;
use tracing::{debug, info, warn};

const SOLANA_CLOCK_SYSVAR_ADDRESS: &str = "SysvarC1ock11111111111111111111111111111111";
const SOLANA_SYSVAR_OWNER_ADDRESS: &str = "Sysvar1111111111111111111111111111111111111";

/// Resolve token program from the mint account owner and validate its base layout.
/// Token-2022 is identified but remains ineligible until epoch-aware fees and
/// TransferHook account resolution are supported through quote and execution.
fn decode_mint_program_and_decimals(owner: &Pubkey, data: &[u8]) -> Option<(TokenProgramKind, u8)> {
    if data.len() < 82 || data[45] != 1 || data[44] > 18 {
        return None;
    }
    let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).ok()?;
    let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).ok()?;
    let program = if *owner == spl {
        if data.len() != 82 {
            return None;
        }
        TokenProgramKind::SplToken
    } else if *owner == token_2022 {
        if data.len() > 82 && (data.len() < 166 || data[165] != 1) {
            return None;
        }
        TokenProgramKind::Token2022
    } else {
        return None;
    };
    Some((program, data[44]))
}

/// DAMM v2 Pool, both vaults, both mints and Clock must share one RPC context.
fn coherent_damm_v2_snapshot_slot(context_slot: u64, slots: [Option<u64>; 6]) -> Option<u64> {
    if context_slot > 0 && slots.iter().all(|slot| *slot == Some(context_slot)) {
        Some(context_slot)
    } else {
        None
    }
}

/// Build a DAMM v2 quote state only from a complete same-context Pool + vaults
/// + mints + Clock snapshot. Shared by HTTP hydration and WebSocket refresh.
pub(crate) fn decode_damm_v2_coherent_pool_state(
    pool_id: PoolId,
    context_slot: u64,
    pool_bytes: &[u8],
    pool_owner: Pubkey,
    vault_a_address: Pubkey,
    vault_b_address: Pubkey,
    mint_a_address: Pubkey,
    mint_b_address: Pubkey,
    vault_a_bytes: &[u8],
    vault_a_owner: Pubkey,
    vault_b_bytes: &[u8],
    vault_b_owner: Pubkey,
    mint_a_bytes: &[u8],
    mint_a_owner: Pubkey,
    mint_b_bytes: &[u8],
    mint_b_owner: Pubkey,
    clock_bytes: &[u8],
    clock_owner: Pubkey,
) -> Option<PoolState> {
    let damm_program = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_PROGRAM_ID).ok()?;
    let spl_token_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).ok()?;
    let token_2022_program = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).ok()?;
    let pool_authority = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_POOL_AUTHORITY).ok()?;
    let expected_clock_owner = Pubkey::from_str(SOLANA_SYSVAR_OWNER_ADDRESS).ok()?;

    if context_slot == 0
        || pool_owner != damm_program
        || clock_owner != expected_clock_owner
        || clock_bytes.len() != 40
    {
        return None;
    }

    let mut pool = arb_dex::MeteoraDammV2Pool::from_account(pool_id, pool_bytes).ok()?;
    if pool.authority != pool_authority
        || pool.token_a_vault != vault_a_address
        || pool.token_b_vault != vault_b_address
        || pool.token_a_mint.0 != mint_a_address
        || pool.token_b_mint.0 != mint_b_address
    {
        return None;
    }

    let expected_a_program = match pool.token_a_program {
        arb_types::TokenProgramKind::SplToken => spl_token_program,
        arb_types::TokenProgramKind::Token2022 => token_2022_program,
    };
    let expected_b_program = match pool.token_b_program {
        arb_types::TokenProgramKind::SplToken => spl_token_program,
        arb_types::TokenProgramKind::Token2022 => token_2022_program,
    };
    if mint_a_owner != expected_a_program
        || mint_b_owner != expected_b_program
        || vault_a_owner != expected_a_program
        || vault_b_owner != expected_b_program
    {
        return None;
    }

    let mint_decimals = |bytes: &[u8]| -> Option<u8> {
        if bytes.len() < 82 || bytes[45] != 1 || bytes[44] > 18 {
            return None;
        }
        if bytes.len() > 82 && (bytes.len() < 166 || bytes[165] != 1) {
            return None;
        }
        Some(bytes[44])
    };
    let decimals_a = mint_decimals(mint_a_bytes)?;
    let decimals_b = mint_decimals(mint_b_bytes)?;

    let vault_amount = |bytes: &[u8], expected_mint: Pubkey| -> Option<u64> {
        if bytes.len() < 72 || bytes[0..32] != expected_mint.to_bytes() {
            return None;
        }
        let authority = Pubkey::new_from_array(bytes[32..64].try_into().ok()?);
        if authority != pool.authority {
            return None;
        }
        Some(u64::from_le_bytes(bytes[64..72].try_into().ok()?))
    };
    let vault_a_amount = vault_amount(vault_a_bytes, pool.token_a_mint.0)?;
    let vault_b_amount = vault_amount(vault_b_bytes, pool.token_b_mint.0)?;
    if vault_a_amount == 0 || vault_b_amount == 0 {
        return None;
    }

    pool.has_risky = (pool.token_a_program == arb_types::TokenProgramKind::Token2022
        && arb_dex::token2022_has_risky_extension(mint_a_bytes))
        || (pool.token_b_program == arb_types::TokenProgramKind::Token2022
            && arb_dex::token2022_has_risky_extension(mint_b_bytes));

    let clock = arb_dex::DammV2ClockSnapshot::decode(clock_bytes, context_slot)?;
    if !pool.set_clock_snapshot(clock, context_slot) || !pool.is_tradable() {
        return None;
    }
    let fee_numerator = u64::try_from(pool.fee_params.effective_fee_numerator_at(
        pool.current_point,
        pool.activation_point,
        pool.sqrt_price_x64,
    )?)
    .ok()?;

    // Compounding uses tracked V1 counters, never physical vault balances.
    let (reserve_a, reserve_b) = if pool.fee_params.collect_fee_mode == 2 {
        if pool.layout_version != 1 {
            return None;
        }
        (
            u128::from(pool.token_a_amount?),
            u128::from(pool.token_b_amount?),
        )
    } else {
        arb_math::damm_v2_virtual_reserves(pool.sqrt_price_x64, pool.liquidity)
    };
    if reserve_a == 0 || reserve_b == 0 {
        return None;
    }

    let damm_quote_state = pool.to_quote_state(true);
    if !damm_quote_state.account_graph_verified {
        return None;
    }
    let execution_accounts = arb_types::PoolExecutionAccounts::MeteoraDammV2 {
        token_a_mint: pool.token_a_mint.0,
        token_b_mint: pool.token_b_mint.0,
        authority: pool.authority,
        vault_a: pool.token_a_vault,
        vault_b: pool.token_b_vault,
        event_authority: pool.event_authority,
    };
    Some(PoolState {
        pool_id,
        protocol: DexProtocol::MeteoraDammV2,
        slot: Slot(context_slot),
        write_version: 0,
        base_mint: pool.token_a_mint,
        quote_mint: pool.token_b_mint,
        decimals_a,
        decimals_b,
        reserve_in: reserve_a,
        reserve_out: reserve_b,
        fee_numerator,
        fee_denominator: 1_000_000_000,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: pool.has_risky,
        kind: if pool.fee_params.collect_fee_mode == 2 {
            arb_types::PoolKind::Cpmm
        } else {
            arb_types::PoolKind::Clmm
        },
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: Some(damm_quote_state),
        transfer_fee_bps: 0,
        token_programs: [pool.token_a_program, pool.token_b_program],
        execution_accounts: Some(execution_accounts),
        last_update_slot: context_slot,
        last_update_ms: 0,
    })
}
/// CPMM pool, both vaults and AmmConfig must be read from one identical RPC
/// context slot. Mixing snapshots can combine pre-swap counters with
/// post-swap vault balances and create a false curve reserve.
fn coherent_cpmm_snapshot_slot(
    pool_slot: Option<u64>,
    vault_0_slot: Option<u64>,
    vault_1_slot: Option<u64>,
    config_slot: Option<u64>,
) -> Option<u64> {
    match (pool_slot, vault_0_slot, vault_1_slot, config_slot) {
        (Some(pool), Some(v0), Some(v1), Some(config))
            if pool > 0 && pool == v0 && pool == v1 && pool == config =>
        {
            Some(pool)
        }
        _ => None,
    }
}

use super::MultiDexRegistry;
use super::registry::*;
use crate::rpc_pool::RpcPool;
use arb_types::{MintId, PoolId};
use std::str::FromStr;

impl MultiDexRegistry {
    /// Batch-fetch LIVE reserve balances for every registered pool in a single
    /// `getMultipleAccounts` call and produce fresh `PoolState`s. Returns only
    /// pools whose both reserves decoded to a non-zero live balance.
    ///
    /// This is the call that ends `MarketGraph` starvation: with opposing Raydium
    /// / Orca / Meteora SOL-USDC pools populated, `find_base_cycles` immediately
    /// discovers real cross-DEX 2-hop arbitrage.
    /// Batched `getMultipleAccounts` (batch-100) through the registry's shared
    /// `RpcPool` — per-provider token buckets, backoffs, and telemetry apply.
    ///
    /// This replaced the previous per-account `PacedAccountFetcher` fan-out:
    /// ~163 pool + ~68 vault + ~200 derived accounts fetched ONE BY ONE every
    /// 750ms cycle (the worker's "parallel batch-100" log label was false).
    /// That is 300-400 HTTP requests per cycle against free-tier endpoints,
    /// guaranteeing permanent 429 cooldowns, which starved every downstream
    /// consumer (seeding, hydration, health, Jito) into fail-closed zeros.
    /// Batch-100 collapses the same data to 4-6 requests per cycle (~60x less).
    async fn batch_fetch_accounts(
        rpc: &RpcPool,
        keys: &[Pubkey],
        into: &mut std::collections::HashMap<Pubkey, (u64, Vec<u8>)>,
        mut owner_into: Option<&mut std::collections::HashMap<Pubkey, Pubkey>>,
    ) -> u64 {
        let mut max_slot = 0u64;
        for chunk in keys.chunks(100) {
            let refs_owned: Vec<String> = chunk.iter().map(|k| k.to_string()).collect();
            let refs: Vec<&str> = refs_owned.iter().map(|s| s.as_str()).collect();
            let Ok((accounts, slots)) = rpc.get_multiple_accounts_with_slots(&refs).await else {
                // RPC starvation used to be swallowed here: pools silently went
                // stale and every downstream tick was skipped (283 skipped ticks
                // measured in one 150s run). Make the failure LOUD on transition
                // and every 20th consecutive failure, plus an explicit recovery.
                let n = CONSECUTIVE_RPC_FAILURES.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 || n.is_multiple_of(20) {
                    warn!(
                        "batch_fetch_accounts: RPC failure x{n} consecutive (chunk of {} keys) — pools will go stale and ticks will be skipped (fail closed); check RPC provider credits/limits",
                        chunk.len()
                    );
                }
                continue;
            };
            let slot = slots.iter().copied().max().unwrap_or(0);
            if CONSECUTIVE_RPC_FAILURES.swap(0, Ordering::Relaxed) > 0 {
                info!(
                    "batch_fetch_accounts: RPC recovered after consecutive failures (slot {slot})"
                );
            }
            max_slot = max_slot.max(slot);
            // DATA-01: stamp each account with the slot of the response that
            // carried it — the sub-chunk fallback fans out across endpoints,
            // so a single max would mask a slow replica's staleness.
            for ((val, chunk_slot), pk) in accounts.into_iter().zip(slots).zip(chunk.iter()) {
                let Some(acc) = val else { continue };
                if let Some(owner_map) = owner_into.as_deref_mut()
                    && let Some(owner) = acc
                        .get("owner")
                        .and_then(|value| value.as_str())
                        .and_then(|value| Pubkey::from_str(value).ok())
                {
                    owner_map.insert(*pk, owner);
                }
                let Some(raw_b64) = acc
                    .get("data")
                    .and_then(|d| d.get(0))
                    .and_then(|s| s.as_str())
                else {
                    continue;
                };
                if let Ok(bytes) = BASE64_STANDARD.decode(raw_b64) {
                    into.insert(*pk, (chunk_slot, bytes));
                }
            }
        }
        max_slot
    }

    pub async fn fetch_live_pool_states(&self, _current_slot: Slot) -> Vec<PoolState> {
        if self.pools.is_empty() {
            return Vec::new();
        }

        let mut pool_data_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        let mut pool_owner_map: std::collections::HashMap<Pubkey, Pubkey> =
            std::collections::HashMap::new();
        // 1. Pool and previously unseen pair-mint accounts. Mint owners and
        // decimals are immutable after initialization, so cache verified metadata
        // and fetch only on first sight or when a new pool introduces a new mint.
        let mut pool_pubkeys: Vec<Pubkey> = self.pools.iter().map(|p| p.pool_id.0).collect();
        let mut mint_pubkeys = Vec::new();
        let mut seen_accounts: std::collections::HashSet<Pubkey> =
            pool_pubkeys.iter().copied().collect();
        {
            let cache = self.mint_metadata_cache.lock();
            for pool in &self.pools {
                if pool.protocol == DexProtocol::MeteoraDammV2 {
                    continue; // DAMM v2 has its own coherent Pool/Vault/Mint/Clock fetch.
                }
                for mint in [pool.base_mint.0, pool.quote_mint.0] {
                    if !cache.contains_key(&mint) && seen_accounts.insert(mint) {
                        mint_pubkeys.push(mint);
                    }
                }
            }
        }
        pool_pubkeys.extend(mint_pubkeys.iter().copied());
        let mut max_observed_slot = Self::batch_fetch_accounts(
            &self.rpc,
            &pool_pubkeys,
            &mut pool_data_map,
            Some(&mut pool_owner_map),
        )
        .await;

        // Cache only successfully decoded, owner-verified Mint accounts. A
        // missing/malformed response is not cached, so the next tick can retry.
        if !mint_pubkeys.is_empty() {
            let mut cache = self.mint_metadata_cache.lock();
            for mint in &mint_pubkeys {
                let Some((slot, data)) = pool_data_map.get(mint) else {
                    continue;
                };
                let Some(owner) = pool_owner_map.get(mint) else {
                    continue;
                };
                if *slot > 0
                    && let Some(metadata) = decode_mint_program_and_decimals(owner, data)
                {
                    cache.insert(*mint, metadata);
                }
            }
        }

        // 2. CPMM vault accounts — batched (1 batch-100 call for ~68 vaults).
        let mut vault_data_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        let mut vault_owner_map: std::collections::HashMap<Pubkey, Pubkey> =
            std::collections::HashMap::new();
        let mut vault_pubkeys: Vec<Pubkey> = Vec::new();
        for p in &self.pools {
            if matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            ) {
                if !vault_pubkeys.contains(&p.base_vault) {
                    vault_pubkeys.push(p.base_vault);
                }
                if !vault_pubkeys.contains(&p.quote_vault) {
                    vault_pubkeys.push(p.quote_vault);
                }
            }
        }
        let vault_slot = Self::batch_fetch_accounts(
            &self.rpc,
            &vault_pubkeys,
            &mut vault_data_map,
            Some(&mut vault_owner_map),
        )
        .await;
        max_observed_slot = max_observed_slot.max(vault_slot);

        // 2b. CPMM AmmConfig accounts — the swap fee lives here (trade_fee_rate
        // u64 LE @12), not in the pool account. Batch-fetched so the fee is
        // always the live on-chain value, never a catalog guess.
        let mut cpmm_config_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        let mut cpmm_config_owner_map: std::collections::HashMap<Pubkey, Pubkey> =
            std::collections::HashMap::new();
        let mut cpmm_config_pubkeys: Vec<Pubkey> = Vec::new();
        for p in &self.pools {
            if p.protocol != DexProtocol::RaydiumCpmm {
                continue;
            }
            if let Some((_, data)) = pool_data_map.get(&p.pool_id.0)
                && let Some(cfg) = raydium_cpmm_amm_config_key(data)
                && !cpmm_config_pubkeys.contains(&cfg)
            {
                cpmm_config_pubkeys.push(cfg);
            }
        }
        let cpmm_cfg_slot = Self::batch_fetch_accounts(
            &self.rpc,
            &cpmm_config_pubkeys,
            &mut cpmm_config_map,
            Some(&mut cpmm_config_owner_map),
        )
        .await;
        max_observed_slot = max_observed_slot.max(cpmm_cfg_slot);

        // 2c. Re-read each CPMM dependency set in ONE getMultipleAccounts
        // context. The first discovery pass cannot know vault/config addresses
        // until PoolState is decoded, so its pool, vault and AmmConfig reads
        // necessarily came from separate RPC contexts. Comparing those slots
        // for equality rejected healthy pools on every tick. This second pass
        // binds PoolState + both vaults + AmmConfig to the SAME RPC response
        // context, preventing mixed pre/post-swap fee counters and balances.
        // Keep groups bounded (<= 20 pools / 80 accounts) for free-tier RPCs.
        let cpmm_pools: Vec<_> = self
            .pools
            .iter()
            .filter(|p| p.protocol == DexProtocol::RaydiumCpmm)
            .filter_map(|p| {
                let (_, pool_data) = pool_data_map.get(&p.pool_id.0)?;
                let state = crate::decoders::decode_raydium_cpmm_pool_accounting(pool_data)?;
                Some((
                    p.pool_id,
                    [
                        p.pool_id.0,
                        state.token_0_vault,
                        state.token_1_vault,
                        state.amm_config,
                    ],
                ))
            })
            .collect();
        for group in cpmm_pools.chunks(20) {
            let mut keys = Vec::with_capacity(group.len() * 4);
            for (_, group_keys) in group {
                for key in group_keys {
                    if !keys.contains(key) {
                        keys.push(*key);
                    }
                }
            }
            let mut coherent_data = std::collections::HashMap::new();
            let mut coherent_owners = std::collections::HashMap::new();
            let coherent_slot = Self::batch_fetch_accounts(
                &self.rpc,
                &keys,
                &mut coherent_data,
                Some(&mut coherent_owners),
            )
            .await;
            if coherent_slot == 0 {
                debug!(
                    "Raydium CPMM coherent dependency refresh failed for group of {} pools",
                    group.len()
                );
                continue;
            }
            for (pool_id, [pool_key, vault_0, vault_1, config_key]) in group {
                let required = [*pool_key, *vault_0, *vault_1, *config_key];
                if required.iter().any(|key| {
                    !coherent_data.contains_key(key) || !coherent_owners.contains_key(key)
                }) {
                    debug!(
                        "Raydium CPMM {pool_id} coherent dependency response incomplete at slot {coherent_slot}"
                    );
                    continue;
                }
                for key in required {
                    let Some((slot, bytes)) = coherent_data.get(&key).cloned() else {
                        continue;
                    };
                    let Some(owner) = coherent_owners.get(&key).copied() else {
                        continue;
                    };
                    if key == *pool_key {
                        pool_data_map.insert(key, (slot, bytes));
                        pool_owner_map.insert(key, owner);
                    } else if key == *config_key {
                        cpmm_config_map.insert(key, (slot, bytes));
                        cpmm_config_owner_map.insert(key, owner);
                    } else {
                        vault_data_map.insert(key, (slot, bytes));
                        vault_owner_map.insert(key, owner);
                    }
                }
            }
            max_observed_slot = max_observed_slot.max(coherent_slot);
        }

        // 2d. Refresh every DAMM v2 dependency set in one getMultipleAccounts
        // request context: Pool + vault A/B + mint A/B + Clock. The earlier
        // pool discovery read is never promoted as a quote snapshot.
        let damm_program = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_PROGRAM_ID).ok();
        let clock_sysvar = Pubkey::from_str(SOLANA_CLOCK_SYSVAR_ADDRESS).ok();
        let damm_candidates: Vec<_> = self
            .pools
            .iter()
            .filter(|p| p.protocol == DexProtocol::MeteoraDammV2)
            .filter_map(|p| {
                let (pool_slot, bytes) = pool_data_map.get(&p.pool_id.0)?;
                if bytes.len() < arb_dex::meteora_damm_v2::METEORA_DAMM_V2_POOL_MIN_SIZE
                    || pool_owner_map.get(&p.pool_id.0) != damm_program.as_ref()
                    || *pool_slot == 0
                {
                    return None;
                }
                let pool = arb_dex::MeteoraDammV2Pool::from_account(p.pool_id, bytes).ok()?;
                let clock = clock_sysvar?;
                Some((
                    p.pool_id,
                    [
                        p.pool_id.0,
                        pool.token_a_vault,
                        pool.token_b_vault,
                        pool.token_a_mint.0,
                        pool.token_b_mint.0,
                        clock,
                    ],
                ))
            })
            .collect();

        let mut damm_v2_pool_states: std::collections::HashMap<PoolId, PoolState> =
            std::collections::HashMap::new();
        for group in damm_candidates.chunks(16) {
            let mut keys = Vec::with_capacity(group.len() * 6);
            for (_, group_keys) in group {
                for key in group_keys {
                    if !keys.contains(key) {
                        keys.push(*key);
                    }
                }
            }
            let mut coherent_data = std::collections::HashMap::new();
            let mut coherent_owners = std::collections::HashMap::new();
            let coherent_slot = Self::batch_fetch_accounts(
                &self.rpc,
                &keys,
                &mut coherent_data,
                Some(&mut coherent_owners),
            )
            .await;
            if coherent_slot == 0 {
                debug!(
                    "Meteora DAMM v2 coherent dependency refresh failed for group of {} pools",
                    group.len()
                );
                continue;
            }
            for (pool_id, group_keys) in group {
                let required = *group_keys;
                let dependency_slots =
                    required.map(|key| coherent_data.get(&key).map(|(slot, _)| *slot));
                if coherent_damm_v2_snapshot_slot(coherent_slot, dependency_slots)
                    != Some(coherent_slot)
                {
                    debug!("Meteora DAMM v2 {pool_id} dependencies not coherent at context slot");
                    continue;
                }
                let Some((_, pool_bytes)) = coherent_data.get(&required[0]) else {
                    continue;
                };
                let Some(pool_owner) = coherent_owners.get(&required[0]) else {
                    continue;
                };
                let Some((_, vault_a_bytes)) = coherent_data.get(&required[1]) else {
                    continue;
                };
                let Some(vault_a_owner) = coherent_owners.get(&required[1]) else {
                    continue;
                };
                let Some((_, vault_b_bytes)) = coherent_data.get(&required[2]) else {
                    continue;
                };
                let Some(vault_b_owner) = coherent_owners.get(&required[2]) else {
                    continue;
                };
                let Some((_, mint_a_bytes)) = coherent_data.get(&required[3]) else {
                    continue;
                };
                let Some(mint_a_owner) = coherent_owners.get(&required[3]) else {
                    continue;
                };
                let Some((_, mint_b_bytes)) = coherent_data.get(&required[4]) else {
                    continue;
                };
                let Some(mint_b_owner) = coherent_owners.get(&required[4]) else {
                    continue;
                };
                let Some((_, clock_bytes)) = coherent_data.get(&required[5]) else {
                    continue;
                };
                let Some(clock_owner) = coherent_owners.get(&required[5]) else {
                    continue;
                };

                let Some(state) = decode_damm_v2_coherent_pool_state(
                    *pool_id,
                    coherent_slot,
                    pool_bytes,
                    *pool_owner,
                    required[1],
                    required[2],
                    required[3],
                    required[4],
                    vault_a_bytes,
                    *vault_a_owner,
                    vault_b_bytes,
                    *vault_b_owner,
                    mint_a_bytes,
                    *mint_a_owner,
                    mint_b_bytes,
                    *mint_b_owner,
                    clock_bytes,
                    *clock_owner,
                ) else {
                    debug!(
                        "Meteora DAMM v2 {pool_id} full coherent account graph validation failed"
                    );
                    continue;
                };
                damm_v2_pool_states.insert(*pool_id, state);
            }
            max_observed_slot = max_observed_slot.max(coherent_slot);
        }
        // 3. Fetch derived accounts (tick arrays / bin arrays) individually
        let mut derived_pubkeys: Vec<Pubkey> = Vec::new();
        for p in &self.pools {
            if let Some((_, data)) = pool_data_map.get(&p.pool_id.0) {
                match p.protocol {
                    DexProtocol::MeteoraDlmm => {
                        if decode_dlmm_state(data).is_some() {
                            // The extension must be fetched before bin-array
                            // selection; its bitmap, not a guessed price span,
                            // determines which arrays actually exist.
                            let ext = bitmap_extension_pda(&p.pool_id.0);
                            if !derived_pubkeys.contains(&ext) {
                                derived_pubkeys.push(ext);
                            }
                        }
                    }
                    DexProtocol::OrcaWhirlpool => {
                        if let Some(c) = decode_clmm_state(data) {
                            let (keys, _) =
                                clmm_tick_array_pdas(&p.pool_id, c.tick_current, c.tick_spacing);
                            for key in keys {
                                if !derived_pubkeys.contains(&key) {
                                    derived_pubkeys.push(key);
                                }
                            }
                        }
                    }
                    // Raydium CLMM is currently telemetry-only: catalog fee metadata is not
                    // a verified AmmConfig snapshot, and dynamic-fee state is not joined.
                    _ => {}
                }
            }
        }

        let mut derived_data_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        let mut derived_owner_map: std::collections::HashMap<Pubkey, Pubkey> =
            std::collections::HashMap::new();
        let derived_slot = Self::batch_fetch_accounts(
            &self.rpc,
            &derived_pubkeys,
            &mut derived_data_map,
            Some(&mut derived_owner_map),
        )
        .await;
        max_observed_slot = max_observed_slot.max(derived_slot);

        // Resolve actual initialized DLMM arrays from the LbPair inline bitmap
        // plus the existence- and owner-validated extension account. We fetch
        // the protocol worst-case initialized-array coverage in each direction;
        // missing bitmap entries are NOT
        // converted into fabricated writable accounts.
        let mut dlmm_selected_indices: std::collections::HashMap<PoolId, Vec<i64>> =
            std::collections::HashMap::new();
        let mut dlmm_array_pubkeys = Vec::new();
        for p in &self.pools {
            if p.protocol != DexProtocol::MeteoraDlmm {
                continue;
            }
            let Some((_, pool_bytes)) = pool_data_map.get(&p.pool_id.0) else {
                continue;
            };
            let Some(d) = decode_dlmm_state(pool_bytes) else {
                debug!(
                    "Meteora DLMM {} skipped: LbPair state decoder rejected account (len={})",
                    p.pool_id,
                    pool_bytes.len()
                );
                continue;
            };
            let Some(internal) = crate::decoders::decode_dlmm_internal_bitmap(pool_bytes) else {
                debug!(
                    "Meteora DLMM {} skipped: internal bitmap layout invalid (len={}, active_bin={})",
                    p.pool_id,
                    pool_bytes.len(),
                    d.active_bin_id
                );
                continue;
            };
            let extension_pda = bitmap_extension_pda(&p.pool_id.0);
            let extension = derived_data_map
                .get(&extension_pda)
                .zip(derived_owner_map.get(&extension_pda))
                .and_then(|((_, bytes), owner)| {
                    arb_dex::decode_bitmap_extension_account(&p.pool_id.0, owner, bytes).ok()
                });
            let mut indices = Vec::new();
            for swap_for_y in [true, false] {
                indices.extend(crate::decoders::dlmm_initialized_bin_array_indices(
                    d.active_bin_id,
                    swap_for_y,
                    &internal,
                    extension.as_ref(),
                    arb_constants::METEORA_MAX_BIN_ARRAYS_PER_SWAP,
                ));
            }
            indices.sort_unstable();
            indices.dedup();
            if indices.is_empty() {
                // Neither direction found a bitmap-proven initialized array.
                debug!(
                    "Meteora DLMM {} skipped: no initialized bin arrays around active_bin={} (extension_present={}, extension_owner_valid={})",
                    p.pool_id,
                    d.active_bin_id,
                    derived_data_map.contains_key(&extension_pda),
                    extension.is_some()
                );
                continue;
            }
            debug!(
                "Meteora DLMM {} selected {} initialized bin arrays around active_bin={} (extension_present={}, extension_valid={})",
                p.pool_id,
                indices.len(),
                d.active_bin_id,
                derived_data_map.contains_key(&extension_pda),
                extension.is_some()
            );
            for index in &indices {
                let key = crate::decoders::derive_meteora_bin_array_pda(&p.pool_id.0, *index);
                if !dlmm_array_pubkeys.contains(&key) {
                    dlmm_array_pubkeys.push(key);
                }
            }
            dlmm_selected_indices.insert(p.pool_id, indices);
        }

        let dlmm_array_slot = Self::batch_fetch_accounts(
            &self.rpc,
            &dlmm_array_pubkeys,
            &mut derived_data_map,
            Some(&mut derived_owner_map),
        )
        .await;
        max_observed_slot = max_observed_slot.max(dlmm_array_slot);

        // 3b. Refresh each DLMM pool + bitmap extension + its selected bin
        // arrays in the SAME getMultipleAccounts context. The earlier discovery
        // pass needs pool/extension state to derive array PDAs, so it cannot be
        // the final quote snapshot. Re-fetching this bounded dependency group
        // removes the measured 14-slot skew that previously caused every
        // Meteora pool to fail the strict coherence gate. If the bitmap or
        // active bin changed between discovery and refresh, discard the pool
        // for this cycle rather than using an incomplete/stale array set.
        let meteora_program = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID).ok();
        let mut coherent_dlmm_groups: Vec<Vec<(PoolId, Vec<Pubkey>)>> = Vec::new();
        let mut current_group = Vec::new();
        let mut current_group_size = 0usize;
        for p in &self.pools {
            if p.protocol != DexProtocol::MeteoraDlmm {
                continue;
            }
            let Some(indices) = dlmm_selected_indices.get(&p.pool_id) else {
                continue;
            };
            if indices.is_empty() {
                continue;
            }
            let mut keys = vec![p.pool_id.0, bitmap_extension_pda(&p.pool_id.0)];
            keys.extend(
                indices.iter().map(|index| {
                    crate::decoders::derive_meteora_bin_array_pda(&p.pool_id.0, *index)
                }),
            );
            keys.sort_unstable();
            keys.dedup();
            if !current_group.is_empty() && current_group_size + keys.len() > 100 {
                coherent_dlmm_groups.push(std::mem::take(&mut current_group));
                current_group_size = 0;
            }
            current_group_size += keys.len();
            current_group.push((p.pool_id, keys));
        }
        if !current_group.is_empty() {
            coherent_dlmm_groups.push(current_group);
        }

        for group in coherent_dlmm_groups {
            let mut keys = Vec::new();
            for (_, group_keys) in &group {
                for key in group_keys {
                    if !keys.contains(key) {
                        keys.push(*key);
                    }
                }
            }
            let mut coherent_data = std::collections::HashMap::new();
            let mut coherent_owners = std::collections::HashMap::new();
            let coherent_slot = Self::batch_fetch_accounts(
                &self.rpc,
                &keys,
                &mut coherent_data,
                Some(&mut coherent_owners),
            )
            .await;
            if coherent_slot == 0 {
                for (pool_id, _) in &group {
                    dlmm_selected_indices.insert(*pool_id, Vec::new());
                }
                continue;
            }
            for (pool_id, group_keys) in group {
                let pool_key = pool_id.0;
                let extension_key = bitmap_extension_pda(&pool_key);
                let Some((pool_slot, pool_bytes)) = coherent_data.get(&pool_key).cloned() else {
                    dlmm_selected_indices.insert(pool_id, Vec::new());
                    continue;
                };
                let pool_owner = coherent_owners.get(&pool_key).copied();
                if pool_slot == 0 || pool_owner != meteora_program {
                    debug!(
                        "Meteora DLMM {pool_id} coherent refresh rejected: invalid LbPair owner/slot"
                    );
                    dlmm_selected_indices.insert(pool_id, Vec::new());
                    continue;
                }
                let Some(refreshed_state) = decode_dlmm_state(&pool_bytes) else {
                    dlmm_selected_indices.insert(pool_id, Vec::new());
                    continue;
                };
                let Some(internal) = crate::decoders::decode_dlmm_internal_bitmap(&pool_bytes)
                else {
                    dlmm_selected_indices.insert(pool_id, Vec::new());
                    continue;
                };
                let extension = coherent_data
                    .get(&extension_key)
                    .zip(coherent_owners.get(&extension_key))
                    .and_then(|((_, bytes), owner)| {
                        arb_dex::decode_bitmap_extension_account(&pool_key, owner, bytes).ok()
                    });
                let mut refreshed_indices = Vec::new();
                for swap_for_y in [true, false] {
                    refreshed_indices.extend(crate::decoders::dlmm_initialized_bin_array_indices(
                        refreshed_state.active_bin_id,
                        swap_for_y,
                        &internal,
                        extension.as_ref(),
                        arb_constants::METEORA_MAX_BIN_ARRAYS_PER_SWAP,
                    ));
                }
                refreshed_indices.sort_unstable();
                refreshed_indices.dedup();
                let mut expected_indices = dlmm_selected_indices
                    .get(&pool_id)
                    .cloned()
                    .unwrap_or_default();
                expected_indices.sort_unstable();
                expected_indices.dedup();
                if refreshed_indices != expected_indices {
                    debug!(
                        "Meteora DLMM {pool_id} changed active bin/bitmap during coherent refresh; discarding this cycle"
                    );
                    dlmm_selected_indices.insert(pool_id, Vec::new());
                    continue;
                }
                let dependencies_present = group_keys.iter().all(|key| {
                    if *key == extension_key {
                        // The extension is optional only when the refreshed
                        // bitmap proves the selected arrays fit the default range.
                        coherent_data.get(key).is_none_or(|(slot, _)| {
                            *slot == pool_slot && coherent_owners.contains_key(key)
                        })
                    } else {
                        coherent_data.get(key).is_some_and(|(slot, _)| {
                            *slot == pool_slot && coherent_owners.contains_key(key)
                        })
                    }
                });
                if !dependencies_present {
                    debug!(
                        "Meteora DLMM {pool_id} coherent refresh missing one or more selected arrays"
                    );
                    dlmm_selected_indices.insert(pool_id, Vec::new());
                    continue;
                }
                pool_data_map.insert(pool_key, (pool_slot, pool_bytes));
                if let Some(owner) = pool_owner {
                    pool_owner_map.insert(pool_key, owner);
                }
                for key in group_keys {
                    if let (Some((slot, bytes)), Some(owner)) =
                        (coherent_data.get(&key), coherent_owners.get(&key))
                    {
                        derived_data_map.insert(key, (*slot, bytes.clone()));
                        derived_owner_map.insert(key, *owner);
                    }
                }
                debug!(
                    "Meteora DLMM {pool_id} coherent snapshot refreshed at slot {coherent_slot} with {} bin arrays",
                    expected_indices.len()
                );
            }
            max_observed_slot = max_observed_slot.max(coherent_slot);
        }

        let mut dlmm_bins: std::collections::HashMap<PoolId, Vec<arb_types::DlmmBin>> =
            std::collections::HashMap::new();
        // Conservative slot for each DLMM dependency group. Unlike the
        // workspace-wide max slot, this records the OLDEST account in the
        // pool/extension/bin-array set so freshness can never be overstated.
        let mut dlmm_coherent_slots: std::collections::HashMap<PoolId, Slot> =
            std::collections::HashMap::new();
        let mut clmm_ticks: std::collections::HashMap<PoolId, Vec<arb_types::ClmmTickArray>> =
            std::collections::HashMap::new();

        for p in &self.pools {
            if let Some((_, data)) = pool_data_map.get(&p.pool_id.0) {
                match p.protocol {
                    DexProtocol::MeteoraDlmm => {
                        if let Some(d) = decode_dlmm_state(data) {
                            let mut by_bin_id = std::collections::BTreeMap::new();
                            let Some(indices) = dlmm_selected_indices.get(&p.pool_id) else {
                                dlmm_bins.insert(p.pool_id, Vec::new());
                                continue;
                            };
                            let pool_slot = pool_data_map
                                .get(&p.pool_id.0)
                                .map(|(slot, _)| *slot)
                                .unwrap_or(0);
                            let mut min_dependency_slot = pool_slot;
                            let mut max_dependency_slot = pool_slot;
                            let mut complete = pool_slot > 0;

                            if crate::decoders::dlmm_requires_bitmap_extension(
                                d.active_bin_id,
                                indices,
                            ) {
                                let extension_pda = bitmap_extension_pda(&p.pool_id.0);
                                match (
                                    derived_data_map.get(&extension_pda),
                                    derived_owner_map.get(&extension_pda),
                                ) {
                                    (Some((slot, bytes)), Some(owner))
                                        if *slot > 0
                                            && arb_dex::validate_bitmap_extension_account(
                                                &p.pool_id.0,
                                                owner,
                                                bytes,
                                            )
                                            .is_ok() =>
                                    {
                                        min_dependency_slot = min_dependency_slot.min(*slot);
                                        max_dependency_slot = max_dependency_slot.max(*slot);
                                    }
                                    _ => complete = false,
                                }
                            }

                            for index in indices {
                                let key = crate::decoders::derive_meteora_bin_array_pda(
                                    &p.pool_id.0,
                                    *index,
                                );
                                let Some((array_slot, bytes)) = derived_data_map.get(&key) else {
                                    complete = false;
                                    break;
                                };
                                let Some(owner) = derived_owner_map.get(&key) else {
                                    complete = false;
                                    break;
                                };
                                if *array_slot == 0 {
                                    complete = false;
                                    break;
                                }
                                min_dependency_slot = min_dependency_slot.min(*array_slot);
                                max_dependency_slot = max_dependency_slot.max(*array_slot);
                                let Ok(validated_bins) = arb_dex::validate_bin_array_account(
                                    &p.pool_id.0,
                                    *index,
                                    owner,
                                    bytes,
                                ) else {
                                    complete = false;
                                    break;
                                };
                                if validated_bins.len()
                                    != arb_constants::METEORA_BINS_PER_BIN_ARRAY as usize
                                {
                                    complete = false;
                                    break;
                                }
                                for bin in validated_bins {
                                    by_bin_id.insert(bin.bin_id, bin);
                                }
                            }
                            // Pool, extension and selected arrays are one
                            // tightly-coupled snapshot. Reject mixed batches
                            // rather than stamping them with the newest slot.
                            if max_dependency_slot.saturating_sub(min_dependency_slot) > 2 {
                                complete = false;
                            }

                            // The on-chain bitmap omits uninitialized arrays.
                            // The local bin walker still needs explicit empty
                            // bins to cross those gaps, but never fake an array
                            // whose bitmap bit says it is initialized.
                            if complete {
                                if let Some((first_array, last_array)) =
                                    crate::decoders::dlmm_bin_array_span(
                                        d.active_bin_id,
                                        indices,
                                        64,
                                    )
                                {
                                    let bins_per_array =
                                        arb_constants::METEORA_BINS_PER_BIN_ARRAY as i32;
                                    let first_bin =
                                        (first_array as i32).saturating_mul(bins_per_array);
                                    let last_bin = (last_array as i32)
                                        .saturating_add(1)
                                        .saturating_mul(bins_per_array)
                                        .saturating_sub(1);
                                    for bin_id in first_bin..=last_bin {
                                        by_bin_id.entry(bin_id).or_insert_with(|| {
                                            arb_types::DlmmBin {
                                                bin_id,
                                                amount_x: 0,
                                                amount_y: 0,
                                                open_order_amount: 0,
                                                processed_order_remaining_amount: 0,
                                                limit_order_ask_side: false,
                                            }
                                        });
                                    }
                                } else {
                                    complete = false;
                                }
                            }
                            let bins: Vec<_> = if complete {
                                by_bin_id.into_values().collect()
                            } else {
                                Vec::new()
                            };
                            if !complete || !bins.iter().any(|bin| bin.bin_id == d.active_bin_id) {
                                debug!(
                                    "Meteora DLMM {} dependency hydration incomplete: complete={} arrays_selected={} bins={} active_bin={} active_present={} slot_span={}",
                                    p.pool_id,
                                    complete,
                                    indices.len(),
                                    bins.len(),
                                    d.active_bin_id,
                                    bins.iter().any(|bin| bin.bin_id == d.active_bin_id),
                                    max_dependency_slot.saturating_sub(min_dependency_slot)
                                );
                                dlmm_bins.insert(p.pool_id, Vec::new());
                            } else {
                                debug!(
                                    "Meteora DLMM {} hydrated {} bins from {} arrays at coherent slot {}",
                                    p.pool_id,
                                    bins.len(),
                                    indices.len(),
                                    min_dependency_slot
                                );
                                dlmm_coherent_slots.insert(p.pool_id, Slot(min_dependency_slot));
                                dlmm_bins.insert(p.pool_id, bins);
                            }
                        }
                    }
                    DexProtocol::OrcaWhirlpool => {
                        if let Some(c) = decode_clmm_state(data) {
                            let (keys, starts) =
                                clmm_tick_array_pdas(&p.pool_id, c.tick_current, c.tick_spacing);
                            let mut arrays = Vec::new();
                            for (key, start_tick) in keys.into_iter().zip(starts) {
                                if let Some((_, bytes)) = derived_data_map.get(&key) {
                                    if let Ok(facade) = arb_math::clmm::parse_tick_array_account(
                                        bytes,
                                        c.tick_spacing,
                                    ) {
                                        if facade.start_tick_index == start_tick {
                                            arrays.push(arb_types::ClmmTickArray {
                                                start_tick_index: facade.start_tick_index,
                                                ticks: facade
                                                    .ticks
                                                    .into_iter()
                                                    .filter(|t| t.initialized)
                                                    .map(|t| arb_types::ClmmTick {
                                                        tick_index: t.tick_index,
                                                        initialized: true,
                                                        liquidity_net: t.liquidity_net,
                                                        limit_order_unfilled: t
                                                            .limit_order_unfilled,
                                                    })
                                                    .collect(),
                                            });
                                        }
                                    }
                                }
                            }
                            clmm_ticks.insert(p.pool_id, arrays);
                        }
                    }
                    DexProtocol::RaydiumClmm => {
                        if let Some(c) = decode_raydium_clmm_state(data, p.fee_bps) {
                            let (keys, starts) = raydium_clmm_tick_array_pdas(
                                &p.pool_id,
                                c.tick_current,
                                c.tick_spacing,
                            );
                            let mut arrays = Vec::new();
                            for (key, start_tick) in keys.into_iter().zip(starts) {
                                if let Some((_, bytes)) = derived_data_map.get(&key) {
                                    if let Ok(facade) =
                                        arb_dex::RaydiumClmmPool::parse_tick_array_state(bytes)
                                    {
                                        if facade.start_tick_index == start_tick {
                                            arrays.push(arb_types::ClmmTickArray {
                                                start_tick_index: facade.start_tick_index,
                                                ticks: facade
                                                    .ticks
                                                    .into_iter()
                                                    .filter(|t| t.initialized)
                                                    .map(|t| arb_types::ClmmTick {
                                                        tick_index: t.tick_index,
                                                        initialized: true,
                                                        liquidity_net: t.liquidity_net,
                                                        limit_order_unfilled: t
                                                            .limit_order_unfilled,
                                                    })
                                                    .collect(),
                                            });
                                        }
                                    }
                                }
                            }
                            clmm_ticks.insert(p.pool_id, arrays);
                        }
                    }
                    _ => {}
                }
            }
        }

        debug!(
            "fetch_live_pool_and_new_mint_accounts={} vaults_fetched={} cpmm_cfg_fetched={} derived_fetched={} for {} registered pools",
            pool_data_map.len(),
            vault_data_map.len(),
            cpmm_config_map.len(),
            derived_data_map.len(),
            self.pools.len()
        );

        let effective_slot = Slot(max_observed_slot);
        let mut states = Vec::with_capacity(self.pools.len());
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);

        for p in &self.pools {
            if p.protocol == DexProtocol::MeteoraDammV2 {
                if let Some(state) = damm_v2_pool_states.remove(&p.pool_id) {
                    states.push(state);
                } else {
                    debug!(
                        "Meteora DAMM v2 {} omitted: no complete same-context Pool/Vault/Mint/Clock state",
                        p.pool_id
                    );
                }
                continue;
            }
            let (base_bal, quote_bal) = if matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            ) {
                let b = vault_data_map
                    .get(&p.base_vault)
                    .and_then(|(_, d)| decode_spl_token_amount_bytes(d))
                    .unwrap_or(0);
                let q = vault_data_map
                    .get(&p.quote_vault)
                    .and_then(|(_, d)| decode_spl_token_amount_bytes(d))
                    .unwrap_or(0);
                (b, q)
            } else {
                (0, 0)
            };

            let needs_vaults = matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            );
            if needs_vaults && (base_bal == 0 || quote_bal == 0) {
                debug!(
                    "MultiDexRegistry: {:?} pool {} missing live reserves (base={} quote={}) vault_entry(base={} quote={}) pool_bytes_len={}",
                    p.protocol,
                    p.pool_id,
                    base_bal,
                    quote_bal,
                    vault_data_map.contains_key(&p.base_vault),
                    vault_data_map.contains_key(&p.quote_vault),
                    pool_data_map
                        .get(&p.pool_id.0)
                        .map(|(_, d)| d.len())
                        .unwrap_or(0)
                );
                continue;
            }

            let pool_bytes = pool_data_map.get(&p.pool_id.0).map(|(_, b)| b.as_slice());
            let cpmm_accounting = if p.protocol == DexProtocol::RaydiumCpmm {
                pool_bytes.and_then(crate::decoders::decode_raydium_cpmm_pool_accounting)
            } else {
                None
            };
            let cpmm_config_key = cpmm_accounting.map(|state| state.amm_config);
            let cpmm_fee_config = cpmm_config_key
                .and_then(|key| cpmm_config_map.get(&key))
                .and_then(|(_, data)| crate::decoders::decode_raydium_cpmm_fee_config(data));
            let cpmm_coherent_slot = if p.protocol == DexProtocol::RaydiumCpmm {
                let pool_slot = pool_data_map.get(&p.pool_id.0).map(|(slot, _)| *slot);
                let vault_0_slot = cpmm_accounting
                    .and_then(|state| vault_data_map.get(&state.token_0_vault))
                    .map(|(slot, _)| *slot);
                let vault_1_slot = cpmm_accounting
                    .and_then(|state| vault_data_map.get(&state.token_1_vault))
                    .map(|(slot, _)| *slot);
                let config_slot = cpmm_config_key
                    .and_then(|key| cpmm_config_map.get(&key))
                    .map(|(slot, _)| *slot);
                coherent_cpmm_snapshot_slot(pool_slot, vault_0_slot, vault_1_slot, config_slot)
            } else {
                None
            };
            if p.protocol == DexProtocol::RaydiumCpmm {
                if cpmm_coherent_slot.is_none() {
                    debug!(
                        "Raydium CPMM {} coherent snapshot unavailable (pool={:?}, vault0={:?}, vault1={:?}, config={:?}); skipping",
                        p.pool_id,
                        pool_data_map.get(&p.pool_id.0).map(|(slot, _)| *slot),
                        cpmm_accounting
                            .and_then(|a| vault_data_map.get(&a.token_0_vault))
                            .map(|(slot, _)| *slot),
                        cpmm_accounting
                            .and_then(|a| vault_data_map.get(&a.token_1_vault))
                            .map(|(slot, _)| *slot),
                        cpmm_config_key
                            .and_then(|key| cpmm_config_map.get(&key))
                            .map(|(slot, _)| *slot),
                    );
                    continue;
                }
                let Ok(cpmm_program) = Pubkey::from_str(arb_constants::RAYDIUM_CPMM_PROGRAM_ID)
                else {
                    continue;
                };
                let Some(pool_owner) = pool_owner_map.get(&p.pool_id.0) else {
                    debug!(
                        "Raydium CPMM {} pool owner unavailable; skipping",
                        p.pool_id
                    );
                    continue;
                };
                let Some(config_key) = cpmm_config_key else {
                    continue;
                };
                if *pool_owner != cpmm_program
                    || cpmm_config_owner_map.get(&config_key) != Some(&cpmm_program)
                {
                    debug!(
                        "Raydium CPMM {} pool/AmmConfig owner mismatch; skipping",
                        p.pool_id
                    );
                    continue;
                }
                let (Some(accounting), Some(fees)) = (cpmm_accounting, cpmm_fee_config) else {
                    debug!(
                        "Raydium CPMM {} missing verified PoolState/AmmConfig accounting; skipping",
                        p.pool_id
                    );
                    continue;
                };
                if !accounting.swap_enabled {
                    debug!("Raydium CPMM {} swap status disabled; skipping", p.pool_id);
                    continue;
                }
                let Ok(spl_token_program) = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)
                else {
                    continue;
                };
                if accounting.token_0_program != spl_token_program
                    || accounting.token_1_program != spl_token_program
                {
                    // Token-2022 remains excluded from the active pilot until
                    // epoch-aware transfer-fee/hook state is wired end-to-end.
                    debug!(
                        "Raydium CPMM {} uses a non-legacy token program; skipping",
                        p.pool_id
                    );
                    continue;
                }
                if fees.trade_fee_rate == 0
                    || fees
                        .trade_fee_rate
                        .checked_add(if accounting.enable_creator_fee {
                            fees.creator_fee_rate
                        } else {
                            0
                        })
                        .is_none_or(|r| r >= 1_000_000)
                    || fees
                        .protocol_fee_rate
                        .checked_add(fees.fund_fee_rate)
                        .is_none_or(|r| r > 1_000_000)
                {
                    debug!("Raydium CPMM {} fee schedule invalid; skipping", p.pool_id);
                    continue;
                }
            }
            let pump_curve = match (p.protocol, pool_bytes) {
                (DexProtocol::PumpSwap, _) => None,
                (DexProtocol::PumpFun, Some(data)) => {
                    arb_dex::pump_fun::PumpFunBondingCurve::decode_account_data(data).ok()
                }
                _ => None,
            };

            let pump_state = pump_curve.clone();

            let mut clmm = match (p.protocol, pool_bytes) {
                (DexProtocol::OrcaWhirlpool, Some(data)) => decode_clmm_state(data),
                (DexProtocol::RaydiumClmm, Some(data)) => {
                    decode_raydium_clmm_state(data, p.fee_bps)
                }
                _ => None,
            };
            let mut dlmm = match (p.protocol, pool_bytes) {
                (DexProtocol::MeteoraDlmm, Some(data)) => decode_dlmm_state(data),
                _ => None,
            };

            if p.protocol == DexProtocol::MeteoraDlmm
                && let Some(d) = dlmm.as_mut()
            {
                d.bins = dlmm_bins.get(&p.pool_id).cloned().unwrap_or_default();
            }

            let onchain_fee_bps = match (p.protocol, pool_bytes) {
                // Pump.fun bonding curves use their own flat FeeConfig schedule;
                // PumpSwap canonical pools have a separate market-cap tier ladder.
                // Until the Pump-program FeeConfig PDA/owner is joined and decoded,
                // do not promote a curve from the 125-bps discovery estimate.
                (DexProtocol::PumpFun, _) => None,
                (DexProtocol::PumpSwap, _) => None,
                (DexProtocol::RaydiumV4, Some(d)) => decode_raydium_fee_bps(d),
                (DexProtocol::RaydiumCpmm, _) => cpmm_fee_config.and_then(|cfg| {
                    u16::try_from(cfg.trade_fee_rate.saturating_add(99) / 100).ok()
                }),
                (DexProtocol::RaydiumClmm, _) => None,
                (DexProtocol::OrcaWhirlpool, Some(d)) => decode_orca_fee_bps(d),
                (DexProtocol::MeteoraDlmm, _) => dlmm.as_ref().and_then(decode_dlmm_fee_bps),
                _ => None,
            };
            let Some(fee_bps) = onchain_fee_bps else {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — on-chain fee undecodable; skipping (fail closed) pool_bytes_len={} clmm_decoded={} dlmm_decoded={}",
                    p.protocol,
                    p.pool_id,
                    pool_bytes.map(|d| d.len()).unwrap_or(0),
                    clmm.is_some(),
                    dlmm.is_some()
                );
                continue;
            };

            if (p.protocol == DexProtocol::OrcaWhirlpool || p.protocol == DexProtocol::RaydiumClmm)
                && let Some(c) = clmm.as_mut()
            {
                c.tick_arrays = clmm_ticks.get(&p.pool_id).cloned().unwrap_or_default();
            }

            let kind = match p.protocol {
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm => arb_types::PoolKind::Clmm,
                DexProtocol::MeteoraDlmm => arb_types::PoolKind::Dlmm,
                _ => arb_types::PoolKind::Cpmm,
            };

            // Canonical DLMM orientation: DexScreener's base/quote ordering is
            // arbitrary, but the on-chain LbPair fixes token_x @88..120 and
            // token_y @120..152. Downstream, `quote_pool` derives the swap
            // direction as `a_to_b = (pool_base == from_mint)`, which is only
            // correct when pool_base == on-chain token_x. A DexScreener
            // inversion made HRYEjwdo3bZ1TpXKWKcezqiwSV2Ywuh4LxMa2PzoCnG6 quote
            // WSOL->USDC as if selling USDC (measured: 100_000 WSOL-raw in ->
            // 1_249_511 "USDC-raw" out instead of 7_998), a 156x phantom edge
            // that flooded the pipeline with implausible-ROI rejections
            // (soak13/14: 4,276 + 1,058, p50 ROI 1,535,076 bps = 153.5x).
            let (state_base, state_quote, _catalog_dec_a, _catalog_dec_b) = if p.protocol
                == DexProtocol::RaydiumCpmm
            {
                let Some(accounting) = cpmm_accounting else {
                    continue;
                };
                if let Some((mint_0, mint_1, decimals_0, decimals_1)) =
                    accounting.canonical_orientation(p.base_mint.0, p.quote_mint.0)
                {
                    // creator_fee_on and reserve orientation both use token0/token1.
                    (MintId(mint_0), MintId(mint_1), decimals_0, decimals_1)
                } else {
                    debug!(
                        "Raydium CPMM {} catalog mints do not match on-chain token0/token1; skipping",
                        p.pool_id
                    );
                    continue;
                }
            } else if p.protocol == DexProtocol::MeteoraDlmm {
                match pool_bytes {
                    Some(d) if d.len() >= 152 => {
                        let token_x = Pubkey::new_from_array(d[88..120].try_into().unwrap());
                        let token_y = Pubkey::new_from_array(d[120..152].try_into().unwrap());
                        (MintId(token_x), MintId(token_y), 0, 0)
                    }
                    _ => (p.base_mint, p.quote_mint, p.decimals_a, p.decimals_b),
                }
            } else if matches!(
                p.protocol,
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm
            ) {
                // Canonical CLMM orientation: same failure class as the DLMM
                // DexScreener inversion above. CLMM base/quote come from HTTP
                // catalogs (Orca `tokenA/tokenB`, Raydium v3 `mintA/mintB`)
                // and were never verified against pool bytes. Downstream,
                // `quote_pool` derives `a_to_b = (pool_base == from_mint)`
                // and `compute_clmm_virtual_reserves` returns (mint_a-side,
                // mint_b-side) reserves, so `pool_base == on-chain mint_a` is
                // a hard correctness invariant. Live probe (clmm_probe.rs,
                // slot 468876781) verified catalog == on-chain 3/3 today, but
                // nothing failed closed if a catalog schema change inverted
                // it. The on-chain pool account is ground truth; a catalog
                // mismatch is now overridden loudly instead of silently
                // trusted. Mints are immutable in both pool layouts, so the
                // WS arm inherits this canonicalization from the cache and
                // must not re-derive it.
                match pool_bytes.and_then(|d| clmm_onchain_mints(p.protocol, d)) {
                    Some((mint_a, mint_b)) => {
                        let catalog_order_verified =
                            p.base_mint.0 == mint_a && p.quote_mint.0 == mint_b;
                        if !catalog_order_verified {
                            warn!(
                                "MultiDexRegistry: {:?} pool {} — catalog orientation \
                                     {:?}/{:?} diverges from on-chain mints {:?}/{:?}; using on-chain order",
                                p.protocol,
                                p.pool_id,
                                p.base_mint,
                                p.quote_mint,
                                MintId(mint_a),
                                MintId(mint_b)
                            );
                        }
                        if p.protocol == DexProtocol::RaydiumClmm {
                            // Raydium PoolState stores mint decimals on-chain
                            // (dec_0 @233, dec_1 @234) — no RPC needed. Values
                            // outside 1..=18 are corrupt; fall back to catalog.
                            let d = pool_bytes.unwrap_or_default();
                            let dec0 = d[233];
                            let dec1 = d[234];
                            let sane = |v: u8, fb: u8| {
                                if (1..=18).contains(&v) { v } else { fb }
                            };
                            (
                                MintId(mint_a),
                                MintId(mint_b),
                                sane(dec0, p.decimals_a),
                                sane(dec1, p.decimals_b),
                            )
                        } else if catalog_order_verified {
                            // Catalog order verified against chain: its
                            // decimals are positionally correct, skip RPC.
                            (MintId(mint_a), MintId(mint_b), p.decimals_a, p.decimals_b)
                        } else {
                            // Inverted catalog: catalog decimals are swapped
                            // too; resolve by mint identity (cached).
                            (MintId(mint_a), MintId(mint_b), 0, 0)
                        }
                    }
                    None => (p.base_mint, p.quote_mint, p.decimals_a, p.decimals_b),
                }
            } else {
                (p.base_mint, p.quote_mint, p.decimals_a, p.decimals_b)
            };

            // Resolve token program and decimals from actual mint accounts and
            // their RPC owner metadata. Missing or unsupported owners fail closed.
            let mint_program_and_decimals =
                |mint: Pubkey| self.mint_metadata_cache.lock().get(&mint).copied();
            let Some((base_token_program, verified_decimals_a)) =
                mint_program_and_decimals(state_base.0)
            else {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — base mint owner/layout unavailable; skipping",
                    p.protocol, p.pool_id
                );
                continue;
            };
            let Some((quote_token_program, verified_decimals_b)) =
                mint_program_and_decimals(state_quote.0)
            else {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — quote mint owner/layout unavailable; skipping",
                    p.protocol, p.pool_id
                );
                continue;
            };
            let token_programs = [base_token_program, quote_token_program];

            let (reserve_in, reserve_out) = match p.protocol {
                DexProtocol::RaydiumCpmm => {
                    let Some(accounting) = cpmm_accounting else {
                        continue;
                    };
                    let Some((_, vault_0_data)) = vault_data_map.get(&accounting.token_0_vault)
                    else {
                        continue;
                    };
                    let Some((_, vault_1_data)) = vault_data_map.get(&accounting.token_1_vault)
                    else {
                        continue;
                    };
                    if vault_owner_map.get(&accounting.token_0_vault)
                        != Some(&accounting.token_0_program)
                        || vault_owner_map.get(&accounting.token_1_vault)
                            != Some(&accounting.token_1_program)
                        || vault_0_data.get(0..32) != Some(accounting.token_0_mint.as_ref())
                        || vault_1_data.get(0..32) != Some(accounting.token_1_mint.as_ref())
                    {
                        debug!(
                            "Raydium CPMM {} vault owner/mint does not match PoolState; skipping",
                            p.pool_id
                        );
                        continue;
                    }
                    let Some(raw_0) = decode_spl_token_amount_bytes(vault_0_data) else {
                        continue;
                    };
                    let Some(raw_1) = decode_spl_token_amount_bytes(vault_1_data) else {
                        continue;
                    };
                    let (Some(net_0), Some(net_1)) = (
                        crate::decoders::raydium_cpmm_curve_reserve(raw_0, &accounting, 0),
                        crate::decoders::raydium_cpmm_curve_reserve(raw_1, &accounting, 1),
                    ) else {
                        debug!(
                            "Raydium CPMM {} accrued fee counters exceed vault balance; skipping",
                            p.pool_id
                        );
                        continue;
                    };
                    // Reserves follow the same canonical token0/token1 order as
                    // state_base/state_quote and creator_fee_on direction mapping.
                    if p.base_mint.0 == accounting.token_0_mint
                        || p.base_mint.0 == accounting.token_1_mint
                    {
                        (u128::from(net_0), u128::from(net_1))
                    } else {
                        continue;
                    }
                }
                DexProtocol::PumpSwap => (0, 0),
                DexProtocol::PumpFun => match pump_state.as_ref() {
                    Some(state)
                        if state.is_tradeable()
                            && state.virtual_sol_reserves > 0
                            && state.virtual_token_reserves > 0 =>
                    {
                        (
                            u128::from(state.virtual_sol_reserves),
                            u128::from(state.virtual_token_reserves),
                        )
                    }
                    _ => (0, 0),
                },
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm => match clmm.as_ref() {
                    Some(c) => compute_clmm_virtual_reserves(c),
                    None => (0, 0),
                },
                DexProtocol::MeteoraDlmm => match (dlmm.as_ref(), pool_bytes) {
                    (Some(d), Some(raw_data)) => {
                        let Some(res) =
                            compute_dlmm_reserves(d, raw_data, &state_base.0, &state_quote.0)
                        else {
                            debug!(
                                "Meteora DLMM {} reserve model rejected snapshot: bins={} active_bin={} bin_step={} mints={}/{}",
                                p.pool_id,
                                d.bins.len(),
                                d.active_bin_id,
                                d.bin_step_bps,
                                state_base,
                                state_quote
                            );
                            continue;
                        };
                        res
                    }
                    _ => (0, 0),
                },
                _ => (base_bal as u128, quote_bal as u128),
            };

            if reserve_in == 0 || reserve_out == 0 {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — zero depth; skipping (fail closed)",
                    p.protocol, p.pool_id
                );
                continue;
            }

            let execution_accounts = match (p.protocol, pool_bytes) {
                (DexProtocol::RaydiumCpmm, Some(d)) if d.len() >= 333 => {
                    let amm_config = Pubkey::new_from_array(d[8..40].try_into().unwrap());
                    let vault_0 = Pubkey::new_from_array(d[72..104].try_into().unwrap());
                    let vault_1 = Pubkey::new_from_array(d[104..136].try_into().unwrap());
                    let observation_state = Pubkey::new_from_array(d[296..328].try_into().unwrap());
                    let prog = Pubkey::from_str(arb_constants::RAYDIUM_CPMM_PROGRAM_ID)
                        .unwrap_or_default();
                    let (authority, _) = Pubkey::find_program_address(
                        &[arb_constants::RAYDIUM_CPMM_AUTH_SEED],
                        &prog,
                    );
                    Some(arb_types::PoolExecutionAccounts::RaydiumCpmm {
                        amm_config,
                        authority,
                        vault_0,
                        vault_1,
                        observation_state,
                    })
                }
                (DexProtocol::RaydiumClmm, Some(d)) if d.len() >= 273 => {
                    let amm_config = Pubkey::new_from_array(d[9..41].try_into().unwrap());
                    let vault_0 = Pubkey::new_from_array(d[137..169].try_into().unwrap());
                    let vault_1 = Pubkey::new_from_array(d[169..201].try_into().unwrap());
                    let observation_state = Pubkey::new_from_array(d[201..233].try_into().unwrap());
                    // Raydium CLMM is telemetry-only until tick arrays and the
                    // optional bitmap extension are fetched, owner-validated,
                    // and joined with this pool at one coherent slot. A derived
                    // PDA alone is not an executable account.
                    Some(arb_types::PoolExecutionAccounts::RaydiumClmm {
                        amm_config,
                        vault_0,
                        vault_1,
                        observation_state,
                        tick_array_pdas: Vec::new(),
                        bitmap_extension: None,
                        pool_mint_0: Pubkey::new_from_array(d[73..105].try_into().unwrap()),
                    })
                }
                (DexProtocol::OrcaWhirlpool, Some(d)) if d.len() >= 245 && clmm.is_some() => {
                    let _whirlpool_config = Pubkey::new_from_array(d[8..40].try_into().unwrap());
                    let token_vault_a = Pubkey::new_from_array(d[133..165].try_into().unwrap());
                    let token_vault_b = Pubkey::new_from_array(d[213..245].try_into().unwrap());
                    let tick_current = i32::from_le_bytes(d[81..85].try_into().unwrap());
                    let tick_spacing = u16::from_le_bytes(d[41..43].try_into().unwrap()).max(1);
                    let (keys, _) = clmm_tick_array_pdas(&p.pool_id, tick_current, tick_spacing);
                    let prog = Pubkey::from_str(arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID)
                        .unwrap_or_default();
                    let (oracle, _) =
                        Pubkey::find_program_address(&[b"oracle", p.pool_id.0.as_ref()], &prog);
                    let tick_arrays = if keys.len() >= 3 {
                        vec![keys[0], keys[1], keys[2]]
                    } else {
                        keys
                    };
                    Some(arb_types::PoolExecutionAccounts::OrcaWhirlpool {
                        vault_a: token_vault_a,
                        vault_b: token_vault_b,
                        oracle,
                        tick_array_pdas: tick_arrays,
                        pool_mint_a: Pubkey::new_from_array(d[101..133].try_into().unwrap()),
                    })
                }
                (DexProtocol::MeteoraDlmm, Some(d)) if d.len() >= 216 => {
                    let reserve_x = Pubkey::new_from_array(d[152..184].try_into().unwrap());
                    let reserve_y = Pubkey::new_from_array(d[184..216].try_into().unwrap());
                    let bin_arrays = dlmm_selected_indices
                        .get(&p.pool_id)
                        .into_iter()
                        .flatten()
                        .map(|index| {
                            crate::decoders::derive_meteora_bin_array_pda(&p.pool_id.0, *index)
                        })
                        .collect();
                    let prog = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
                        .unwrap_or_default();
                    let bitmap_ext_pda = bitmap_extension_pda(&p.pool_id.0);
                    let (oracle, _) =
                        Pubkey::find_program_address(&[b"oracle", p.pool_id.0.as_ref()], &prog);
                    // EXISTENCE-PROVEN: Some only when the extension account was
                    // actually fetched from the chain in this batch. A derived
                    // PDA alone is NOT proof of initialization — passing a
                    // nonexistent writable account fails the swap on-chain.
                    let bitmap_extension = match (
                        derived_data_map.get(&bitmap_ext_pda),
                        derived_owner_map.get(&bitmap_ext_pda),
                    ) {
                        (Some((_, bytes)), Some(owner))
                            if arb_dex::validate_bitmap_extension_account(
                                &p.pool_id.0,
                                owner,
                                bytes,
                            )
                            .is_ok() =>
                        {
                            Some(bitmap_ext_pda)
                        }
                        _ => None,
                    };
                    Some(arb_types::PoolExecutionAccounts::MeteoraDlmm {
                        reserve_x,
                        reserve_y,
                        oracle,
                        bin_array_pdas: bin_arrays,
                        bitmap_extension,
                        pool_token_x: Pubkey::new_from_array(d[88..120].try_into().unwrap()),
                        pool_token_y: Pubkey::new_from_array(d[120..152].try_into().unwrap()),
                    })
                }
                (DexProtocol::RaydiumV4, Some(d)) if d.len() >= 528 => {
                    let pool_coin_token_account =
                        Pubkey::new_from_array(d[336..368].try_into().unwrap());
                    let pool_pc_token_account =
                        Pubkey::new_from_array(d[368..400].try_into().unwrap());
                    let open_orders = Pubkey::new_from_array(d[496..528].try_into().unwrap());
                    let target_orders = if d.len() >= 624 {
                        let t = Pubkey::new_from_array(d[592..624].try_into().unwrap());
                        if t == Pubkey::default() {
                            Some(p.pool_id.0)
                        } else {
                            Some(t)
                        }
                    } else {
                        Some(p.pool_id.0)
                    };
                    let serum_market = if d.len() >= 560 {
                        Pubkey::new_from_array(d[528..560].try_into().unwrap())
                    } else {
                        Pubkey::default()
                    };
                    let market_program = if d.len() >= 592 {
                        Pubkey::new_from_array(d[560..592].try_into().unwrap())
                    } else {
                        Pubkey::from_str(arb_constants::OPENBOOK_PROGRAM_ID).unwrap()
                    };
                    let prog =
                        Pubkey::from_str(arb_constants::RAYDIUM_V4_PROGRAM_ID).unwrap_or_default();
                    let (amm_authority, _) =
                        Pubkey::find_program_address(&[b"amm authority"], &prog);
                    Some(arb_types::PoolExecutionAccounts::RaydiumV4 {
                        amm_authority,
                        open_orders,
                        target_orders: target_orders.unwrap_or(p.pool_id.0),
                        base_vault: pool_coin_token_account,
                        quote_vault: pool_pc_token_account,
                        serum_program: market_program,
                        serum_market,
                        serum_bids: Pubkey::default(),
                        serum_asks: Pubkey::default(),
                        serum_event_queue: Pubkey::default(),
                        serum_coin_vault: Pubkey::default(),
                        serum_pc_vault: Pubkey::default(),
                        serum_vault_signer: Pubkey::default(),
                        swap_version: arb_types::RaydiumV4SwapVersion::V2,
                    })
                }
                (DexProtocol::MeteoraDammV2, Some(_)) => {
                    // Pool offsets alone do not prove the swap dependency graph.
                    // The adapter currently has no coherent same-context proof
                    // for pool owner, vault owners/mints/balances, token programs,
                    // Clock activation point, and fee/mint extensions. Never
                    // publish guessed addresses as executable accounts.
                    debug!(
                        "Meteora DAMM v2 {} has no complete verified execution-account snapshot; keeping it telemetry-only",
                        p.pool_id
                    );
                    None
                }
                (DexProtocol::MeteoraDbc, Some(d)) if d.len() >= 256 => {
                    let config = Pubkey::new_from_array(d[8..40].try_into().unwrap());
                    let vault_base = Pubkey::new_from_array(d[104..136].try_into().unwrap());
                    let vault_quote = Pubkey::new_from_array(d[136..168].try_into().unwrap());
                    let prog =
                        Pubkey::from_str(arb_constants::METEORA_DBC_PROGRAM_ID).unwrap_or_default();
                    let (authority, _) = Pubkey::find_program_address(
                        &[b"pool_authority", p.pool_id.0.as_ref()],
                        &prog,
                    );
                    let (event_authority, _) =
                        Pubkey::find_program_address(&[b"__event_authority"], &prog);
                    Some(arb_types::PoolExecutionAccounts::MeteoraDbc {
                        config,
                        authority,
                        vault_base,
                        vault_quote,
                        event_authority,
                    })
                }
                (DexProtocol::RaydiumLaunchLab, Some(d)) if d.len() >= 366 => {
                    let global_config = Pubkey::new_from_array(d[141..173].try_into().unwrap());
                    let platform_config = Pubkey::new_from_array(d[173..205].try_into().unwrap());
                    let base_vault = Pubkey::new_from_array(d[269..301].try_into().unwrap());
                    let quote_vault = Pubkey::new_from_array(d[301..333].try_into().unwrap());
                    let prog = Pubkey::from_str(arb_constants::RAYDIUM_LAUNCHLAB_PROGRAM_ID)
                        .unwrap_or_default();
                    let (authority, _) = Pubkey::find_program_address(&[b"vault_auth_seed"], &prog);
                    let (event_authority, _) =
                        Pubkey::find_program_address(&[b"__event_authority"], &prog);
                    Some(arb_types::PoolExecutionAccounts::RaydiumLaunchLab {
                        authority,
                        global_config,
                        platform_config,
                        base_vault,
                        quote_vault,
                        event_authority,
                    })
                }
                (DexProtocol::PumpSwap, Some(d)) if d.len() >= 245 => {
                    (|| -> Option<arb_types::PoolExecutionAccounts> {
                        let header = arb_dex::PumpSwapPoolHeader::decode(d).ok()?;
                        let global_config =
                            Pubkey::from_str(arb_constants::PUMP_SWAP_GLOBAL_CONFIG).ok()?;
                        let pump_swap_program =
                            Pubkey::from_str(arb_constants::PUMP_SWAP_PROGRAM_ID).ok()?;
                        let pump_fees_program =
                            Pubkey::from_str(arb_constants::PUMP_FEES_PROGRAM_ID).ok()?;
                        let (fee_config, _) = Pubkey::find_program_address(
                            &[b"fee_config", pump_swap_program.as_ref()],
                            &pump_fees_program,
                        );
                        let protocol_fee_recipient =
                            Pubkey::from_str(arb_constants::PUMP_FUN_FEE_RECIPIENT)
                                .unwrap_or_default();
                        let pool_version = if header.is_cashback_coin {
                            arb_types::PumpSwapPoolVersion::V1Legacy
                        } else {
                            arb_types::PumpSwapPoolVersion::V2
                        };
                        Some(arb_types::PoolExecutionAccounts::PumpSwap {
                            global_config,
                            pool_base_vault: header.base_vault,
                            pool_quote_vault: header.quote_vault,
                            protocol_fee_recipient,
                            protocol_fee_recipient_ata: Pubkey::default(),
                            coin_creator_vault_authority: header.coin_creator,
                            coin_creator_vault_ata: Pubkey::default(),
                            fee_config,
                            pool_version,
                        })
                    })()
                }
                _ => None,
            };

            let state_slot = if p.protocol == DexProtocol::RaydiumCpmm {
                let Some(slot) = cpmm_coherent_slot else {
                    continue;
                };
                Slot(slot)
            } else if p.protocol == DexProtocol::MeteoraDlmm {
                let Some(slot) = dlmm_coherent_slots.get(&p.pool_id).copied() else {
                    // Do not publish a DLMM PoolState without a coherent,
                    // owner-verified pool/bitmap/bin-array dependency snapshot.
                    continue;
                };
                slot
            } else {
                effective_slot
            };
            let wv = next_pool_write_version();
            let (fee_numerator, fee_denominator) = if p.protocol == DexProtocol::RaydiumCpmm {
                let Some(config) = cpmm_fee_config else {
                    continue;
                };
                (config.trade_fee_rate, 1_000_000)
            } else {
                (u64::from(fee_bps), 10_000)
            };

            states.push(PoolState {
                pool_id: p.pool_id,
                protocol: p.protocol,
                slot: state_slot,
                write_version: wv,
                base_mint: state_base,
                quote_mint: state_quote,
                decimals_a: verified_decimals_a,
                decimals_b: verified_decimals_b,
                reserve_in,
                reserve_out,
                fee_numerator,
                fee_denominator,
                raydium_cpmm_fee: if p.protocol == DexProtocol::RaydiumCpmm {
                    let accounting = cpmm_accounting.expect("CPMM accounting validated above");
                    let fees = cpmm_fee_config.expect("CPMM fee config validated above");
                    Some(arb_types::RaydiumCpmmFeeState {
                        trade_fee_rate_ppm: fees.trade_fee_rate,
                        creator_fee_rate_ppm: fees.creator_fee_rate,
                        protocol_fee_rate: fees.protocol_fee_rate,
                        fund_fee_rate: fees.fund_fee_rate,
                        enable_creator_fee: accounting.enable_creator_fee,
                        creator_fee_on: accounting.creator_fee_on,
                    })
                } else {
                    None
                },
                is_active: true,
                // Token-2022 stays disabled until epoch-aware fees and transfer
                // hook account resolution are verified across quote + execution.
                has_risky_token: token_programs.contains(&TokenProgramKind::Token2022),
                last_update_slot: state_slot.0,
                last_update_ms: now_ms,
                kind,
                clmm,
                dlmm,
                pump_fun: pump_state,
                damm_v2: None,
                token_programs,
                execution_accounts,
                transfer_fee_bps: 0,
            });
        }

        states
    }
}

#[cfg(test)]
mod mint_token_program_tests {
    use super::decode_mint_program_and_decimals;
    use arb_types::TokenProgramKind;
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    #[test]
    fn classifies_spl_and_token_2022_from_verified_account_owner() {
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();

        let mut legacy = vec![0u8; 82];
        legacy[44] = 6;
        legacy[45] = 1;
        assert_eq!(
            decode_mint_program_and_decimals(&spl, &legacy),
            Some((TokenProgramKind::SplToken, 6))
        );

        let mut extensionless_2022 = vec![0u8; 82];
        extensionless_2022[44] = 9;
        extensionless_2022[45] = 1;
        assert_eq!(
            decode_mint_program_and_decimals(&token_2022, &extensionless_2022),
            Some((TokenProgramKind::Token2022, 9))
        );

        let mut extended_2022 = vec![0u8; 170];
        extended_2022[44] = 6;
        extended_2022[45] = 1;
        extended_2022[165] = 1;
        assert_eq!(
            decode_mint_program_and_decimals(&token_2022, &extended_2022),
            Some((TokenProgramKind::Token2022, 6))
        );
    }

    #[test]
    fn rejects_unknown_owners_uninitialized_and_malformed_mints() {
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
        let mut mint = vec![0u8; 82];
        mint[44] = 6;
        mint[45] = 1;

        assert!(decode_mint_program_and_decimals(&Pubkey::new_unique(), &mint).is_none());

        let mut uninitialized = mint.clone();
        uninitialized[45] = 0;
        assert!(decode_mint_program_and_decimals(&spl, &uninitialized).is_none());

        let mut bad_decimals = mint.clone();
        bad_decimals[44] = 19;
        assert!(decode_mint_program_and_decimals(&spl, &bad_decimals).is_none());

        assert!(decode_mint_program_and_decimals(&spl, &[0u8; 170]).is_none());
        assert!(decode_mint_program_and_decimals(&token_2022, &[0u8; 120]).is_none());

        let mut wrong_account_type = vec![0u8; 170];
        wrong_account_type[45] = 1;
        assert!(decode_mint_program_and_decimals(&token_2022, &wrong_account_type).is_none());
    }
}

#[cfg(test)]
mod cpmm_snapshot_slot_tests {
    use super::coherent_cpmm_snapshot_slot;

    #[test]
    fn accepts_only_one_identical_nonzero_slot_for_all_dependencies() {
        assert_eq!(
            coherent_cpmm_snapshot_slot(Some(42), Some(42), Some(42), Some(42)),
            Some(42)
        );
        assert_eq!(
            coherent_cpmm_snapshot_slot(Some(0), Some(0), Some(0), Some(0)),
            None
        );
        assert_eq!(
            coherent_cpmm_snapshot_slot(Some(42), Some(43), Some(42), Some(42)),
            None
        );
        assert_eq!(
            coherent_cpmm_snapshot_slot(Some(42), Some(42), None, Some(42)),
            None
        );
    }
}

#[cfg(test)]
mod damm_v2_coherent_account_graph_tests {
    use super::decode_damm_v2_coherent_pool_state;
    use arb_types::{DexProtocol, PoolId, PoolKind};
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    struct Fixture {
        pool_id: PoolId,
        pool: Vec<u8>,
        vault_a: Vec<u8>,
        vault_b: Vec<u8>,
        mint_a: Vec<u8>,
        mint_b: Vec<u8>,
        clock: Vec<u8>,
        pool_owner: Pubkey,
        vault_a_address: Pubkey,
        vault_b_address: Pubkey,
        mint_a_address: Pubkey,
        mint_b_address: Pubkey,
        token_owner: Pubkey,
        clock_owner: Pubkey,
    }

    fn fixture() -> Fixture {
        let pool_id = PoolId(Pubkey::new_unique());
        let mint_a_key = Pubkey::new_unique();
        let mint_b_key = Pubkey::new_unique();
        let vault_a_key = Pubkey::new_unique();
        let vault_b_key = Pubkey::new_unique();
        let mut pool = vec![0u8; arb_dex::meteora_damm_v2::METEORA_DAMM_V2_POOL_MIN_SIZE];
        pool[..8].copy_from_slice(&arb_dex::meteora_damm_v2::METEORA_DAMM_V2_POOL_DISCRIMINATOR);
        pool[8..16].copy_from_slice(&100_000u64.to_le_bytes());
        pool[16] = 2; // RateLimiter/deprecated fixed cliff fee
        pool[48] = 10; // protocol percentage
        pool[50] = 0; // no referral account
        pool[54..56].copy_from_slice(&2_000u16.to_le_bytes());
        pool[168..200].copy_from_slice(mint_a_key.as_ref());
        pool[200..232].copy_from_slice(mint_b_key.as_ref());
        pool[232..264].copy_from_slice(vault_a_key.as_ref());
        pool[264..296].copy_from_slice(vault_b_key.as_ref());
        pool[360..376].copy_from_slice(&1_000_000_000_000u128.to_le_bytes());
        pool[424..440].copy_from_slice(&(1u128 << 63).to_le_bytes());
        pool[440..456].copy_from_slice(&(1u128 << 65).to_le_bytes());
        pool[456..472].copy_from_slice(&(1u128 << 64).to_le_bytes());
        pool[472..480].copy_from_slice(&1u64.to_le_bytes());
        pool[480] = 1; // Unix timestamp activation
        pool[481] = 0; // pool enabled
        pool[482] = 0; // Token A uses SPL Token
        pool[483] = 0; // Token B uses SPL Token
        pool[484] = 0; // BothToken
        pool[486] = 0; // fee version V0
        pool[696] = 0; // layout V0 is valid for concentrated liquidity

        let pool_decoder = arb_dex::MeteoraDammV2Pool::from_account(pool_id, &pool).unwrap();
        let token_owner = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        let pool_owner = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_PROGRAM_ID).unwrap();
        let clock_owner = Pubkey::from_str(super::SOLANA_SYSVAR_OWNER_ADDRESS).unwrap();

        let mut vault_a = vec![0u8; 165];
        vault_a[..32].copy_from_slice(mint_a_key.as_ref());
        vault_a[32..64].copy_from_slice(pool_decoder.authority.as_ref());
        vault_a[64..72].copy_from_slice(&2_000_000_000u64.to_le_bytes());
        let mut vault_b = vec![0u8; 165];
        vault_b[..32].copy_from_slice(mint_b_key.as_ref());
        vault_b[32..64].copy_from_slice(pool_decoder.authority.as_ref());
        vault_b[64..72].copy_from_slice(&3_000_000u64.to_le_bytes());

        let mut mint_a = vec![0u8; 82];
        mint_a[44] = 9;
        mint_a[45] = 1; // initialized Mint
        let mut mint_b = vec![0u8; 82];
        mint_b[44] = 6;
        mint_b[45] = 1; // initialized Mint

        let mut clock = vec![0u8; 40];
        clock[0..8].copy_from_slice(&777u64.to_le_bytes());
        clock[16..24].copy_from_slice(&1_097u64.to_le_bytes());
        clock[32..40].copy_from_slice(&1_800_000_100i64.to_le_bytes());

        Fixture {
            pool_id,
            pool,
            vault_a,
            vault_b,
            mint_a,
            mint_b,
            clock,
            pool_owner,
            vault_a_address: vault_a_key,
            vault_b_address: vault_b_key,
            mint_a_address: mint_a_key,
            mint_b_address: mint_b_key,
            token_owner,
            clock_owner,
        }
    }

    fn decode(f: &Fixture) -> Option<arb_types::PoolState> {
        decode_damm_v2_coherent_pool_state(
            f.pool_id,
            777,
            &f.pool,
            f.pool_owner,
            f.vault_a_address,
            f.vault_b_address,
            f.mint_a_address,
            f.mint_b_address,
            &f.vault_a,
            f.token_owner,
            &f.vault_b,
            f.token_owner,
            &f.mint_a,
            f.token_owner,
            &f.mint_b,
            f.token_owner,
            &f.clock,
            f.clock_owner,
        )
    }

    #[test]
    fn accepts_complete_same_context_pool_vault_mint_clock_graph() {
        let f = fixture();
        let state = decode(&f).expect("complete verified account graph must decode");
        assert_eq!(state.protocol, DexProtocol::MeteoraDammV2);
        assert_eq!(state.kind, PoolKind::Clmm);
        assert_eq!(state.slot.0, 777);
        assert_eq!(state.decimals_a, 9);
        assert_eq!(state.decimals_b, 6);
        assert!(state.is_active);
        let damm = state.damm_v2.as_ref().unwrap();
        assert!(damm.account_graph_verified);
        assert_eq!(damm.clock_slot, 777);
        assert_eq!(damm.clock_epoch, 1_097);
    }

    #[test]
    fn rejects_pool_or_clock_owner_mismatch() {
        let mut f = fixture();
        f.pool_owner = Pubkey::new_unique();
        assert!(decode(&f).is_none());

        let mut f = fixture();
        f.clock_owner = Pubkey::new_unique();
        assert!(decode(&f).is_none());
    }

    #[test]
    fn rejects_vault_mint_or_authority_mismatch() {
        let mut f = fixture();
        f.vault_a[0..32].copy_from_slice(Pubkey::new_unique().as_ref());
        assert!(decode(&f).is_none());

        let mut f = fixture();
        f.vault_b[32..64].copy_from_slice(Pubkey::new_unique().as_ref());
        assert!(decode(&f).is_none());
    }

    #[test]
    fn rejects_dependency_addresses_that_do_not_match_current_pool_bytes() {
        let mut f = fixture();
        f.pool[232..264].copy_from_slice(Pubkey::new_unique().as_ref());
        assert!(decode(&f).is_none());

        let mut f = fixture();
        f.pool[200..232].copy_from_slice(Pubkey::new_unique().as_ref());
        assert!(decode(&f).is_none());
    }
    #[test]
    fn rejects_clock_slot_mismatch_and_disabled_pool() {
        let mut f = fixture();
        f.clock[0..8].copy_from_slice(&778u64.to_le_bytes());
        assert!(decode(&f).is_none());

        let mut f = fixture();
        f.pool[481] = 1;
        assert!(decode(&f).is_none());
    }
}
