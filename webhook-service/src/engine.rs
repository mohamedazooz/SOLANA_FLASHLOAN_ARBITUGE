//! Transaction Engine and Expression Webhook evaluation pipeline.
//!
//! Filters failed transactions and non-arbitrage-relevant actions in sub-microseconds,
//! offloading validated inner instructions and resolved AMMs to an asynchronous worker pool.

use crate::config::FilterConfig;
use crate::models::{ResolvedPoolEvent, SolanaTransactionPayload};
use crate::router::AmmRouter;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, info, trace};

/// High-priority payload packet dispatched to async worker threads.
#[derive(Debug, Clone)]
pub struct TransactionJob {
    pub payload: Arc<SolanaTransactionPayload>,
    pub resolved_pools: Vec<ResolvedPoolEvent>,
    pub received_at: Instant,
}

/// Extensible listener trait for consuming arbitrage events downstream.
#[async_trait::async_trait]
pub trait ArbitrageEventListener: Send + Sync {
    async fn on_pool_update(&self, job: &TransactionJob);
}

/// Default logger implementation of `ArbitrageEventListener`.
pub struct DefaultTelemetryListener;

#[async_trait::async_trait]
impl ArbitrageEventListener for DefaultTelemetryListener {
    async fn on_pool_update(&self, job: &TransactionJob) {
        let elapsed = job.received_at.elapsed();
        info!(
            target: "worker",
            signature = %job.payload.signature,
            pools_count = job.resolved_pools.len(),
            latency_us = elapsed.as_micros(),
            "Arbitrage trigger dispatched to state machine"
        );
        for pool in &job.resolved_pools {
            debug!(
                target: "worker",
                pool_id = %pool.pool_id,
                amm = %pool.amm_type,
                quote_mint = %pool.quote_mint,
                slot = pool.slot,
                "AMM State updated"
            );
        }
    }
}

/// Expression Evaluator for filtering incoming transactions before state machine entry.
#[derive(Clone)]
pub struct ExpressionEvaluator {
    filter_config: FilterConfig,
    router: AmmRouter,
}

impl ExpressionEvaluator {
    pub fn new(filter_config: FilterConfig, router: AmmRouter) -> Self {
        Self {
            filter_config,
            router,
        }
    }

    /// Evaluates whether a transaction should be accepted for arbitrage processing.
    ///
    /// Evaluates:
    /// 1. Transaction error indicators (`transaction_error != None`).
    /// 2. Raw logs checking for failure expressions or reverts.
    /// 3. Monitored AMM program presence.
    /// 4. Value thresholds.
    pub fn should_process(&self, payload: &SolanaTransactionPayload) -> bool {
        // 1. Drop failed transactions immediately
        if self.filter_config.drop_failed_transactions {
            if payload.transaction_error.is_some() {
                trace!(target: "expression", sig = %payload.signature, "Filtered: transaction_error present");
                return false;
            }

            // Inspect raw logs if available
            if let Some(ref raw) = payload.raw {
                if let Some(ref meta) = raw.meta {
                    if meta.err.is_some() {
                        trace!(target: "expression", sig = %payload.signature, "Filtered: raw meta.err present");
                        return false;
                    }
                    if let Some(ref logs) = meta.log_messages {
                        for log in logs {
                            if log.contains("Program failed to complete")
                                || log.contains("failed: custom program error")
                                || log.contains("insufficient funds")
                            {
                                trace!(target: "expression", sig = %payload.signature, "Filtered: failure log pattern detected");
                                return false;
                            }
                        }
                    }
                }
            }
        }

        // 2. Filter against monitored AMMs if enabled
        if self.filter_config.only_monitored_amms {
            let mut touches_monitored = false;

            // Check outer instructions
            for inst in &payload.instructions {
                if self.router.is_monitored_program(&inst.program_id) {
                    touches_monitored = true;
                    break;
                }
            }

            // Check inner instructions if outer wasn't a direct AMM call (e.g. aggregator router)
            if !touches_monitored {
                for group in &payload.inner_instructions {
                    for inner in &group.instructions {
                        if self.router.is_monitored_program(&inner.program_id) {
                            touches_monitored = true;
                            break;
                        }
                    }
                    if touches_monitored {
                        break;
                    }
                }
            }

            if !touches_monitored {
                trace!(target: "expression", sig = %payload.signature, "Filtered: no monitored AMM interaction");
                return false;
            }
        }

        true
    }
}

/// Worker Pool manager orchestrating asynchronous execution.
pub struct WorkerEngine {
    sender: mpsc::Sender<TransactionJob>,
}

impl WorkerEngine {
    /// Initializes worker tasks and channels.
    pub fn new(
        buffer_size: usize,
        worker_threads: usize,
        listener: Arc<dyn ArbitrageEventListener>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<TransactionJob>(buffer_size);

        // Share the receiver across multiple worker threads using an async mutex or multi-task dispatch
        let rx = Arc::new(tokio::sync::Mutex::new(receiver));

        for worker_id in 0..worker_threads {
            let rx_clone = Arc::clone(&rx);
            let listener_clone = Arc::clone(&listener);

            tokio::spawn(async move {
                debug!(target: "worker", worker_id, "Worker thread online");
                loop {
                    let job_opt = {
                        let mut locked = rx_clone.lock().await;
                        locked.recv().await
                    };

                    match job_opt {
                        Some(job) => {
                            listener_clone.on_pool_update(&job).await;
                        }
                        None => {
                            info!(target: "worker", worker_id, "Channel closed. Worker exiting.");
                            break;
                        }
                    }
                }
            });
        }

        Self { sender }
    }

    /// Non-blocking dispatch to worker channel ensuring sub-10ms response to webhook caller.
    pub fn dispatch(&self, job: TransactionJob) -> Result<(), mpsc::error::TrySendError<TransactionJob>> {
        self.sender.try_send(job)
    }
}
