//! Multi-DEX Active Pool Registry & Live Reserve Ingestion.
//!
//! Registers verified mainnet pools across Raydium V4, Orca Whirlpool, and
//! Meteora DLMM and batch-fetches their live reserve balances each tick. The
//! reserve VALUES (the live market data) are decoded directly from on-chain
//! token-vault account balances via `getMultipleAccounts` — nothing is
//! hardcoded. Only the permanent on-chain account ADDRESSES (pool + vault
//! pubkeys, which never change after pool creation) are recorded, matching
//! the existing pattern used for Raydium / Orca vaults.
//!
//! Feeding these live multi-pool states into `MarketGraph` ends the previous
//! single-pool starvation where `find_base_cycles` could never find a second
//! pool to form a 2-hop cross-DEX arbitrage.

use crate::RpcPool;
use arb_types::{DexProtocol, MintId, PoolId, PoolState, Slot};
use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use tracing::{debug, warn};

/// Maximum accounts per `getMultipleAccounts` request (RPC hard limit is 100).
const RPC_BATCH: usize = 100;

/// A verified mainnet pool with its two token-vault addresses for live balance reads.
#[derive(Debug, Clone)]
pub struct RegisteredPool {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    /// Mint decimals for `(base_mint, quote_mint)`, read from a verified mint
    /// account. `0` means unknown; such pools are rejected by the engine.
    pub decimals_a: u8,
    pub decimals_b: u8,
    /// SPL token account holding the base-mint reserve (live balance read each tick).
    pub base_vault: Pubkey,
    /// SPL token account holding the quote-mint reserve (live balance read each tick).
    pub quote_vault: Pubkey,
    /// Protocol swap fee in basis points (used for graph edge weights).
    pub fee_bps: u16,
}

/// Registry of verified multi-DEX pools that streams live reserve states into the
/// `MarketStateCache` / `MarketGraph`.
///
/// Registry configuration is immutable during a fetch. The supervisor stores
/// replacement registries behind ArcSwap so refreshes do not mutate in-flight reads.
#[derive(Clone)]
pub struct MultiDexRegistry {
    pools: Vec<RegisteredPool>,
    rpc: RpcPool,
}

