//! Bounded close queue and finalizer worker lifecycle.
//!
//! Callers select a concrete serial or parallel mode through FinalizerConfig.
//! Worker implementation details live in the serial and parallel modules.

mod parallel;
mod serial;

#[cfg(test)]
mod tests;

use crate::close_pipeline::ParallelComputedClosePayload;
use crate::close_queue::{CloseJobCompletion, QueuedCloseJob, QueuedClosePayload};
use crate::metrics::BlockProductionMetrics;
use crate::BlockProductionTask;
use anyhow::{anyhow, bail, Context, Result};
use futures::future::BoxFuture;
use mc_db::close_pipeline_contract::{ClosePreconfirmedResult, QueuedMeta};
use opentelemetry::KeyValue;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Selects how queued blocks are finalized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FinalizerMode {
    /// Close each drained boundary batch through the sequential DB path.
    Serial,
    /// Prepare at most root_workers Merkle roots concurrently, then commit in order.
    Parallel { root_workers: usize },
}

/// Validated queue and worker limits used when the finalizer is spawned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FinalizerConfig {
    capacity: usize,
    mode: FinalizerMode,
}

impl FinalizerConfig {
    /// Creates a serial finalizer configuration with at least one queue slot.
    pub fn serial(capacity: usize) -> Self {
        Self { capacity: capacity.max(1), mode: FinalizerMode::Serial }
    }

    /// Creates a parallel finalizer configuration with non-zero queue and worker limits.
    pub fn parallel(capacity: usize, root_workers: usize) -> Self {
        Self { capacity: capacity.max(1), mode: FinalizerMode::Parallel { root_workers: root_workers.max(1) } }
    }

    /// Returns the bounded close-queue capacity.
    pub fn capacity(self) -> usize {
        self.capacity
    }

    /// Returns the selected worker mode.
    pub fn mode(self) -> FinalizerMode {
        self.mode
    }
}

/// Type-erased serial worker seam used to keep test callbacks out of the public API.
pub(super) type SerialExecute = Arc<
    dyn Fn(Arc<BlockProductionMetrics>, Vec<QueuedClosePayload>) -> BoxFuture<'static, Vec<Result<CloseJobCompletion>>>
        + Send
        + Sync,
>;
/// Type-erased root-preparation seam used internally by the scheduler.
pub(super) type ParallelPrepare = Arc<
    dyn Fn(Arc<BlockProductionMetrics>, QueuedClosePayload) -> BoxFuture<'static, Result<ParallelComputedClosePayload>>
        + Send
        + Sync,
>;
/// Type-erased ordered-commit seam used internally by the scheduler.
pub(super) type ParallelCommit = Arc<
    dyn Fn(Arc<BlockProductionMetrics>, ParallelComputedClosePayload) -> BoxFuture<'static, Result<CloseJobCompletion>>
        + Send
        + Sync,
>;

/// Concrete callbacks bound to one selected finalizer mode.
pub(super) enum FinalizerWorkers {
    Serial { execute: SerialExecute },
    Parallel { root_workers: usize, prepare: ParallelPrepare, commit: ParallelCommit },
}

impl FinalizerWorkers {
    /// Binds a validated mode to the production close-pipeline functions.
    /// Callback type erasure keeps scheduler code independent of generic function types.
    fn production(mode: FinalizerMode) -> Self {
        match mode {
            FinalizerMode::Serial => Self::Serial {
                execute: Arc::new(|metrics, payloads| {
                    Box::pin(BlockProductionTask::execute_close_payload_batch(metrics, payloads))
                }),
            },
            FinalizerMode::Parallel { root_workers } => Self::Parallel {
                root_workers,
                prepare: Arc::new(|metrics, payload| {
                    Box::pin(BlockProductionTask::compute_close_payload_parallel_root(metrics, payload))
                }),
                commit: Arc::new(|metrics, computed| {
                    Box::pin(BlockProductionTask::execute_close_payload_parallel_precomputed_job(metrics, computed))
                }),
            },
        }
    }
}

/// Keeps the in-flight gauge balanced for every accepted worker job.
pub(super) struct InFlightGaugeGuard {
    metrics: Arc<BlockProductionMetrics>,
    in_flight: Arc<AtomicUsize>,
    job_count: usize,
}

impl InFlightGaugeGuard {
    /// Increments the shared count and records the resulting gauge value.
    pub(super) fn acquire(metrics: Arc<BlockProductionMetrics>, in_flight: Arc<AtomicUsize>, job_count: usize) -> Self {
        let current = in_flight.fetch_add(job_count, Ordering::Relaxed) + job_count;
        metrics.close_queue_in_flight.record(current as u64, &[]);
        Self { metrics, in_flight, job_count }
    }
}

