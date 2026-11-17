//! Full-path rehearsal: live capture → quote → size → build → simulate.
//!
//! Strictly non-broadcast: the only network side effects are read-only RPC
//! (`getMultipleAccounts`, `getBalance`) and `simulateTransaction`
//! (sigVerify:false, replaceRecentBlockhash:true). The wire bytes are never
//! passed to `sendTransaction`, Jito, or any other submission transport.
//!
//! Each sweep fetches every unique candidate pool's fresh state exactly once,
//! then evaluates BOTH the 2-pool Kamino-flash builder and the 3-leg
//! WSOL→A→B→WSOL pipeline over the same live states.
//!
//! Usage (from a directory containing .env with SOLANA_PRIVATE_KEY + RPC env):
//!   REHEARSAL_LOOPS=60 REHEARSAL_INTERVAL_SECS=10 \
//!   cargo run -p arb-sim --example full_path_rehearsal --release

use arb_engine::{
    DlmmFlashArbConfig, DlmmFlashArbEvaluator, DlmmFlashArbPool, DlmmPoolAllowlist,
    ThreeLegMemeContextParams, build_three_leg_meme_pipeline, enumerate_routes,
};
use arb_ingest::{DiscoveredPoolState, DiscoveryConfig, PoolDiscovery, RpcPool};
use arb_sim::{
    MeteoraDlmmFreshStatePipeline, MeteoraDlmmMainnetSimulationHarness, MeteoraFreshPipelineConfig,
};
use arb_tx::KeypairSigner;
use arb_tx::{
    AdaptiveTxCompiler, ExecutionMode, FlashArbParams, FlashArbPoolLeg, FormatSelectionReason,
    KaminoFlashParams, V1ConfigParams, assemble_flash_funded_two_pool, plan_bin_arrays,
    verify_atomic_integrity,
};
use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolState};
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const ANCHOR_MINTS: [&str; 3] = [
    "So11111111111111111111111111111111111111112",  // WSOL
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", // USDC
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB", // USDT
];

fn is_wsol(mint: &Pubkey) -> bool {
    WSOL.parse::<Pubkey>().as_ref() == Ok(mint)
}

fn is_anchor(mint: &Pubkey) -> bool {
    ANCHOR_MINTS
        .iter()
        .any(|a| a.parse::<Pubkey>().as_ref() == Ok(mint))
}

fn to_flash_pool(s: &PoolState) -> Option<DlmmFlashArbPool> {
    match s.execution_accounts.as_ref()? {
        PoolExecutionAccounts::MeteoraDlmm {
            reserve_x,
            reserve_y,
            oracle,
            pool_token_x,
            pool_token_y,
            ..
        } => Some(DlmmFlashArbPool {
            pool_id: s.pool_id.0,
            state: s.clone(),
            token_x_mint: MintId(*pool_token_x),
            token_y_mint: MintId(*pool_token_y),
            reserve_x: *reserve_x,
            reserve_y: *reserve_y,
            oracle: *oracle,
            token_x_program: s.token_programs[0],
            token_y_program: s.token_programs[1],
            transfer_fee_bps: [0, 0],
        }),
        _ => None,
    }
}