impl MultiDexRegistry {
    /// Build a registry pre-loaded with verified mainnet SOL-USDC pools across
    /// Raydium V4, Orca Whirlpool, and Meteora DLMM, sharing the supplied RPC pool.
    pub fn mainnet_default(rpc: RpcPool) -> Self {
        // Addresses come from `arb-constants`; nothing is re-typed here.
        let pk = |s: &str| Pubkey::from_str(s).expect("static mainnet address is valid base58");
        let wsol = MintId(pk(arb_constants::WSOL_MINT));
        let usdc = MintId(pk(arb_constants::USDC_MINT));

        let pools = vec![
            // Raydium AMM v4 — SOL/USDC
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::RAYDIUM_SOL_USDC_POOL)),
                protocol: DexProtocol::RaydiumV4,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::RAYDIUM_SOL_VAULT),
                quote_vault: pk(arb_constants::RAYDIUM_USDC_VAULT),
                fee_bps: 25,
            },
            // Orca Whirlpools — SOL/USDC (tick-spacing 4 pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::ORCA_SOL_USDC_POOL)),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::ORCA_SOL_VAULT),
                quote_vault: pk(arb_constants::ORCA_USDC_VAULT),
                fee_bps: 5,
            },
            // Orca Whirlpools — SOL/USDC, additional fee tiers. These unlock the
            // LOW-FEE routes (tick 1 => 1 bps, tick 2 => 2 bps, tick 8 => 8 bps;
            // the real fee is read on-chain). Vaults are placeholders: CLMM
            // reserves come from the pool account (sqrt_price + liquidity), not
            // the vault balances, so the registry does not require them here.
            RegisteredPool {
                pool_id: PoolId(pk("83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d")),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d"),
                quote_vault: pk("83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d"),
                fee_bps: 1,
            },
            RegisteredPool {
                pool_id: PoolId(pk("FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q")),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q"),
                quote_vault: pk("FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q"),
                fee_bps: 2,
            },
            RegisteredPool {
                pool_id: PoolId(pk("7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm")),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm"),
                quote_vault: pk("7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm"),
                fee_bps: 8,
            },
            // Meteora DLMM — SOL/USDC (bin_step 4, verified live mainnet pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::METEORA_SOL_USDC_POOL)),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::METEORA_SOL_RESERVE),
                quote_vault: pk(arb_constants::METEORA_USDC_RESERVE),
                fee_bps: arb_constants::METEORA_SOL_USDC_BIN_STEP,
            },
            // Raydium CPMM — SOL/USDC (1 bps fee tier, lowest on Solana)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::RAYDIUM_CPMM_SOL_USDC_POOL)),
                protocol: DexProtocol::RaydiumCpmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::RAYDIUM_CPMM_SOL_VAULT),
                quote_vault: pk(arb_constants::RAYDIUM_CPMM_USDC_VAULT),
                fee_bps: 1,
            },
        ];

        Self { pools, rpc }
    }

    /// Register an additional verified pool dynamically (e.g. a DexScreener-discovered
    /// trending token pool with >= 2 venues).
    pub fn register(&mut self, pool: RegisteredPool) {
        if !self.pools.iter().any(|p| p.pool_id == pool.pool_id) {
            self.pools.push(pool);
        }
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    pub fn pools(&self) -> &[RegisteredPool] {
        &self.pools
    }

    /// Batch-fetch LIVE reserve balances for every registered pool in a single
    /// `getMultipleAccounts` call and produce fresh `PoolState`s. Returns only
    /// pools whose both reserves decoded to a non-zero live balance.
    ///
    /// This is the call that ends `MarketGraph` starvation: with opposing Raydium
    /// / Orca / Meteora SOL-USDC pools populated, `find_base_cycles` immediately
    /// discovers real cross-DEX 2-hop arbitrage.
    pub async fn fetch_live_pool_states(&self, current_slot: Slot) -> Vec<PoolState> {
        if self.pools.is_empty() {
            return Vec::new();
        }

        // Vault balances are only needed by CPMM pools: a CLMM/DLMM reserve is
        // derived from the pool account (sqrt price / bins), so fetching its two
        // vaults would double the request without changing a quote.
        let mut vault_keys: Vec<String> = Vec::new();
        let mut vault_slot: Vec<Option<(usize, usize)>> = vec![None; self.pools.len()];
        for (i, p) in self.pools.iter().enumerate() {
            if matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            ) {
                vault_slot[i] = Some((vault_keys.len(), vault_keys.len() + 1));
                vault_keys.push(p.base_vault.to_string());
                vault_keys.push(p.quote_vault.to_string());
            }
        }
        let vault_accounts = fetch_accounts_chunked(&self.rpc, &vault_keys).await;

        // Pool accounts carry the CLMM/DLMM state *and* each pool's real fee.
        // Chunked at the RPC's 100-account limit so a large watch set is never
        // rejected outright.
        let pool_keys: Vec<String> = self.pools.iter().map(|p| p.pool_id.to_string()).collect();
        let pool_accounts = fetch_accounts_chunked(&self.rpc, &pool_keys).await;

        // ── Derived accounts (tick arrays / bin arrays), one batched request ──
        // Doing this per pool was N sequential round trips: with 60 pools the
        // tick would take longer than the slot it is trying to catch.
        let mut derived_keys: Vec<String> = Vec::new();
        let mut dlmm_meta: Vec<(PoolId, usize)> = Vec::new();
        let mut clmm_meta: Vec<(PoolId, Vec<i32>, u16, usize)> = Vec::new();
        for (i, p) in self.pools.iter().enumerate() {
            let data = pool_accounts
                .get(i)
                .and_then(|a| a.as_ref())
                .and_then(decode_account_data);
            match p.protocol {
                DexProtocol::MeteoraDlmm => {
                    if let Some(d) = data.as_deref().and_then(decode_dlmm_state) {
                        let start = derived_keys.len();
                        for key in dlmm_bin_array_pdas(&p.pool_id.0, d.active_bin_id) {
                            derived_keys.push(key.to_string());
                        }
                        dlmm_meta.push((p.pool_id, start));
                    }
                }
                DexProtocol::OrcaWhirlpool => {
                    if let Some(c) = data.as_deref().and_then(decode_clmm_state) {
                        let (keys, starts) =
                            clmm_tick_array_pdas(&p.pool_id, c.tick_current, c.tick_spacing);
                        let start = derived_keys.len();
                        for key in keys {
                            derived_keys.push(key.to_string());
                        }
                        clmm_meta.push((p.pool_id, starts, c.tick_spacing, start));
                    }
                }
                _ => {}
            }
        }
        let derived_accounts = fetch_accounts_chunked(&self.rpc, &derived_keys).await;

        let mut dlmm_bins: std::collections::HashMap<PoolId, Vec<arb_types::DlmmBin>> =
            std::collections::HashMap::new();
        for (pool_id, start) in dlmm_meta {
            let mut bins = Vec::new();
            for slot in start..start + 3 {
                if let Some(bytes) = derived_accounts
                    .get(slot)
                    .and_then(|a| a.as_ref())
                    .and_then(decode_account_data)
                {
                    bins.extend(decode_bin_array(&bytes));
                }
            }
            dlmm_bins.insert(pool_id, bins);
        }

        let mut clmm_ticks: std::collections::HashMap<PoolId, Vec<arb_types::ClmmTickArray>> =
            std::collections::HashMap::new();
        for (pool_id, starts, tick_spacing, start) in clmm_meta {
            let mut arrays = Vec::new();
            for (offset, start_tick) in starts.iter().enumerate() {
                let Some(bytes) = derived_accounts
                    .get(start + offset)
                    .and_then(|a| a.as_ref())
                    .and_then(decode_account_data)
                else {
                    continue;
                };
                let Ok(facade) = arb_math::clmm::parse_tick_array_account(&bytes, tick_spacing)
                else {
                    continue;
                };
                // A PDA that decodes but starts elsewhere is not this pool's
                // array — never attach it.
                if facade.start_tick_index != *start_tick {
                    continue;
                }
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
                        })
                        .collect(),
                });
            }
            clmm_ticks.insert(pool_id, arrays);
        }

        let mut states = Vec::with_capacity(self.pools.len());
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);
        for (i, p) in self.pools.iter().enumerate() {
            let (base_bal, quote_bal) = match vault_slot[i] {
                Some((b, q)) => (
                    decode_token_amount(vault_accounts.get(b).and_then(|a| a.as_ref())),
                    decode_token_amount(vault_accounts.get(q).and_then(|a| a.as_ref())),
                ),
                None => (0, 0),
            };

            // CPMM (Raydium) prices off the vault balances, so it needs them.
            // CLMM/DLMM derive their reserves from the pool account (virtual
            // reserves / bins), so a missing vault is not fatal for them.
            let needs_vaults = matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            );
            if needs_vaults && (base_bal == 0 || quote_bal == 0) {
                debug!(
                    "MultiDexRegistry: {:?} pool {} missing live reserves (base={} quote={})",
                    p.protocol, p.pool_id, base_bal, quote_bal
                );
                continue;
            }

            // Decode the pool account ONCE, then derive CLMM/DLMM state and the
            // pool's REAL fee from it.
            let pool_data: Option<Vec<u8>> = pool_accounts
                .get(i)
                .and_then(|a| a.as_ref())
                .and_then(decode_account_data);

            let pump_curve = match (p.protocol, pool_data.as_deref()) {
                (DexProtocol::PumpSwap, _) => None,
                (DexProtocol::PumpFun, Some(data)) => {
                    arb_dex::pump_fun::PumpFunBondingCurve::decode_account_data(data).ok()
                }
                _ => None,
            };

            let pump_state = pump_curve;

            let mut clmm = match (p.protocol, pool_data.as_deref()) {
                (DexProtocol::OrcaWhirlpool, Some(data)) => decode_clmm_state(data),
                _ => None,
            };
            let mut dlmm = match (p.protocol, pool_data.as_deref()) {
                (DexProtocol::MeteoraDlmm, Some(data)) => decode_dlmm_state(data),
                _ => None,
            };
            // Attach the BinArrays covering the active bin so the DLMM can be
            // priced honestly (dynamic bins + fee). Empty on failure → the pool
            // is skipped below (fail closed), never priced off vault balances.
            if p.protocol == DexProtocol::MeteoraDlmm
                && let Some(d) = dlmm.as_mut()
            {
                d.bins = dlmm_bins.get(&p.pool_id).cloned().unwrap_or_default();
            }

            // The pool's REAL fee must come from the protocol account/config. Raydium
            // and Orca fees are decoded here; DLMM uses its exact dynamic quote fee.
            // Pump.fun is deliberately skipped until its curve + Global + Pump Fees
            // FeeConfig are joined at a coherent Yellowstone slot. Never assume 125 bps.
            // Fail closed: if the account/config cannot be decoded the pool is skipped.
            let onchain_fee_bps = match (p.protocol, pool_data.as_deref()) {
                (DexProtocol::PumpFun, _) => None,
                (DexProtocol::PumpSwap, _) => None,
                (DexProtocol::RaydiumV4, Some(d)) => decode_raydium_fee_bps(d),
                (DexProtocol::RaydiumCpmm, Some(d)) => decode_raydium_cpmm_fee_bps(d),
                (DexProtocol::OrcaWhirlpool, Some(d)) => decode_orca_fee_bps(d),
                (DexProtocol::MeteoraDlmm, _) => dlmm.as_ref().and_then(|s| {
                    let prm = s.params.as_ref()?;
                    let base = (prm.base_factor as u64) * (s.bin_step_bps as u64) * 10;
                    let scaled = if prm.base_fee_power_factor == 0 {
                        base
                    } else {
                        base.checked_mul(10u64.checked_pow(prm.base_fee_power_factor as u32)?)?
                    };
                    // `scaled` is in 1e9 units; convert to bps.
                    u16::try_from(scaled.saturating_mul(10_000) / 1_000_000_000).ok()
                }),
                _ => None,
            };
            let Some(fee_bps) = onchain_fee_bps else {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — on-chain fee undecodable; skipping (fail closed)",
                    p.protocol, p.pool_id
                );
                continue;
            };

            // Attach the tick arrays fetched earlier so concentrated-liquidity
            // quotes can cross ticks instead of being capped at the current tick.
            if p.protocol == DexProtocol::OrcaWhirlpool
                && let Some(c) = clmm.as_mut()
            {
                c.tick_arrays = clmm_ticks.get(&p.pool_id).cloned().unwrap_or_default();
            }

            let kind = match p.protocol {
                DexProtocol::OrcaWhirlpool => arb_types::PoolKind::Clmm,
                DexProtocol::MeteoraDlmm => arb_types::PoolKind::Dlmm,
                _ => arb_types::PoolKind::Cpmm,
            };

            // CLMM pools: the raw vault balances are NOT the CPMM-equivalent
            // reserves — concentrated liquidity means the price is set by
            // `sqrt_price`, not the vault ratio. Using vault balances would price
            // an Orca SOL/USDC pool at the wrong ratio (observed ~34.8 vs the true
            // ~96.5 USDC/SOL). Use the instantaneous virtual reserves
            // x = L/sqrt(P), y = L*sqrt(P) so the graph sizes and prices correctly.
            //
            // DLMM pools: same idea — the vault balances are not the local depth.
            // Use the summed bin amounts around the active bin, oriented to the
            // pool's base/quote mints (Meteora orders tokens X/Y by pubkey).
            let (reserve_in, reserve_out) = match p.protocol {
                DexProtocol::PumpSwap => (0, 0),
                DexProtocol::PumpFun => match pump_curve {
                    Some((v_tok, v_sol, _, _, _, false)) if v_sol > 0 && v_tok > 0 => {
                        (v_sol as u128, v_tok as u128)
                    }
                    _ => (0, 0),
                },
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm => match clmm.as_ref() {
                    Some(c) if c.liquidity > 0 && c.sqrt_price > 0 => {
                        let l = ruint::aliases::U256::from(c.liquidity);
                        let sp = ruint::aliases::U256::from(c.sqrt_price);
                        let x_virt = (l << 64usize) / sp;
                        let y_virt = (l * sp) >> 64usize;
                        (
                            x_virt.try_into().unwrap_or(u128::MAX),
                            y_virt.try_into().unwrap_or(u128::MAX),
                        )
                    }
                    _ => (0, 0),
                },
                DexProtocol::MeteoraDlmm => match dlmm.as_ref() {
                    // No decoded bins means no local depth and no price. Summing
                    // zero bins and clamping to 1 would invent a 1:1 pool, which
                    // reads to the graph as a hugely profitable venue.
                    Some(d) if !d.bins.is_empty() => {
                        // Price of token X in token Y, Q64.64, from the ACTIVE
                        // bin. Summing bin amounts does NOT give the price (bins
                        // sit at different prices); the active bin defines it.
                        let Some(p_x_in_y) =
                            arb_math::dlmm::get_price_from_id(d.active_bin_id, d.bin_step_bps)
                                .ok()
                                .filter(|p| *p > 0)
                        else {
                            continue; // no trustworthy price → skip this pool
                        };
                        // token_x_mint @88 of the LbPair account. Without it the
                        // base/quote orientation is unknown, and a flipped ratio
                        // is a fake edge.
                        let Some(tx) = pool_data
                            .as_deref()
                            .filter(|b| b.len() >= 152)
                            .and_then(|b| b.get(88..120))
                            .and_then(|s| <[u8; 32]>::try_from(s).ok())
                            .map(Pubkey::new_from_array)
                        else {
                            continue;
                        };
                        // Instantaneous active-bin capacity linearization (O(1)) per SEEEVV.MD
                        let active_bin = d.bins.iter().find(|b| b.bin_id == d.active_bin_id);
                        let (bin_x, bin_y) = match active_bin {
                            Some(b) => (b.amount_x, b.amount_y),
                            None => (
                                d.bins.iter().map(|b| b.amount_x).sum(),
                                d.bins.iter().map(|b| b.amount_y).sum(),
                            ),
                        };
                        if tx == p.base_mint.0 {
                            let rx = if bin_x > 0 {
                                bin_x
                            } else {
                                d.bins.iter().map(|b| b.amount_x).sum()
                            };
                            let ry = (rx.saturating_mul(p_x_in_y)) >> 64usize;
                            (rx, ry)
                        } else if tx == p.quote_mint.0 {
                            let ry = if bin_y > 0 {
                                bin_y
                            } else {
                                d.bins.iter().map(|b| b.amount_y).sum()
                            };
                            let rx = (ry << 64usize) / p_x_in_y;
                            (ry, rx)
                        } else {
                            (0, 0)
                        }
                    }
                    _ => (0, 0),
                },
                _ => (base_bal as u128, quote_bal as u128),
            };

            // Fail closed on a pool with no depth. A zero-reserve edge is not a
            // cheap venue — it is an absent one, and feeding it to the graph
            // produces either a divide-by-zero or a fake infinite-price edge.
            if reserve_in == 0 || reserve_out == 0 {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — zero depth; skipping (fail closed)",
                    p.protocol, p.pool_id
                );
                continue;
            }

            states.push(PoolState {
                pool_id: p.pool_id,
                protocol: p.protocol,
                slot: current_slot,
                write_version: 1,
                base_mint: p.base_mint,
                quote_mint: p.quote_mint,
                decimals_a: p.decimals_a,
                decimals_b: p.decimals_b,
                reserve_in,
                reserve_out,
                fee_numerator: fee_bps as u64,
                fee_denominator: 10_000,
                is_active: true,
                has_risky_token: false,
                last_update_slot: current_slot.0,
                last_update_ms: now_ms,
                kind,
                clmm,
                dlmm,
                pump_fun: pump_state,
                transfer_fee_bps: 0,
            });
        }

        states
    }
}

