//! Protocol Quote Engines Module.

pub mod meteora_dlmm;
pub mod orca_whirlpool;
pub mod pump_bonding_curve;
pub mod pump_swap;
pub mod raydium_clmm;
pub mod raydium_cpmm;
pub mod raydium_v4;

pub use meteora_dlmm::MeteoraDlmmQuoteEngine;
pub use orca_whirlpool::OrcaWhirlpoolQuoteEngine;
pub use pump_bonding_curve::PumpBondingCurveQuoteEngine;
pub use pump_swap::PumpSwapQuoteEngine;
pub use raydium_clmm::RaydiumClmmQuoteEngine;
pub use raydium_cpmm::RaydiumCpmmQuoteEngine;
pub use raydium_v4::RaydiumAmmV4QuoteEngine;

use crate::quote_engine::{QuoteEngine, QuoteRequest, QuoteResult};
use arb_types::{ArbError, DexProtocol, PoolState, Result};

/// Unified Quote Dispatcher delegating to the appropriate independent mathematical engine.
#[derive(Debug, Default, Clone)]
pub struct UniversalQuoteDispatcher {
    pub dlmm: MeteoraDlmmQuoteEngine,
    pub cpmm: RaydiumCpmmQuoteEngine,
    pub raydium_v4: RaydiumAmmV4QuoteEngine,
    pub raydium_clmm: RaydiumClmmQuoteEngine,
    pub orca: OrcaWhirlpoolQuoteEngine,
    pub pump_swap: PumpSwapQuoteEngine,
    pub pump_curve: PumpBondingCurveQuoteEngine,
}

impl UniversalQuoteDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Quote exact-in routing to the correct independent protocol engine.
    pub fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        match state.protocol {
            DexProtocol::MeteoraDlmm => self.dlmm.quote_exact_in(state, request),
            DexProtocol::RaydiumCpmm => self.cpmm.quote_exact_in(state, request),
            DexProtocol::RaydiumV4 => self.raydium_v4.quote_exact_in(state, request),
            DexProtocol::RaydiumClmm => self.raydium_clmm.quote_exact_in(state, request),
            DexProtocol::OrcaWhirlpool => self.orca.quote_exact_in(state, request),
            DexProtocol::PumpFun => {
                if let Some(p) = &state.pump_fun {
                    if p.complete {
                        self.pump_swap.quote_exact_in(state, request)
                    } else {
                        self.pump_curve.quote_exact_in(state, request)
                    }
                } else {
                    self.pump_curve.quote_exact_in(state, request)
                }
            }
            DexProtocol::PumpSwap => self.pump_swap.quote_exact_in(state, request),
            other => Err(ArbError::InvalidInput(format!(
                "Protocol {other:?} quote_exact_in not supported by dispatcher"
            ))),
        }
    }

    /// Quote exact-out routing to the correct independent protocol engine.
    pub fn quote_exact_out(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        match state.protocol {
            DexProtocol::MeteoraDlmm => self.dlmm.quote_exact_out(state, request),
            DexProtocol::RaydiumCpmm => self.cpmm.quote_exact_out(state, request),
            DexProtocol::RaydiumV4 => self.raydium_v4.quote_exact_out(state, request),
            DexProtocol::RaydiumClmm => self.raydium_clmm.quote_exact_out(state, request),
            DexProtocol::OrcaWhirlpool => self.orca.quote_exact_out(state, request),
            DexProtocol::PumpFun => self.pump_curve.quote_exact_out(state, request),
            DexProtocol::PumpSwap => self.pump_swap.quote_exact_out(state, request),
            other => Err(ArbError::InvalidInput(format!(
                "Protocol {other:?} quote_exact_out not supported by dispatcher"
            ))),
        }
    }
}
