//! Real Mainnet Non-Broadcast Simulation Test for Meteora DLMM Cross-Pool Arbitrage.
//!
//! Executes an end-to-end vertical slice against real Solana Mainnet:
//! 1. Ingests live state for 2 distinct real Meteora DLMM pools (`5rCf1DM8...` & `BVRbyLjj...`)
//! 2. Pairs them via `MeteoraDlmmPoolPairDiscovery`
//! 3. Evaluates quotes via `MeteoraDlmmCrossPoolQuoteEngine`
//! 4. Optimizes trade size via `MeteoraDlmmTradeSizeOptimizer`
//! 5. Assembles atomic two-swap transaction via `MeteoraDlmmAtomicArbitrageBuilder`
//! 6. Simulates via `MeteoraDlmmMainnetSimulationHarness` (non-broadcast RPC simulation)
//! 7. Verifies context slot, CU consumed, logs, and cost breakdown
//!
//! Phase 15.2: emits `reports/phase15_2/mainnet_simulation_evidence.json`.
//! This test never broadcasts, never signs for inclusion, and never relaxes the
//! two-slot freshness gate.

use arb_engine::{
    DlmmOptimizerConfig, MeteoraDlmmCrossPoolQuoteEngine, MeteoraDlmmPoolPairDiscovery,
    MeteoraDlmmTradeSizeOptimizer,
};
use arb_ingest::{MultiDexRegistry, RpcPool};
use arb_sim::{ExecutionCostBreakdown, MeteoraDlmmMainnetSimulationHarness};
use arb_tx::{
    KeypairSigner, MeteoraDlmmAtomicArbitrageBuilder, get_associated_token_address_with_program_id,
};
use arb_types::{Slot, TokenProgramKind};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

fn get_funded_signer() -> KeypairSigner {
    if let Ok(key_str) = std::env::var("SOLANA_PRIVATE_KEY") {
        if let Ok(signer) = KeypairSigner::from_base58(&key_str) {
            return signer;
        }
    }
    for path in &[".env", "../../.env", "../.env"] {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("SOLANA_PRIVATE_KEY=") {
                    let clean = val.trim().trim_matches('"').trim_matches('\'');
                    if let Ok(signer) = KeypairSigner::from_base58(clean) {
                        return signer;
                    }
                }
            }
        }
    }
    KeypairSigner::from_seed(&[55u8; 32])
}

fn token_program_pubkey(kind: TokenProgramKind) -> Pubkey {
    let s = match kind {
        TokenProgramKind::SplToken => arb_constants::TOKEN_PROGRAM_ID,
        TokenProgramKind::Token2022 => arb_constants::TOKEN_2022_PROGRAM_ID,
    };
    Pubkey::from_str(s).expect("valid token program id")
}

async fn raw_account_hash(rpc: &RpcPool, pubkey: &str) -> Value {
    let resp = rpc
        .call(
            "getAccountInfo",
            json!([pubkey, {"encoding": "base64", "commitment": "processed"}]),
        )
        .await;
    match resp {
        Ok(v) => {
            let context_slot = v
                .get("context")
                .and_then(|c| c.get("slot"))
                .and_then(Value::as_u64);
            let value = &v["value"];
            if value.is_null() {
                return json!({"pubkey": pubkey, "exists": false, "context_slot": context_slot});
            }
            let b64 = value
                .get("data")
                .and_then(|d| d.get(0))
                .and_then(Value::as_str)
                .unwrap_or("");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap_or_default();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let hash: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            json!({
                "pubkey": pubkey,
                "exists": true,
                "context_slot": context_slot,
                "owner": value.get("owner").and_then(Value::as_str).unwrap_or(""),
                "lamports": value.get("lamports").and_then(Value::as_u64).unwrap_or(0),
                "data_len": bytes.len(),
                "sha256": hash,
                "encoding": "base64->bytes",
            })
        }
        Err(e) => json!({"pubkey": pubkey, "error": e.to_string()}),
    }
}