/// Decode a Raydium v4 `AmmInfo` account's two token vaults.
///
/// `coin_vault @336`, `pc_vault @368` (verified against live mainnet pools).
pub fn decode_raydium_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 400 {
        return None;
    }
    let a: [u8; 32] = data.get(336..368)?.try_into().ok()?;
    let b: [u8; 32] = data.get(368..400)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decode Orca Whirlpool token vaults (`token_vault_a @133..165`, `token_vault_b @213..245`).
pub fn decode_orca_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 245 {
        return None;
    }
    let a: [u8; 32] = data.get(133..165)?.try_into().ok()?;
    let b: [u8; 32] = data.get(213..245)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decode Raydium CPMM token vaults (`token_0_vault @72..104`, `token_1_vault @104..136`).
pub fn decode_raydium_cpmm_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 136 {
        return None;
    }
    let a: [u8; 32] = data.get(72..104)?.try_into().ok()?;
    let b: [u8; 32] = data.get(104..136)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decoded metadata from an on-chain pool account needed for live balance reads.
#[derive(Debug, Clone)]
pub struct DecodedPoolInfo {
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub fee_bps: u16,
    pub decimals_a: Option<u8>,
    pub decimals_b: Option<u8>,
}

/// Decode on-chain pool accounts across Raydium V4, Raydium CPMM, Orca Whirlpool, and Meteora DLMM.
pub fn decode_pool_info(protocol: DexProtocol, data: &[u8]) -> Option<DecodedPoolInfo> {
    match protocol {
        DexProtocol::RaydiumV4 => {
            if data.len() < 464 {
                return None;
            }
            let va: [u8; 32] = data.get(336..368)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(368..400)?.try_into().ok()?;
            let ma: [u8; 32] = data.get(400..432)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(432..464)?.try_into().ok()?;
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let fee_bps = decode_raydium_fee_bps(data).unwrap_or(25);
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps,
                decimals_a: None,
                decimals_b: None,
            })
        }
        DexProtocol::RaydiumCpmm => {
            if data.len() < 333 {
                return None;
            }
            let va: [u8; 32] = data.get(72..104)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(104..136)?.try_into().ok()?;
            let ma: [u8; 32] = data.get(168..200)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(200..232)?.try_into().ok()?;
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let fee_bps = decode_raydium_cpmm_fee_bps(data).unwrap_or(25);
            let dec_a = data[331];
            let dec_b = data[332];
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps,
                decimals_a: if (1..=18).contains(&dec_a) {
                    Some(dec_a)
                } else {
                    None
                },
                decimals_b: if (1..=18).contains(&dec_b) {
                    Some(dec_b)
                } else {
                    None
                },
            })
        }
        DexProtocol::OrcaWhirlpool => {
            if data.len() < 245 {
                return None;
            }
            let ma: [u8; 32] = data.get(101..133)?.try_into().ok()?;
            let va: [u8; 32] = data.get(133..165)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(181..213)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(213..245)?.try_into().ok()?;
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_mint = Pubkey::new_from_array(mb);
            let token_b_vault = Pubkey::new_from_array(vb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let fee_bps = decode_orca_fee_bps(data).unwrap_or(5);
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps,
                decimals_a: None,
                decimals_b: None,
            })
        }
        DexProtocol::MeteoraDlmm => {
            if data.len() < 216 {
                return None;
            }
            let ma: [u8; 32] = data.get(88..120)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(120..152)?.try_into().ok()?;
            let va: [u8; 32] = data.get(152..184)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(184..216)?.try_into().ok()?;
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let bin_step = u16::from_le_bytes(data.get(80..82)?.try_into().ok()?).max(1);
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps: bin_step,
                decimals_a: None,
                decimals_b: None,
            })
        }
        _ => None,
    }
}

