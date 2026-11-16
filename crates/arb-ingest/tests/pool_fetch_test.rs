use arb_ingest::{MultiDexRegistry, RpcPool};
use arb_types::Slot;

#[tokio::test]
async fn test_live_pool_fetch() {
    dotenvy::dotenv().ok();
    let rpc = RpcPool::from_env();
    let reg = MultiDexRegistry::mainnet_default(rpc);
    println!("Registered pools: {}", reg.pool_count());
    let pools = reg.fetch_live_pool_states(Slot(467229372)).await;
    println!("Fetched live pool states count: {}", pools.len());
    for p in &pools {
        println!(
            "Pool: {} ({:?}) in: {} out: {} fee: {}",
            p.pool_id, p.protocol, p.reserve_in, p.reserve_out, p.fee_numerator
        );
    }
}
