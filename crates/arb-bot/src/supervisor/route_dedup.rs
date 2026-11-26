//! Canonical route identity for merging candidates from independent discovery paths.

use arb_types::{ExecutionOpportunity, MintId, PoolId, PumpSwapTradeIntent, SwapHop};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HopSignature {
    pool_id: PoolId,
    protocol: arb_types::DexProtocol,
    mint_in: MintId,
    mint_out: MintId,
    trade_intent: u8,
}

/// A route key preserves the ordered, directed execution legs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RouteSignature(Vec<HopSignature>);

impl RouteSignature {
    fn from_hops(hops: &[SwapHop]) -> Self {
        Self(
            hops.iter()
                .map(|hop| HopSignature {
                    pool_id: hop.pool_id,
                    protocol: hop.protocol,
                    mint_in: hop.mint_in,
                    mint_out: hop.mint_out,
                    trade_intent: match hop.trade_intent {
                        PumpSwapTradeIntent::ExactIn => 0,
                        PumpSwapTradeIntent::ExactOutBase => 1,
                    },
                })
                .collect(),
        )
    }
}

/// Deduplicate the same directed route while retaining its highest-net-profit quote.
///
/// Empty routes are kept as-is: they are invalid candidates and should be rejected by
/// their normal validation gates, not accidentally collapsed into one another here.
pub(super) fn dedupe_best_by_route(
    opportunities: impl IntoIterator<Item = ExecutionOpportunity>,
) -> Vec<ExecutionOpportunity> {
    let mut unique = Vec::new();
    let mut positions = HashMap::<RouteSignature, usize>::new();

    for opportunity in opportunities {
        if opportunity.hops.is_empty() {
            unique.push(opportunity);
            continue;
        }

        let signature = RouteSignature::from_hops(&opportunity.hops);
        if let Some(&index) = positions.get(&signature) {
            if opportunity.net_profit.0 > unique[index].net_profit.0 {
                unique[index] = opportunity;
            }
        } else {
            positions.insert(signature, unique.len());
            unique.push(opportunity);
        }
    }

    unique
}

#[cfg(test)]
mod tests {
    use super::{RouteSignature, dedupe_best_by_route};
    use arb_types::{
        Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, PumpSwapTradeIntent,
        RouteId, Slot, SwapHop, TokenAmount, TokenProgramKind,
    };
    use solana_pubkey::Pubkey;

    fn hop(pool: u8, protocol: DexProtocol, mint_in: u8, mint_out: u8) -> SwapHop {
        SwapHop {
            pool_id: PoolId(Pubkey::new_from_array([pool; 32])),
            protocol,
            mint_in: MintId(Pubkey::new_from_array([mint_in; 32])),
            mint_out: MintId(Pubkey::new_from_array([mint_out; 32])),
            fee_bps: 0,
            active_bin_id: None,
            token_programs: [TokenProgramKind::SplToken; 2],
            trade_intent: PumpSwapTradeIntent::ExactIn,
            execution_accounts: None,
        }
    }

    fn opportunity(hops: Vec<SwapHop>, route_id: u32, net_profit: u64) -> ExecutionOpportunity {
        ExecutionOpportunity {
            route_id: RouteId(route_id),
            slot: Slot(1),
            size_in: TokenAmount(10),
            expected_out: TokenAmount(11),
            hop_inputs: Vec::new(),
            gross_profit: Lamports(net_profit),
            net_profit: Lamports(net_profit),
            base_decimals: 9,
            price_impact_bps: Bps(0),
            hops,
            expected_min_final_output: 0,
            hop_slippage_bps: 0,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
        }
    }

    #[test]
    fn signature_preserves_direction_and_hop_order() {
        let forward = vec![
            hop(1, DexProtocol::MeteoraDlmm, 10, 20),
            hop(2, DexProtocol::RaydiumCpmm, 20, 10),
        ];
        let reverse = vec![
            hop(2, DexProtocol::RaydiumCpmm, 10, 20),
            hop(1, DexProtocol::MeteoraDlmm, 20, 10),
        ];

        assert_ne!(
            RouteSignature::from_hops(&forward),
            RouteSignature::from_hops(&reverse),
        );
    }

    #[test]
    fn signature_preserves_three_hop_order() {
        let first = vec![
            hop(1, DexProtocol::MeteoraDlmm, 10, 20),
            hop(2, DexProtocol::RaydiumCpmm, 20, 30),
            hop(3, DexProtocol::OrcaWhirlpool, 30, 10),
        ];
        let reordered = vec![first[1].clone(), first[0].clone(), first[2].clone()];

        assert_ne!(
            RouteSignature::from_hops(&first),
            RouteSignature::from_hops(&reordered),
        );
    }

    #[test]
    fn duplicate_route_keeps_highest_net_profit_candidate() {
        let route = vec![
            hop(1, DexProtocol::MeteoraDlmm, 10, 20),
            hop(2, DexProtocol::RaydiumCpmm, 20, 10),
        ];
        let candidates = vec![opportunity(route.clone(), 1, 50), opportunity(route, 2, 75)];

        let deduped = dedupe_best_by_route(candidates);

        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].route_id, RouteId(2));
        assert_eq!(deduped[0].net_profit, Lamports(75));
    }

    #[test]
    fn reverse_routes_using_the_same_pool_set_are_both_retained() {
        let forward = opportunity(
            vec![
                hop(1, DexProtocol::MeteoraDlmm, 10, 20),
                hop(2, DexProtocol::RaydiumCpmm, 20, 10),
            ],
            1,
            50,
        );
        let reverse = opportunity(
            vec![
                hop(2, DexProtocol::RaydiumCpmm, 10, 20),
                hop(1, DexProtocol::MeteoraDlmm, 20, 10),
            ],
            2,
            40,
        );

        assert_eq!(dedupe_best_by_route([forward, reverse]).len(), 2);
    }

    #[test]
    fn empty_routes_are_not_collapsed() {
        let candidates = vec![opportunity(Vec::new(), 1, 0), opportunity(Vec::new(), 2, 0)];

        assert_eq!(dedupe_best_by_route(candidates).len(), 2);
    }
}
