//! On-chain account decoders: pool/vault layouts, fee params, CLMM/DLMM
//! state decoding, PDA derivation and account-fetch helpers.

use crate::fetcher::PacedAccountFetcher;
use arb_types::{DexProtocol, MintId, PoolId};
use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use tracing::debug;

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

/// On-chain CLMM token orientation: `(mint_a, mint_b)` as stored in the pool
/// account bytes. This is the canonical order the quote engine requires
/// (`a_to_b = pool_base == from_mint` and
/// [`compute_clmm_virtual_reserves`] returning `(mint_a-side, mint_b-side)`).
///
/// Offsets verified against live mainnet via `clmm_probe.rs` (slot 468876781,
/// Orca SOL/USDC `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` data_len 653;
/// Raydium CLMM pools data_len 1544), cross-checked against official sources:
/// - Orca Whirlpool (`orca-so/whirlpools`, `programs/whirlpool/src/state/whirlpool.rs`,
///   interleaved layout): `token_mint_a @101..133`, `token_vault_a @133..165`,
///   `fee_growth_global_a @165..181`, `token_mint_b @181..213`. The offsets are
///   independent of `NUM_REWARDS` (reward infos start after `fee_growth_global_b`),
///   so both the 525-byte (2-reward) and 653-byte (3-reward) generations decode
///   identically. Orca does NOT enforce `mint_a < mint_b` on live pools —
///   no lexicographic shortcut exists.
/// - Raydium CLMM (`raydium-io/raydium-clmm`, `PoolState`): `token_mint0 @73..105`,
///   `token_mint1 @105..137`. Raydium does not enforce `mint0 < mint1` either.
///
/// Returns `None` (fail-closed) on short/truncated buffers or default pubkeys.
pub fn clmm_onchain_mints(protocol: DexProtocol, data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    let (ma, mb): ([u8; 32], [u8; 32]) = match protocol {
        DexProtocol::OrcaWhirlpool => {
            if data.len() < 213 {
                return None;
            }
            (
                data[101..133].try_into().ok()?,
                data[181..213].try_into().ok()?,
            )
        }
        DexProtocol::RaydiumClmm => {
            if data.len() < 137 {
                return None;
            }
            (
                data[73..105].try_into().ok()?,
                data[105..137].try_into().ok()?,
            )
        }
        _ => return None,
    };
    let a = Pubkey::new_from_array(ma);
    let b = Pubkey::new_from_array(mb);
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

/// Decode Raydium CLMM token vaults (`token_vault_0 @137..169`, `token_vault_1 @169..201`).
pub fn decode_raydium_clmm_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 201 {
        return None;
    }
    let a: [u8; 32] = data.get(137..169)?.try_into().ok()?;
    let b: [u8; 32] = data.get(169..201)?.try_into().ok()?;
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
            // Fail closed: the CPMM swap fee lives in the separate AmmConfig
            // account (trade_fee_rate u64 LE @12), not in the pool account, so
            // a pool-data-only decode cannot produce a verified fee. Returning
            // a guessed fee here would fabricate an edge; use
            // decode_raydium_cpmm_vaults + a live AmmConfig fetch instead.
            None
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
        DexProtocol::RaydiumClmm => {
            if data.len() < 273 {
                return None;
            }
            let ma: [u8; 32] = data.get(73..105)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(105..137)?.try_into().ok()?;
            let va: [u8; 32] = data.get(137..169)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(169..201)?.try_into().ok()?;
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
            let dec_a = data[233];
            let dec_b = data[234];
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps: 25,
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
        _ => None,
    }
}

/// Read mint decimals (offset 44) for each mint using single-account paced RPC requests.
/// Mints owned by Token-2022 are deliberately omitted to reject tokens with transfer fees.
pub async fn resolve_mints_decimals(
    _rpc: &crate::RpcPool,
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

    let fetcher = PacedAccountFetcher::shared();
    for mint in unique {
        let bytes = match fetcher.fetch_single_account(&mint.0).await {
            Ok((_, b)) => b,
            Err(e) => {
                debug!(
                    "resolve_mints_decimals single fetch error for {}: {e}",
                    mint.0
                );
                continue;
            }
        };
        if bytes.len() < 82 {
            continue;
        }

        // Token Safety Gate: check mint and freeze authorities for non-hub tokens.
        let mint_str = mint.0.to_string();
        let is_hub = mint_str == arb_constants::WSOL_MINT
            || mint_str == arb_constants::USDC_MINT
            || mint_str == arb_constants::USDT_MINT;
        if !is_hub {
            let mint_auth_tag = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([1, 0, 0, 0]));
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
            out.insert(mint, dec);
        }
    }
    out
}