/// G6: live economic selection of the Kamino flash-loan reserve. Sweeps the
/// known main-market reserves, decodes each reserve plus its supply vault
/// from live bytes, and picks the best eligible reserve for `borrow_mint`:
/// active, flash loans enabled, live liquidity ≥ `min_borrow`, lowest fee
/// rate (ties → deepest liquidity). Fail-closed: no eligible reserve is an
/// error, never a silent fallback to constants.
async fn select_flash_reserve_live(
    rpc: &RpcPool,
    borrow_mint: &Pubkey,
    min_borrow: u64,
) -> Result<(arb_tx::KaminoReserveCandidate, usize), Box<dyn std::error::Error>> {
    use base64::Engine as _;
    use std::str::FromStr;
    let reserves = [
        arb_constants::KAMINO_WSOL_RESERVE,
        arb_constants::KAMINO_USDC_RESERVE,
    ];
    let accounts = rpc
        .get_multiple_accounts(&reserves)
        .await
        .map_err(|e| format!("reserve sweep failed: {e}"))?;
    let mut vault_addrs_owned: Vec<String> = Vec::new();
    let mut decoded_reserves: Vec<(usize, Vec<u8>)> = Vec::new();
    for (idx, acc) in accounts.iter().enumerate() {
        let Some(acc) = acc else {
            return Err(format!("reserve {} not found on-chain", reserves[idx]).into());
        };
        let b64 = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
            .ok_or("reserve account payload missing base64 data")?;
        let bytes = base64::prelude::BASE64_STANDARD
            .decode(b64)
            .map_err(|e| format!("reserve base64 decode failed: {e}"))?;
        let vault = arb_tx::decode_reserve_supply_vault(&bytes)
            .map_err(|e| format!("reserve {} decode failed: {e}", reserves[idx]))?;
        vault_addrs_owned.push(vault.to_string());
        decoded_reserves.push((idx, bytes));
    }
    let vault_addrs: Vec<&str> = vault_addrs_owned.iter().map(|s| s.as_str()).collect();
    let vault_accounts = rpc
        .get_multiple_accounts(&vault_addrs)
        .await
        .map_err(|e| format!("vault sweep failed: {e}"))?;
    let mut candidates = Vec::with_capacity(decoded_reserves.len());
    for (idx, bytes) in &decoded_reserves {
        let Some(vault_acc) = &vault_accounts[*idx] else {
            return Err(format!("supply vault for {} not found", reserves[*idx]).into());
        };
        let b64 = vault_acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
            .ok_or("vault account payload missing base64 data")?;
        let vault_bytes = base64::prelude::BASE64_STANDARD
            .decode(b64)
            .map_err(|e| format!("vault base64 decode failed: {e}"))?;
        let reserve = solana_pubkey::Pubkey::from_str(reserves[*idx])?;
        candidates.push(arb_tx::decode_reserve_candidate(
            reserve,
            bytes,
            &vault_bytes,
        )?);
    }
    let swept = candidates.len();
    arb_tx::select_kamino_flash_reserve(&candidates, borrow_mint, min_borrow)
        .map(|best| (best, swept))
        .map_err(|e| e.into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,arb_sim=warn,arb_ingest=warn".into()),
        )
        .init();

    let wsol: Pubkey = WSOL.parse().unwrap();
    let usdc: Pubkey = USDC.parse().unwrap();

    // ── 1. Live meme-universe discovery (read-only) ──────────────────────────
    let cfg = DiscoveryConfig {
        min_tvl_usd: 1_000.0,
        max_pools: 500,
        max_per_venue: 100,
        ..Default::default()
    };

    let discovery = PoolDiscovery::new(RpcPool::from_env());
    let pools = discovery.discover(&cfg).await;
    println!(
        "[1] discovery: {} pools registered ({} MeteoraDlmm)",
        pools.len(),
        pools
            .iter()
            .filter(|p| p.protocol == DexProtocol::MeteoraDlmm)
            .count()
    );

    // ── 2. Candidate meme/SOL DLMM pairs + meme/meme 3-leg bridge pools ──────
    let dlmm: Vec<&DiscoveredPoolState> = pools
        .iter()
        .filter(|p| p.protocol == DexProtocol::MeteoraDlmm)
        .collect();

    let mut by_meme: HashMap<(String, String), Vec<&DiscoveredPoolState>> = HashMap::new();
    let mut bridges: Vec<&DiscoveredPoolState> = Vec::new();
    for p in &dlmm {
        let (a, b) = (p.base_mint.0, p.quote_mint.0);
        if is_wsol(&a) && !is_anchor(&b) {
            by_meme
                .entry((b.to_string(), a.to_string()))
                .or_default()
                .push(p);
        } else if is_wsol(&b) && !is_anchor(&a) {
            by_meme
                .entry((a.to_string(), b.to_string()))
                .or_default()
                .push(p);
        } else if is_anchor(&a) && !is_anchor(&b) {
            // meme/USDC (or meme/USDT) pool — the shared anchor side is a
            // valid flash-loan denomination.
            by_meme
                .entry((b.to_string(), a.to_string()))
                .or_default()
                .push(p);
        } else if is_anchor(&b) && !is_anchor(&a) {
            by_meme
                .entry((a.to_string(), b.to_string()))
                .or_default()
                .push(p);
        } else if !is_anchor(&a) && !is_anchor(&b) && a != b {
            // meme/meme pool: potential middle leg A→B of a 3-leg cycle.
            bridges.push(p);
        }
    }
    bridges.sort_by(|x, y| {
        y.liquidity_usd
            .partial_cmp(&x.liquidity_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let bridge_cap: usize = std::env::var("REHEARSAL_BRIDGE_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let bridges: Vec<&DiscoveredPoolState> = bridges.into_iter().take(bridge_cap).collect();

    let mut candidates: Vec<(&(String, String), &Vec<&DiscoveredPoolState>)> =
        by_meme.iter().filter(|(_, v)| v.len() >= 2).collect();
    candidates.sort_by(|(_, va), (_, vb)| {
        let ka = va.iter().map(|p| p.liquidity_usd).fold(0.0_f64, f64::max);
        let kb = vb.iter().map(|p| p.liquidity_usd).fold(0.0_f64, f64::max);
        kb.partial_cmp(&ka).unwrap_or(std::cmp::Ordering::Equal)
    });
    let scan_cap: usize = std::env::var("REHEARSAL_SCAN_CAP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let candidates: Vec<_> = candidates.into_iter().take(scan_cap).collect();
    println!(
        "[2] meme DLMM groups with ≥2 pools sharing a quote mint: {} (scanning up to {}); meme/meme bridge pools: {}",
        by_meme.iter().filter(|(_, v)| v.len() >= 2).count(),
        candidates.len(),
        bridges.len()
    );

    // ── 2b. TOP-30 DLMM pools by liquidity (live watch record) ───────────────
    let mut top30: Vec<&DiscoveredPoolState> = dlmm.to_vec();
    top30.sort_by(|x, y| {
        y.liquidity_usd
            .partial_cmp(&x.liquidity_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    println!("[2b] TOP-30 DLMM pools by liquidity (live):");
    for (i, p) in top30.iter().take(30).enumerate() {
        println!(
            "    {:>2}. {} liq=${:<10.0} venue={:?} {}",
            i + 1,
            &p.pool_id.0.to_string()[..8],
            p.liquidity_usd,
            p.protocol,
            mint_label(&p.base_mint.0, &p.quote_mint.0)
        );
    }

    // ── 3. Signer (operator wallet; local signing only) ──────────────────────
    let secret = std::env::var("SOLANA_PRIVATE_KEY")
        .map_err(|_| "SOLANA_PRIVATE_KEY not set — cannot rehearse signing path")?;
    let signer = KeypairSigner::from_base58(&secret)?;
    println!("[3] signer ready: {}", signer.pubkey());

    let pipeline_cfg = MeteoraFreshPipelineConfig {
        max_retries: 2,
        max_allowed_slot_lag: 2,
        base_backoff_ms: 30,
        max_backoff_ms: 150,
        slippage_bps: 100,
        cu_limit: arb_constants::APPLICATION_SAFETY_CU_LIMIT,
        priority_fee_micro_lamports: arb_constants::FLASH_ARB_PRIORITY_FEE_MICRO_LAMPORTS,
        min_input_amount: 10_000_000,
        max_input_amount: 200_000_000,
        sol_usd_price: Some(180),
        requested_bin_array_depth: 1,
        two_pool_v1: true,
    };
    let rpc = Arc::new(RpcPool::from_env());
    let pipeline = MeteoraDlmmFreshStatePipeline::new(rpc.clone(), pipeline_cfg);

    // ── 4. Flash evaluation sweep across all candidate pairs ─────────────────
    let mut flash_cfg = DlmmFlashArbConfig {
        allowlist: DlmmPoolAllowlist::default(),
        min_borrow: 10_000_000,                    // 0.01 SOL
        max_borrow: 100_000_000,                   // 0.10 SOL
        kamino_available_liquidity: 5_000_000_000, // 5 SOL rehearsal ceiling
        min_profit_raw: 1,
        payer: signer.pubkey(),
        kamino: KaminoFlashParams::for_user(signer.pubkey()),
        ..Default::default()
    };
    // Optional non-WSOL borrow asset (FLASH_BORROW_MINT + decimals). The
    // Kamino reserve/vaults must be overridden too, or evaluate() fails
    // closed with FLASH_MINT_RESERVE_MISMATCH.
    match arb_engine::flash_borrow_mint_from_env_value(
        std::env::var("FLASH_BORROW_MINT").ok().as_deref(),
        std::env::var("FLASH_BORROW_MINT_DECIMALS").ok().as_deref(),
    ) {
        Ok(Some((mint, decimals))) => {
            flash_cfg.flash_mint = mint;
            flash_cfg.flash_mint_decimals = decimals;
        }
        Ok(None) => {}
        Err(e) => return Err(e.into()),
    }
    // G6: replace the constant reserve/vault/fee/liquidity block with a live
    // economic selection across the known main-market reserves. Disable with
    // FLASH_RESERVE_SELECT=0 (offline dev only).
    if std::env::var("FLASH_RESERVE_SELECT")
        .map(|v| !matches!(v.trim(), "0" | "false" | "no"))
        .unwrap_or(true)
    {
        let (best, swept) =
            select_flash_reserve_live(&rpc, &flash_cfg.flash_mint.0, flash_cfg.min_borrow).await?;
        println!(
            "[2c] live Kamino reserve selection: reserve {} mint {} decimals {} fee_sf {} liquidity {} raw ({} candidate(s) swept)",
            best.reserve,
            best.mint,
            best.decimals,
            best.flash_loan_fee_sf,
            best.available_liquidity,
            swept
        );
        flash_cfg.flash_mint_decimals = best.decimals;
        flash_cfg.kamino_flash_fee_sf = best.flash_loan_fee_sf;
        flash_cfg.kamino_available_liquidity = best.available_liquidity;
        flash_cfg.kamino = KaminoFlashParams::from_candidate(signer.pubkey(), &best);
    }

    let loops: usize = std::env::var("REHEARSAL_LOOPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let interval_secs: u64 = std::env::var("REHEARSAL_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    // ▼▼▼ LIVE SWEEP LOOP ▼▼▼
    let mut best_entry: Option<(
        (Pubkey, Pubkey),
        String,
        MintId,
        arb_engine::DlmmFlashArbEvaluation,
    )> = None;
    // Best below-floor pair (net, pool_a, pool_b, label, flash_mint, decimals):
    // used for the forced flash dry-run when nothing is feasible.
    let mut best_infeasible: Option<(i64, Pubkey, Pubkey, String, MintId, u8)> = None;
    for iteration in 0..loops {
        if iteration > 0 {
            println!(
                "--- sweep {}/{}: re-capturing fresh states ({}s interval) ---",
                iteration + 1,
                loops,
                interval_secs
            );
            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
        }

        // Fetch every unique pool's fresh state exactly once per sweep.
        let mut fresh: HashMap<Pubkey, PoolState> = HashMap::new();
        let mut fetched_slot: u64 = 0;

        for (meme, pair_pools) in &candidates {
            let pool_a = pair_pools[0].pool_id.0;
            let pool_b = pair_pools[1].pool_id.0;
            let Ok((sa, sb, slot, _, _)) =
                pipeline.fetch_fresh_pool_pair(pool_a, pool_b, None).await
            else {
                println!(
                    "    sweep {:<10} fetch failed (fail-closed)",
                    &meme.0[..meme.0.len().min(10)]
                );
                continue;
            };
            fetched_slot = fetched_slot.max(slot);
            fresh.insert(sa.pool_id.0, sa);
            fresh.insert(sb.pool_id.0, sb);
        }

        // Bridge pools: fetch pairwise through the same batched API.
        for chunk in bridges.chunks(2) {
            if chunk.len() == 2 {
                if let Ok((sa, sb, slot, _, _)) = pipeline
                    .fetch_fresh_pool_pair(chunk[0].pool_id.0, chunk[1].pool_id.0, None)
                    .await
                {
                    fetched_slot = fetched_slot.max(slot);
                    fresh.insert(sa.pool_id.0, sa);
                    fresh.insert(sb.pool_id.0, sb);
                }
            } else if let Ok((sa, _, slot, _, _)) = pipeline
                .fetch_fresh_pool_pair(chunk[0].pool_id.0, chunk[0].pool_id.0, None)
                .await
            {
                fetched_slot = fetched_slot.max(slot);
                fresh.insert(sa.pool_id.0, sa);
            }
        }

        // ── 4a. 2-pool Kamino-flash evaluation per meme pair ──
        // Flash mint candidates: WSOL and USDC — whichever side BOTH pools
        // share. Many deep meme DLMM pools are USDC-quoted, so a WSOL-only
        // sweep would never see their cross-pool edges.
        for (meme, pair_pools) in &candidates {
            let meme_label = &meme.0[..meme.0.len().min(10)];
            let pool_a = pair_pools[0].pool_id.0;
            let pool_b = pair_pools[1].pool_id.0;
            let (Some(fp_a), Some(fp_b)) = (
                fresh.get(&pool_a).and_then(to_flash_pool),
                fresh.get(&pool_b).and_then(to_flash_pool),
            ) else {
                continue;
            };
            let holds =
                |p: &DlmmFlashArbPool, m: &Pubkey| p.token_x_mint.0 == *m || p.token_y_mint.0 == *m;
            let mut flash_mints: Vec<(Pubkey, u8)> = Vec::new();
            if holds(&fp_a, &wsol) && holds(&fp_b, &wsol) {
                flash_mints.push((wsol, 9));
            }
            if holds(&fp_a, &usdc) && holds(&fp_b, &usdc) {
                flash_mints.push((usdc, 6));
            }
            for (fm, dec) in flash_mints {
                let tag = if fm == wsol {
                    "2pool-wsol"
                } else {
                    "2pool-usdc"
                };
                let pair_flash_cfg = DlmmFlashArbConfig {
                    allowlist: DlmmPoolAllowlist::new(vec![pool_a, pool_b]),
                    flash_mint: MintId(fm),
                    flash_mint_decimals: dec,
                    min_borrow: 10_000_000,
                    max_borrow: 100_000_000,
                    ..flash_cfg.clone()
                };
                let routes = enumerate_routes(
                    &[fp_a.clone(), fp_b.clone()],
                    &pair_flash_cfg.allowlist,
                    MintId(fm),
                );
                let evaluator = DlmmFlashArbEvaluator {
                    config: &pair_flash_cfg,
                };
                if let Some(r) = routes.first() {
                    let e = evaluator.evaluate(r, pair_flash_cfg.min_borrow);
                    if !e.feasible {
                        if let Some(
                            arb_engine::dlmm_flash_arbitrage::FlashArbRejection::ProfitBelowFloor {
                                net,
                                ..
                            },
                        ) = e.rejection.as_ref()
                        {
                            let cand = (
                                *net,
                                pool_a,
                                pool_b,
                                meme_label.to_string(),
                                MintId(fm),
                                dec,
                            );
                            if best_infeasible
                                .as_ref()
                                .map(|(n, _, _, _, _, _)| *net > *n)
                                .unwrap_or(true)
                            {
                                best_infeasible = Some(cand);
                            }
                        }
                    }
                    println!(
                        "    sweep {:<10} {tag}: feasible={} net={} rejection={:?}",
                        meme_label,
                        e.feasible,
                        e.net_profit,
                        e.rejection.as_ref().map(|x| x.as_str())
                    );
                    if let Some(rej) = e.rejection.as_ref() {
                        println!("      detail: {rej:?}");
                    }
                    // Size-ladder diagnostic: is ANY size close to feasible?
                    let mut ladder_best: Option<(u64, i64)> = None;
                    for size in [
                        pair_flash_cfg.min_borrow,
                        pair_flash_cfg.min_borrow.saturating_mul(5),
                        pair_flash_cfg.max_borrow,
                    ] {
                        let ev = evaluator.evaluate(r, size);
                        if ladder_best.map(|(_, n)| ev.net_profit > n).unwrap_or(true) {
                            ladder_best = Some((size, ev.net_profit));
                        }
                    }
                    if let Some((size, net)) = ladder_best {
                        println!("      ladder-best: net={net} at borrow={size}");
                    }
                }
                if let Some(found) = routes
                    .iter()
                    .filter_map(|r| evaluator.optimize(r))
                    .filter(|e| e.feasible)
                    .max_by_key(|e| e.net_profit)
                {
                    best_entry =
                        Some(((pool_a, pool_b), meme_label.to_string(), MintId(fm), found));
                    break;
                }
            }
            if best_entry.is_some() {
                break;
            }
        }

        // ── 4b. 3-leg WSOL→A→B→WSOL evaluation over the whole live set ──
        if best_entry.is_none() && fresh.len() >= 3 {
            let all_states: Vec<PoolState> = fresh.values().cloned().collect();
            let params = ThreeLegMemeContextParams {
                snapshot_slot: fetched_slot,
                cu_limit: arb_constants::COMPUTE_BUDGET_LIMIT_3HOP,
                priority_fee_micro_lamports: flash_cfg.priority_fee_micro_lamports,
                jito_tip_account: arb_constants::jito_tip_account_at(0),
                jito_tip_lamports: arb_constants::DEFAULT_JITO_TIP_FLOOR_LAMPORTS,
                slippage_bps: 100,
                min_profit_lamports: 1,
                flash_loan_fee_sf: flash_cfg.kamino_flash_fee_sf,
                network_cost_lamports: flash_cfg.network_cost_lamports,
                input_size: 10_000_000,
                size_search_sizes: vec![10_000_000, 30_000_000, 100_000_000],
                ..ThreeLegMemeContextParams::for_discovery(signer.pubkey())
            };
            match build_three_leg_meme_pipeline(&all_states, &params) {
                Ok(r) => {
                    println!(
                        "    sweep 3leg: FEASIBLE pools {:?} mints [{}, {}] net={} raw (gross={} flash_fee={}) best_input={}",
                        [
                            &r.pool_ids[0].to_string()[..8],
                            &r.pool_ids[1].to_string()[..8],
                            &r.pool_ids[2].to_string()[..8]
                        ],
                        &r.meme_mints[0].0.to_string()[..6],
                        &r.meme_mints[1].0.to_string()[..6],
                        r.net_profit_lamports,
                        r.gross_profit_lamports,
                        r.flash_fee_lamports,
                        r.size_search.best_input
                    );
                    println!(
                        "    sweep 3leg: NOTE — 3-leg build requires the provisioned StaticMasterALT; evaluation recorded, build deferred to the supervisor path."
                    );
                }
                Err(e) => {
                    println!(
                        "    sweep 3leg: no cycle ({e}) | {}",
                        arb_engine::diagnose_missing_cycles(&all_states)
                    );
                }
            }
        }

        if best_entry.is_some() {
            break;
        }
    }
    // ▲▲▲ END LIVE SWEEP LOOP ▲▲▲

    let ((pool_a, pool_b), meme, flash_mint, best) = match best_entry {
        Some(v) => v,
        None => {
            println!(
                "[5] NO feasible flash route across all {} scanned pairs in {} sweeps -- markets are efficient at this instant (fail-closed, nothing built, nothing broadcast).",
                candidates.len(),
                loops
            );
            let Some((net, pa, pb, label, fm, dec)) = best_infeasible else {
                println!(
                    "[5b] no below-floor pair with a complete quote either -- nothing to dry-run."
                );
                return Ok(());
            };
            println!(
                "[5b] FORCED FLASH DRY-RUN on {label} (best net {net} raw, below floor): build + fork simulation only, never broadcast."
            );
            // Fresh states + fresh evaluation so the built transaction matches
            // the chain at simulation time, not at sweep time.
            let Ok((state_a, state_b, _, _, _)) =
                pipeline.fetch_fresh_pool_pair(pa, pb, None).await
            else {
                return Err("dry-run refetch failed".into());
            };
            let fp_a = to_flash_pool(&state_a).ok_or("missing execution accounts A")?;
            let fp_b = to_flash_pool(&state_b).ok_or("missing execution accounts B")?;
            let pair_cfg = DlmmFlashArbConfig {
                allowlist: DlmmPoolAllowlist::new(vec![pa, pb]),
                flash_mint: fm,
                flash_mint_decimals: dec,
                min_borrow: 10_000_000,
                max_borrow: 100_000_000,
                ..flash_cfg.clone()
            };
            let routes = enumerate_routes(&[fp_a, fp_b], &pair_cfg.allowlist, fm);
            let route = routes
                .iter()
                .find(|r| r.flash_mint == fm)
                .ok_or("dry-run route vanished on refetch")?;
            let evaluator = DlmmFlashArbEvaluator { config: &pair_cfg };
            let e = evaluator.evaluate(route, pair_cfg.min_borrow);
            if !e.feasible && e.hop_a_min_out == 0 {
                println!(
                    "[5b] dry-run aborted: fresh evaluation has no complete quote context ({:?})",
                    e.rejection.as_ref().map(|x| x.as_str())
                );
                return Ok(());
            }
            let repay = e.borrow_amount.saturating_add(e.flash_fee);
            if e.hop_b_min_out < repay {
                println!(
                    "[5b] dry-run verdict: the builder gate refuses this transaction -- conservative final output {} < repayment {} raw. The flash loan cannot be repaid at the slippage floor: the route is economically negative at the quote level itself. Fail-closed confirmed at the last line of defense; nothing built, nothing broadcast.",
                    e.hop_b_min_out, repay
                );
                return Ok(());
            }
            ((pa, pb), label, fm, e)
        }
    };
    println!(
        "[5] best flash route: meme={} pools {}+{} borrow={} raw net_profit={} raw ({} bps) flash_fee={} bins_a={} bins_b={}",
        meme,
        pool_a,
        pool_b,
        best.borrow_amount,
        best.net_profit,
        best.net_profit_bps,
        best.flash_fee,
        best.bins_a,
        best.bins_b
    );

    // ── 6. Re-fetch fresh states and assemble the flash transaction ──────────
    let (state_a, state_b, fresh_slot, _, _) =
        pipeline.fetch_fresh_pool_pair(pool_a, pool_b, None).await?;
    let fp_a = to_flash_pool(&state_a).ok_or("missing execution accounts A")?;
    let fp_b = to_flash_pool(&state_b).ok_or("missing execution accounts B")?;
    let allowlist = DlmmPoolAllowlist::new(vec![pool_a, pool_b]);
    let routes = enumerate_routes(&[fp_a, fp_b], &allowlist, flash_mint);
    let route = routes
        .iter()
        .find(|r| r.flash_mint == flash_mint)
        .ok_or("route vanished on refetch")?;

    let leg_build = |pool: &DlmmFlashArbPool,
                     swap_for_y: bool,
                     amount_in: u64,
                     min_amount_out: u64,
                     bins_traversed: u32|
     -> Result<FlashArbPoolLeg, Box<dyn std::error::Error>> {
        let active_bin_id = pool
            .state
            .dlmm
            .as_ref()
            .map(|d| d.active_bin_id)
            .ok_or("missing dlmm state")?;
        let plan = plan_bin_arrays(
            &pool.pool_id,
            active_bin_id,
            swap_for_y,
            bins_traversed.max(1),
        )
        .map_err(|e| format!("bin-array plan failed: {e:?}"))?;
        Ok(FlashArbPoolLeg {
            lb_pair: pool.pool_id,
            reserve_x: pool.reserve_x,
            reserve_y: pool.reserve_y,
            token_x_mint: pool.token_x_mint.0,
            token_y_mint: pool.token_y_mint.0,
            oracle: pool.oracle,
            token_x_program: pool.token_x_program.program_id(),
            token_y_program: pool.token_y_program.program_id(),
            swap_for_y,
            amount_in,
            min_amount_out,
            bin_array_pdas: plan.pdas,
            bitmap_extension: plan
                .requires_bitmap_extension
                .then_some(plan.bitmap_extension_pda),
            host_fee_in: None,
        })
    };

    let swap_a_for_y = route
        .pool_a
        .swap_for_y(&route.flash_mint, &route.mid_mint)
        .ok_or("leg A cannot trade flash→mid")?;
    let swap_b_for_y = route
        .pool_b
        .swap_for_y(&route.mid_mint, &route.flash_mint)
        .ok_or("leg B cannot trade mid→flash")?;

    let leg_a = leg_build(
        &route.pool_a,
        swap_a_for_y,
        best.borrow_amount,
        best.hop_a_min_out,
        best.bins_a,
    )?;
    let leg_b = leg_build(
        &route.pool_b,
        swap_b_for_y,
        best.hop_a_min_out,
        best.hop_b_min_out,
        best.bins_b,
    )?;

    let params = FlashArbParams {
        payer: signer.pubkey(),
        kamino: flash_cfg.kamino.clone(),
        borrow_amount: best.borrow_amount,
        flash_fee: best.flash_fee,
        leg_a,
        leg_b,
        cu_limit: flash_cfg.cu_limit,
        priority_fee_micro_lamports: flash_cfg.priority_fee_micro_lamports,
    };

    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_two_pool(&params)?;
    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )?;
    println!(
        "[6] atomicity self-check: is_atomic={} reason={:?}",
        verification.is_atomic, verification.rejection_reason
    );
    if !verification.is_atomic {
        return Err("flash transaction failed the atomicity self-check (fail-closed)".into());
    }

    // ── 7. Sign locally (dummy blockhash; sim replaces it) and simulate ──────
    let priority_fee_lamports = ((flash_cfg.priority_fee_micro_lamports as u128)
        .saturating_mul(flash_cfg.cu_limit as u128)
        .saturating_add(999_999))
        / 1_000_000;
    let v1_cfg = V1ConfigParams {
        priority_fee_lamports: Some(u64::try_from(priority_fee_lamports).unwrap_or(u64::MAX)),
        compute_unit_limit: Some(flash_cfg.cu_limit),
        loaded_accounts_data_size_limit: Some(8_388_608),
        heap_size: Some(256 * 1024),
    };
    let accounts_count = {
        let mut keys: Vec<Pubkey> = ixs
            .iter()
            .flat_map(|ix| ix.accounts.iter().map(|a| a.pubkey))
            .chain(std::iter::once(signer.pubkey()))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys.len()
    };
    let compiled = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &ixs,
        [1u8; 32],
        v1_cfg,
        FormatSelectionReason::LargePayloadV1 {
            accounts_count,
            wire_size: 0,
        },
    )?;
    println!(
        "[7] flash transaction built: {} bytes (V1), {} instructions, borrow_idx={}, repay_idx={}, repay={} raw",
        compiled.wire_bytes.len(),
        ixs.len(),
        borrow_idx,
        repay_idx,
        params.repay_amount()
    );

    let flash_mint = route.flash_mint.0;
    let mid_mint = route.mid_mint.0;
    let flash_program = if swap_a_for_y {
        route.pool_a.token_x_program.program_id()
    } else {
        route.pool_a.token_y_program.program_id()
    };
    let mid_program = if swap_a_for_y {
        route.pool_a.token_y_program.program_id()
    } else {
        route.pool_a.token_x_program.program_id()
    };
    let settlement = arb_tx::get_associated_token_address_with_program_id(
        &signer.pubkey(),
        &flash_mint,
        &flash_program,
    );
    let intermediate = arb_tx::get_associated_token_address_with_program_id(
        &signer.pubkey(),
        &mid_mint,
        &mid_program,
    );

    let harness = MeteoraDlmmMainnetSimulationHarness::from_env();
    let costs = arb_sim::ExecutionCostBreakdown::standard(
        u64::try_from(priority_fee_lamports).unwrap_or(u64::MAX),
        0,
        0,
        0,
        best.flash_fee,
    );
    let sim = harness
        .simulate_candidate_with_watch(
            "flash_rehearsal",
            &compiled.wire_bytes,
            pool_a,
            pool_b,
            "flash_two_pool",
            best.borrow_amount,
            best.hop_a_out,
            best.hop_b_out,
            best.net_profit,
            costs,
            fresh_slot,
            &[settlement, intermediate],
        )
        .await?;

    let token_delta = |idx: u8| -> Option<i128> {
        let pre = sim
            .pre_token_balances
            .iter()
            .find(|b| b.account_index == idx)
            .and_then(|b| b.amount.parse::<i128>().ok())
            .unwrap_or(0);
        let post = sim
            .post_token_balances
            .iter()
            .find(|b| b.account_index == idx)
            .and_then(|b| b.amount.parse::<i128>().ok())
            .unwrap_or(0);
        if sim.pre_token_balances.is_empty() && sim.post_token_balances.is_empty() {
            return None;
        }
        Some(post - pre)
    };

    println!("[8] flash simulation record:");
    println!("    success        = {}", sim.simulation_success);
    println!("    error          = {:?}", sim.error_status);
    println!("    cu_consumed    = {}", sim.compute_units_consumed);
    println!("    loaded_data    = {:?}", sim.loaded_accounts_data_size);
    println!(
        "    context_slot   = {:?} (quote slot {fresh_slot})",
        sim.simulation_context_slot
    );
    println!(
        "    token_deltas   = settlement {:?}, intermediate {:?}",
        token_delta(0),
        token_delta(1)
    );
    println!(
        "    realized_net   = {:?} (expected net {})",
        sim.realized_net_raw, sim.expected_net_profit
    );
    for log in sim.program_logs.iter().take(15) {
        println!("    log: {log}");
    }
    println!(
        "NOTE: flash path rehearsed up to simulation only — borrow/swap/repay were never broadcast."
    );

    Ok(())
}

/// Short human-readable pair label for the watch record.
fn mint_label(a: &Pubkey, b: &Pubkey) -> String {
    let short = |m: &Pubkey| {
        let s = m.to_string();
        s[..6].to_string()
    };
    format!("{}/{}", short(a), short(b))
}
