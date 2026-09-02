//! Parallel Merkle root scheduler with strictly ordered commit delivery.

use super::{InFlightGaugeGuard, ParallelCommit, ParallelPrepare};
use crate::close_pipeline::ParallelComputedClosePayload;
use crate::close_queue::{CloseJobCompletion, QueuedCloseJob};
use crate::metrics::BlockProductionMetrics;
use anyhow::{anyhow, Context, Result};
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};

/// Result of one root-preparation future, including its completion ownership.
struct RootTaskResult {
    block_n: u64,
    completion: oneshot::Sender<Result<CloseJobCompletion>>,
    in_flight_guard: InFlightGaugeGuard,
    result: Result<ParallelComputedClosePayload>,
}

/// Prepared work waiting for its ordered commit turn.
enum ReadyCommitEntry {
    Success {
        output: Box<ParallelComputedClosePayload>,
        completion: oneshot::Sender<Result<CloseJobCompletion>>,
        in_flight_guard: InFlightGaugeGuard,
        ready_at: Instant,
    },
    Failed {
        error: anyhow::Error,
        completion: oneshot::Sender<Result<CloseJobCompletion>>,
        in_flight_guard: InFlightGaugeGuard,
    },
}

type RootFuture = BoxFuture<'static, RootTaskResult>;

/// Owns every mutable collection that participates in parallel close scheduling.
struct ParallelFinalizer {
    receiver: mpsc::Receiver<QueuedCloseJob>,
    receiver_closed: bool,
    waiting_jobs: VecDeque<QueuedCloseJob>,
    active_roots: FuturesUnordered<RootFuture>,
    ready_to_commit: BTreeMap<u64, ReadyCommitEntry>,
    next_commit_block_n: Option<u64>,
    root_workers: usize,
    metrics: Arc<BlockProductionMetrics>,
    in_flight: Arc<AtomicUsize>,
    prepare: ParallelPrepare,
    commit: ParallelCommit,
}

impl ParallelFinalizer {
    /// Creates an empty scheduler over the supplied bounded receiver.
    fn new(
        receiver: mpsc::Receiver<QueuedCloseJob>,
        metrics: Arc<BlockProductionMetrics>,
        in_flight: Arc<AtomicUsize>,
        root_workers: usize,
        prepare: ParallelPrepare,
        commit: ParallelCommit,
    ) -> Self {
        Self {
            receiver,
            receiver_closed: false,
            waiting_jobs: VecDeque::new(),
            active_roots: FuturesUnordered::new(),
            ready_to_commit: BTreeMap::new(),
            next_commit_block_n: None,
            root_workers: root_workers.max(1),
            metrics,
            in_flight,
            prepare,
            commit,
        }
    }

    /// Runs until the input is closed and every accepted block is committed.
    async fn run(mut self) -> Result<()> {
        loop {
            self.dispatch_waiting_roots();
            self.commit_ready_in_order().await?;

            if self.input_and_work_drained() {
                self.ensure_no_ordering_gap()?;
                break;
            }

            tokio::select! {
                maybe_job = self.receiver.recv(), if !self.receiver_closed => {
                    self.accept_received_job(maybe_job);
                }
                Some(result) = self.active_roots.next(), if !self.active_roots.is_empty() => {
                    self.record_root_result(result)?;
                }
            }
        }

        record_pipeline_gauges(&self.metrics, 0, 0, 0);
        Ok(())
    }

    /// Starts queued root jobs until the configured worker limit is reached.
    fn dispatch_waiting_roots(&mut self) {
        while self.active_roots.len() < self.root_workers {
            let Some(job) = self.waiting_jobs.pop_front() else {
                break;
            };
            self.dispatch_root(job);
        }
    }