impl Drop for InFlightGaugeGuard {
    /// Releases the guarded jobs and publishes the resulting in-flight count.
    fn drop(&mut self) {
        let current = self.in_flight.fetch_sub(self.job_count, Ordering::Relaxed).saturating_sub(self.job_count);
        self.metrics.close_queue_in_flight.record(current as u64, &[]);
    }
}

/// Enqueues close jobs and exposes queue-depth telemetry to block production.
pub(crate) struct FinalizerHandle {
    sender: mpsc::Sender<QueuedCloseJob>,
    configured_capacity: usize,
    in_flight: Arc<AtomicUsize>,
    metrics: Arc<BlockProductionMetrics>,
}

/// Joins the finalizer worker after all senders have been dropped.
#[must_use = "Finalizer task handle must be joined for clean shutdown"]
pub(crate) struct FinalizerTaskHandle {
    join_handle: tokio::task::JoinHandle<Result<()>>,
}

impl FinalizerHandle {
    /// Spawns the configured production worker and returns its queue and join handles.
    /// Queue and worker limits have already been normalized by `FinalizerConfig`.
    pub fn spawn(config: FinalizerConfig, metrics: Arc<BlockProductionMetrics>) -> (Self, FinalizerTaskHandle) {
        let workers = FinalizerWorkers::production(config.mode());
        Self::spawn_with_workers(config.capacity(), metrics, workers)
    }

    /// Creates the bounded channel and starts the selected worker implementation.
    /// The returned handle owns enqueue access while the task handle owns graceful joining.
    fn spawn_with_workers(
        capacity: usize,
        metrics: Arc<BlockProductionMetrics>,
        workers: FinalizerWorkers,
    ) -> (Self, FinalizerTaskHandle) {
        let (sender, receiver) = mpsc::channel(capacity);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let worker_in_flight = Arc::clone(&in_flight);
        let worker_metrics = Arc::clone(&metrics);
        parallel::record_pipeline_gauges(&metrics, 0, 0, 0);

        let join_handle = match workers {
            FinalizerWorkers::Serial { execute } => {
                tokio::spawn(serial::run(receiver, worker_metrics, worker_in_flight, execute))
            }
            FinalizerWorkers::Parallel { root_workers, prepare, commit } => {
                tokio::spawn(parallel::run(receiver, worker_metrics, worker_in_flight, root_workers, prepare, commit))
            }
        };

        let handle = Self { sender, configured_capacity: capacity, in_flight, metrics };
        (handle, FinalizerTaskHandle { join_handle })
    }

    /// Returns the configured number of queued jobs allowed by the channel.
    pub fn configured_capacity(&self) -> usize {
        self.configured_capacity
    }

    /// Returns the current number of jobs occupying queue capacity.
    pub fn current_depth(&self) -> usize {
        self.configured_capacity.saturating_sub(self.sender.capacity())
    }

    /// Returns the number of jobs currently owned by a worker stage.
    pub fn current_in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Enqueues one close payload or returns a backpressure/closed-channel error.
    /// Successful calls return both queue metadata and the block-specific completion receiver.
    pub fn try_enqueue(
        &self,
        payload: QueuedClosePayload,
    ) -> Result<(ClosePreconfirmedResult, oneshot::Receiver<Result<CloseJobCompletion>>)> {
        let block_n = payload.close_job_payload.block_n;
        let (completion, receiver) = oneshot::channel();
        let job = QueuedCloseJob { payload, completion };

        match self.sender.try_send(job) {
            Ok(()) => {
                let queued = QueuedMeta { block_n, queue_depth: self.current_depth() };
                Ok((ClosePreconfirmedResult::Queued(queued), receiver))
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.metrics.close_queue_enqueue_failures_total.add(1, &[KeyValue::new("reason", "full")]);
                tracing::warn!(
                    block_number = block_n,
                    queue_depth = self.current_depth(),
                    queue_capacity = self.configured_capacity,
                    queue_in_flight = self.current_in_flight(),
                    "close_queue_backpressure"
                );
                bail!("Close queue is full (capacity={}), invariant/config violation", self.configured_capacity)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.close_queue_enqueue_failures_total.add(1, &[KeyValue::new("reason", "closed")]);
                Err(anyhow!("Close queue is closed"))
            }
        }
    }
}

impl FinalizerTaskHandle {
    /// Waits for a drained worker to exit and preserves a panic as an error.
    pub async fn join(self) -> Result<()> {
        self.join_handle.await.context("Finalizer worker task panicked")?
    }
}