/// Read mint decimals (offset 44) for each mint in batched `getMultipleAccounts` calls.
/// Mints owned by Token-2022 are deliberately omitted to reject tokens with transfer fees.
pub async fn resolve_mints_decimals(
    rpc: &crate::RpcPool,
    mints: &[MintId],
) -> std::collections::HashMap<MintId, u8> {
    let mut out = std::collections::HashMap::new();
    let unique: Vec<MintId> = {
        let mut seen = std::collections::HashSet::new();
        mints.iter().copied().filter(|m| seen.insert(*m)).collect()
    };
    if unique.is_empty() {
        return out;
    }

    for chunk in unique.chunks(RPC_BATCH) {
        let keys: Vec<String> = chunk.iter().map(|m| m.0.to_string()).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let accounts = match rpc.get_multiple_accounts(&refs).await {
            Ok(a) => a,
            Err(e) => {
                warn!("resolve_mints_decimals RPC failure: {e}");
                continue;
            }
        };
        for (mint, acc) in chunk.iter().zip(accounts.iter()) {
            let Some(acc) = acc else { continue };
            if acc.get("owner").and_then(|v| v.as_str()) != Some(arb_constants::TOKEN_PROGRAM_ID) {
                continue;
            }
            let Some(b64) = acc
                .get("data")
                .and_then(|d| d.get(0))
                .and_then(|s| s.as_str())
            else {
                continue;
            };
            let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(b64) else {
                continue;
            };
            if bytes.len() < 82 {
                continue;
            }
            // Token Safety Gate: check mint and freeze authorities for non-hub tokens.
            let mint_str = mint.0.to_string();
            let is_hub = mint_str == arb_constants::WSOL_MINT
                || mint_str == arb_constants::USDC_MINT
                || mint_str == "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
            if !is_hub {
                let mint_auth_tag =
                    u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([1, 0, 0, 0]));
                let freeze_auth_tag =
                    u32::from_le_bytes(bytes[46..50].try_into().unwrap_or([1, 0, 0, 0]));
                if mint_auth_tag != 0 || freeze_auth_tag != 0 {
                    debug!(
                        "Safety gate: dropping meme token {} (mint_auth={}, freeze_auth={})",
                        mint_str, mint_auth_tag, freeze_auth_tag
                    );
                    continue;
                }
            }
            let dec = bytes[44];
            if dec <= 18 {
                out.insert(*mint, dec);
            }
        }
    }
    out
}