    /// Moves one waiting job into the active root-future set.
    fn dispatch_root(&mut self, job: QueuedCloseJob) {
        let queue_wait = job.payload.enqueued_at.elapsed().as_secs_f64();
        self.metrics.close_queue_wait_duration.record(queue_wait, &[]);
        self.metrics.close_queue_wait_last.record(queue_wait, &[]);

        let block_n = job.payload.close_job_payload.block_n;
        let completion = job.completion;
        let payload = job.payload;
        let prepare = Arc::clone(&self.prepare);
        let metrics = Arc::clone(&self.metrics);
        let guard = InFlightGaugeGuard::acquire(Arc::clone(&self.metrics), Arc::clone(&self.in_flight), 1);

        self.record_gauges_with_active_delta(1);
        tracing::debug!(
            "parallel_root_scheduler_dispatched block_number={} active_root_jobs={} queued_root_jobs={} ready_to_commit={} queue_in_flight={} root_workers={}",
            block_n,
            self.active_roots.len() + 1,
            self.waiting_jobs.len(),
            self.ready_to_commit.len(),
            self.in_flight.load(Ordering::Relaxed),
            self.root_workers
        );

        self.active_roots.push(Box::pin(async move {
            let result = (prepare)(metrics, payload).await;
            RootTaskResult { block_n, completion, in_flight_guard: guard, result }
        }));
    }

    /// Commits consecutive ready blocks, stopping at the first ordering gap.
    async fn commit_ready_in_order(&mut self) -> Result<()> {
        while let Some(block_n) = self.next_commit_block_n {
            let Some(entry) = self.ready_to_commit.remove(&block_n) else {
                break;
            };
            self.commit_one(block_n, entry).await?;
            self.next_commit_block_n = block_n.checked_add(1);
        }
        Ok(())
    }

    /// Delivers one prepared root through the commit function or fails the worker.
    async fn commit_one(&mut self, block_n: u64, entry: ReadyCommitEntry) -> Result<()> {
        match entry {
            ReadyCommitEntry::Success { output, completion, in_flight_guard, ready_at } => {
                let wait = ready_at.elapsed().as_secs_f64();
                self.metrics.parallel_root_ready_to_commit_wait_duration.record(wait, &[]);
                self.metrics.parallel_root_ready_to_commit_wait_last.record(wait, &[]);

                let result = (self.commit)(Arc::clone(&self.metrics), *output)
                    .await
                    .with_context(|| format!("Ordered close commit failed for block #{block_n}"));
                self.deliver_commit_result(block_n, completion, result)?;
                drop(in_flight_guard);
                Ok(())
            }
            ReadyCommitEntry::Failed { error, completion, in_flight_guard } => {
                self.metrics.close_job_failures_total.add(1, &[]);
                tracing::error!(block_number = block_n, error = ?error, "parallel_root_job_failed");
                let _ = completion.send(Err(error));
                drop(in_flight_guard);
                Err(anyhow!("Parallel root precompute failed for block #{block_n}"))
            }
        }
    }

    /// Sends a commit result to its caller and converts commit failure into worker failure.
    fn deliver_commit_result(
        &self,
        block_n: u64,
        completion: oneshot::Sender<Result<CloseJobCompletion>>,
        result: Result<CloseJobCompletion>,
    ) -> Result<()> {
        match result {
            Ok(completed) => {
                self.record_gauges();
                tracing::debug!(
                    "parallel_close_commit_complete block_number={} queued_root_jobs={} ready_to_commit={} active_root_jobs={} queue_in_flight={}",
                    block_n,
                    self.waiting_jobs.len(),
                    self.ready_to_commit.len(),
                    self.active_roots.len(),
                    self.in_flight.load(Ordering::Relaxed)
                );
                if completion.send(Ok(completed)).is_err() {
                    tracing::debug!("Close job completion receiver dropped before ordered commit send");
                }
                Ok(())
            }
            Err(error) => {
                self.metrics.close_job_failures_total.add(1, &[]);
                tracing::error!(block_number = block_n, error = ?error, "parallel_close_commit_failed");
                let _ = completion.send(Err(error));
                Err(anyhow!("Ordered close commit failed for block #{block_n}"))
            }
        }
    }

