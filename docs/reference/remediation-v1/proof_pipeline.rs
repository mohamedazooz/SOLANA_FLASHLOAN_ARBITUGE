use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mint(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitAmount {
    pub mint: Mint,
    pub decimals: u8,
    pub atoms: u128,
}

impl UnitAmount {
    pub fn checked_add(self, rhs: Self) -> Result<Self, GateDecision> {
        self.ensure_same_asset(rhs)?;
        Ok(Self { atoms: self.atoms.checked_add(rhs.atoms).ok_or(GateDecision::ArithmeticOverflow)?, ..self })
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, GateDecision> {
        self.ensure_same_asset(rhs)?;
        Ok(Self { atoms: self.atoms.checked_sub(rhs.atoms).ok_or(GateDecision::NegativeNetProfit)?, ..self })
    }

    pub fn ensure_same_asset(self, rhs: Self) -> Result<(), GateDecision> {
        if self.mint != rhs.mint || self.decimals != rhs.decimals {
            return Err(GateDecision::AssetUnitMismatch);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub slot: u64,
    pub observed_at_ms: u64,
    pub max_age_slots: u64,
}

impl Snapshot {
    pub fn is_fresh_at(self, current_slot: u64) -> bool {
        current_slot >= self.slot && current_slot - self.slot <= self.max_age_slots
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostKind {
    FlashLoan,
    DexFee,
    TransferFee,
    AccountCreation,
    NetworkFee,
    PriorityFee,
    ValidatorTip,
    OtherVerified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostItem {
    pub kind: CostKind,
    pub amount: UnitAmount,
    /// Stable source label, e.g. `kamino-reserve:<pubkey>@slot:<n>`.
    pub source_id: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostLedger {
    pub base_mint: Mint,
    pub base_decimals: u8,
    pub items: Vec<CostItem>,
}

impl CostLedger {
    pub fn total(&self) -> Result<UnitAmount, GateDecision> {
        let mut total = UnitAmount { mint: self.base_mint, decimals: self.base_decimals, atoms: 0 };
        let mut seen = BTreeSet::new();
        for item in &self.items {
            if item.source_id.is_empty() || !seen.insert((item.kind as u8, item.source_id)) {
                return Err(GateDecision::DuplicateOrUnprovenCost);
            }
            if item.amount.mint != self.base_mint || item.amount.decimals != self.base_decimals {
                return Err(GateDecision::AssetUnitMismatch);
            }
            total = total.checked_add(item.amount)?;
        }
        Ok(total)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulationStatus {
    Passed,
    NotRun,
    Disabled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolLegQuote {
    pub pool_id: [u8; 32],
    /// Legs in one phase execute in parallel and share the same mint pair.
    pub phase: u8,
    pub protocol: &'static str,
    pub mint_in: Mint,
    pub mint_out: Mint,
    pub amount_in: u128,
    pub expected_out: u128,
    pub min_out: u128,
    pub fee_bps: u16,
    pub state_slot: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opportunity {
    pub base_mint: Mint,
    pub base_decimals: u8,
    pub borrow_amount: UnitAmount,
    pub expected_final_amount: UnitAmount,
    pub min_profit: UnitAmount,
    pub legs: Vec<PoolLegQuote>,
    pub snapshot: Snapshot,
    pub costs: CostLedger,
    pub simulation: SimulationStatus,
    /// Independently computed from the aggregate re-quote, never averaged from
    /// the pre-split route's impact or inferred by dividing it by leg count.
    pub aggregate_impact_bps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateConfig {
    pub current_slot: u64,
    pub max_aggregate_impact_bps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    Approved { net_profit_atoms: u128 },
    EmptyRoute,
    StaleSnapshot,
    InvalidLeg,
    DuplicatePool,
    RouteDoesNotClose,
    AssetUnitMismatch,
    DuplicateOrUnprovenCost,
    SimulationNotProven,
    ImpactTooHigh,
    NegativeNetProfit,
    BelowMinimumProfit,
    ArithmeticOverflow,
}

pub fn evaluate_opportunity(opp: &Opportunity, cfg: GateConfig) -> GateDecision {
    if opp.legs.is_empty() { return GateDecision::EmptyRoute; }
    if !opp.snapshot.is_fresh_at(cfg.current_slot) { return GateDecision::StaleSnapshot; }
    if opp.simulation != SimulationStatus::Passed { return GateDecision::SimulationNotProven; }
    if opp.aggregate_impact_bps > cfg.max_aggregate_impact_bps { return GateDecision::ImpactTooHigh; }

    if opp.borrow_amount.mint != opp.base_mint
        || opp.expected_final_amount.mint != opp.base_mint
        || opp.min_profit.mint != opp.base_mint
        || opp.costs.base_mint != opp.base_mint
        || opp.borrow_amount.decimals != opp.base_decimals
        || opp.expected_final_amount.decimals != opp.base_decimals
        || opp.min_profit.decimals != opp.base_decimals
        || opp.costs.base_decimals != opp.base_decimals
    { return GateDecision::AssetUnitMismatch; }

    let mut pools = BTreeSet::new();
    let mut index = 0usize;
    let mut expected_phase = 0u8;
    let mut previous_output: Option<(Mint, u128)> = None;
    let mut final_mint = None;

    while index < opp.legs.len() {
        let first = &opp.legs[index];
        if first.phase != expected_phase { return GateDecision::RouteDoesNotClose; }
        let phase_in = first.mint_in;
        let phase_out = first.mint_out;
        let mut phase_input = 0u128;
        let mut phase_output = 0u128;

        while index < opp.legs.len() && opp.legs[index].phase == expected_phase {
            let leg = &opp.legs[index];
            if leg.mint_in != phase_in || leg.mint_out != phase_out
                || leg.amount_in == 0 || leg.expected_out == 0
                || leg.min_out > leg.expected_out || leg.fee_bps >= 10_000
                || leg.state_slot > opp.snapshot.slot
            { return GateDecision::InvalidLeg; }
            if !pools.insert(leg.pool_id) { return GateDecision::DuplicatePool; }
            phase_input = match phase_input.checked_add(leg.amount_in) {
                Some(v) => v,
                None => return GateDecision::ArithmeticOverflow,
            };
            phase_output = match phase_output.checked_add(leg.expected_out) {
                Some(v) => v,
                None => return GateDecision::ArithmeticOverflow,
            };
            index += 1;
        }

        match previous_output {
            Some((expected_mint, expected_atoms)) => {
                if phase_in != expected_mint || phase_input != expected_atoms {
                    return GateDecision::RouteDoesNotClose;
                }
            }
            None => {
                if phase_in != opp.base_mint || phase_input != opp.borrow_amount.atoms {
                    return GateDecision::RouteDoesNotClose;
                }
            }
        }
        previous_output = Some((phase_out, phase_output));
        final_mint = Some(phase_out);
        expected_phase = match expected_phase.checked_add(1) {
            Some(v) => v,
            None => return GateDecision::ArithmeticOverflow,
        };
    }
    if final_mint != Some(opp.base_mint)
        || previous_output.map(|(_, atoms)| atoms) != Some(opp.expected_final_amount.atoms)
    { return GateDecision::RouteDoesNotClose; }

    let gross = match opp.expected_final_amount.atoms.checked_sub(opp.borrow_amount.atoms) {
        Some(v) => v,
        None => return GateDecision::NegativeNetProfit,
    };
    let total_costs = match opp.costs.total() {
        Ok(v) => v.atoms,
        Err(e) => return e,
    };
    let net = match gross.checked_sub(total_costs) {
        Some(v) => v,
        None => return GateDecision::NegativeNetProfit,
    };
    if net < opp.min_profit.atoms { return GateDecision::BelowMinimumProfit; }
    GateDecision::Approved { net_profit_atoms: net }
}