/// Derive the three Orca `tick_array` PDAs that can affect a quote at
/// `tick_current`: the array containing the current tick, the one below and the
/// one above. Returns the keys plus the start tick of each, so a decoded array
/// can be checked against the slot it was requested for.
fn clmm_tick_array_pdas(
    pool_id: &PoolId,
    tick_current: i32,
    tick_spacing: u16,
) -> (Vec<Pubkey>, Vec<i32>) {
    let Ok(program_id) = Pubkey::from_str(arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID) else {
        return (Vec::new(), Vec::new());
    };
    let spacing = tick_spacing.max(1);
    let ticks_per_array = (88i32) * spacing as i32;
    let start = tick_current.div_euclid(ticks_per_array) * ticks_per_array;
    let starts = [start, start - ticks_per_array, start + ticks_per_array];

    let mut keys = Vec::with_capacity(3);
    for s in starts {
        let s_str = s.to_string();
        let seeds: &[&[u8]] = &[b"tick_array", pool_id.0.as_ref(), s_str.as_bytes()];
        keys.push(Pubkey::find_program_address(seeds, &program_id).0);
    }
    (keys, starts.to_vec())
}

/// Derive the three Meteora `BinArray` PDAs covering the active bin.
fn dlmm_bin_array_pdas(pool: &Pubkey, active_id: i32) -> Vec<Pubkey> {
    let index = bin_id_to_bin_array_index(active_id);
    [index - 1, index, index + 1]
        .iter()
        .map(|i| derive_meteora_bin_array_pda(pool, *i))
        .collect()
}

/// Decode an RPC account's `data[0]` base64 payload.
pub fn decode_account_data(acc: &serde_json::Value) -> Option<Vec<u8>> {
    let b64 = acc.get("data")?.get(0)?.as_str()?;
    base64::prelude::BASE64_STANDARD.decode(b64).ok()
}

/// `getMultipleAccounts` in chunks of [`RPC_BATCH`].
///
/// A single call is capped at 100 accounts by the RPC, so a watch set larger
/// than 50 pools (two keys each) used to fail wholesale. Chunking keeps the
/// request valid and returns a flat, index-aligned vector.
pub async fn fetch_accounts_chunked(
    rpc: &crate::RpcPool,
    keys: &[String],
) -> Vec<Option<serde_json::Value>> {
    let mut out: Vec<Option<serde_json::Value>> = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(RPC_BATCH) {
        let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
        match rpc.get_multiple_accounts(&refs).await {
            Ok(mut accs) => {
                // Keep the result index-aligned with the requested keys even if
                // the node returns a short array.
                accs.resize(chunk.len(), None);
                out.extend(accs);
            }
            Err(e) => {
                warn!("fetch_accounts_chunked: getMultipleAccounts failed: {e}");
                out.extend(std::iter::repeat_n(None, chunk.len()));
            }
        }
    }
    out
}

/// Decode the Raydium v4 swap fee in basis points from the AMM account.
///
/// `AmmInfo` layout (no discriminator): `swap_fee_numerator` @176 (u64),
/// `swap_fee_denominator` @184 (u64). Verified live on the SOL/USDC pool:
/// 25 / 10_000 = 25 bps. Returns `None` on a short/zero-denominator account so
/// the caller can fail closed rather than price with a guessed fee.
pub fn decode_raydium_fee_bps(data: &[u8]) -> Option<u16> {
    if data.len() < 192 {
        return None;
    }
    let num = u64::from_le_bytes(data[176..184].try_into().ok()?);
    let den = u64::from_le_bytes(data[184..192].try_into().ok()?);
    if den == 0 {
        return None;
    }
    u16::try_from((num as u128 * 10_000) / den as u128).ok()
}

/// Decode the Orca Whirlpool fee in basis points from the whirlpool account.
///
/// `fee_rate` @45 (u16) is in HUNDREDTHS OF A BIP (denominator 1_000_000), so
/// bps = fee_rate / 100. Verified live on the SOL/USDC pool: 400 => 4 bps.
pub fn decode_orca_fee_bps(data: &[u8]) -> Option<u16> {
    if data.len() < 47 {
        return None;
    }
    let fee_rate = u16::from_le_bytes(data[45..47].try_into().ok()?);
    Some(fee_rate / 100)
}

/// Decode the Raydium CPMM (constant product) swap fee in basis points from the pool account.
///
/// Raydium CPMM `PoolState` layout (Anchor, with 8-byte discriminator):
/// - `trade_fee_rate` @8+136 (u64) — fee rate in hundredths of a basis point, denominator 1_000_000.
///   bps = trade_fee_rate / 100. Verified: 25 bps = 2500.
///
/// Returns `None` on short account or zero denominator so the caller fails closed.
pub fn decode_raydium_cpmm_fee_bps(data: &[u8]) -> Option<u16> {
    // Anchor discriminator (8) + PoolState fields up to trade_fee_rate
    // PoolState: amm_config (32) + pool_creator (32) + token_0_vault (32) + token_1_vault (32)
    // + token_0_mint (32) + token_1_mint (32) + observation_key (32) + auth_bump (1) + status (1)
    // + lp_mint (32) + lp_mint_decimals (1) + token_0_decimals (1) + token_1_decimals (1)
    // + token_0_price (8) + token_1_price (8) + lp_supply (16) + protocol_fee_rate (4)
    // + trade_fee_rate (8) ...
    // Total before trade_fee_rate: 8(disc) + 32*8 + 1+1 + 1+1+1 + 8+8 + 16 + 4 = 8+256+3+3+16+4 = 290 bytes
    // trade_fee_rate @8+282 = 290
    const OFFSET: usize = 8 + 282;
    if data.len() < OFFSET + 8 {
        return None;
    }
    let rate = u64::from_le_bytes(data[OFFSET..OFFSET + 8].try_into().ok()?);
    if rate == 0 {
        return None;
    }
    // rate is in hundredths of a basis point (denominator 1,000,000)
    // 2500 => 25 bps
    u16::try_from(rate / 100).ok()
}

#[cfg(test)]
mod fee_decoder_tests {
    use super::*;

    #[test]
    fn raydium_fee_reads_live_value() {
        // swap_fee_numerator=25, swap_fee_denominator=10_000 => 25 bps.
        let mut d = vec![0u8; 192];
        d[176..184].copy_from_slice(&25u64.to_le_bytes());
        d[184..192].copy_from_slice(&10_000u64.to_le_bytes());
        assert_eq!(decode_raydium_fee_bps(&d), Some(25));
    }

    #[test]
    fn orca_fee_reads_live_value() {
        // fee_rate = 400 hundredths of a bip => 4 bps (NOT the old hardcoded 5).
        let mut d = vec![0u8; 47];
        d[45..47].copy_from_slice(&400u16.to_le_bytes());
        assert_eq!(decode_orca_fee_bps(&d), Some(4));
    }