    /// Accepts a newly received job or marks the input as closed.
    fn accept_received_job(&mut self, maybe_job: Option<QueuedCloseJob>) {
        let Some(job) = maybe_job else {
            self.receiver_closed = true;
            return;
        };
        let block_n = job.payload.close_job_payload.block_n;
        self.next_commit_block_n.get_or_insert(block_n);
        self.waiting_jobs.push_back(job);
        self.record_gauges();
        tracing::debug!(
            "parallel_root_scheduler_enqueued block_number={} queued_root_jobs={} ready_to_commit={} active_root_jobs={} root_workers={}",
            block_n,
            self.waiting_jobs.len(),
            self.ready_to_commit.len(),
            self.active_roots.len(),
            self.root_workers
        );
    }

    /// Stores one completed root result until its block reaches the commit frontier.
    fn record_root_result(&mut self, result: RootTaskResult) -> Result<()> {
        let RootTaskResult { block_n, completion, in_flight_guard, result } = result;
        if self.ready_to_commit.contains_key(&block_n) {
            return Err(anyhow!("Parallel finalizer produced duplicate ready result for block #{block_n}"));
        }

        let event = match result {
            Ok(output) => {
                self.ready_to_commit.insert(
                    block_n,
                    ReadyCommitEntry::Success {
                        output: Box::new(output),
                        completion,
                        in_flight_guard,
                        ready_at: Instant::now(),
                    },
                );
                "parallel_root_ready_for_commit"
            }
            Err(error) => {
                self.ready_to_commit.insert(block_n, ReadyCommitEntry::Failed { error, completion, in_flight_guard });
                "parallel_root_failed_ready_for_ordered_delivery"
            }
        };
        self.record_gauges();
        tracing::debug!(
            "{} block_number={} queued_root_jobs={} ready_to_commit={} active_root_jobs={} queue_in_flight={}",
            event,
            block_n,
            self.waiting_jobs.len(),
            self.ready_to_commit.len(),
            self.active_roots.len(),
            self.in_flight.load(Ordering::Relaxed)
        );
        Ok(())
    }

    /// Returns true after channel close when no queued or active roots remain.
    fn input_and_work_drained(&self) -> bool {
        self.receiver_closed && self.waiting_jobs.is_empty() && self.active_roots.is_empty()
    }

    /// Rejects shutdown if ready results cannot reach the expected commit frontier.
    fn ensure_no_ordering_gap(&self) -> Result<()> {
        if let Some(next_block_n) = self.next_commit_block_n.filter(|_| !self.ready_to_commit.is_empty()) {
            return Err(anyhow!("Parallel finalizer drained without next ordered result for block #{next_block_n}"));
        }
        Ok(())
    }

    /// Records gauges for the scheduler's current collections.
    fn record_gauges(&self) {
        record_pipeline_gauges(
            &self.metrics,
            self.active_roots.len(),
            self.waiting_jobs.len(),
            self.ready_to_commit.len(),
        );
    }

    /// Records gauges before a newly created future is inserted into active_roots.
    fn record_gauges_with_active_delta(&self, active_delta: usize) {
        record_pipeline_gauges(
            &self.metrics,
            self.active_roots.len() + active_delta,
            self.waiting_jobs.len(),
            self.ready_to_commit.len(),
        );
    }
}

/// Runs the parallel scheduler until all accepted close jobs commit or one fails.
pub(super) async fn run(
    receiver: mpsc::Receiver<QueuedCloseJob>,
    metrics: Arc<BlockProductionMetrics>,
    in_flight: Arc<AtomicUsize>,
    root_workers: usize,
    prepare: ParallelPrepare,
    commit: ParallelCommit,
) -> Result<()> {
    ParallelFinalizer::new(receiver, metrics, in_flight, root_workers, prepare, commit).run().await
}

/// Records the number of blocks in each parallel scheduler stage.
pub(super) fn record_pipeline_gauges(
    metrics: &BlockProductionMetrics,
    active_root_jobs: usize,
    waiting_root_jobs: usize,
    ready_to_commit_jobs: usize,
) {
    metrics.parallel_root_active_jobs.record(active_root_jobs as u64, &[]);
    metrics.parallel_root_waiting_jobs.record(waiting_root_jobs as u64, &[]);
    metrics.parallel_root_ready_to_commit_jobs.record(ready_to_commit_jobs as u64, &[]);
}