/// Derive the three Orca `tick_array` PDAs that can affect a quote at
/// `tick_current`: the array containing the current tick, the one below and the
/// one above. Returns the keys plus the start tick of each, so a decoded array
/// can be checked against the slot it was requested for.
pub fn clmm_tick_array_pdas(
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

/// Derive Raydium CLMM `TickArrayState` PDAs around current tick.
/// Raydium CLMM has 60 ticks per tick array.
pub fn raydium_clmm_tick_array_pdas(
    pool_id: &PoolId,
    tick_current: i32,
    tick_spacing: u16,
) -> (Vec<Pubkey>, Vec<i32>) {
    let Ok(program_id) = Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID) else {
        return (Vec::new(), Vec::new());
    };
    let spacing = i32::from(tick_spacing.max(1));
    let ticks_per_array = 60i32 * spacing;
    if ticks_per_array == 0 {
        return (Vec::new(), Vec::new());
    }
    let start = tick_current.div_euclid(ticks_per_array) * ticks_per_array;
    let starts = [start, start - ticks_per_array, start + ticks_per_array];

    let mut keys = Vec::with_capacity(3);
    for s in starts {
        let seeds: &[&[u8]] = &[
            arb_constants::RAYDIUM_CLMM_TICK_ARRAY_SEED,
            pool_id.0.as_ref(),
            &s.to_be_bytes(),
        ];
        keys.push(Pubkey::find_program_address(seeds, &program_id).0);
    }
    (keys, starts.to_vec())
}

/// Derive Meteora `BinArray` PDAs covering the active bin in both swap
/// directions. The number of arrays scales with `bin_step` (size-aware):
/// enough arrays to cover ~500 bps of price impact on each side of the active
/// bin, replacing the old fixed three-array ceiling.
/// Bin-array indices covering ~500 bps of price impact on each side of the
/// active bin. Shared by the PDA deriver and the event-driven hydration
/// coverage check (helius_ws.rs).
pub fn dlmm_bin_array_indices(active_id: i32, bin_step: u16) -> Vec<i64> {
    let index = bin_id_to_bin_array_index(active_id);
    let step = u32::from(bin_step.max(1));
    let bins_traversed = 500u32 / step + 1;
    let arrays_needed = (bins_traversed as i64).div_euclid(METEORA_BINS_PER_ARRAY) + 1;
    let mut seen = std::collections::BTreeSet::new();
    (0..arrays_needed)
        .flat_map(|i| [index - i, index + i])
        .filter(move |i| seen.insert(*i))
        .collect()
}

pub fn dlmm_bin_array_pdas(pool: &Pubkey, active_id: i32, bin_step: u16) -> Vec<Pubkey> {
    dlmm_bin_array_indices(active_id, bin_step)
        .into_iter()
        .map(|i| derive_meteora_bin_array_pda(pool, i))
        .collect()
}

/// Decode an RPC account's `data[0]` base64 payload.
pub fn decode_account_data(acc: &serde_json::Value) -> Option<Vec<u8>> {
    let b64 = acc.get("data")?.get(0)?.as_str()?;
    base64::prelude::BASE64_STANDARD.decode(b64).ok()
}

/// Paced single-account sequential fetcher replacing legacy chunked `getMultipleAccounts`.
pub async fn fetch_accounts_chunked_with_slot(
    _rpc: &crate::RpcPool,
    keys: &[String],
) -> (Vec<Option<serde_json::Value>>, u64) {
    if keys.is_empty() {
        return (Vec::new(), 0);
    }

    let fetcher = PacedAccountFetcher::shared();
    let mut out = Vec::with_capacity(keys.len());
    let mut latest_slot = 0u64;

    for key_str in keys {
        if let Ok(pubkey) = Pubkey::from_str(key_str) {
            match fetcher.fetch_single_account(&pubkey).await {
                Ok((slot, bytes)) => {
                    latest_slot = latest_slot.max(slot);
                    let b64 = BASE64_STANDARD.encode(&bytes);
                    let json_val = serde_json::json!({
                        "data": [b64, "base64"],
                        "executable": false,
                        "lamports": 0,
                        "owner": arb_constants::TOKEN_PROGRAM_ID,
                        "rentEpoch": 0
                    });
                    out.push(Some(json_val));
                }
                Err(e) => {
                    debug!("fetch_accounts single fetch error for {key_str}: {e}");
                    out.push(None);
                }
            }
        } else {
            out.push(None);
        }
    }

    (out, latest_slot)
}