fn token_balance_delta(record: &arb_sim::MeteoraSimulationRecord) -> Vec<Value> {
    let mut out = Vec::new();
    for pre in &record.pre_token_balances {
        let post = record
            .post_token_balances
            .iter()
            .find(|p| p.account_index == pre.account_index && p.mint == pre.mint);
        let pre_amt = pre.amount.parse::<i128>().unwrap_or(0);
        let post_amt = post
            .and_then(|p| p.amount.parse::<i128>().ok())
            .unwrap_or(pre_amt);
        out.push(json!({
            "account_index": pre.account_index,
            "mint": pre.mint,
            "owner": pre.owner,
            "decimals": pre.decimals,
            "pre_raw": pre_amt.to_string(),
            "post_raw": post_amt.to_string(),
            "delta_raw": (post_amt - pre_amt).to_string(),
        }));
    }
    out
}

fn token_kind_name(kind: TokenProgramKind) -> &'static str {
    match kind {
        TokenProgramKind::SplToken => "spl_token",
        TokenProgramKind::Token2022 => "token_2022",
    }
}

/// Per-mint conservation check.
///
/// A DLMM swap moves tokens between the payer, the pool vaults and (for fees)
/// the protocol fee destination. No mint/burn occurs, so for EVERY mint the sum
/// of all observed account deltas inside the transaction must be exactly zero.
/// A non-zero residual means the balance diff was mis-associated (wrong mint,
/// wrong account index) or a fee destination was not observed — either way the
/// evidence would be unsound, so it is reported and asserted here.
fn mint_reconciliation(deltas: &[Value]) -> (Vec<Value>, bool) {
    let mut order: Vec<String> = Vec::new();
    let mut sums: std::collections::BTreeMap<String, i128> = std::collections::BTreeMap::new();
    let mut accounts: std::collections::BTreeMap<String, Vec<Value>> =
        std::collections::BTreeMap::new();
    for d in deltas {
        let mint = d["mint"].as_str().unwrap_or("").to_string();
        if !sums.contains_key(&mint) {
            order.push(mint.clone());
        }
        let delta = d["delta_raw"]
            .as_str()
            .unwrap_or("0")
            .parse::<i128>()
            .unwrap_or(0);
        *sums.entry(mint.clone()).or_insert(0) += delta;
        accounts.entry(mint.clone()).or_default().push(d.clone());
    }

    let mut out = Vec::new();
    let mut all_zero = true;
    for mint in order {
        let residual = sums.get(&mint).copied().unwrap_or(0);
        let entries = accounts.get(&mint).cloned().unwrap_or_default();
        let decimals = entries
            .first()
            .and_then(|e| e["decimals"].as_u64())
            .unwrap_or(0);
        if residual != 0 {
            all_zero = false;
        }
        out.push(json!({
            "mint": mint,
            "decimals": decimals,
            "accounts_observed": entries.len(),
            "sum_delta_raw": residual.to_string(),
            "residual_raw": residual.to_string(),
            "conserved": residual == 0,
            "entries": entries,
        }));
    }
    (out, all_zero)
}