    #[test]
    fn raydium_cpmm_fee_reads_live_value() {
        // trade_fee_rate = 100 => 1 bps (CPMM 1 bps tier).
        // Offset = 8 + 282 = 290
        let mut d = vec![0u8; 300];
        d[290..298].copy_from_slice(&100u64.to_le_bytes());
        assert_eq!(decode_raydium_cpmm_fee_bps(&d), Some(1));
    }

    #[test]
    fn raydium_cpmm_fee_reads_25bps_value() {
        // trade_fee_rate = 2500 => 25 bps.
        let mut d = vec![0u8; 300];
        d[290..298].copy_from_slice(&2500u64.to_le_bytes());
        assert_eq!(decode_raydium_cpmm_fee_bps(&d), Some(25));
    }

    #[test]
    fn fee_decoders_fail_closed() {
        assert_eq!(decode_raydium_fee_bps(&[0u8; 10]), None);
        assert_eq!(decode_orca_fee_bps(&[0u8; 10]), None);
        let mut zero_den = vec![0u8; 192];
        zero_den[176..184].copy_from_slice(&25u64.to_le_bytes());
        assert_eq!(decode_raydium_fee_bps(&zero_den), None);
    }

    #[test]
    fn bin_array_index_uses_floor_division() {
        assert_eq!(bin_id_to_bin_array_index(0), 0);
        assert_eq!(bin_id_to_bin_array_index(69), 0);
        assert_eq!(bin_id_to_bin_array_index(70), 1);
        assert_eq!(bin_id_to_bin_array_index(-1), -1);
        assert_eq!(bin_id_to_bin_array_index(-70), -1);
        assert_eq!(bin_id_to_bin_array_index(-71), -2);
        // Live SOL/USDC pool active bin observed on-chain.
        assert_eq!(bin_id_to_bin_array_index(-5840), -84);
    }

    #[test]
    fn decode_bin_array_reads_bins() {
        let mut d = vec![0u8; METEORA_BIN_ARRAY_HEADER + 70 * METEORA_BIN_SIZE];
        d[8..16].copy_from_slice(&2i64.to_le_bytes()); // bin-array index 2
        // bin[0] @ header: amount_x @0, amount_y @8
        d[56..64].copy_from_slice(&123u64.to_le_bytes());
        d[64..72].copy_from_slice(&456u64.to_le_bytes());
        let bins = decode_bin_array(&d);
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0].bin_id, 140); // index 2 * 70 + 0
        assert_eq!(bins[0].amount_x, 123);
        assert_eq!(bins[0].amount_y, 456);
    }

    #[test]
    fn decode_dlmm_state_reads_params_and_active_id() {
        let mut d = vec![0u8; 96];
        d[8..10].copy_from_slice(&10_000u16.to_le_bytes()); // base_factor
        d[16..20].copy_from_slice(&120_000u32.to_le_bytes()); // variable_fee_control
        d[32..34].copy_from_slice(&1_000u16.to_le_bytes()); // protocol_share
        d[40..44].copy_from_slice(&20_039u32.to_le_bytes()); // volatility_accumulator
        d[76..80].copy_from_slice(&(-5840i32).to_le_bytes()); // active_id
        d[80..82].copy_from_slice(&4u16.to_le_bytes()); // bin_step
        d[82] = 0; // status active
        let s = decode_dlmm_state(&d).expect("valid LbPair");
        assert_eq!(s.active_bin_id, -5840);
        assert_eq!(s.bin_step_bps, 4);
        assert_eq!(s.volatility_accumulator, 20_039);
        let prm = s.params.expect("params");
        assert_eq!(prm.base_factor, 10_000);
        assert_eq!(prm.protocol_share, 1_000);
    }
}

pub fn decode_clmm_state(data: &[u8]) -> Option<arb_types::ClmmState> {
    if data.len() < 85 {
        return None;
    }
    let tick_spacing = u16::from_le_bytes(data[41..43].try_into().ok()?);
    let fee_rate = u16::from_le_bytes(data[45..47].try_into().ok()?);
    let liquidity = u128::from_le_bytes(data[49..65].try_into().ok()?);
    let sqrt_price = u128::from_le_bytes(data[65..81].try_into().ok()?);
    let tick_current = i32::from_le_bytes(data[81..85].try_into().ok()?);

    if liquidity == 0 || sqrt_price == 0 {
        return None;
    }

    Some(arb_types::ClmmState {
        sqrt_price,
        liquidity,
        tick_current,
        tick_spacing,
        fee_rate,
        tick_arrays: Vec::new(),
    })
}

/// Decode DLMM state (Meteora `LbPair` layout, verified against the live
/// SOL/USDC pool `5rCf1DM8...`) from raw account bytes.
///
/// Layout: discriminator(8), StaticParameters(32), VariableParameters(32),
/// bump_seed(1), bin_step_seed(2), pair_type(1), active_id(i32 @76),
/// bin_step(u16 @80), status(u8 @82). Bins live in separate `BinArray`
/// accounts and are attached by `fetch_dlmm_bins`.
pub fn decode_dlmm_state(data: &[u8]) -> Option<arb_types::DlmmState> {
    if data.len() < 88 {
        return None;
    }
    // StaticParameters @8
    let base_factor = u16::from_le_bytes(data[8..10].try_into().ok()?);
    let filter_period = u16::from_le_bytes(data[10..12].try_into().ok()?);
    let decay_period = u16::from_le_bytes(data[12..14].try_into().ok()?);
    let reduction_factor = u16::from_le_bytes(data[14..16].try_into().ok()?);
    let variable_fee_control = u32::from_le_bytes(data[16..20].try_into().ok()?);
    let max_volatility_accumulator = u32::from_le_bytes(data[20..24].try_into().ok()?);
    let protocol_share = u16::from_le_bytes(data[32..34].try_into().ok()?);
    let base_fee_power_factor = data[34];
    // VariableParameters @40
    let volatility_accumulator = u32::from_le_bytes(data[40..44].try_into().ok()?);
    let volatility_reference = u32::from_le_bytes(data[44..48].try_into().ok()?);
    let last_update_timestamp = i64::from_le_bytes(data[56..64].try_into().ok()?);
    // active_id @76, bin_step @80, status @82
    let active_bin_id = i32::from_le_bytes(data[76..80].try_into().ok()?);
    let bin_step_bps = u16::from_le_bytes(data[80..82].try_into().ok()?);
    let status = data[82];
    if status != 0 || bin_step_bps == 0 {
        return None; // pool disabled / invalid
    }

    Some(arb_types::DlmmState {
        active_bin_id,
        bin_step_bps,
        bins: Vec::new(), // attached separately from BinArray accounts
        params: Some(arb_types::DlmmFeeParams {
            base_factor,
            filter_period,
            decay_period,
            reduction_factor,
            variable_fee_control,
            max_volatility_accumulator,
            protocol_share,
            base_fee_power_factor,
        }),
        volatility_accumulator,
        volatility_reference,
        last_update_timestamp,
    })
}