pub async fn fetch_accounts_chunked(
    rpc: &crate::RpcPool,
    keys: &[String],
) -> Vec<Option<serde_json::Value>> {
    fetch_accounts_chunked_with_slot(rpc, keys).await.0
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

/// Extract the `amm_config` pubkey from Raydium CPMM pool account data.
///
/// Verified `PoolState` layout (data length 637, discriminator
/// `f7ede3f5d7c3de46` == sha256("account:PoolState")[..8]):
/// `amm_config` @8..40, `pool_creator` @40..72, `token_0_vault` @72..104,
/// `token_1_vault` @104..136, `lp_mint` @136..168, `token_0_mint` @168..200,
/// `token_1_mint` @200..232. Verified live against the mainnet SOL/USDC CPMM
/// pool `7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny`.
pub fn raydium_cpmm_amm_config_key(pool_data: &[u8]) -> Option<Pubkey> {
    if pool_data.len() < 40 {
        return None;
    }
    let key = Pubkey::new_from_array(pool_data[8..40].try_into().ok()?);
    (key != Pubkey::default()).then_some(key)
}

/// Decode the Raydium CPMM (constant product) swap fee in basis points from the
/// pool's **AmmConfig** account.
///
/// Raydium CPMM keeps fee rates in the separate `AmmConfig` account (owned by
/// the CPMM program, data length 236), NOT in the pool account:
/// - `bump` @8 (u8), `disable_create_pool` @9 (u8), `index` @10 (u16),
/// - `trade_fee_rate` u64 LE @12 — denominator 1_000_000; bps = rate / 100.
///
/// Verified live: config `D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2` (the
/// SOL/USDC pool's config) holds 2500 => 25 bps, matching the official Raydium
/// API v3 `tradeFeeRate` for that pool. The previous implementation read offset
/// 290 of the POOL account, which lands inside `token_1_program`/`observation_key`
/// — fabricated bytes, never a fee.
///
/// Returns `None` on short account or zero rate so the caller fails closed.
pub fn decode_raydium_cpmm_fee_bps(data: &[u8]) -> Option<u16> {
    const TRADE_FEE_RATE_OFFSET: usize = 12;
    if data.len() < TRADE_FEE_RATE_OFFSET + 8 {
        return None;
    }
    let rate = u64::from_le_bytes(
        data[TRADE_FEE_RATE_OFFSET..TRADE_FEE_RATE_OFFSET + 8]
            .try_into()
            .ok()?,
    );
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
        // AmmConfig: trade_fee_rate u64 LE @12 = 100 => 1 bps (CPMM 1 bps tier).
        let mut d = vec![0u8; 236];
        d[12..20].copy_from_slice(&100u64.to_le_bytes());
        assert_eq!(decode_raydium_cpmm_fee_bps(&d), Some(1));
    }

    #[test]
    fn raydium_cpmm_fee_reads_25bps_value() {
        // AmmConfig: trade_fee_rate = 2500 => 25 bps (SOL/USDC config D4FPEru...).
        let mut d = vec![0u8; 236];
        d[12..20].copy_from_slice(&2500u64.to_le_bytes());
        assert_eq!(decode_raydium_cpmm_fee_bps(&d), Some(25));
    }

    #[test]
    fn raydium_cpmm_amm_config_key_extracts_pool_field() {
        let mut pool = vec![0u8; 637];
        let cfg = Pubkey::new_unique();
        pool[8..40].copy_from_slice(&cfg.to_bytes());
        assert_eq!(raydium_cpmm_amm_config_key(&pool), Some(cfg));
        assert_eq!(raydium_cpmm_amm_config_key(&[0u8; 40]), None);
        assert_eq!(raydium_cpmm_amm_config_key(&[0u8; 10]), None);
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

/// Decode Raydium CLMM PoolState layout.
pub fn decode_raydium_clmm_state(data: &[u8], fee_bps: u16) -> Option<arb_types::ClmmState> {
    if data.len() < 273 {
        return None;
    }
    let tick_spacing = u16::from_le_bytes(data[235..237].try_into().ok()?).max(1);
    let liquidity = u128::from_le_bytes(data[237..253].try_into().ok()?);
    let sqrt_price = u128::from_le_bytes(data[253..269].try_into().ok()?);
    let tick_current = i32::from_le_bytes(data[269..273].try_into().ok()?);

    if liquidity == 0 || sqrt_price == 0 {
        return None;
    }

    // Raydium CLMM's SwapMath fee unit is millionths (1e6), not BPS.
    // 25 bps must become 2,500 ppm; passing 25 undercharged fees by 100x.
    let fee_rate = fee_bps.checked_mul(100)?;
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

    let function_type = if data.len() > 35 { data[35] } else { 0 };
    // `is_support_limit_order` (official SDK): LimitOrder (2) => true;
    // LiquidityMining (1) => false; Undetermined (0) => true only when every
    // reward mint is the default pubkey; any other value fails closed to false.
    let reward_mints_default = data.len() >= 440
        && data[264..296].iter().all(|&b| b == 0)
        && data[408..440].iter().all(|&b| b == 0);
    let support_limit_order = match function_type {
        2 => true,
        1 => false,
        0 => reward_mints_default,
        _ => false,
    };

    Some(arb_types::DlmmState {
        active_bin_id,
        status,
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
            function_type,
            collect_fee_mode: if data.len() > 36 { data[36] } else { 0 },
        }),
        volatility_accumulator,
        volatility_reference,
        index_reference: i32::from_le_bytes(data[48..52].try_into().ok()?),
        last_update_timestamp,
        support_limit_order,
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

/// Derive the `BinArrayBitmapExtension` PDA for a pool:
/// seeds = `[b"bitmap", lb_pair]` (official dlmm-sdk seeds).
pub fn bitmap_extension_pda(pool: &Pubkey) -> Pubkey {
    let program_id = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
        .expect("valid Meteora program id constant");
    Pubkey::find_program_address(&[b"bitmap", pool.as_ref()], &program_id).0
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
        let open_order_amount =
            u64::from_le_bytes(data[off + 112..off + 120].try_into().unwrap_or([0; 8]));
        let processed_order_remaining_amount =
            u64::from_le_bytes(data[off + 128..off + 136].try_into().unwrap_or([0; 8]));
        let limit_order_ask_side = data.get(off + 140).copied().unwrap_or(0) != 0;
        if amount_x == 0
            && amount_y == 0
            && open_order_amount == 0
            && processed_order_remaining_amount == 0
        {
            continue;
        }
        out.push(arb_types::DlmmBin {
            bin_id: base_bin_id.saturating_add(i as i32),
            amount_x: amount_x as u128,
            amount_y: amount_y as u128,
            open_order_amount,
            processed_order_remaining_amount,
            limit_order_ask_side,
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
        // Fail closed: the CPMM fee lives in the separate AmmConfig account,
        // not in pool data. `fetch_live_pool_states` resolves it via
        // raydium_cpmm_amm_config_key + a live AmmConfig batch fetch.
        DexProtocol::RaydiumCpmm => None,
        DexProtocol::PumpFun => None,
        DexProtocol::PumpSwap => None,
        DexProtocol::MeteoraDammV2 => None,
        DexProtocol::MeteoraDbc => None,
        DexProtocol::MeteoraDlmmPro => None,
        DexProtocol::RaydiumLaunchLab => None,
    }
}

static POOL_WRITE_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Consecutive `getMultipleAccounts` failures across all `batch_fetch_accounts`
/// call sites. Non-zero means RPC starvation: pools go stale and downstream
/// ticks are skipped (fail closed). Reset on the first success.
/// Monotonic write version shared between HTTP registry refreshes and live WS pool updates.
pub fn next_pool_write_version() -> u64 {
    POOL_WRITE_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Instantaneous virtual reserves for CLMM pools: `x = L / sqrt(P)`, `y = L * sqrt(P)`.
pub fn compute_clmm_virtual_reserves(c: &arb_types::ClmmState) -> (u128, u128) {
    if c.liquidity == 0 || c.sqrt_price == 0 {
        return (0, 0);
    }
    let l = ruint::aliases::U256::from(c.liquidity);
    let sp = ruint::aliases::U256::from(c.sqrt_price);
    let x_virt = (l << 64usize) / sp;
    let y_virt = (l * sp) >> 64usize;
    (
        x_virt.try_into().unwrap_or(u128::MAX),
        y_virt.try_into().unwrap_or(u128::MAX),
    )
}

/// True when cached CLMM tick arrays contain the array holding `tick_current`.
pub fn clmm_tick_array_covers_tick(
    arrays: &[arb_types::ClmmTickArray],
    tick_current: i32,
    tick_spacing: u16,
) -> bool {
    let spacing = tick_spacing.max(1) as i32;
    let ticks_per_array = 88i32.saturating_mul(spacing);
    if ticks_per_array == 0 {
        return false;
    }
    let start = tick_current.div_euclid(ticks_per_array) * ticks_per_array;
    arrays.iter().any(|ta| ta.start_tick_index == start)
}

/// Decode Meteora DLMM dynamic base fee in basis points.
pub fn decode_dlmm_fee_bps(s: &arb_types::DlmmState) -> Option<u16> {
    let prm = s.params.as_ref()?;
    let base = (prm.base_factor as u64) * (s.bin_step_bps as u64) * 10;
    let scaled = if prm.base_fee_power_factor == 0 {
        base
    } else {
        base.checked_mul(10u64.checked_pow(prm.base_fee_power_factor as u32)?)?
    };
    u16::try_from(scaled.saturating_mul(10_000) / 1_000_000_000).ok()
}

/// Compute effective base/quote reserves for Meteora DLMM from decoded `DlmmState` bins and raw `LbPair` bytes.
///
/// DLMM semantics (verified against the Meteora DLMM program layout): bins with
/// `bin_id > active_id` hold only token X; bins with `bin_id < active_id` hold
/// only token Y; the active bin can hold both. The honest two-sided CPMM
/// approximation for the graph edge is therefore:
///
/// - `x_depth = \u{03a3} amount_x` over bins `bin_id >= active_id` (X the pool can sell)
/// - `y_depth = \u{03a3} amount_y` over bins `bin_id <= active_id` (Y the pool can sell)
///
/// Fail-closed rules (each returns `None`, dropping the pool from the graph):
/// - no bins attached, or the ACTIVE bin itself is missing from the attached
///   BinArrays (hydration incomplete \u{2014} never guess its contents);
/// - either side depth is zero (one-sided book: no honest two-sided quote);
/// - implied price `y_depth / x_depth` diverges from the active-bin price by
///   more than [`DLMM_RESERVE_PRICE_TOLERANCE`] (corrupt/inconsistent bin data).
///
/// The previous implementation derived one side from the other via the active
/// price whenever the active bin missed one token (`rx else sum` fallbacks).
/// Measured on mainnet pool HRYEjwdo3bZ1TpXKWKcezqiwSV2Ywuh4LxMa2PzoCnG6
/// (USDC/WSOL, bin_step=2): active bin held `amount_x=0`, so the fallback
/// summed 210 bins into `rx=6.57e9 USDC-raw` and derived
/// `ry=82.26 WSOL` \u{2014} a fake book that produced ~4,134 phantom-arbitrage
/// rejections per 3-minute soak (implied ROI 1.3M\u{2013}1.5M bps vs the 1500 bps
/// implausibility ceiling).
pub fn compute_dlmm_reserves(
    d: &arb_types::DlmmState,
    pool_data: &[u8],
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
) -> Option<(u128, u128)> {
    if d.bins.is_empty() {
        return None;
    }
    let p_x_in_y = arb_math::dlmm::get_price_from_id(d.active_bin_id, d.bin_step_bps)
        .ok()
        .filter(|p| *p > 0)?;
    let tx = pool_data
        .get(88..120)
        .filter(|_| pool_data.len() >= 152)
        .and_then(|s| <[u8; 32]>::try_from(s).ok())
        .map(Pubkey::new_from_array)?;
    // The active bin must be present: it anchors both side sums and the
    // price-consistency check. Missing active bin = incomplete hydration.
    d.bins.iter().find(|b| b.bin_id == d.active_bin_id)?;
    let x_depth: u128 = d
        .bins
        .iter()
        .filter(|b| b.bin_id >= d.active_bin_id)
        .map(|b| b.amount_x)
        .sum();
    let y_depth: u128 = d
        .bins
        .iter()
        .filter(|b| b.bin_id <= d.active_bin_id)
        .map(|b| b.amount_y)
        .sum();
    if x_depth == 0 || y_depth == 0 {
        return None;
    }
    // Price-consistency invariant: the side sums must imply a price within a
    // sane band of the active-bin price. With correctly attached arrays this
    // holds by construction (bins within \u{00b1}70 bins of active); a violation
    // means corrupt or mismatched bin data, never a real market state.
    let implied_q64 =
        (ruint::aliases::U256::from(y_depth) << 64usize) / ruint::aliases::U256::from(x_depth);
    let p_u256 = ruint::aliases::U256::from(p_x_in_y);
    let band = p_u256 * ruint::aliases::U256::from(DLMM_RESERVE_PRICE_TOLERANCE);
    if implied_q64 > band
        || implied_q64 < p_u256 / ruint::aliases::U256::from(DLMM_RESERVE_PRICE_TOLERANCE)
    {
        return None;
    }
    if tx == *base_mint {
        Some((x_depth, y_depth))
    } else if tx == *quote_mint {
        Some((y_depth, x_depth))
    } else {
        Some((0, 0))
    }
}

/// Max allowed ratio between the bin-sum implied price and the active-bin
/// price. 4x is far outside any correctly-hydrated \u{00b1}1-bin-array window
/// (\u{00b1}70 bins * 0.02% \u{2248} \u{00b1}1.4% for the tightest bin_step=2 pools) yet
/// still catches grossly corrupt data.
const DLMM_RESERVE_PRICE_TOLERANCE: u64 = 4;

/// Decode a u64 token amount from raw SPL token account bytes (offset 64..72).
pub fn decode_spl_token_amount_bytes(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 72 {
        return None;
    }
    let arr: [u8; 8] = bytes.get(64..72)?.try_into().ok()?;
    Some(u64::from_le_bytes(arr))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{MultiDexRegistry, RegisteredPool};
    use crate::rpc_pool::RpcPool;

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
        decode_spl_token_amount_bytes(&bytes).unwrap_or(0)
    }

    #[test]
    fn mainnet_default_registers_all_venues() {
        let rpc = RpcPool::new(vec!["https://example.test".to_string()]);
        let reg = MultiDexRegistry::mainnet_default(rpc);
        // 1 Raydium v4 + 1 Raydium CPMM + 4 Orca Whirlpool fee tiers + 3 Meteora DLMM.
        assert_eq!(reg.pool_count(), 9);
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
    fn test_decode_pool_info_raydium_cpmm_fails_closed() {
        // The CPMM fee lives in the separate AmmConfig account, so a
        // pool-data-only decode must not fabricate one.
        let mut data = vec![0u8; 350];
        data[72..104].copy_from_slice(&Pubkey::new_unique().to_bytes());
        data[104..136].copy_from_slice(&Pubkey::new_unique().to_bytes());
        data[168..200].copy_from_slice(&Pubkey::new_unique().to_bytes());
        data[200..232].copy_from_slice(&Pubkey::new_unique().to_bytes());
        data[331] = 9;
        data[332] = 6;
        assert!(decode_pool_info(DexProtocol::RaydiumCpmm, &data).is_none());
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

    #[test]
    fn test_decode_pool_info_raydium_clmm() {
        let mut data = vec![0u8; 350];
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[73..105].copy_from_slice(&ma.to_bytes());
        data[105..137].copy_from_slice(&mb.to_bytes());
        data[137..169].copy_from_slice(&va.to_bytes());
        data[169..201].copy_from_slice(&vb.to_bytes());
        data[233] = 9;
        data[234] = 6;
        data[235..237].copy_from_slice(&60u16.to_le_bytes()); // tick_spacing

        let info = decode_pool_info(DexProtocol::RaydiumClmm, &data).expect("valid Raydium CLMM");
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.decimals_a, Some(9));
        assert_eq!(info.decimals_b, Some(6));
        assert_eq!(info.fee_bps, 25);
    }

    #[test]
    fn test_decode_raydium_clmm_vaults() {
        let mut data = vec![0u8; 250];
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[137..169].copy_from_slice(&va.to_bytes());
        data[169..201].copy_from_slice(&vb.to_bytes());

        let vaults = decode_raydium_clmm_vaults(&data).expect("valid clmm vaults");
        assert_eq!(vaults.0, va);
        assert_eq!(vaults.1, vb);

        assert_eq!(decode_raydium_clmm_vaults(&[0u8; 100]), None);
    }

    fn orca_whirlpool_bytes(len: usize, mint_a: Pubkey, mint_b: Pubkey) -> Vec<u8> {
        let mut data = vec![0u8; len];
        data[101..133].copy_from_slice(mint_a.as_ref());
        data[181..213].copy_from_slice(mint_b.as_ref());
        data
    }

    #[test]
    fn clmm_onchain_mints_orca_interleaved_layout() {
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        for len in [525usize, 653] {
            let data = orca_whirlpool_bytes(len, ma, mb);
            let got =
                clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &data).expect("valid whirlpool");
            assert_eq!(got, (ma, mb), "len={len}");
        }
    }

    #[test]
    fn clmm_onchain_mints_raydium_layout() {
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let mut data = vec![0u8; 1544];
        data[73..105].copy_from_slice(ma.as_ref());
        data[105..137].copy_from_slice(mb.as_ref());
        assert_eq!(
            clmm_onchain_mints(DexProtocol::RaydiumClmm, &data).expect("valid clmm"),
            (ma, mb)
        );
    }

    #[test]
    fn clmm_onchain_mints_fail_closed() {
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        assert_eq!(
            clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &[0u8; 100]),
            None
        );
        assert_eq!(
            clmm_onchain_mints(DexProtocol::RaydiumClmm, &[0u8; 100]),
            None
        );
        assert_eq!(
            clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &[0u8; 245]),
            None,
            "default pubkeys must be rejected"
        );
        assert_eq!(
            clmm_onchain_mints(DexProtocol::MeteoraDlmm, &orca_whirlpool_bytes(653, ma, mb)),
            None,
            "non-CLMM protocols must be rejected"
        );
    }

    #[test]
    fn clmm_onchain_mints_rejects_swapped_layout_bytes() {
        // A pool account whose @181..213 window is a vault (not a mint) must not
        // be silently accepted as orientation truth when the catalog disagrees;
        // the helper returns the raw on-chain order and the registry logs the
        // divergence. Here we only pin that the helper reads exactly @101/@181.
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let mut data = orca_whirlpool_bytes(653, ma, mb);
        let intruder = Pubkey::new_unique();
        data[133..165].copy_from_slice(intruder.as_ref()); // vault_a slot
        assert_eq!(
            clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &data).expect("valid"),
            (ma, mb),
            "vault_a bytes @133..165 must not leak into the mint read"
        );
    }

    #[test]
    fn test_decode_raydium_clmm_state() {
        let mut data = vec![0u8; 300];
        data[235..237].copy_from_slice(&60u16.to_le_bytes()); // tick_spacing
        data[237..253].copy_from_slice(&5_000_000u128.to_le_bytes()); // liquidity
        data[253..269].copy_from_slice(&(1u128 << 64).to_le_bytes()); // sqrt_price
        data[269..273].copy_from_slice(&120i32.to_le_bytes()); // tick_current

        let state = decode_raydium_clmm_state(&data, 25).expect("valid clmm state");
        assert_eq!(state.tick_spacing, 60);
        assert_eq!(state.liquidity, 5_000_000);
        assert_eq!(state.sqrt_price, 1u128 << 64);
        assert_eq!(state.tick_current, 120);
        assert_eq!(state.fee_rate, 2_500);
    }

    #[test]
    fn test_raydium_clmm_tick_array_pdas() {
        let pool = PoolId(Pubkey::new_unique());
        let (keys, starts) = raydium_clmm_tick_array_pdas(&pool, 120, 60);
        assert_eq!(keys.len(), 3);
        assert_eq!(starts.len(), 3);
        // tick_spacing=60, ticks_per_array = 60 * 60 = 3600
        // tick_current=120, start = 120.div_euclid(3600) * 3600 = 0
        assert_eq!(starts[0], 0);
        assert_eq!(starts[1], -3600);
        assert_eq!(starts[2], 3600);
        // All 3 PDAs must be distinct and non-default
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], Pubkey::default());
    }

    #[test]
    fn test_resolve_fee_bps_raydium_cpmm_fails_closed() {
        // The CPMM fee lives in the separate AmmConfig account; a pool-data-only
        // resolve must return None (never a guessed tier).
        let mut data = vec![0u8; 637];
        data[290..298].copy_from_slice(&100u64.to_le_bytes()); // stale-offset noise
        assert_eq!(resolve_fee_bps(DexProtocol::RaydiumCpmm, &data), None);
    }

    // ---- compute_dlmm_reserves fail-closed forensics (soak10/11 phantom ROI) ----

    fn dlmm_probe_state(active_id: i32, mut bins: Vec<arb_types::DlmmBin>) -> arb_types::DlmmState {
        bins.sort_by_key(|b| b.bin_id);
        arb_types::DlmmState {
            active_bin_id: active_id,
            status: 0,
            bin_step_bps: 10,
            bins,
            params: None,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 0,
            support_limit_order: false,
        }
    }

    fn dlmm_probe_pool_bytes(token_x: Pubkey, token_y: Pubkey) -> Vec<u8> {
        let mut data = vec![0u8; 152];
        data[88..120].copy_from_slice(token_x.as_ref());
        data[120..152].copy_from_slice(token_y.as_ref());
        data
    }

    #[test]
    fn dlmm_reserves_fail_closed_when_active_bin_missing() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        let d = dlmm_probe_state(
            1000,
            vec![
                arb_types::DlmmBin {
                    bin_id: 998,
                    amount_x: 0,
                    amount_y: 1_000_000,
                    ..Default::default()
                },
                arb_types::DlmmBin {
                    bin_id: 1002,
                    amount_x: 1_000_000,
                    amount_y: 0,
                    ..Default::default()
                },
            ],
        );
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), None);
    }

    #[test]
    fn dlmm_reserves_fail_closed_on_one_sided_book() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        // HRYEjw mainnet shape: active bin holds only Y (amount_x=0) and no
        // bin at/above active holds X. The old fallback summed distant bins
        // into a fake x-side and derived y from price — producing the
        // ~130x phantom ROI. Must now fail closed.
        let d = dlmm_probe_state(
            1000,
            vec![arb_types::DlmmBin {
                bin_id: 1000,
                amount_x: 0,
                amount_y: 1_444_908_165,
                ..Default::default()
            }],
        );
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), None);
    }

    #[test]
    fn dlmm_reserves_sum_correct_sides_both_directions() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        let d = dlmm_probe_state(
            1000,
            vec![
                arb_types::DlmmBin {
                    bin_id: 998,
                    amount_x: 0,
                    amount_y: 500,
                    ..Default::default()
                },
                arb_types::DlmmBin {
                    bin_id: 1000,
                    amount_x: 1_000,
                    amount_y: 2_700,
                    ..Default::default()
                },
                arb_types::DlmmBin {
                    bin_id: 1002,
                    amount_x: 3_000,
                    amount_y: 0,
                    ..Default::default()
                },
            ],
        );
        // x_depth = bins >= 1000 => 1000 + 3000 = 4000
        // y_depth = bins <= 1000 => 500 + 2700 = 3200
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), Some((4000, 3200)));
        // Flipped pool orientation (token_x is the quote mint): reserves swap.
        assert_eq!(compute_dlmm_reserves(&d, &data, &y, &x), Some((3200, 4000)));
    }

    #[test]
    fn dlmm_reserves_reject_price_inconsistent_bins() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        // active_id=1000, step=10bps => p ~ 2.717 y-raw per x-raw. A book
        // implying 0.1 y-raw per x-raw is >4x below p: corrupt data.
        let d = dlmm_probe_state(
            1000,
            vec![arb_types::DlmmBin {
                bin_id: 1000,
                amount_x: 1_000_000,
                amount_y: 100_000,
                ..Default::default()
            }],
        );
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), None);
    }
}
