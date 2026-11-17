//! Real-mainnet, non-broadcast route simulation.
//!
//! This module is the P0 "real-mainnet non-broadcast simulation" primitive. It
//! submits an already-serialized transaction to the Solana JSON-RPC
//! `simulateTransaction` method with:
//!
//! * `replaceRecentBlockhash: true` — the RPC substitutes a fresh blockhash, so
//!   the payload never needs to be a broadcast-valid transaction;
//! * `sigVerify: false` — no signature is checked, so nothing has to be signed
//!   to obtain evidence;
//! * `innerInstructions: true` and an explicit `accounts` list — the response
//!   carries the full execution evidence (logs, inner instructions, CU,
//!   loaded-account-data size, pre/post lamports and token balances).
//!
//! It never signs, never sends, and never broadcasts. There is no code path in
//! this module that can submit a transaction to the network.
//!
//! Evidence captured here is a *simulation* result against the RPC node's
//! current bank. It is not proof of inclusion and it is not realized profit.
//! Phase 4 (the full profitability gate) builds on this type; this module alone
//! does not close Phase 4.

use arb_ingest::RpcPool;
use arb_types::{ArbError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, warn};

/// Default timeout for a single non-broadcast simulation round trip.
pub const DEFAULT_FORK_SIM_TIMEOUT_MS: u64 = 2500;

/// Optional pre-fetched account state for a simulation.
///
/// The current implementation executes against the RPC node's live bank; the
/// maps are retained so a future offline fork (LiteSVM / bank fork) can be fed
/// the same snapshot without changing the caller contract. They are never used
/// to bypass the live RPC: an empty state is the normal case.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkState {
    /// Slot the snapshots were observed at, if pinned.
    pub context_slot: Option<u64>,
    /// Program account data, keyed by base58 pubkey.
    pub program_accounts: HashMap<String, Vec<u8>>,
    /// Pool / user account data, keyed by base58 pubkey.
    pub account_data: HashMap<String, Vec<u8>>,
    /// Sysvar account data, keyed by base58 pubkey.
    pub sysvars: HashMap<String, Vec<u8>>,
}

impl ForkState {
    pub fn is_empty(&self) -> bool {
        self.context_slot.is_none()
            && self.program_accounts.is_empty()
            && self.account_data.is_empty()
            && self.sysvars.is_empty()
    }
}

/// A single account snapshot returned by `simulateTransaction`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimAccountSnapshot {
    pub pubkey: String,
    pub lamports: u64,
    pub owner: String,
    /// Base64 account data. `None` when the RPC omitted the account.
    pub data_base64: Option<String>,
    pub executable: bool,
}

/// A token balance entry from the simulation response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimTokenBalance {
    pub account_index: u8,
    pub mint: String,
    pub owner: Option<String>,
    pub amount: String,
    pub decimals: u8,
}

/// Full evidence from one non-broadcast route simulation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkSimResult {
    /// True only when the RPC executed the transaction without an error.
    pub success: bool,
    /// Bank slot the simulation ran against (`context.slot`).
    pub context_slot: Option<u64>,
    /// Blockhash the RPC substituted, if any.
    pub replacement_blockhash: Option<String>,
    /// Program error, if the transaction failed.
    pub error: Option<String>,
    pub units_consumed: u64,
    /// `loadedAccountsDataSize` when the RPC reports it.
    pub loaded_accounts_data_size: Option<u64>,
    pub logs: Vec<String>,
    pub inner_instruction_count: usize,
    pub pre_balances: Vec<u64>,
    pub post_balances: Vec<u64>,
    /// Net lamport change of the fee payer (index 0 of `pre/post_balances`).
    pub fee_payer_lamport_delta: i128,
    pub pre_token_balances: Vec<SimTokenBalance>,
    pub post_token_balances: Vec<SimTokenBalance>,
    /// Realized net in raw WSOL units when a watch account was supplied and the
    /// mint is WSOL. `None` means "not computed", never "zero".
    pub realized_net_lamports: Option<i64>,
    /// Expected net supplied by the caller, echoed for audit.
    pub expected_net_lamports: u64,
    /// Accounts the RPC echoed back, in request order.
    pub accounts: Vec<SimAccountSnapshot>,
    pub return_data: Option<String>,
}