/// Meteora bins per `BinArray` account.
///
/// Single source of truth: aliased to `arb_constants::METEORA_BINS_PER_BIN_ARRAY`
/// so the ingestion decoder and the `swap2` instruction builder can never
/// disagree on the BinArray index divisor again.
pub const METEORA_BINS_PER_ARRAY: i64 = arb_constants::METEORA_BINS_PER_BIN_ARRAY;
/// Size of a single Meteora `Bin` in bytes (verified from the IDL + live account).
pub const METEORA_BIN_SIZE: usize = 144;
/// `BinArray` header: disc(8) + index(8) + version(1) + padding(7) + lb_pair(32).
pub const METEORA_BIN_ARRAY_HEADER: usize = 56;

/// Derive the `BinArray` PDA for a pool and bin-array index:
/// seeds = `[b"bin_array", lb_pair, index.to_le_bytes()]`.
pub fn derive_meteora_bin_array_pda(pool: &Pubkey, index: i64) -> Pubkey {
    let program_id = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
        .expect("valid Meteora program id constant");
    Pubkey::find_program_address(
        &[b"bin_array", pool.as_ref(), &index.to_le_bytes()],
        &program_id,
    )
    .0
}

/// Bin-array index that contains `bin_id` (floor division, correct for negatives).
pub fn bin_id_to_bin_array_index(bin_id: i32) -> i64 {
    bin_id.div_euclid(METEORA_BINS_PER_ARRAY as i32) as i64
}

/// Decode the bins of one `BinArray` account. Bins with zero liquidity are
/// dropped; `bin_id = index * 70 + i`.
pub fn decode_bin_array(data: &[u8]) -> Vec<arb_types::DlmmBin> {
    let need = METEORA_BIN_ARRAY_HEADER + (METEORA_BINS_PER_ARRAY as usize) * METEORA_BIN_SIZE;
    if data.len() < need {
        return Vec::new();
    }
    let index = i64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
    let base_bin_id = index.saturating_mul(METEORA_BINS_PER_ARRAY) as i32;
    let mut out = Vec::with_capacity(METEORA_BINS_PER_ARRAY as usize);
    for i in 0..METEORA_BINS_PER_ARRAY as usize {
        let off = METEORA_BIN_ARRAY_HEADER + i * METEORA_BIN_SIZE;
        let amount_x = u64::from_le_bytes(data[off..off + 8].try_into().unwrap_or([0; 8]));
        let amount_y = u64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap_or([0; 8]));
        if amount_x == 0 && amount_y == 0 {
            continue;
        }
        out.push(arb_types::DlmmBin {
            bin_id: base_bin_id.saturating_add(i as i32),
            amount_x: amount_x as u128,
            amount_y: amount_y as u128,
        });
    }
    out
}

/// Resolve a pool's fee in bps from its live account data.
///
/// Returns `None` when the data is too short or the protocol is unsupported.
/// Callers must treat `None` as "skip this pool": a guessed fee is a fake edge.
///
/// Meteora bin arrays are decoded in one batched request inside
/// `fetch_live_pool_states` (see `dlmm_bin_array_pdas`), not here.
pub fn resolve_fee_bps(protocol: DexProtocol, pool_account_data: &[u8]) -> Option<u16> {
    match protocol {
        DexProtocol::RaydiumV4 => {
            if pool_account_data.len() < 192 {
                return None;
            }
            let num = u64::from_le_bytes(pool_account_data[176..184].try_into().ok()?);
            let den = u64::from_le_bytes(pool_account_data[184..192].try_into().ok()?);
            if den == 0 {
                return None;
            }
            let bps = u16::try_from(num as u128 * 10_000 / den as u128).unwrap_or(10_000);
            if bps > 10_000 { None } else { Some(bps) }
        }
        DexProtocol::OrcaWhirlpool => {
            if pool_account_data.len() < 47 {
                return None;
            }
            let fee_rate = u16::from_le_bytes(pool_account_data[45..47].try_into().ok()?);
            let bps = fee_rate / 100;
            if bps > 10_000 { None } else { Some(bps) }
        }
        DexProtocol::MeteoraDlmm => {
            // Reuse the verified LbPair decoder instead of carrying a second,
            // stale offset table: `base_factor` lives at 8 and `bin_step` at 80,
            // not at the 9/11 used by the old copy of this layout.
            let state = decode_dlmm_state(pool_account_data)?;
            let prm = state.params?;
            let base = (prm.base_factor as u64) * (state.bin_step_bps as u64) * 10;
            let scaled = if prm.base_fee_power_factor == 0 {
                base
            } else {
                base.checked_mul(10u64.checked_pow(prm.base_fee_power_factor as u32)?)?
            };
            let bps = u16::try_from(scaled.saturating_mul(10_000) / 1_000_000_000).ok()?;
            if bps > 10_000 { None } else { Some(bps) }
        }
        DexProtocol::RaydiumClmm => None,
        DexProtocol::RaydiumCpmm => decode_raydium_cpmm_fee_bps(pool_account_data),
        DexProtocol::PumpFun => None,
        DexProtocol::PumpSwap => None,
    }
}

