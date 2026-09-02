//! Sequential close worker.

use super::{InFlightGaugeGuard, SerialExecute};
use crate::close_queue::{CloseJobCompletion, QueuedCloseJob, QueuedClosePayload};
use crate::metrics::BlockProductionMetrics;
use anyhow::{anyhow, Result};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Queue-wait summary for one boundary-limited serial batch.
struct SerialBatch {
    jobs: Vec<QueuedCloseJob>,
    block_numbers: Vec<u64>,
    queue_wait: QueueWaitSummary,
}

/// Minimum, mean, and maximum queue wait for one serial batch.
#[derive(Clone, Copy)]
struct QueueWaitSummary {
    min_ms: f64,
    avg_ms: f64,
    max_ms: f64,
}

/// Owned data passed from batch draining into serial execution and delivery.
struct SerialBatchParts {
    block_numbers: Vec<u64>,
    payloads: Vec<QueuedClosePayload>,
    completions: Vec<oneshot::Sender<Result<CloseJobCompletion>>>,
    queue_wait: QueueWaitSummary,
}

impl SerialBatch {
    /// Drains immediately available jobs, stopping after the first boundary block.
    fn drain(receiver: &mut mpsc::Receiver<QueuedCloseJob>, first_job: QueuedCloseJob) -> Self {
        let mut jobs = vec![first_job];
        while !jobs.last().is_some_and(|job| job.payload.is_boundary) {
            match receiver.try_recv() {
                Ok(job) => jobs.push(job),
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        let block_numbers = jobs.iter().map(|job| job.payload.close_job_payload.block_n).collect();
        let waits: Vec<_> = jobs.iter().map(|job| job.payload.enqueued_at.elapsed().as_secs_f64() * 1000.0).collect();
        let queue_wait = QueueWaitSummary {
            min_ms: waits.iter().copied().fold(f64::INFINITY, f64::min),
            avg_ms: waits.iter().sum::<f64>() / waits.len() as f64,
            max_ms: waits.iter().copied().fold(0.0, f64::max),
        };

        Self { jobs, block_numbers, queue_wait }
    }

    /// Separates worker payloads from completion channels without changing order.
    fn into_parts(self) -> SerialBatchParts {
        let mut payloads = Vec::with_capacity(self.jobs.len());
        let mut completions = Vec::with_capacity(self.jobs.len());
        for job in self.jobs {
            payloads.push(job.payload);
            completions.push(job.completion);
        }
        SerialBatchParts { block_numbers: self.block_numbers, payloads, completions, queue_wait: self.queue_wait }
    }
}

/// Runs boundary-limited serial close batches until the sender side is dropped.
pub(super) async fn run(
    mut receiver: mpsc::Receiver<QueuedCloseJob>,
    metrics: Arc<BlockProductionMetrics>,
    in_flight: Arc<AtomicUsize>,
    execute: SerialExecute,
) -> Result<()> {
    while let Some(first_job) = receiver.recv().await {
        let batch = SerialBatch::drain(&mut receiver, first_job);
        record_queue_waits(&metrics, &batch.jobs);
        let batch_len = batch.jobs.len();
        let guard = InFlightGaugeGuard::acquire(Arc::clone(&metrics), Arc::clone(&in_flight), batch_len);
        let SerialBatchParts { block_numbers, payloads, completions, queue_wait } = batch.into_parts();
        let (first_block_n, last_block_n) = batch_ends(&block_numbers);
        log_batch_started(
            first_block_n,
            last_block_n,
            &block_numbers,
            batch_len,
            in_flight.load(std::sync::atomic::Ordering::Relaxed),
            queue_wait,
        );

        let started = std::time::Instant::now();
        let results = normalize_results((execute)(Arc::clone(&metrics), payloads).await, batch_len);
        record_failures(&metrics, &block_numbers, &results);
        log_batch_finished(
            first_block_n,
            last_block_n,
            batch_len,
            results.iter().filter(|result| result.is_ok()).count(),
            started.elapsed().as_secs_f64() * 1000.0,
            in_flight.load(std::sync::atomic::Ordering::Relaxed),
        );
        deliver_results(completions, results);
        drop(guard);
    }
    Ok(())
}

/// Records how long every job waited before the serial worker accepted it.
fn record_queue_waits(metrics: &BlockProductionMetrics, jobs: &[QueuedCloseJob]) {
    for job in jobs {
        let queue_wait = job.payload.enqueued_at.elapsed().as_secs_f64();
        metrics.close_queue_wait_duration.record(queue_wait, &[]);
        metrics.close_queue_wait_last.record(queue_wait, &[]);
    }
}

/// Returns the inclusive block-number range represented by a non-empty batch.
fn batch_ends(block_numbers: &[u64]) -> (u64, u64) {
    (
        *block_numbers.first().expect("serial close batch has a first block"),
        *block_numbers.last().expect("serial close batch has a last block"),
    )
}

/// Converts a malformed worker result count into one error per queued job.
fn normalize_results(results: Vec<Result<CloseJobCompletion>>, expected_len: usize) -> Vec<Result<CloseJobCompletion>> {
    if results.len() == expected_len {
        return results;
    }
    let message =
        format!("Finalizer batch executor returned {} results for {} queued jobs", results.len(), expected_len);
    tracing::error!(
        returned_results = results.len(),
        expected_results = expected_len,
        "close_job_batch_result_mismatch"
    );
    (0..expected_len).map(|_| Err(anyhow!(message.clone()))).collect()
}

/// Records failures against the block number that owned each result.
fn record_failures(metrics: &BlockProductionMetrics, block_numbers: &[u64], results: &[Result<CloseJobCompletion>]) {
    for (block_n, result) in block_numbers.iter().zip(results) {
        if let Err(error) = result {
            metrics.close_job_failures_total.add(1, &[]);
            tracing::error!(block_number = block_n, error = ?error, "close_job_processing_failed");
        }
    }
}

/// Completes callers in the same order in which their jobs were drained.
fn deliver_results(
    completions: Vec<oneshot::Sender<Result<CloseJobCompletion>>>,
    results: Vec<Result<CloseJobCompletion>>,
) {
    for (completion, result) in completions.into_iter().zip(results) {
        if completion.send(result).is_err() {
            tracing::debug!("Close job completion receiver dropped before finalizer send");
        }
    }
}

/// Emits one compact start event for a serial close batch.
fn log_batch_started(
    first_block_n: u64,
    last_block_n: u64,
    block_numbers: &[u64],
    batch_len: usize,
    in_flight: usize,
    queue_wait: QueueWaitSummary,
) {
    tracing::debug!(
        "finalizer_batch_started start_block={} end_block={} batch_size={} blocks={:?} in_flight={} queue_wait_min_ms={} queue_wait_avg_ms={} queue_wait_max_ms={}",
        first_block_n,
        last_block_n,
        batch_len,
        block_numbers,
        in_flight,
        queue_wait.min_ms,
        queue_wait.avg_ms,
        queue_wait.max_ms
    );
}

/// Emits one compact completion event for a serial close batch.
fn log_batch_finished(
    first_block_n: u64,
    last_block_n: u64,
    batch_len: usize,
    successful: usize,
    execute_duration_ms: f64,
    in_flight: usize,
) {
    tracing::debug!(
        "finalizer_batch_finished start_block={} end_block={} batch_size={} successful={} execute_duration_ms={} in_flight={}",
        first_block_n,
        last_block_n,
        batch_len,
        successful,
        execute_duration_ms,
        in_flight
    );
}
