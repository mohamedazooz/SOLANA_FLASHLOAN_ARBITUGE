//! Per-tick strategy cycle (R9 split from run.rs): one slot-advanced tick —
//! verified economic inputs, market snapshot, candidate graph, opportunity
//! capture (2-pool DLMM / Bellman-Ford / multi-hop / 3-leg), risk gates,
//! transaction assembly and dispatch. Extracted verbatim; outer-tick `continue`
//! statements became `return Ok(())`, which is behavior-preserving because the
//! only code after the call site in `run()` is the loop bottom.
use super::*;

impl Supervisor {
    pub(super) async fn strategy_cycle(
        &self,
        current_slot: Slot,
        expected_stream_generation: u64,
        payer: Pubkey,
        kamino_params: &KaminoFlashParams,
    ) -> Result<()> {
        // Promote PumpSwap only after ingestion has supplied the Pool, both
        // vaults, both mints and canonical Pump Fees FeeConfig at coherent slots.
        // The graph uses a conservative whole-BPS fee for candidate discovery;
        // exact route validation remains protocol-adapter driven.
        let pump_swap_report = self
            .ingest
            .refresh_pump_swap_states(self.config.strategy.max_state_age_slots);
        if pump_swap_report.pools_promoted > 0 || pump_swap_report.pools_rejected > 0 {
            debug!(
                "PumpSwap promotion: owner_accounts={} tracked={} joined={} promoted={} rejected={}",
                pump_swap_report.pool_accounts_seen,
                pump_swap_report.dependencies_tracked,
                pump_swap_report.pools_joined,
                pump_swap_report.pools_promoted,
                pump_swap_report.pools_rejected,
            );
        }
        self.previous_tips
            .lock()
            .retain(|(slot, _), _| *slot + 2 >= current_slot.0);

        // 0. Verified economic inputs (fail closed): Kamino reserve fee and
        // WSOL mint decimals must exist in the ingest account journal.
        // Jito cost uses the configured conservative tip ceiling until a
        // slot-aware tip-floor feed is integrated. Missing/stale chain data
        // skips the whole tick; no HTTP JSON-RPC or stale-cache fallback.
        let verified = match self.build_verified_inputs(current_slot).await {
            Ok(v) => v,
            Err(e) => {
                debug!("verified economic inputs unavailable ({e}); skipping tick (fail closed)");
                return Ok(());
            }
        };
        let economic_config = verified
            .clone()
            .into_economic_config(TokenAmount(self.config.strategy.min_net_profit_abs));
        {
            let mut graph = self.graph.lock();
            graph.set_economic_config(economic_config.clone());
        }

        // HTTP catalog/trending discovery is disabled. Pool candidates must arrive
        // through the configured WebSocket account-update stream and pass account validation.

        // 1. Ingest: take one pool snapshot per strategy cycle; account updates mutate one key.
        //    Raw stream updates are promoted only after protocol-specific decoding.
        let quote_ingest_time = std::time::Instant::now();
        let market_snap = self.ingest.snapshot();
        let pools: Vec<PoolState> = market_snap.snapshot_pools();
        // Reset every cycle: an absent fresh SOL/USDC pool must not reuse an old price.
        *self.last_known_live_sol_price.write() = 0.0;

        // Track live SOL price from any active SOL-USDC pool (Raydium, Orca, Meteora).
        // Using all three venues improves price freshness and cross-checks accuracy.
        let wsol_str = arb_tx::WSOL_MINT;
        let usdc_str = arb_tx::USDC_MINT;
        for p in &pools {
            if current_slot.0.saturating_sub(p.last_update_slot)
                > self.config.strategy.max_state_age_slots
            {
                continue;
            }
            let is_sol_usdc = (p.base_mint.0.to_string() == wsol_str
                && p.quote_mint.0.to_string() == usdc_str)
                || (p.base_mint.0.to_string() == usdc_str
                    && p.quote_mint.0.to_string() == wsol_str);
            if is_sol_usdc && p.reserve_in > 0 && p.reserve_out > 0 {
                let (sol_res, usdc_res) = if p.base_mint.0.to_string() == wsol_str {
                    (p.reserve_in as f64 / 1e9, p.reserve_out as f64 / 1e6)
                } else {
                    (p.reserve_out as f64 / 1e9, p.reserve_in as f64 / 1e6)
                };
                if sol_res > 0.0 {
                    *self.last_known_live_sol_price.write() = usdc_res / sol_res;
                    break; // first live SOL/USDC price is sufficient
                }
            }
        }

        // 2. Update Graph using fresh states only. Trade sizing is governed by RiskGate.
        // Finite WSOL-denominated candidate ceiling derived from the
        // configured USD notional and a fresh SOL/USD reference. The graph
        // is deliberately WSOL-anchored because Kamino's configured reserve
        // lends WSOL and repayment must close in the exact borrowed mint.
        let max_sizing = match self.live_sol_price() {
            Some(sol_usd)
                if sol_usd.is_finite()
                    && sol_usd > 0.0
                    && self.config.risk.max_trade_size_usd.is_finite()
                    && self.config.risk.max_trade_size_usd > 0.0 =>
            {
                (self.config.risk.max_trade_size_usd / sol_usd * 1_000_000_000.0)
                    .floor()
                    .clamp(0.0, u64::MAX as f64) as u64
            }
            _ => 0,
        };
        if max_sizing == 0 {
            debug!("Missing fresh SOL/USD reference or invalid USD size cap; skip graph sizing");
            return Ok(());
        }

        // Filter out stale pools from graph to prevent phantom arbitrage cycles
        // caused by price divergence against frozen/unupdated pools.
        // Graph freshness is an explicit policy: pools older than the
        // configured graph age never enter the candidate graph. Defaults
        // to the quote-freshness policy; no arbitrary multiplier.
        // Graph freshness must never exceed quote/state freshness: a wider
        // horizon (e.g. 300 slots) admits stale pools whose prices have
        // diverged, producing phantom arbitrage and implausible ROI. Cap the
        // graph horizon at the configured state-age window so only pools
        // that are still quote-fresh can enter the candidate graph.
        let max_graph_age_slots = self
            .config
            .strategy
            .graph_max_age_slots
            .unwrap_or(self.config.strategy.max_state_age_slots)
            .min(self.config.strategy.max_state_age_slots)
            .max(1);
        let fresh_pools: Vec<PoolState> = pools
            .iter()
            .filter(|p| {
                p.slot.0 > 0 && current_slot.0.saturating_sub(p.slot.0) <= max_graph_age_slots
            })
            .cloned()
            .collect();

        // Fail-closed: if no fresh pools within the graph max age window,
        // skip this tick entirely. No stale pool may enter a candidate route.
        if fresh_pools.is_empty() {
            debug!("No fresh pools within graph max age; skipping tick (fail closed)");
            return Ok(());
        }

        let opportunities = {
            let mut graph = self.graph.lock();
            graph.build_from_pools(&fresh_pools);
            let (mut opps, base_stats) =
                graph.find_base_cycles_with_stats(current_slot, max_sizing);
            if base_stats.opportunities == 0 {
                debug!(
                    "base-cycle scan: anchors={} 2hop_gross={} 3hop_gross={} gate_rejected={}",
                    base_stats.base_anchors,
                    base_stats.two_hop_gross_profitable,
                    base_stats.three_hop_gross_profitable,
                    base_stats.gate_rejected
                );
            }

            // ── MeteoraDlmmCrossPoolArbitrageStrategy (P0 first strategy) ──
            // First-class two-pool DLMM path: distinct same-pair pools,
            // both directions, bounded integer size optimizer, real bin
            // traversal. Merged with the DFS candidates (deduped below).
            let dcp_cfg = self
                .config
                .strategy
                .dlmm_cross_pool
                .clone()
                .unwrap_or_default();
            let scan = {
                // Split borrow: config comes from the graph-hosting block.
                let cfg = arb_engine::DlmmCrossPoolConfig {
                    max_pairs: dcp_cfg.max_pairs,
                    min_input: dcp_cfg.min_input,
                    max_input: dcp_cfg.max_input.min(max_sizing),
                    coarse_samples: dcp_cfg.coarse_samples,
                    refine_samples: dcp_cfg.refine_samples,
                    min_spread_bps: dcp_cfg.min_spread_bps,
                    min_liquidity_bin_raw: dcp_cfg.min_liquidity_bin_raw,
                };
                graph.find_cross_pool_dlmm_cycles(current_slot, max_sizing, &cfg)
            };
            let mut cross_opps = scan.opportunities;
            let cross_stats = scan.stats;
            let cross_rejections = scan.rejections;
            {
                use arb_engine::CrossPoolRejectionReason as RR;
                let mut by: std::collections::HashMap<&'static str, u64> = Default::default();
                for r in &cross_rejections {
                    let label = match r.reason {
                        RR::EdgeBelowHurdle => "EDGE_BELOW_HURDLE",
                        RR::InsufficientLiquidity => "INSUFFICIENT_LIQUIDITY",
                        RR::StalePoolState => "STALE_POOL_STATE",
                        RR::MissingAccounts => "MISSING_ACCOUNTS",
                        RR::MaxTradeSizeExceeded => "MAX_TRADE_SIZE_EXCEEDED",
                        RR::TokenProgramMismatch => "TOKEN_PROGRAM_MISMATCH",
                        RR::EconomicGateRejected => "ECONOMIC_GATE_REJECTED",
                    };
                    *by.entry(label).or_insert(0) += 1;
                }
                info!(
                    "DLMM cross-pool strategy: pairs={} directions={} opportunities={} rejections={}",
                    cross_stats.pairs_scanned,
                    cross_stats.directions_evaluated,
                    cross_opps.len(),
                    by.iter()
                        .map(|(k, v)| format!("{k}:{v}"))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            // Dedupe vs DFS candidates by (pool pair, size).
            let mut seen_pairs: HashSet<(Pubkey, Pubkey)> = Default::default();
            for o in &opps {
                if let (Some(h1), Some(h2)) = (o.hops.first(), o.hops.last()) {
                    seen_pairs.insert((h1.pool_id.0, h2.pool_id.0));
                }
            }
            cross_opps.retain(|o| {
                if let (Some(h1), Some(h2)) = (o.hops.first(), o.hops.last()) {
                    seen_pairs.insert((h1.pool_id.0, h2.pool_id.0))
                } else {
                    false
                }
            });
            opps.extend(cross_opps);

            // ── Cross-platform multi-hop capture (always evaluated) ──
            // Best practice: evaluate every candidate class each tick and
            // merge by profitability instead of short-circuiting at the
            // first non-empty class. Previously Bellman-Ford / multi-hop
            // DFS ran only when the base-cycle set was empty, which hid
            // longer cross-DEX routes (3..5 hops) whenever any 2-hop cycle
            // existed — even when the longer route was more profitable.
            let bf_opps = {
                let bf_engine = arb_engine::BellmanFordEngine::from_pools(&fresh_pools)
                    .with_economic_config(economic_config.clone());
                let found = bf_engine.find_negative_cycles(current_slot, max_sizing);
                if !found.is_empty() {
                    info!(
                        "🎯 Bellman-Ford detected {} arbitrage cycle(s)",
                        found.len()
                    );
                }
                found
            };
            let multi_opps = graph.find_multi_hop_cycles(current_slot, max_sizing, 5);
            if !multi_opps.is_empty() {
                info!("🔁 Multi-hop DFS detected {} route(s)", multi_opps.len());
            }

            // Graph stats for the tick log (snapshot while still locked).
            let (graph_edges, graph_anchors) =
                (graph.edges_by_from.len(), graph.base_anchors.len());

            // Merge + dedupe by sorted pool set, then rank by net profit so
            // the most profitable route is processed first.
            let mut merged: Vec<ExecutionOpportunity> = opps;
            let mut seen_pool_sets: HashSet<Vec<Pubkey>> = Default::default();
            for o in &merged {
                let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
                ids.sort_unstable();
                seen_pool_sets.insert(ids);
            }
            for mut src in [bf_opps, multi_opps] {
                src.retain(|o| {
                    let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
                    ids.sort_unstable();
                    seen_pool_sets.insert(ids)
                });
                merged.extend(src);
            }
            merged.sort_by_key(|o| std::cmp::Reverse(o.net_profit.0));
            info!(
                "Slot {}: graph discovered {} candidate route(s) after dedupe; graph edges={}, base_anchors={}",
                current_slot.0,
                merged.len(),
                graph_edges,
                graph_anchors
            );
            (merged, seen_pool_sets)
        };
        let (mut merged, mut seen_pool_sets) = opportunities;

        // ── Meteora 3-leg (WSOL -> A -> B -> WSOL) pipeline ──
        // Third candidate class: three distinct DLMM pools on Meteora,
        // chained-quote discovery + 3-leg-native size search. Converted
        // into a standard ExecutionOpportunity so the same economics,
        // risk and dispatch path applies. Fails closed to an empty vec.
        // Fail-closed liquidity proof: when flash funding is enabled, the
        // actual supply-vault liquidity must be verifiable — an
        // unverifiable reserve skips the capture entirely.
        let flash_liquidity = self.kamino_available_flash_liquidity(current_slot);
        let flash_liquidity_unverified =
            self.config.strategy.use_flash_loan && flash_liquidity.is_none();
        if flash_liquidity_unverified {
            debug!(
                "3-leg capture skipped: flash funding enabled but Kamino supply-vault liquidity unverifiable at slot {}",
                current_slot.0
            );
        }
        let three_leg_opps = if flash_liquidity_unverified {
            Vec::new()
        } else {
            self.capture_three_leg_dlmm(
                &fresh_pools,
                current_slot,
                max_sizing,
                verified.flash_loan_fee_sf.value,
                flash_liquidity,
                verified.tip_lamports.value.0,
            )
            .await
        };
        if !three_leg_opps.is_empty() {
            info!(
                "🟢 Meteora 3-leg DLMM pipeline captured {} chain(s)",
                three_leg_opps.len()
            );
        }
        if let Some(control) = self.control() {
            for o in three_leg_opps.iter().take(16) {
                control.record_opportunity(crate::control_service::OpportunityRecord {
                    captured_at_unix: 0,
                    slot: current_slot.0,
                    route: o
                        .hops
                        .iter()
                        .map(|h| format!("{}", h.pool_id))
                        .collect::<Vec<_>>()
                        .join("\u{2192}"),
                    protocol: "meteora-dlmm-3leg".to_string(),
                    borrow_lamports: o.size_in.0,
                    expected_output_lamports: o.expected_out.0,
                    net_profit_lamports: o.net_profit.0 as i64,
                    eligible: true,
                    reject_reason: None,
                });
            }
        }

        // Merge the 3-leg class with the same sorted-pool-set dedupe, then
        // rank by net profit so the most profitable route is processed first.
        let mut three_leg_opps = three_leg_opps;
        three_leg_opps.retain(|o| {
            let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
            ids.sort_unstable();
            seen_pool_sets.insert(ids)
        });
        merged.extend(three_leg_opps);
        merged.sort_by_key(|o| std::cmp::Reverse(o.net_profit.0));
        let opportunities = merged;

        let t_ingest_to_quote_us = quote_ingest_time.elapsed().as_micros() as u64;
        self.heartbeat
            .observe_quote(current_slot.0, opportunities.len());

        self.process_opportunities(
            &opportunities,
            super::opportunity::OpportunityPass {
                current_slot,
                expected_stream_generation,
                verified: verified.clone(),
                payer,
                kamino_params,
                t_ingest_to_quote_us,
                pools: &pools,
            },
        )
        .await;

        Ok(())
    }
}
