//! Transaction assembly, route resolution, and verified-input helpers.
use super::*;

fn verified_rpc_account_snapshot(
    pubkey: Pubkey,
    expected_owner: Pubkey,
    context_slot: u64,
    generation: u64,
    account: &serde_json::Value,
) -> Option<arb_ingest::AccountSnapshot> {
    use base64::Engine as _;

    if context_slot == 0 {
        return None;
    }
    let owner = account.get("owner")?.as_str()?.parse::<Pubkey>().ok()?;
    if owner != expected_owner {
        return None;
    }
    let encoded = account.get("data")?.as_array()?.first()?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if bytes.is_empty() {
        return None;
    }

    Some(arb_ingest::AccountSnapshot {
        pubkey,
        owner,
        slot: context_slot,
        write_version: arb_ingest::next_pool_write_version(),
        is_startup: true,
        provider_id: arb_ingest::HTTP_RPC_PROVIDER_ID,
        bank_id: None,
        generation,
        data: std::sync::Arc::new(bytes),
    })
}
impl Supervisor {
    /// Assemble native (Raydium / Orca / real-DLMM-fallback) arbitrage instructions.
    /// Encapsulates the guarded-flash, plain-flash, and non-flash native builders.
    #[allow(clippy::too_many_arguments)]
    pub async fn assemble_native_ixs(
        &self,
        payer: Pubkey,
        tip_account: Pubkey,
        tip: Lamports,
        cu_limit: u32,
        cu_price: u64,
        opp: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
        min_profit_raw: TokenAmount,
    ) -> anyhow::Result<Vec<arb_tx::CompiledInstruction>> {
        let network_fee_raw = crate::supervisor::BASE_FEE_LAMPORTS
            .checked_add(arb_tx::priority_fee_lamports(cu_price, cu_limit))
            .ok_or_else(|| anyhow::anyhow!("network fee arithmetic overflow"))?;
        self.assemble_native_ixs_for_anchor(
            payer,
            tip_account,
            tip,
            cu_limit,
            cu_price,
            opp,
            kamino_params,
            flash_loan_fee_sf,
            network_fee_raw,
            tip.as_u64(),
            min_profit_raw,
        )
        .await
    }