impl ForkSimResult {
    /// Net token delta (raw units) for a watch account between pre and post.
    pub fn token_delta(&self, account_index: u8) -> Option<i128> {
        let pre = self
            .pre_token_balances
            .iter()
            .find(|b| b.account_index == account_index)
            .and_then(|b| b.amount.parse::<i128>().ok())
            .unwrap_or(0);
        let post = self
            .post_token_balances
            .iter()
            .find(|b| b.account_index == account_index)
            .and_then(|b| b.amount.parse::<i128>().ok())
            .unwrap_or(0);
        if self.pre_token_balances.is_empty() && self.post_token_balances.is_empty() {
            return None;
        }
        Some(post - pre)
    }
}

/// Non-broadcast simulator backed by a Solana JSON-RPC endpoint.
#[derive(Clone)]
pub struct ForkSimulator {
    rpc_url: Option<String>,
    client: reqwest::Client,
    state: ForkState,
    /// Rotating, cooldown-aware RPC pool. When present it takes precedence
    /// over the pinned `rpc_url` so a single rate-limited endpoint cannot
    /// stall simulation (a 429 HTML body previously surfaced as a JSON parse
    /// error instead of a rotation).
    rpc_pool: Option<Arc<RpcPool>>,
}

impl std::fmt::Debug for ForkSimulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkSimulator")
            .field(
                "rpc_url",
                &self.rpc_url.as_ref().map(|u| RpcPool::redact_url(u)),
            )
            .field(
                "rpc_pool",
                &self.rpc_pool.as_ref().map(|p| p.endpoints_count()),
            )
            .field("state", &self.state)
            .finish()
    }
}

impl ForkSimulator {
    /// Construct from an explicit state and optional RPC URL.
    ///
    /// With `rpc_url = None` the simulator fails closed on every call: a
    /// simulation that cannot run must never be reported as a success.
    pub fn new(state: ForkState, rpc_url: Option<String>) -> Self {
        let timeout_ms = std::env::var("ARB_SIM_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_FORK_SIM_TIMEOUT_MS);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .tcp_nodelay(true)
            .pool_max_idle_per_host(8)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            rpc_url: rpc_url.filter(|u| !u.trim().is_empty()),
            client,
            state,
            rpc_pool: None,
        }
    }

