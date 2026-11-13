//! Transaction assembly, route resolution, and verified-input helpers.
use super::*;

impl Supervisor {
    /// Assemble native (Raydium / Orca / real-DLMM-fallback) arbitrage instructions.
    /// Encapsulates the guarded-flash, plain-flash, and non-flash native builders.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn assemble_native_ixs(
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
            let skip_close = self.preexisting_intermediate_atas(opp, payer).await;
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
                &skip_close,
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

    /// Probe the RPC for intermediate-mint ATAs that already exist for the payer.
    /// The flash builder closes every ephemeral intermediate ATA at the end of the
    /// transaction; closing a PRE-EXISTING ATA would fail on a non-empty balance
    /// (or destroy balances held outside this trade), so those mints are excluded
    /// from the close list. On any RPC failure the probe fails safe: ALL
    /// intermediate closes are skipped (rent stays parked, no revert, no loss).
    async fn preexisting_intermediate_atas(
        &self,
        opp: &ExecutionOpportunity,
        payer: Pubkey,
    ) -> Vec<Pubkey> {
        let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).unwrap_or_default();
        let usdc = Pubkey::from_str(arb_tx::USDC_MINT).unwrap_or_default();
        let mut probes: Vec<(Pubkey, Pubkey)> = Vec::new();
        for hop in &opp.hops {
            for (mint, program) in [
                (hop.mint_in.0, hop.token_programs[0].program_id()),
                (hop.mint_out.0, hop.token_programs[1].program_id()),
            ] {
                if mint != wsol && mint != usdc && !probes.iter().any(|(m, _)| *m == mint) {
                    let ata = arb_tx::get_associated_token_address_with_program_id(
                        &payer, &mint, &program,
                    );
                    probes.push((mint, ata));
                }
            }
        }
        if probes.is_empty() {
            return Vec::new();
        }
        let keys: Vec<String> = probes.iter().map(|(_, ata)| ata.to_string()).collect();
        let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        match self.rpc_pool.get_multiple_accounts(&key_refs).await {
            Ok(accounts) => probes
                .iter()
                .zip(accounts.iter())
                .filter(|(_, acc)| acc.is_some())
                .map(|((mint, _), _)| *mint)
                .collect(),
            Err(e) => {
                debug!("ATA existence probe failed ({e}); skipping all intermediate ATA closes");
                probes.into_iter().map(|(mint, _)| mint).collect()
            }
        }
    }

    /// Returns the blockhash plus its `lastValidBlockHeight` (recovery
    /// evidence for expiration; `None` only if the RPC omits it).
    pub(super) async fn fetch_recent_blockhash(&self) -> ([u8; 32], Option<i64>) {
        // Reuse a fresh cached hash: a blockhash stays valid for ~60-90s, so a
        // few seconds of reuse removes an RPC round-trip from the send path
        // without risking an expired hash.
        {
            let cache = self.blockhash_cache.lock();
            if let Some((bh, at, lvbh)) = *cache
                && at.elapsed() < std::time::Duration::from_secs(3)
            {
                return (bh, lvbh);
            }
        }
        let req_params = serde_json::json!([{"commitment": "confirmed"}]);
        if let Ok(val) = self.rpc_pool.call("getLatestBlockhash", req_params).await {
            let value = val
                .get("value")
                .cloned()
                .or_else(|| val.get("result").and_then(|r| r.get("value")).cloned());
            let bh_str = value
                .as_ref()
                .and_then(|v| v.get("blockhash"))
                .and_then(|b| b.as_str());
            let lvbh = value
                .as_ref()
                .and_then(|v| v.get("lastValidBlockHeight"))
                .and_then(|h| h.as_i64());

            if let Some(bh) = bh_str
                && let Ok(bytes) = bs58::decode(bh).into_vec()
                && let Ok(arr) = bytes.try_into()
            {
                *self.blockhash_cache.lock() = Some((arr, Instant::now(), lvbh));
                return (arr, lvbh);
            }
        }
        ([0u8; 32], None)
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