    /// Multi-mint (`WSOL` + `USDC`) instruction assembly with pre-converted
    /// base-mint raw costs for `network_fee_base_raw` and `tip_base_raw`.
    #[allow(clippy::too_many_arguments)]
    pub async fn assemble_native_ixs_for_anchor(
        &self,
        payer: Pubkey,
        tip_account: Pubkey,
        tip: Lamports,
        cu_limit: u32,
        cu_price: u64,
        opp: &ExecutionOpportunity,
        kamino_params: &KaminoFlashParams,
        flash_loan_fee_sf: u64,
        network_fee_base_raw: u64,
        tip_base_raw: u64,
        min_profit_raw: TokenAmount,
    ) -> anyhow::Result<Vec<arb_tx::CompiledInstruction>> {
        if self.config.strategy.use_flash_loan {
            let proven_zero_intermediate_mints: Vec<Pubkey> = Vec::new();

            if arb_tx::cross_dex_flash_arb::is_cross_dex_opportunity(opp) {
                return arb_tx::cross_dex_flash_arb::assemble_cross_dex_opportunity_with_converted_costs(
                    payer,
                    tip_account,
                    tip,
                    cu_limit,
                    cu_price,
                    opp,
                    kamino_params,
                    flash_loan_fee_sf,
                    network_fee_base_raw,
                    tip_base_raw,
                    min_profit_raw,
                    &proven_zero_intermediate_mints,
                );
            }

            Ok(TxBuilder::assemble_flash_arbitrage_with_converted_costs(
                payer,
                tip_account,
                tip,
                cu_limit,
                cu_price,
                opp,
                kamino_params,
                flash_loan_fee_sf,
                network_fee_base_raw,
                tip_base_raw,
                min_profit_raw,
                &proven_zero_intermediate_mints,
            )?)
        } else {
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

    /// Collect `(mint, ata, program)` for every intermediate mint of the
    /// route (excluding the WSOL settlement mint and USDC). Pure helper for
    /// the post-landing settlement probe.
    pub(super) fn intermediate_probes(
        opp: &ExecutionOpportunity,
        payer: Pubkey,
    ) -> Vec<(Pubkey, Pubkey, Pubkey)> {
        let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).unwrap_or_default();
        let usdc = Pubkey::from_str(arb_tx::USDC_MINT).unwrap_or_default();
        let mut probes: Vec<(Pubkey, Pubkey, Pubkey)> = Vec::new();
        for hop in &opp.hops {
            for (mint, program) in [
                (hop.mint_in.0, hop.token_programs[0].program_id()),
                (hop.mint_out.0, hop.token_programs[1].program_id()),
            ] {
                if mint != wsol && mint != usdc && !probes.iter().any(|(m, _, _)| *m == mint) {
                    let ata = arb_tx::get_associated_token_address_with_program_id(
                        &payer, &mint, &program,
                    );
                    probes.push((mint, ata, program));
                }
            }
        }
        probes
    }

    /// Post-landing rent reclaim for intermediate ATAs (F-01, Option B).
    ///
    /// Post-landing rent reclaim for intermediate ATAs (F-01, Option B).
    ///
    /// The atomic arbitrage transaction never closes intermediate ATAs: a
    /// route that beats its quote leaves a residual balance, and closing a
    /// non-empty token account reverts the entire atomic transaction. This
    /// settlement runs AFTER a landed attempt, in a SEPARATE transaction:
    ///
    /// 1. Probe every intermediate ATA of the settled route.
    /// 2. Close ONLY accounts whose balance is verifiably zero. A failing
    ///    close in a settlement tx is isolated and harmless; it never
    ///    affects arb capital or the route guarantee.
    /// 3. Non-zero residuals are logged with their amount and left
    ///    untouched: they are the user's own tokens and can never be
    ///    consumed by a route (every leg input is clamped to the previous
    ///    leg's guaranteed min-out, independently of wallet balances).
    ///
    /// Live-only: in every other mode the plan is logged and no transaction
    /// is broadcast. Free fn so the dispatch reconciliation task can run it
    /// on cloned handles without capturing the `Supervisor`. Returns the
    /// number of accounts reclaimed.
    pub(super) async fn settle_idle_intermediate_atas_with(
        rpc: arb_ingest::RpcPool,
        tx_rpc: &arb_ingest::TransactionRpcAdapter,
        signer: &KeypairSigner,
        live: bool,
        opp: &ExecutionOpportunity,
        payer: Pubkey,
    ) -> usize {
        if !live || !crate::live_execution_enabled() {
            debug!("settlement skipped: live execution is not armed (no broadcast, plan only)");
            return 0;
        }
        let probes = {
            let all = Self::intermediate_probes(opp, payer);
            if all.is_empty() {
                Vec::new()
            } else {
                let keys: Vec<String> = all.iter().map(|(_, ata, _)| ata.to_string()).collect();
                let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
                match rpc.get_multiple_accounts(&key_refs).await {
                    Ok(accounts) => all
                        .into_iter()
                        .zip(accounts)
                        .filter_map(|(probe, acc)| acc.map(|_| probe))
                        .collect(),
                    Err(e) => {
                        debug!("ATA existence probe failed ({e}); settlement skipped");
                        Vec::new()
                    }
                }
            }
        };
        if probes.is_empty() {
            return 0;
        }
        let keys: Vec<String> = probes.iter().map(|(_, ata, _)| ata.to_string()).collect();
        let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let accounts = match rpc.get_multiple_accounts(&key_refs).await {
            Ok(accounts) => accounts,
            Err(e) => {
                debug!("settlement probe failed ({e}); skipped");
                return 0;
            }
        };

        let mut close_ixs: Vec<arb_tx::CompiledInstruction> = Vec::new();
        let mut residual_mints: Vec<(Pubkey, u64)> = Vec::new();
        for ((mint, ata, program), acc) in probes.iter().zip(accounts.iter()) {
            let Some(acc) = acc else { continue };
            // RPC returns raw JSON (`data: [base64, "base64"]`); decode to raw
            // bytes. SPL/Token-2022 base layout: `amount` at bytes 64..72
            // (extension data may follow). An undecodable or malformed account
            // is skipped: neither closed nor logged as residual — the ATA is
            // retried on the next cycle.
            let Some(data) = arb_ingest::decode_account_data(acc) else {
                continue;
            };
            if data.len() < 72 {
                continue;
            }
            let amount = u64::from_le_bytes(data[64..72].try_into().unwrap_or([0; 8]));
            if amount == 0 {
                close_ixs.push(arb_tx::build_close_token_account_for_program(
                    *ata, payer, payer, *program,
                ));
            } else {
                residual_mints.push((*mint, amount));
            }
        }
        for (mint, amount) in &residual_mints {
            tracing::info!(
                target: "arb_events",
                "INTERMEDIATE_ATA_RESIDUAL: mint={} amount={} (open; not closed, not consumed)",
                mint,
                amount
            );
        }
        if close_ixs.is_empty() {
            return 0;
        }

        let Ok((blockhash, _lvbh)) = tx_rpc.get_latest_blockhash().await else {
            debug!("settlement skipped: no valid blockhash");
            return 0;
        };
        if blockhash == [0u8; 32] {
            debug!("settlement skipped: zero blockhash");
            return 0;
        }
        let compiled = arb_tx::AdaptiveTxCompiler::build_signed_legacy_transaction(
            signer,
            &close_ixs,
            blockhash,
            arb_tx::FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::Legacy),
        )
        .map(|c| c.wire_bytes);
        match compiled {
            Ok(bytes) => {
                let b64 =
                    base64::Engine::encode(&base64::prelude::BASE64_STANDARD, bytes.as_slice());
                match rpc.send_raw_transaction(&b64).await {
                    Ok(sig) => {
                        tracing::info!(
                            target: "arb_events",
                            "ATA_RENT_RECLAIMED: closes={} signature={}",
                            close_ixs.len(),
                            sig
                        );
                        close_ixs.len()
                    }
                    Err(e) => {
                        debug!(
                            "settlement broadcast failed (harmless, retried on next cycle): {e}"
                        );
                        0
                    }
                }
            }
            Err(e) => {
                debug!("settlement compile failed (harmless): {e}");
                0
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
        if let Ok((arr, lvbh)) = self.tx_rpc.get_latest_blockhash().await {
            *self.blockhash_cache.lock() = Some((arr, Instant::now(), lvbh));
            return (arr, lvbh);
        }
        ([0u8; 32], None)
    }

    /// Latest live SOL/USD mid from the tracked SOL-USDC pools or live market feed,
    /// or `None` when no verified price is available. Never a guessed constant.
    pub(super) fn live_sol_price(&self) -> Option<f64> {
        let p = *self.last_known_live_sol_price.read();
        if p > 0.0 && p.is_finite() {
            Some(p)
        } else {
            let tg_p = self.telegram.sol_price();
            if tg_p > 0.0 && tg_p.is_finite() {
                Some(tg_p)
            } else {
                None
            }
        }
    }

    /// Live SOL/USD price along with the exact slot at which it was observed on-chain.
    pub(super) fn live_sol_price_with_slot(&self) -> Option<(f64, Slot)> {
        let p = *self.last_known_live_sol_price.read();
        let slot = *self.last_known_live_sol_price_slot.read();
        if p > 0.0 && p.is_finite() && slot > 0 {
            Some((p, Slot(slot)))
        } else {
            None
        }
    }

    /// Build verified on-chain economic inputs for one quoting cycle.
    ///
    /// Reads the Kamino flash-loan fee and base-mint decimals live from the
    /// RPC endpoint in the config (never hardcoded), and takes the current
    /// market Jito tip floor from the oracle. Fail closed: any unreadable or
    /// stale source is an `Err` and the tick is skipped (FIN-01/FIN-04).
    pub async fn build_verified_inputs(
        &self,
        current_slot: Slot,
    ) -> anyhow::Result<VerifiedQuoteInputs> {
        self.build_verified_inputs_for_mint(crate::verified_config::wsol_mint(), current_slot)
            .await
    }

    /// Build verified on-chain economic inputs for a supported Kamino anchor
    /// mint (`WSOL` or `USDC`). Fail closed on unsupported mints or stale data.
    pub async fn build_verified_inputs_for_mint(
        &self,
        anchor_mint: Pubkey,
        current_slot: Slot,
    ) -> anyhow::Result<VerifiedQuoteInputs> {
        let anchor = crate::verified_config::kamino_anchor_for_mint(&anchor_mint)
            .ok_or_else(|| anyhow::anyhow!("unsupported Kamino anchor mint {anchor_mint}"))?;
        let base_mint = MintId(anchor.mint);
        let reserve = anchor.reserve;
        let supply_vault = anchor.supply_vault;

        // Ensure Kamino reserve, WSOL mint, and supply-vault snapshots exist and
        // stay fresh.
        // Note: `AccountSnapshotStore::apply` refuses `is_startup: true` overwrites
        // once a live WS write (`ACCOUNT_WEBSOCKET_PROVIDER_ID`, `is_startup: false`) exists, so we
        // evaluate fresh RPC reads against `self.kamino_fallback_store` if needed
        // while preserving `store`'s stream provenance invariants.
        let store = self.ingest.account_store();
        let is_set_fresh = |s: &arb_ingest::AccountSnapshotStore| {
            matches!(
                (s.get(&reserve), s.get(&base_mint.0), s.get(&supply_vault)),
                (Some(r), Some(m), Some(v))
                    if r.slot <= current_slot.0
                        && m.slot <= current_slot.0
                        && v.slot <= current_slot.0
                        && current_slot.0.saturating_sub(r.slot)
                            <= arb_constants::KAMINO_ZAP_FRESHNESS_TOLERANCE_SLOTS
                        && current_slot.0.saturating_sub(m.slot)
                            <= arb_constants::KAMINO_ZAP_FRESHNESS_TOLERANCE_SLOTS
                        && current_slot.0.saturating_sub(v.slot)
                            <= arb_constants::KAMINO_SUPPLY_VAULT_MAX_AGE_SLOTS
            )
        };
        let primary_fresh = is_set_fresh(&store);
        let fallback_fresh = !primary_fresh && is_set_fresh(&self.kamino_fallback_store);

        let mut use_fallback_store = fallback_fresh;
        let mut eval_slot = current_slot;

        if !primary_fresh && !fallback_fresh {
            let reserve_key = reserve.to_string();
            let mint_key = base_mint.0.to_string();
            let vault_key = supply_vault.to_string();
            let keys = [reserve_key.as_str(), mint_key.as_str(), vault_key.as_str()];
            if let Ok((accounts, context_slot)) =
                self.rpc_pool.get_multiple_accounts_with_slot(&keys).await
            {
                let klend_owner = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID)?;
                let token_owner = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID)?;
                let account_keys = [reserve, base_mint.0, supply_vault];
                let expected_owners = [klend_owner, token_owner, token_owner];

                // Account RPC context is the only acceptable slot provenance.
                // Never relabel a zero/unknown context as the current WS slot,
                // and never infer owner from the address being requested.
                if context_slot > 0
                    && context_slot <= current_slot.0
                    && current_slot.0.saturating_sub(context_slot)
                        <= arb_constants::KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS
                    && accounts.len() == account_keys.len()
                {
                    let generation = self.ingest.stream_generation();
                    let updates = accounts
                        .iter()
                        .zip(account_keys)
                        .zip(expected_owners)
                        .map(|((account, pubkey), owner)| {
                            account.as_ref().and_then(|account| {
                                verified_rpc_account_snapshot(
                                    pubkey,
                                    owner,
                                    context_slot,
                                    generation,
                                    account,
                                )
                            })
                        })
                        .collect::<Option<Vec<_>>>();

                    if let Some(updates) = updates {
                        // Keep RPC fallback provenance isolated from the live
                        // WebSocket journal. Publish the dependency set atomically.
                        if self.kamino_fallback_store.apply_batch(updates) {
                            use_fallback_store = true;
                            eval_slot = current_slot;
                        }
                    }
                }
            }
        }

        let active_store = if use_fallback_store {
            &self.kamino_fallback_store
        } else {
            &store
        };

        // Discovery uses the configured landed-tip percentile (`p75` in
        // production) capped by the configured ceiling, aligning Discovery with
        // the execution bidder. If the oracle is stale/missing, retain the
        // ceiling rather than assuming a zero tip.
        let fresh_tip_estimate =
            self.jito_service
                .oracle()
                .current(arb_land::now_ms())
                .map(|floor| {
                    crate::verified_config::tip_floor_for_percentile(
                        &floor,
                        &self.config.jito.tip_floor_percentile,
                    )
                });
        let discovery_tip = crate::verified_config::bounded_jito_tip_discovery_estimate(
            Lamports(self.config.jito.max_tip_lamports),
            fresh_tip_estimate,
        );

        let (sol_price_usd, sol_price_observed_slot) = match self.live_sol_price_with_slot() {
            Some((price, slot)) => (Some(price), Some(slot)),
            None => (self.live_sol_price(), None),
        };
        let inputs = crate::verified_config::build_verified_inputs_from_snapshots_with_price_slot(
            active_store,
            &base_mint,
            &reserve,
            &base_mint.0,
            discovery_tip,
            eval_slot,
            sol_price_usd,
            sol_price_observed_slot,
        )
        .map_err(anyhow::Error::msg)?;
        if !inputs.is_fresh(arb_constants::KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS) {
            anyhow::bail!(
                "verified Yellowstone economic inputs exceed Kamino RPC snapshot age ceiling"
            );
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

#[cfg(test)]
mod verified_rpc_snapshot_tests {
    use super::*;
    use base64::Engine as _;

    fn account(owner: Option<Pubkey>) -> serde_json::Value {
        let mut value = serde_json::json!({
            "data": [base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]), "base64"]
        });
        if let Some(owner) = owner {
            value["owner"] = serde_json::Value::String(owner.to_string());
        }
        value
    }

    #[test]
    fn rpc_snapshot_rejects_zero_context_slot() {
        let owner = Pubkey::new_unique();
        assert!(
            verified_rpc_account_snapshot(Pubkey::new_unique(), owner, 0, 1, &account(Some(owner)))
                .is_none()
        );
    }

    #[test]
    fn rpc_snapshot_rejects_missing_or_mismatched_owner() {
        let expected = Pubkey::new_unique();
        assert!(
            verified_rpc_account_snapshot(Pubkey::new_unique(), expected, 100, 1, &account(None))
                .is_none()
        );
        assert!(
            verified_rpc_account_snapshot(
                Pubkey::new_unique(),
                expected,
                100,
                1,
                &account(Some(Pubkey::new_unique()))
            )
            .is_none()
        );
    }

    #[test]
    fn rpc_snapshot_preserves_actual_context_slot_and_owner() {
        let expected = Pubkey::new_unique();
        let key = Pubkey::new_unique();
        let snapshot =
            verified_rpc_account_snapshot(key, expected, 123_456, 7, &account(Some(expected)))
                .expect("valid owner and nonzero RPC context must produce a snapshot");
        assert_eq!(snapshot.pubkey, key);
        assert_eq!(snapshot.owner, expected);
        assert_eq!(snapshot.slot, 123_456);
        assert_eq!(snapshot.generation, 7);
        assert!(snapshot.is_startup);
        assert_eq!(snapshot.provider_id, arb_ingest::HTTP_RPC_PROVIDER_ID);
    }
}