/// Decode a u64 token amount from an SPL token account JSON value.
/// SPL token accounts store the amount as a little-endian u64 at byte offset 64,
/// base64-encoded in the `data[0]` field of the RPC response.
fn decode_token_amount(acc: Option<&serde_json::Value>) -> u64 {
    let Some(acc) = acc else {
        return 0;
    };
    let Some(data_str) = acc
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|s| s.as_str())
    else {
        return 0;
    };
    let Ok(bytes) = BASE64_STANDARD.decode(data_str) else {
        return 0;
    };
    if bytes.len() < 72 {
        return 0;
    }
    u64::from_le_bytes(bytes[64..72].try_into().unwrap_or([0; 8]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_default_registers_all_venues() {
        let rpc = RpcPool::new(vec!["https://example.test".to_string()]);
        let reg = MultiDexRegistry::mainnet_default(rpc);
        // 1 Raydium v4 + 1 Raydium CPMM + 4 Orca Whirlpool fee tiers + 1 Meteora DLMM.
        assert_eq!(reg.pool_count(), 7);
        let protocols: Vec<_> = reg.pools().iter().map(|p| p.protocol).collect();
        assert!(protocols.contains(&DexProtocol::RaydiumV4));
        assert!(protocols.contains(&DexProtocol::RaydiumCpmm));
        assert!(protocols.contains(&DexProtocol::OrcaWhirlpool));
        assert!(protocols.contains(&DexProtocol::MeteoraDlmm));
        // At least one sub-5 bps Orca tier must be present (the low-fee route).
        let orca_fees: Vec<u16> = reg
            .pools()
            .iter()
            .filter(|p| p.protocol == DexProtocol::OrcaWhirlpool)
            .map(|p| p.fee_bps)
            .collect();
        assert!(orca_fees.iter().any(|f| *f <= 2));
        // Raydium CPMM 1 bps tier must be present.
        let cpmm_fees: Vec<u16> = reg
            .pools()
            .iter()
            .filter(|p| p.protocol == DexProtocol::RaydiumCpmm)
            .map(|p| p.fee_bps)
            .collect();
        assert!(cpmm_fees.contains(&1));
    }

    #[test]
    fn registration_is_idempotent() {
        let rpc = RpcPool::new(vec!["https://example.test".to_string()]);
        let mut reg = MultiDexRegistry::mainnet_default(rpc);
        let before = reg.pool_count();
        let wsol = MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap());
        let usdc = MintId(Pubkey::from_str(arb_constants::USDC_MINT).unwrap());
        let dup = RegisteredPool {
            pool_id: PoolId(Pubkey::from_str(arb_constants::RAYDIUM_SOL_USDC_POOL).unwrap()),
            protocol: DexProtocol::RaydiumV4,
            base_mint: wsol,
            quote_mint: usdc,
            decimals_a: 9,
            decimals_b: 6,
            base_vault: Pubkey::default(),
            quote_vault: Pubkey::default(),
            fee_bps: 25,
        };
        reg.register(dup);
        assert_eq!(
            reg.pool_count(),
            before,
            "duplicate pool must not be re-registered"
        );
    }

    #[test]
    fn decode_token_amount_handles_missing() {
        assert_eq!(decode_token_amount(None), 0);
        assert_eq!(decode_token_amount(Some(&serde_json::json!({}))), 0);
    }

    #[test]
    fn resolve_fee_bps_raydium() {
        let mut data = vec![0u8; 192];
        data[176..184].copy_from_slice(&25u64.to_le_bytes());
        data[184..192].copy_from_slice(&10_000u64.to_le_bytes());
        assert_eq!(resolve_fee_bps(DexProtocol::RaydiumV4, &data), Some(25));
    }

    #[test]
    fn resolve_fee_bps_orca() {
        let mut data = vec![0u8; 47];
        data[45..47].copy_from_slice(&500u16.to_le_bytes()); // 5 bps
        assert_eq!(resolve_fee_bps(DexProtocol::OrcaWhirlpool, &data), Some(5));
    }

    #[test]
    fn resolve_fee_bps_meteora() {
        // Real layout: base_factor @8, bin_step @80, status @82.
        let mut data = vec![0u8; 88];
        data[8..10].copy_from_slice(&10_000u16.to_le_bytes());
        data[80..82].copy_from_slice(&4u16.to_le_bytes());
        data[82] = 0;
        assert_eq!(resolve_fee_bps(DexProtocol::MeteoraDlmm, &data), Some(4));
    }

    #[test]
    fn resolve_fee_bps_short_returns_none() {
        assert_eq!(resolve_fee_bps(DexProtocol::RaydiumV4, &[0; 10]), None);
        assert_eq!(resolve_fee_bps(DexProtocol::OrcaWhirlpool, &[0; 10]), None);
        assert_eq!(resolve_fee_bps(DexProtocol::MeteoraDlmm, &[0; 10]), None);
    }

    #[test]
    fn test_decode_pool_info_raydium_v4() {
        let mut data = vec![0u8; 500];
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        data[336..368].copy_from_slice(&va.to_bytes());
        data[368..400].copy_from_slice(&vb.to_bytes());
        data[400..432].copy_from_slice(&ma.to_bytes());
        data[432..464].copy_from_slice(&mb.to_bytes());
        data[176..184].copy_from_slice(&25u64.to_le_bytes());
        data[184..192].copy_from_slice(&10_000u64.to_le_bytes());

        let info = decode_pool_info(DexProtocol::RaydiumV4, &data).expect("valid Raydium V4");
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.fee_bps, 25);
        assert_eq!(info.decimals_a, None);
    }

    #[test]
    fn test_decode_pool_info_raydium_cpmm() {
        let mut data = vec![0u8; 350];
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        data[72..104].copy_from_slice(&va.to_bytes());
        data[104..136].copy_from_slice(&vb.to_bytes());
        data[168..200].copy_from_slice(&ma.to_bytes());
        data[200..232].copy_from_slice(&mb.to_bytes());
        data[331] = 9;
        data[332] = 6;
        data[290..298].copy_from_slice(&2500u64.to_le_bytes());

        let info = decode_pool_info(DexProtocol::RaydiumCpmm, &data).expect("valid Raydium CPMM");
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.fee_bps, 25);
        assert_eq!(info.decimals_a, Some(9));
        assert_eq!(info.decimals_b, Some(6));
    }

    #[test]
    fn test_decode_pool_info_orca_whirlpool() {
        let mut data = vec![0u8; 250];
        let ma = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[101..133].copy_from_slice(&ma.to_bytes());
        data[133..165].copy_from_slice(&va.to_bytes());
        data[181..213].copy_from_slice(&mb.to_bytes());
        data[213..245].copy_from_slice(&vb.to_bytes());
        data[45..47].copy_from_slice(&500u16.to_le_bytes()); // 500 hundredths of bip = 5 bps

        let info = decode_pool_info(DexProtocol::OrcaWhirlpool, &data).expect("valid Orca");
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.fee_bps, 5);
    }

    #[test]
    fn test_decode_pool_info_meteora_dlmm() {
        let mut data = vec![0u8; 220];
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[80..82].copy_from_slice(&4u16.to_le_bytes()); // bin step = 4
        data[88..120].copy_from_slice(&ma.to_bytes());
        data[120..152].copy_from_slice(&mb.to_bytes());
        data[152..184].copy_from_slice(&va.to_bytes());
        data[184..216].copy_from_slice(&vb.to_bytes());

        let info = decode_pool_info(DexProtocol::MeteoraDlmm, &data).expect("valid Meteora DLMM");
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.fee_bps, 4);
    }
}
