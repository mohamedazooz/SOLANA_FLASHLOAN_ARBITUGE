//! Transaction assembly, route resolution, and verified-input helpers.
use super::*;

impl Supervisor {
    /// Assemble native (Raydium / Orca / real-DLMM-fallback) arbitrage instructions.
    /// Encapsulates the guarded-flash, plain-flash, and non-flash native builders.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn assemble_native_ixs(
        &self,
        payer: Pubkey,
        tip_account: Pubkey,
        tip: Lamports,
        cu_limit: u32,
        cu_price: u64,
        opp: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
    ) -> anyhow::Result<Vec<arb_tx::CompiledInstruction>> {
        if self.config.strategy.use_flash_loan {
            // min_profit_raw from economic config (in base-mint raw units)
            let min_profit_raw = {
                let graph = self.graph.lock();
                graph.min_profit_raw()
            };
            Ok(TxBuilder::assemble_flash_arbitrage(
                payer,
                tip_account,
                tip,
                cu_limit,
                cu_price,
                opp,
                kamino_params,
                flash_loan_fee_sf,
                min_profit_raw,
            )?)
        } else {
            // min_profit_raw from economic config (in base-mint raw units)
            let min_profit_raw = {
                let graph = self.graph.lock();
                graph.min_profit_raw()
            };
            Ok(TxBuilder::assemble_arbitrage(
                payer,
                tip_account,
                tip,
                cu_limit,
                cu_price,
                opp,
                min_profit_raw,
            )?)
        }
    }

    /// Resolve a multi-hop route to LIVE Jupiter v6 swap-instructions and wrap them
    /// in an atomic Kamino flash-loan bundle. Used for exotic / Meteora DLMM routes
    /// where the native builder lacks live bin-array state — Jupiter reads live
    /// on-chain state and returns byte-exact, fully account-resolved instructions.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn resolve_jupiter_swap_route(
        &self,
        opp: &ExecutionOpportunity,
        payer: Pubkey,
        tip: Lamports,
        cu_limit: u32,
        cu_price: u64,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
    ) -> anyhow::Result<Vec<arb_tx::CompiledInstruction>> {
        let mut swap_ixs: Vec<arb_tx::CompiledInstruction> = Vec::new();
        let mut current_amount = opp.size_in.as_u64();

        for hop in &opp.hops {
            let in_mint = hop.mint_in.0.to_string();
            let out_mint = hop.mint_out.0.to_string();
            let quote = self
                .jupiter
                .get_quote(&in_mint, &out_mint, current_amount, 50)
                .await?;

            let out_amount = quote
                .get("outAmount")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                })
                .ok_or_else(|| anyhow::anyhow!("Jupiter quote missing outAmount"))?;

            let ixs = self.jupiter.get_swap_instructions(&payer, quote).await?;
            swap_ixs.extend(ixs);
            current_amount = out_amount;
        }

        TxBuilder::assemble_jupiter_flash_arbitrage(
            payer,
            arb_land::get_random_tip_account(),
            tip,
            cu_limit,
            cu_price,
            opp.size_in.as_u64(),
            kamino_params,
            swap_ixs,
            flash_loan_fee_sf,
        )
    }

    pub(super) async fn fetch_recent_blockhash(&self) -> [u8; 32] {
        // Reuse a fresh cached hash: a blockhash stays valid for ~60-90s, so a
        // few seconds of reuse removes an RPC round-trip from the send path
        // without risking an expired hash.
        {
            let cache = self.blockhash_cache.lock();
            if let Some((bh, at)) = *cache
                && at.elapsed() < std::time::Duration::from_secs(3)
            {
                return bh;
            }
        }
        let req_params = serde_json::json!([{"commitment": "confirmed"}]);
        if let Ok(val) = self.rpc_pool.call("getLatestBlockhash", req_params).await {
            let bh_str = val
                .get("value")
                .and_then(|v| v.get("blockhash"))
                .and_then(|b| b.as_str())
                .or_else(|| {
                    val.get("result")
                        .and_then(|r| r.get("value"))
                        .and_then(|v| v.get("blockhash"))
                        .and_then(|b| b.as_str())
                });

            if let Some(bh) = bh_str
                && let Ok(bytes) = bs58::decode(bh).into_vec()
                && let Ok(arr) = bytes.try_into()
            {
                *self.blockhash_cache.lock() = Some((arr, Instant::now()));
                return arr;
            }
        }
        [0u8; 32]
    }

    /// Latest live SOL/USD mid from the tracked SOL-USDC pools, or `None` when
    /// no verified price is available. Never a guessed constant.
    pub(super) fn live_sol_price(&self) -> Option<f64> {
        let p = *self.last_known_live_sol_price.read();
        if p > 0.0 { Some(p) } else { None }
    }

    /// Build verified on-chain economic inputs for one quoting cycle.
    ///
    /// Reads the Kamino flash-loan fee and base-mint decimals live from the
    /// RPC endpoint in the config (never hardcoded), and takes the current
    /// market Jito tip floor from the oracle. Fail closed: any unreadable or
    /// stale source is an `Err` and the tick is skipped (FIN-01/FIN-04).
    pub(super) async fn build_verified_inputs(
        &self,
        current_slot: Slot,
    ) -> anyhow::Result<VerifiedQuoteInputs> {
        let base_mint = MintId(Pubkey::from_str(arb_tx::WSOL_MINT)?);
        let reserve = Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE)?;

        // Ensure Kamino reserve & WSOL mint snapshots exist and stay fresh.
        // Note: `AccountSnapshotStore::apply` refuses `is_startup: true` overwrites
        // once a live WS write (`ProviderId(2)`, `is_startup: false`) exists, so we
        // evaluate fresh RPC reads against `self.kamino_fallback_store` if needed
        // while preserving `store`'s stream provenance invariants.
        let store = self.ingest.account_store();
        let primary_fresh = matches!(
            (store.get(&reserve), store.get(&base_mint.0)),
            (Some(r), Some(m))
                if current_slot.0.saturating_sub(r.slot) <= 1000
                    && current_slot.0.saturating_sub(m.slot) <= 1000
        );
        let fallback_fresh = !primary_fresh
            && matches!(
                (
                    self.kamino_fallback_store.get(&reserve),
                    self.kamino_fallback_store.get(&base_mint.0),
                ),
                (Some(r), Some(m))
                    if current_slot.0.saturating_sub(r.slot) <= 1000
                        && current_slot.0.saturating_sub(m.slot) <= 1000
            );

        let mut use_fallback_store = fallback_fresh;
        let mut eval_slot = current_slot;

        if !primary_fresh && !fallback_fresh {
            let key1 = reserve.to_string();
            let key2 = base_mint.0.to_string();
            let keys = [key1.as_str(), key2.as_str()];
            if let Ok((accounts, ctx_slot)) =
                self.rpc_pool.get_multiple_accounts_with_slot(&keys).await
            {
                use base64::Engine;
                let klend_owner = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID)?;
                let token_owner = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)?;
                let pkeys = [reserve, base_mint.0];
                let default_owners = [klend_owner, token_owner];
                let snap_slot = if ctx_slot > 0 {
                    ctx_slot
                } else {
                    current_slot.0
                };
                let stream_gen = self.ingest.stream_generation();
                let mut decoded_both = 0usize;

                for (i, acc_opt) in accounts.into_iter().enumerate() {
                    let Some(acc_val) = acc_opt else { continue };
                    let Some(b64_str) = acc_val
                        .get("data")
                        .and_then(|d| d.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|s| s.as_str())
                    else {
                        continue;
                    };
                    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64_str)
                    else {
                        continue;
                    };
                    let owner = acc_val
                        .get("owner")
                        .and_then(|o| o.as_str())
                        .and_then(|s| Pubkey::from_str(s).ok())
                        .unwrap_or(default_owners[i]);
                    let data_arc = std::sync::Arc::new(bytes);
                    let wv = arb_ingest::next_pool_write_version();

                    self.kamino_fallback_store
                        .apply(arb_ingest::AccountSnapshot {
                            pubkey: pkeys[i],
                            owner,
                            slot: snap_slot,
                            write_version: wv,
                            is_startup: true,
                            provider_id: arb_ingest::ProviderId(0),
                            bank_id: None,
                            generation: 0,
                            data: std::sync::Arc::clone(&data_arc),
                        });
                    store.apply(arb_ingest::AccountSnapshot {
                        pubkey: pkeys[i],
                        owner,
                        slot: snap_slot,
                        write_version: wv,
                        is_startup: true,
                        provider_id: arb_ingest::ProviderId(0),
                        bank_id: None,
                        generation: stream_gen,
                        data: data_arc,
                    });
                    decoded_both += 1;
                }

                if decoded_both == 2 && current_slot.0.saturating_sub(snap_slot) <= 1500 {
                    use_fallback_store = true;
                    eval_slot = Slot(current_slot.0.max(snap_slot));
                }
            }
        }

        let active_store = if use_fallback_store {
            &self.kamino_fallback_store
        } else {
            &store
        };

        let inputs = crate::verified_config::build_verified_inputs_from_snapshots(
            active_store,
            &base_mint,
            &reserve,
            &base_mint.0,
            Lamports(self.config.jito.max_tip_lamports),
            eval_slot,
            self.live_sol_price(),
        )
        .map_err(anyhow::Error::msg)?;
        if !inputs.is_fresh(1500) {
            anyhow::bail!("verified Yellowstone economic inputs exceed max_state_age_slots");
        }
        Ok(inputs)
    }
}

/// Maximum age, in slots, across every pool an opportunity depends on.
///
/// Slots are the only unit that survives the 400→200 ms slot-duration feature
/// gate, so freshness is never derived from wall-clock. Fails closed: a hop
/// whose pool is missing from the snapshot — or that was never stamped — makes
/// the whole route maximally stale rather than silently fresh.
pub(super) fn max_state_age_slots(
    hops: &[SwapHop],
    pools: &[PoolState],
    current_slot: Slot,
) -> u64 {
    hops.iter()
        .map(|h| {
            pools
                .iter()
                .find(|p| p.pool_id == h.pool_id)
                .map(|p| current_slot.0.saturating_sub(p.last_update_slot))
                .unwrap_or(u64::MAX)
        })
        .max()
        .unwrap_or(u64::MAX)
}