    /// Construct backed by the shared rotating RPC pool (preferred).
    pub fn with_rpc_pool(pool: Arc<RpcPool>) -> Self {
        Self {
            rpc_url: None,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(
                    std::env::var("ARB_SIM_TIMEOUT_MS")
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                        .filter(|v| *v > 0)
                        .unwrap_or(DEFAULT_FORK_SIM_TIMEOUT_MS),
                ))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            state: ForkState::default(),
            rpc_pool: Some(pool),
        }
    }

    /// Construct with the same endpoint resolution order the supervisor uses
    /// (`RpcPool::from_env`): `SOLANA_RPC_PRIMARY` first, then `FAST_ENDPOINT`,
    /// then the legacy single-endpoint variables.
    pub fn from_env() -> Self {
        let url = std::env::var("SOLANA_RPC_PRIMARY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                std::env::var("SOLANA_RPC_URL")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("FAST_ENDPOINT")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("HELIUS_RPC_URL")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            });
        let mut sim = Self::new(ForkState::default(), url);
        sim.rpc_pool = Some(Arc::new(RpcPool::from_env()));
        sim
    }

    pub fn with_rpc_url(url: String) -> Self {
        Self::new(ForkState::default(), Some(url))
    }

    pub fn state(&self) -> &ForkState {
        &self.state
    }

    pub fn has_endpoint(&self) -> bool {
        self.rpc_url.is_some()
    }

    /// Simulate a serialized transaction without broadcasting it.
    ///
    /// `watch_accounts` are requested from the RPC so their pre/post token
    /// balances are available. When `expected_net_lamports > 0` and the first
    /// watch account carries WSOL, the realized delta is compared to the
    /// expectation and a shortfall turns the result into a failure (fail
    /// closed). With no watch account the profit comparison is skipped and
    /// `realized_net_lamports` stays `None` rather than pretending to be zero.
    pub async fn simulate_route(
        &self,
        tx_bytes: &[u8],
        watch_accounts: &[Pubkey],
        expected_net_lamports: u64,
    ) -> Result<ForkSimResult> {
        if tx_bytes.is_empty() {
            return Err(ArbError::InvalidInput(
                "fork simulation received an empty transaction".into(),
            ));
        }

        let b64 = base64_encode(tx_bytes);
        let addresses: Vec<String> = watch_accounts.iter().map(ToString::to_string).collect();
        let params = json!([
            b64,
            {
                "encoding": "base64",
                "commitment": "processed",
                "replaceRecentBlockhash": true,
                "sigVerify": false,
                "innerInstructions": true,
                "maxSupportedTransactionVersion": 1,
                "accounts": {
                    "encoding": "base64",
                    "addresses": addresses
                }
            }
        ]);

        let result: Value = if let Some(pool) =
            self.rpc_pool.as_ref().filter(|p| p.endpoints_count() > 0)
        {
            pool.call_simulation_priority("simulateTransaction", params)
                .await
                .map_err(|e| ArbError::Internal(format!("fork simulation pool call error: {e}")))?
        } else {
            let rpc_url = self.rpc_url.as_ref().ok_or_else(|| {
                ArbError::Internal(
                    "fork simulation requested but no RPC endpoint is configured (fail closed)"
                        .into(),
                )
            })?;
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "simulateTransaction",
                "params": params,
            });

            let resp = self
                .client
                .post(rpc_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    ArbError::Internal(format!("fork simulation RPC connection error: {e}"))
                })?;

            let val: Value = resp.json().await.map_err(|e| {
                ArbError::Internal(format!("fork simulation JSON parse error: {e}"))
            })?;

            if let Some(err) = val.get("error") {
                return Err(ArbError::SimulationFailed(format!(
                    "RPC rejected simulateTransaction: {err}"
                )));
            }

            val.get("result")
                .cloned()
                .ok_or_else(|| ArbError::SimulationFailed("missing result".into()))?
        };

        let value = result
            .get("value")
            .ok_or_else(|| ArbError::SimulationFailed("missing result.value".into()))?;

        let context_slot = result
            .get("context")
            .and_then(|c| c.get("slot"))
            .and_then(Value::as_u64);

        let error = value.get("err").and_then(|e| {
            if e.is_null() {
                None
            } else {
                Some(e.to_string())
            }
        });
        let units_consumed = value
            .get("unitsConsumed")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let loaded_accounts_data_size = value.get("loadedAccountsDataSize").and_then(Value::as_u64);
        let logs = value
            .get("logs")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let inner_instruction_count = value
            .get("innerInstructions")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|group| {
                        group
                            .get("instructions")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len)
                    })
                    .sum()
            })
            .unwrap_or(0);
        let replacement_blockhash = value
            .get("replacementBlockhash")
            .and_then(|b| b.get("blockhash"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let return_data = value
            .get("returnData")
            .and_then(|d| d.get("data"))
            .and_then(|d| d.get(0))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        let pre_balances = parse_u64_array(value.get("preBalances"));
        let post_balances = parse_u64_array(value.get("postBalances"));
        let fee_payer_lamport_delta = match (pre_balances.first(), post_balances.first()) {
            (Some(pre), Some(post)) => i128::from(*post) - i128::from(*pre),
            _ => 0,
        };

        let pre_token_balances = parse_token_balances(value.get("preTokenBalances"));
        let post_token_balances = parse_token_balances(value.get("postTokenBalances"));

        let accounts = value
            .get("accounts")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .enumerate()
                    .filter_map(|(i, acc)| {
                        if acc.is_null() {
                            return None;
                        }
                        let pubkey = watch_accounts
                            .get(i)
                            .map(ToString::to_string)
                            .unwrap_or_default();
                        Some(SimAccountSnapshot {
                            pubkey,
                            lamports: acc.get("lamports").and_then(Value::as_u64).unwrap_or(0),
                            owner: acc
                                .get("owner")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            data_base64: acc
                                .get("data")
                                .and_then(|d| d.get(0))
                                .and_then(Value::as_str)
                                .map(ToOwned::to_owned),
                            executable: acc
                                .get("executable")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut result = ForkSimResult {
            success: error.is_none(),
            context_slot,
            replacement_blockhash,
            error,
            units_consumed,
            loaded_accounts_data_size,
            logs,
            inner_instruction_count,
            pre_balances,
            post_balances,
            fee_payer_lamport_delta,
            pre_token_balances,
            post_token_balances,
            realized_net_lamports: None,
            expected_net_lamports,
            accounts,
            return_data,
        };

        // Realized net is only computable when the caller told us which account
        // receives the profit. Account index 0 is the first watch account, which
        // is also the first requested address in the response.
        if !watch_accounts.is_empty() {
            if let Some(delta) = result.token_delta(0) {
                result.realized_net_lamports = Some(i64::try_from(delta).unwrap_or(i64::MAX));
            }
        }

        if result.success
            && expected_net_lamports > 0
            && let Some(realized) = result.realized_net_lamports
            && realized < i64::try_from(expected_net_lamports).unwrap_or(i64::MAX)
        {
            result.success = false;
            result.error = Some(format!(
                "realized net {realized} below expected {expected_net_lamports} (fail closed)"
            ));
        }

        debug!(
            slot = ?result.context_slot,
            success = result.success,
            units = result.units_consumed,
            inner = result.inner_instruction_count,
            fee_payer_delta = result.fee_payer_lamport_delta,
            "non-broadcast route simulation complete"
        );
        if !result.success {
            warn!(error = ?result.error, "route simulation rejected");
        }

        Ok(result)
    }
}

/// Actual balance changes extracted from a landed transaction's
/// `getTransaction` metadata (jsonParsed encoding).
///
/// Every field is computed only from on-chain evidence; nothing is invented.
/// `realized_net_wsol` is the honest realized PnL in raw WSOL lamports:
/// the fee payer's lamport delta plus any WSOL token-balance delta owned by
/// the payer (profit may land as unwrapped lamports or wrapped WSOL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LandedBalanceChanges {
    /// `postBalances[0] - preBalances[0]` of the fee payer. Negative = fees and
    /// tips paid; positive = net lamports received.
    pub fee_payer_lamport_delta: i128,
    /// Summed raw WSOL token delta across every token account owned by the
    /// payer. `None` when the metadata carries no payer WSOL token balances.
    pub wsol_token_delta: Option<i128>,
    /// Realized net in raw WSOL lamports when any balance evidence exists:
    /// `fee_payer_lamport_delta + wsol_token_delta` (token part treated as 0
    /// when absent). `None` only when neither lamport nor token evidence exists.
    pub realized_net_wsol: Option<i128>,
}

/// Parse a landed `getTransaction` response into actual balance changes.
///
/// `meta` is the full RPC response value (the function reads its `meta`
/// object). `payer` and `wsol_mint` are base58 strings used to attribute
/// token-balance entries; pool vaults owned by other authorities are ignored.
pub fn parse_landed_balance_changes(
    meta: &Value,
    payer: &str,
    wsol_mint: &str,
) -> LandedBalanceChanges {
    let meta_obj = meta.get("meta").unwrap_or(meta);

    let pre = parse_u64_array(meta_obj.get("preBalances"));
    let post = parse_u64_array(meta_obj.get("postBalances"));
    let fee_payer_lamport_delta = match (pre.first(), post.first()) {
        (Some(p), Some(q)) => *q as i128 - *p as i128,
        _ => 0,
    };
    let has_lamport_evidence = !pre.is_empty() && !post.is_empty();

    let payer_token_delta = |side: &Value| -> HashMap<u8, i128> {
        let mut map = HashMap::new();
        if let Some(arr) = side.as_array() {
            for b in arr {
                let idx = match b.get("accountIndex").and_then(Value::as_u64) {
                    Some(i) => match u8::try_from(i) {
                        Ok(i) => i,
                        Err(_) => continue,
                    },
                    None => continue,
                };
                let mint = b.get("mint").and_then(Value::as_str).unwrap_or("");
                if mint != wsol_mint {
                    continue;
                }
                if let Some(owner) = b.get("owner").and_then(Value::as_str)
                    && owner != payer
                {
                    continue;
                }
                if let Some(amount) = b
                    .get("uiTokenAmount")
                    .and_then(|u| u.get("amount"))
                    .and_then(Value::as_str)
                    && let Ok(a) = amount.parse::<i128>()
                {
                    map.insert(idx, a);
                }
            }
        }
        map
    };

    let pre_tokens = payer_token_delta(meta_obj.get("preTokenBalances").unwrap_or(&Value::Null));
    let post_tokens = payer_token_delta(meta_obj.get("postTokenBalances").unwrap_or(&Value::Null));

    let wsol_token_delta = if pre_tokens.is_empty() && post_tokens.is_empty() {
        None
    } else {
        let mut keys: Vec<u8> = pre_tokens
            .keys()
            .chain(post_tokens.keys())
            .copied()
            .collect();
        keys.sort_unstable();
        keys.dedup();
        Some(
            keys.iter()
                .map(|k| {
                    post_tokens.get(k).copied().unwrap_or(0)
                        - pre_tokens.get(k).copied().unwrap_or(0)
                })
                .sum::<i128>(),
        )
    };

    let realized_net_wsol = if wsol_token_delta.is_some() || has_lamport_evidence {
        Some(fee_payer_lamport_delta + wsol_token_delta.unwrap_or(0))
    } else {
        None
    };

    LandedBalanceChanges {
        fee_payer_lamport_delta,
        wsol_token_delta,
        realized_net_wsol,
    }
}

fn parse_u64_array(value: Option<&Value>) -> Vec<u64> {
    value
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

fn parse_token_balances(value: Option<&Value>) -> Vec<SimTokenBalance> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    Some(SimTokenBalance {
                        account_index: u8::try_from(b.get("accountIndex")?.as_u64()?).ok()?,
                        mint: b.get("mint")?.as_str()?.to_owned(),
                        owner: b
                            .get("owner")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        amount: b.get("uiTokenAmount")?.get("amount")?.as_str()?.to_owned(),
                        decimals: u8::try_from(b.get("uiTokenAmount")?.get("decimals")?.as_u64()?)
                            .ok()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn base64_encode(input: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYER: &str = "Payer11111111111111111111111111111111111111";
    const WSOL: &str = "So11111111111111111111111111111111111111112";

    fn landed_meta(
        pre_balances: Vec<u64>,
        post_balances: Vec<u64>,
        pre_tokens: Value,
        post_tokens: Value,
    ) -> Value {
        json!({
            "meta": {
                "preBalances": pre_balances,
                "postBalances": post_balances,
                "preTokenBalances": pre_tokens,
                "postTokenBalances": post_tokens,
            }
        })
    }

    fn payer_wsol_entry(idx: u64, amount: &str) -> Value {
        json!({
            "accountIndex": idx,
            "mint": WSOL,
            "owner": PAYER,
            "uiTokenAmount": {"amount": amount, "decimals": 9}
        })
    }

    #[test]
    fn landed_profit_in_wsol_is_summed_from_payer_accounts_only() {
        let meta = landed_meta(
            vec![1_000_000_000, 5_000_000_000],
            vec![995_000_000, 5_000_000_000],
            json!([
                payer_wsol_entry(2, "10000000"),
                json!({"accountIndex": 3, "mint": WSOL, "owner": "Vault11111111111111111111111111111111111111",
                       "uiTokenAmount": {"amount": "999", "decimals": 9}}),
            ]),
            json!([
                payer_wsol_entry(2, "15500000"),
                json!({"accountIndex": 3, "mint": WSOL, "owner": "Vault11111111111111111111111111111111111111",
                       "uiTokenAmount": {"amount": "5000", "decimals": 9}}),
            ]),
        );
        let c = parse_landed_balance_changes(&meta, PAYER, WSOL);
        assert_eq!(c.fee_payer_lamport_delta, -5_000_000);
        assert_eq!(c.wsol_token_delta, Some(5_500_000));
        assert_eq!(c.realized_net_wsol, Some(500_000));
    }

    #[test]
    fn landed_profit_in_unwrapped_lamports_has_no_token_delta() {
        let meta = landed_meta(
            vec![1_000_000_000],
            vec![1_001_200_000],
            Value::Array(vec![]),
            Value::Array(vec![]),
        );
        let c = parse_landed_balance_changes(&meta, PAYER, WSOL);
        assert_eq!(c.fee_payer_lamport_delta, 1_200_000);
        assert_eq!(c.wsol_token_delta, None);
        assert_eq!(c.realized_net_wsol, Some(1_200_000));
    }

    #[test]
    fn closed_payer_token_account_counts_as_full_negative_delta() {
        let meta = landed_meta(
            vec![1_000_000_000],
            vec![999_000_000],
            json!([payer_wsol_entry(1, "7000000")]),
            json!([]),
        );
        let c = parse_landed_balance_changes(&meta, PAYER, WSOL);
        assert_eq!(c.wsol_token_delta, Some(-7_000_000));
        assert_eq!(c.realized_net_wsol, Some(-8_000_000));
    }

    #[test]
    fn missing_balance_evidence_yields_none_not_zero() {
        let meta = json!({"meta": {}});
        let c = parse_landed_balance_changes(&meta, PAYER, WSOL);
        assert_eq!(c.fee_payer_lamport_delta, 0);
        assert_eq!(c.wsol_token_delta, None);
        assert_eq!(c.realized_net_wsol, None);
    }

    #[test]
    fn non_wsol_mints_are_ignored() {
        let mut entry = payer_wsol_entry(1, "100");
        entry["mint"] = json!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        let meta = landed_meta(
            vec![1_000_000_000],
            vec![999_000_000],
            json!([entry]),
            json!([entry]),
        );
        let c = parse_landed_balance_changes(&meta, PAYER, WSOL);
        assert_eq!(c.wsol_token_delta, None);
        assert_eq!(c.realized_net_wsol, Some(-1_000_000));
    }

    #[test]
    fn simulator_without_endpoint_fails_closed() {
        let sim = ForkSimulator::new(ForkState::default(), None);
        assert!(!sim.has_endpoint());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(sim.simulate_route(&[1, 2, 3], &[], 0))
            .expect_err("no endpoint must not produce a simulated success");
        assert!(err.to_string().contains("no RPC endpoint"));
    }

    #[test]
    fn empty_payload_is_rejected_before_any_rpc_call() {
        let sim = ForkSimulator::with_rpc_url("http://127.0.0.1:1".into());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(sim.simulate_route(&[], &[], 0))
            .expect_err("empty payload must be rejected");
        assert!(err.to_string().contains("empty transaction"));
    }

    #[test]
    fn token_delta_is_none_without_token_balances() {
        let result = ForkSimResult {
            success: true,
            context_slot: Some(1),
            replacement_blockhash: None,
            error: None,
            units_consumed: 0,
            loaded_accounts_data_size: None,
            logs: Vec::new(),
            inner_instruction_count: 0,
            pre_balances: Vec::new(),
            post_balances: Vec::new(),
            fee_payer_lamport_delta: 0,
            pre_token_balances: Vec::new(),
            post_token_balances: Vec::new(),
            realized_net_lamports: None,
            expected_net_lamports: 0,
            accounts: Vec::new(),
            return_data: None,
        };
        assert_eq!(result.token_delta(0), None);
    }

    #[test]
    fn token_delta_is_computed_from_pre_and_post() {
        let mut result = ForkSimResult {
            success: true,
            context_slot: Some(1),
            replacement_blockhash: None,
            error: None,
            units_consumed: 0,
            loaded_accounts_data_size: None,
            logs: Vec::new(),
            inner_instruction_count: 0,
            pre_balances: Vec::new(),
            post_balances: Vec::new(),
            fee_payer_lamport_delta: 0,
            pre_token_balances: vec![SimTokenBalance {
                account_index: 0,
                mint: "So11111111111111111111111111111111111111112".into(),
                owner: None,
                amount: "1000".into(),
                decimals: 9,
            }],
            post_token_balances: vec![SimTokenBalance {
                account_index: 0,
                mint: "So11111111111111111111111111111111111111112".into(),
                owner: None,
                amount: "1750".into(),
                decimals: 9,
            }],
            realized_net_lamports: None,
            expected_net_lamports: 0,
            accounts: Vec::new(),
            return_data: None,
        };
        assert_eq!(result.token_delta(0), Some(750));
        result.realized_net_lamports = Some(750);
        assert_eq!(result.realized_net_lamports, Some(750));
    }
}