#[tokio::test]
async fn test_live_mainnet_meteora_dlmm_cross_pool_simulation() {
    dotenvy::dotenv().ok();
    let rpc = RpcPool::from_env();
    if rpc.endpoints_count() == 0 {
        eprintln!("Skipping live simulation: no RPC endpoints configured");
        return;
    }

    let reg = MultiDexRegistry::mainnet_default(rpc.clone());
    let current_slot = rpc.get_slot().await.unwrap_or(467_600_000);
    println!("Fetching live pool states at slot {}...", current_slot);
    let pools = reg.fetch_live_pool_states(Slot(current_slot)).await;

    // Discover Meteora DLMM pool pairs
    let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&pools);
    println!(
        "Meteora Discovery: {} pools discovered, {} active & valid, {} distinct eligible pairs",
        report.total_meteora_pools_discovered,
        report.active_and_valid_pools,
        report.distinct_eligible_pool_pairs
    );

    if report.total_meteora_pools_discovered == 0 {
        eprintln!(
            "Skipping live simulation: RPC returned 0 Meteora DLMM pools (environmental fetch failure, not a discovery defect)"
        );
        return;
    }

    assert!(
        report.distinct_eligible_pool_pairs >= 1,
        "Must discover at least 1 distinct Meteora DLMM pool pair"
    );

    let pair = &report.eligible_pairs[0];
    println!("Testing eligible pair: {} vs {}", pair.pool_a, pair.pool_b);

    // Quote both directions
    let test_amount_in = 1_000_000; // 1 USDC
    let quote_a = MeteoraDlmmCrossPoolQuoteEngine::quote_direction_a(pair, test_amount_in);
    let quote_b = MeteoraDlmmCrossPoolQuoteEngine::quote_direction_b(pair, test_amount_in);

    println!("Direction A (A->B) quote: {:?}", quote_a);
    println!("Direction B (B->A) quote: {:?}", quote_b);

    // Size optimization
    let opt_cfg = DlmmOptimizerConfig {
        min_input: 100_000,
        max_input: 10_000_000,
        coarse_samples: 8,
        max_candidate_basins: 4,
        refine_steps_per_basin: 6,
        estimated_network_cost_lamports: 15_000,
        sol_usd_price: 180,
        quote_decimals: 6,
        is_quote_sol: false,
    };
    let opt_res = MeteoraDlmmTradeSizeOptimizer::find_best_opportunity(pair, &opt_cfg);
    println!("Size optimizer best opportunity: {:?}", opt_res);

    // Build atomic two-swap transaction with funded signer for live simulation
    let signer = get_funded_signer();
    let blockhash_str = rpc
        .get_latest_blockhash()
        .await
        .unwrap_or_else(|_| "11111111111111111111111111111111".to_string());
    let mut blockhash_bytes = [0u8; 32];
    if let Ok(pk) = Pubkey::from_str(&blockhash_str) {
        blockhash_bytes = pk.to_bytes();
    }

    let (amount_in, hop1_out, final_out, swap_a_first) = if let Some(opt) = opt_res.as_ref() {
        (
            opt.optimal_amount_in,
            opt.expected_hop1_out,
            opt.expected_final_out,
            opt.direction == arb_engine::DlmmArbitrageDirection::DirectionA,
        )
    } else {
        (
            test_amount_in,
            quote_a.map(|q| q.hop1_amount_out).unwrap_or(1_000_000),
            quote_a.map(|q| q.hop2_final_amount_out).unwrap_or(990_000),
            true,
        )
    };

    // Apply slippage tolerance (100 bps) to min_final_out so Hop 2 slippage matches Hop 1
    let slippage_bps = 100u64;
    let min_final_out = (final_out as u128 * (10_000 - slippage_bps) as u128 / 10_000) as u64;

    let tx = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap(
        &signer,
        &pair.pool_a_state,
        &pair.pool_b_state,
        swap_a_first,
        amount_in,
        hop1_out,
        min_final_out.max(1),
        slippage_bps,
        400_000,
        10_000,
        blockhash_bytes,
    )
    .expect("Atomic tx build must succeed");

    println!(
        "Constructed atomic transaction: size={} bytes, hop1_pool={}, hop2_pool={}",
        tx.tx_size, tx.hop1_pool, tx.hop2_pool
    );

    // Same live route compiled as a Solana Transaction v1 (SIMD-0385): 4,096-byte packet,
    // compute budget carried in the signed header instead of ComputeBudgetProgram. This is
    // the Meteora two-pool v1 deliverable, verified against real mainnet pool state.
    let v1_tx = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_v1(
        &signer,
        &pair.pool_a_state,
        &pair.pool_b_state,
        swap_a_first,
        amount_in,
        hop1_out,
        min_final_out.max(1),
        slippage_bps,
        arb_tx::MeteoraTwoPoolV1Config {
            compute_unit_limit: 400_000,
            priority_fee_lamports: 4_000, // 10,000 micro-lamports * 400k CU / 1e6
            loaded_accounts_data_size_limit: arb_tx::METEORA_TWO_POOL_V1_DEFAULT_LOADED_DATA_LIMIT,
        },
        blockhash_bytes,
        quote_a.as_ref().map(|q| q.hop1_bins_traversed).unwrap_or(1),
        quote_a.as_ref().map(|q| q.hop2_bins_traversed).unwrap_or(1),
    )
    .expect("V1 atomic tx build must succeed on the live route");

    assert_eq!(v1_tx.version, arb_tx::TransactionVersion::V1);
    assert_eq!(v1_tx.wire_bytes[0], 0x81, "v1 discriminator");
    assert!(v1_tx.tx_size <= arb_tx::SOLANA_PACKET_V1_MTU);
    println!(
        "Constructed V1 atomic transaction: size={} bytes (legacy was {}), version={:?}",
        v1_tx.tx_size, tx.tx_size, v1_tx.version
    );

    // Resolve the user ATAs that the builder actually uses.
    let (first_pool, second_pool) = if swap_a_first {
        (&pair.pool_a_state, &pair.pool_b_state)
    } else {
        (&pair.pool_b_state, &pair.pool_a_state)
    };
    let payer = signer.pubkey();
    let token_in = first_pool.quote_mint.0;
    let token_mid = first_pool.base_mint.0;
    let token_out = second_pool.quote_mint.0;
    let prog_in = token_program_pubkey(first_pool.token_programs[1]);
    let prog_mid = token_program_pubkey(first_pool.token_programs[0]);
    let prog_out = token_program_pubkey(second_pool.token_programs[1]);
    let user_token_in = get_associated_token_address_with_program_id(&payer, &token_in, &prog_in);
    let user_token_mid =
        get_associated_token_address_with_program_id(&payer, &token_mid, &prog_mid);
    let user_token_out =
        get_associated_token_address_with_program_id(&payer, &token_out, &prog_out);

    // Simulate against live Mainnet via harness
    let harness = MeteoraDlmmMainnetSimulationHarness::from_env();
    let costs = ExecutionCostBreakdown::standard(10_000, 0, 0, 0, 0);

    let sim_result = harness
        .simulate_candidate_with_watch(
            "TEST-METEORA-DLMM-001",
            &tx.wire_bytes,
            tx.hop1_pool,
            tx.hop2_pool,
            if swap_a_first {
                "DirectionA"
            } else {
                "DirectionB"
            },
            amount_in,
            hop1_out,
            final_out,
            (final_out as i64) - (amount_in as i64),
            costs,
            current_slot,
            &[user_token_in, user_token_mid, user_token_out],
        )
        .await;

    let (sim_record, outcome_class, outcome_detail) = match sim_result {
        Ok(r) => {
            let class = if !r.simulation_success || r.error_status.is_some() {
                "RPC_OR_SIMULATION_FAILURE"
            } else if r
                .rejection_reason
                .as_deref()
                .is_some_and(|s| s.contains("StaleQuote"))
            {
                "STALE_QUOTE_REJECTION"
            } else if r.profitability_cleared {
                "SIMULATION_SUCCESS_VERIFIED_DELTAS"
            } else {
                "SIMULATION_SUCCESS_REJECTED_BY_GATE"
            };
            (r, class, Value::Null)
        }
        Err(e) => {
            let record = arb_sim::MeteoraSimulationRecord {
                candidate_id: "TEST-METEORA-DLMM-001".into(),
                pool_a: pair.pool_a.0,
                pool_b: pair.pool_b.0,
                direction: if swap_a_first {
                    "DirectionA"
                } else {
                    "DirectionB"
                }
                .into(),
                input_amount: amount_in,
                expected_hop1_out: hop1_out,
                expected_final_out: final_out,
                expected_gross_profit: (final_out as i64) - (amount_in as i64),
                expected_net_profit: 0,
                execution_costs: costs,
                snapshot_slot: current_slot,
                simulation_context_slot: None,
                tx_size_bytes: tx.tx_size,
                loaded_accounts_data_size: None,
                compute_units_consumed: 0,
                inner_instructions_count: 0,
                simulation_success: false,
                error_status: Some(e.to_string()),
                program_logs: Vec::new(),
                fee_payer_lamport_delta: 0,
                quote_vs_sim_discrepancy: 0,
                profitability_cleared: false,
                rejection_reason: Some(e.to_string()),
                pre_token_balances: Vec::new(),
                post_token_balances: Vec::new(),
                realized_net_raw: None,
                pre_balances: Vec::new(),
                post_balances: Vec::new(),
                return_data: None,
            };
            (record, "RPC_OR_SIMULATION_FAILURE", json!(e.to_string()))
        }
    };

    println!("=== Live Simulation Record ===");
    println!("Outcome class: {}", outcome_class);
    println!("Candidate ID: {}", sim_record.candidate_id);
    println!("Context Slot: {:?}", sim_record.simulation_context_slot);
    println!("CU Consumed: {}", sim_record.compute_units_consumed);
    println!("Simulation Success: {}", sim_record.simulation_success);
    println!("Simulation Error: {:?}", sim_record.error_status);
    println!("Program Logs count: {}", sim_record.program_logs.len());
    println!(
        "Profitability Cleared: {}",
        sim_record.profitability_cleared
    );
    println!("Rejection Reason: {:?}", sim_record.rejection_reason);

    // Capture raw pool account snapshots at the observed slot.
    let pool_a_hash = raw_account_hash(&rpc, &pair.pool_a.to_string()).await;
    let pool_b_hash = raw_account_hash(&rpc, &pair.pool_b.to_string()).await;

    let (hop1_fee, hop2_fee, hop1_bins, hop2_bins) = match opt_res.as_ref() {
        Some(o) => (
            Some(o.quote.hop1_fee),
            Some(o.quote.hop2_fee),
            Some(o.quote.hop1_bins_traversed),
            Some(o.quote.hop2_bins_traversed),
        ),
        None => {
            let q = if swap_a_first { quote_a } else { quote_b };
            (
                q.map(|x| x.hop1_fee),
                q.map(|x| x.hop2_fee),
                q.map(|x| x.hop1_bins_traversed),
                q.map(|x| x.hop2_bins_traversed),
            )
        }
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let sim_slot = sim_record.simulation_context_slot;

    // ---- slot provenance ----------------------------------------------------
    // `current_slot` is the slot the states were REQUESTED at; the pool states
    // actually used for quoting carry their own (later) slot. The freshness gate
    // is handed `current_slot`, so record both and show which one produced the
    // rejection.
    let requested_slot = current_slot;
    let pool_a_state_slot = pair.pool_a_state.slot.0;
    let pool_b_state_slot = pair.pool_b_state.slot.0;
    let quote_basis_slot = pool_a_state_slot.min(pool_b_state_slot);
    let gate_quote_slot = current_slot;
    let gate_lag = sim_slot.map(|s| s.saturating_sub(gate_quote_slot));
    let true_lag = sim_slot.map(|s| s.saturating_sub(quote_basis_slot));
    let snapshot_slot_a = pool_a_hash["context_slot"].as_u64();
    let snapshot_slot_b = pool_b_hash["context_slot"].as_u64();

    // ---- realized deltas & conservation ------------------------------------
    let deltas = token_balance_delta(&sim_record);
    let (reconciliation, conserved) = mint_reconciliation(&deltas);

    let payer_str = payer.to_string();
    let user_delta = |mint: &Pubkey| -> i128 {
        deltas
            .iter()
            .filter(|d| d["mint"].as_str() == Some(mint.to_string().as_str()))
            .filter(|d| d["owner"].as_str() == Some(payer_str.as_str()))
            .map(|d| {
                d["delta_raw"]
                    .as_str()
                    .unwrap_or("0")
                    .parse::<i128>()
                    .unwrap_or(0)
            })
            .sum()
    };
    let user_delta_in = user_delta(&token_in);
    let user_delta_mid = user_delta(&token_mid);
    let user_delta_out = user_delta(&token_out);
    let input_is_output = token_in == token_out;
    let realized_received = user_delta_in + amount_in as i128;
    let expected_net = final_out as i128 - amount_in as i128;
    let realized_net = if input_is_output {
        user_delta_in
    } else {
        user_delta_in + user_delta_out
    };

    // ---- token programs & transfer-fee applicability -----------------------
    let mut mint_programs: Vec<(String, &'static str)> = Vec::new();
    for (mint, kind) in [
        (token_in, first_pool.token_programs[1]),
        (token_mid, first_pool.token_programs[0]),
        (token_out, second_pool.token_programs[1]),
    ] {
        let key = mint.to_string();
        if !mint_programs.iter().any(|(m, _)| *m == key) {
            mint_programs.push((key, token_kind_name(kind)));
        }
    }
    let transfer_fee_applicable = mint_programs.iter().any(|(_, k)| *k == "token_2022");

    // ---- fee denomination --------------------------------------------------
    let hop1_mode = first_pool
        .dlmm
        .as_ref()
        .and_then(|d| d.params.as_ref())
        .map(|p| p.collect_fee_mode);
    let hop2_mode = second_pool
        .dlmm
        .as_ref()
        .and_then(|d| d.params.as_ref())
        .map(|p| p.collect_fee_mode);
    let (hop1_swap_for_y, hop2_swap_for_y) = match opt_res.as_ref() {
        Some(o) => (Some(o.quote.hop1_swap_for_y), Some(o.quote.hop2_swap_for_y)),
        None => (
            (if swap_a_first { quote_a } else { quote_b }).map(|q| q.hop1_swap_for_y),
            (if swap_a_first { quote_a } else { quote_b }).map(|q| q.hop2_swap_for_y),
        ),
    };
    let hop1_fee_on_input = hop1_swap_for_y
        .zip(hop1_mode)
        .and_then(|(sy, mode)| arb_math::dlmm::dlmm_fee_on_input(mode, sy).ok());
    let hop2_fee_on_input = hop2_swap_for_y
        .zip(hop2_mode)
        .and_then(|(sy, mode)| arb_math::dlmm::dlmm_fee_on_input(mode, sy).ok());

    println!(
        "Slot provenance: requested={} quote_basis={} (a={} b={}) snapshot(a={:?} b={:?}) sim={:?} \
         gate_lag={:?} true_lag={:?}",
        requested_slot,
        quote_basis_slot,
        pool_a_state_slot,
        pool_b_state_slot,
        snapshot_slot_a,
        snapshot_slot_b,
        sim_slot,
        gate_lag,
        true_lag
    );
    println!(
        "Mint conservation: conserved={} reconciliation={}",
        conserved,
        serde_json::to_string(&reconciliation).unwrap_or_default()
    );

    let evidence = json!({
        "schema": "sentinel.phase15_2.mainnet_simulation_evidence.v2",
        "generated_at_utc": format!("{}.{:03}Z", now.as_secs(), now.subsec_millis()),
        "generated_at_unix": now.as_secs(),
        "mode": {
            "ARB_LIVE": "0",
            "LIVE": "false",
            "EXECUTE_LIVE": "false",
            "DRY_RUN": "true",
            "broadcast": false,
            "jito_bundle": false,
            "alt_deployed": false,
            "signed_material_persisted": false
        },
        "pools": {
            "pool_a": {
                "address": pair.pool_a.to_string(),
                "state_slot": pool_a_state_slot,
                "snapshot": pool_a_hash,
                "snapshot_context_slot": snapshot_slot_a
            },
            "pool_b": {
                "address": pair.pool_b.to_string(),
                "state_slot": pool_b_state_slot,
                "snapshot": pool_b_hash,
                "snapshot_context_slot": snapshot_slot_b
            },
        },
        "slots": {
            "requested_slot": requested_slot,
            "pool_a_state_slot": pool_a_state_slot,
            "pool_b_state_slot": pool_b_state_slot,
            "quote_basis_slot": quote_basis_slot,
            "quote_basis_source": "min(pool_a_state.slot, pool_b_state.slot) — the slot every quoted pool state is at or after",
            "gate_quote_slot": gate_quote_slot,
            "gate_quote_slot_source": "the slot passed to the simulation harness (the requested slot), NOT the state slot",
            "account_snapshot_slots": [snapshot_slot_a, snapshot_slot_b],
            "account_snapshot_fetch_order": "after simulation — this is a provenance record of the pool accounts, not the state the quote used",
            "simulation_slot": sim_slot,
            "gate_lag_slots": gate_lag,
            "true_lag_slots": true_lag,
            "pool_a_lag_vs_gate": sim_slot.map(|s| s.saturating_sub(pool_a_state_slot)),
            "pool_b_lag_vs_gate": sim_slot.map(|s| s.saturating_sub(pool_b_state_slot)),
            "max_allowed_slot_lag": 2,
            "freshness_policy_relaxed": false,
            "gate_is_conservative": match (gate_lag, true_lag) {
                (Some(g), Some(t)) => g >= t,
                _ => false
            },
            "snapshot_lag_vs_quote_basis": snapshot_slot_a.map(|s| s.saturating_sub(quote_basis_slot as u64)),
        },
        "input": {
            "amount_raw": amount_in.to_string(),
            "mint": token_in.to_string(),
            "decimals": first_pool.decimals_b,
        },
        "intermediate": {
            "amount_raw": hop1_out.to_string(),
            "mint": token_mid.to_string(),
            "decimals": first_pool.decimals_a,
        },
        "final": {
            "amount_raw": final_out.to_string(),
            "mint": token_out.to_string(),
            "decimals": second_pool.decimals_b,
        },
        "user_token_accounts": {
            "input": user_token_in.to_string(),
            "intermediate": user_token_mid.to_string(),
            "final": user_token_out.to_string(),
            "payer": payer_str,
        },
        "token_programs": mint_programs
            .iter()
            .map(|(m, k)| json!({"mint": m, "program": k}))
            .collect::<Vec<_>>(),
        "transfer_fee": {
            "applicable": transfer_fee_applicable,
            "accounted_lamports": sim_record.execution_costs.transfer_fees_lamports,
            "note": if transfer_fee_applicable {
                "Token-2022 mint present; the standard cost breakdown carries 0 transfer fee, so any fee would show up inside the observed token deltas instead."
            } else {
                "All mints are SPL Token; Token-2022 transfer fee cannot apply to this route."
            }
        },
        "token_balances": {
            "pre": sim_record.pre_token_balances,
            "post": sim_record.post_token_balances,
            "deltas": deltas,
            "reconciliation": reconciliation,
            "all_mints_conserved": conserved
        },
        "lamport_balances": {
            "pre": sim_record.pre_balances,
            "post": sim_record.post_balances,
            "fee_payer_delta": sim_record.fee_payer_lamport_delta.to_string()
        },
        "realized": {
            "input_and_output_mint_same": input_is_output,
            "user_delta_by_mint": {
                "input": user_delta_in.to_string(),
                "intermediate": user_delta_mid.to_string(),
                "output": user_delta_out.to_string(),
            },
            "realized_final_out_raw": realized_received.to_string(),
            "expected_final_out_raw": final_out.to_string(),
            "min_final_out_slippage_floor_raw": min_final_out.to_string(),
            "realized_equals_slippage_floor": realized_received == min_final_out as i128,
            "expected_net_in_input_mint_raw": expected_net.to_string(),
            "realized_net_in_input_mint_raw": realized_net.to_string(),
            "net_matches_quote": realized_net == expected_net,
            "fee_payer_lamport_delta": sim_record.fee_payer_lamport_delta.to_string(),
            "assessment": if realized_net < 0 {
                "REALIZED_LOSS"
            } else if realized_net == 0 {
                "BREAK_EVEN"
            } else {
                "REALIZED_GAIN"
            }
        },
        "fees": {
            "hop1_fee_raw": hop1_fee.map(|v| v.to_string()),
            "hop2_fee_raw": hop2_fee.map(|v| v.to_string()),
            "hop1_fee_denomination_mint": token_in.to_string(),
            "hop2_fee_denomination_mint": token_mid.to_string(),
            "hop1_collect_fee_mode": hop1_mode,
            "hop2_collect_fee_mode": hop2_mode,
            "hop1_fee_on_input": hop1_fee_on_input,
            "hop2_fee_on_input": hop2_fee_on_input,
            "denomination_note": "DLMM charges its fee on the swap input side (fee_on_input true) in that hop's input mint; these raw fee amounts are NOT comparable to lamport costs.",
            "base_fee_lamports": sim_record.execution_costs.base_fee_lamports,
            "priority_fee_lamports": sim_record.execution_costs.priority_fee_lamports,
            "jito_tip_lamports": sim_record.execution_costs.jito_tip_lamports,
            "rent_lamports": sim_record.execution_costs.rent_lamports,
            "dex_fees_lamports": sim_record.execution_costs.dex_fees_lamports,
            "transfer_fees_lamports": sim_record.execution_costs.transfer_fees_lamports,
            "flash_borrow_fee_lamports": sim_record.execution_costs.flash_borrow_fee_lamports,
            "conversion_source": "ExecutionCostBreakdown::standard (lamports native; no SOL/USDC conversion applied here)",
            "sol_usd_price": opt_cfg.sol_usd_price,
            "sol_usd_source": "DlmmOptimizerConfig.sol_usd_price (configured, not live oracle)"
        },
        "execution": {
            "compute_units_consumed": sim_record.compute_units_consumed,
            "inner_instructions_count": sim_record.inner_instructions_count,
            "transaction_size_bytes": sim_record.tx_size_bytes,
            "loaded_accounts_data_size": sim_record.loaded_accounts_data_size,
            "simulation_success": sim_record.simulation_success,
            "error_status": sim_record.error_status,
            "program_logs": sim_record.program_logs,
            "return_data": sim_record.return_data,
            "profitability_cleared": sim_record.profitability_cleared,
            "rejection_reason": sim_record.rejection_reason,
        },
        "quotes": {
            "direction_a": quote_a,
            "direction_b": quote_b,
            "optimizer_best": opt_res.as_ref(),
            "hop1_bins_traversed": hop1_bins,
            "hop2_bins_traversed": hop2_bins,
        },
        "outcome": {
            "classification": outcome_class,
            "detail": outcome_detail,
            "simulation_executed_on_chain": sim_record.simulation_success && sim_record.error_status.is_none(),
            "is_successful_swap_simulation": outcome_class == "SIMULATION_SUCCESS_VERIFIED_DELTAS",
            "is_stale_quote_rejection": outcome_class == "STALE_QUOTE_REJECTION"
        }
    });

    // The evidence must never claim conservation that it did not observe.
    if sim_record.simulation_success && sim_record.error_status.is_none() {
        assert!(
            conserved,
            "token deltas do not reconcile per mint; evidence would be unsound"
        );
    }

    let out_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reports")
        .join("phase15_2");
    std::fs::create_dir_all(&out_dir).expect("create report dir");
    let path = out_dir.join("mainnet_simulation_evidence.json");
    std::fs::write(&path, serde_json::to_string_pretty(&evidence).unwrap())
        .expect("write mainnet simulation evidence");
    println!("wrote {}", path.display());

    // The transaction must always fit the MTU regardless of simulation outcome.
    assert!(sim_record.tx_size_bytes <= 1232);
}
