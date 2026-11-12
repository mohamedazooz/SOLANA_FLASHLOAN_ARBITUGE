//! Structured ArbError with root-cause messages and log_level.
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArbError {
    #[error("Ingest failed: pool snapshot empty (source: {src:?})")]
    IngestEmptySnapshot { src: Option<String> },
    #[error("Graph build failed: no active pools (count: {pool_count})")]
    GraphNoActivePools { pool_count: usize },
    #[error("Cycle detection failed: engine={engine} tried={tried:?}")]
    NoCyclesFound { engine: String, tried: Vec<String> },
    #[error("Sizing invalid: size={size} min={min} max_impact={max_impact_bps}")]
    SizingInvalid {
        size: u64,
        min: u64,
        max_impact_bps: u64,
    },
    #[error("Risk gate rejected: stage={stage} reason={reason} slot={slot}")]
    RiskGateRejected {
        stage: String,
        reason: String,
        slot: u64,
    },
    #[error("CU calc failed: measured={measured} base={base} buffer={buffer}")]
    CUCalculationFailed {
        measured: u32,
        base: u32,
        buffer: f64,
    },
    #[error("Flash loan fee failed: fee_sf={fee_sf} size={size}")]
    FlashLoanFeeFailed { fee_sf: u64, size: u64 },
    #[error("Slippage curve missing for pool {pool_id:?}")]
    SlippageCurveMissing { pool_id: String },
}

impl ArbError {
    pub fn log_level(&self) -> tracing::Level {
        match self {
            ArbError::IngestEmptySnapshot { .. } => tracing::Level::WARN,
            ArbError::GraphNoActivePools { .. } => tracing::Level::WARN,
            ArbError::NoCyclesFound { .. } => tracing::Level::DEBUG,
            ArbError::SizingInvalid { .. } => tracing::Level::DEBUG,
            ArbError::RiskGateRejected { .. } => tracing::Level::DEBUG,
            ArbError::CUCalculationFailed { .. } => tracing::Level::WARN,
            ArbError::FlashLoanFeeFailed { .. } => tracing::Level::WARN,
            ArbError::SlippageCurveMissing { .. } => tracing::Level::WARN,
        }
    }
}
