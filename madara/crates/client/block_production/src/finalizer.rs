use crate::close_queue::{CloseJobCompletion, QueuedCloseJob, QueuedClosePayload};
use crate::metrics::BlockProductionMetrics;
use anyhow::{anyhow, Context, Result};
use mc_db::close_pipeline_contract::{ClosePreconfirmedResult, QueuedMeta};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

struct InFlightGaugeGuard {
    metrics: Arc<BlockProductionMetrics>,
    in_flight: Arc<AtomicUsize>,
}

impl InFlightGaugeGuard {
    fn new(metrics: Arc<BlockProductionMetrics>, in_flight: Arc<AtomicUsize>) -> Self {
        let current = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
        metrics.close_queue_in_flight.record(current as u64, &[]);
        Self { metrics, in_flight }
    }
}

impl Drop for InFlightGaugeGuard {
    fn drop(&mut self) {
        let current = self.in_flight.fetch_sub(1, Ordering::Relaxed).saturating_sub(1);
        self.metrics.close_queue_in_flight.record(current as u64, &[]);
    }
}

/// Handle used by the caller to enqueue close jobs into the finalizer pipeline.
///
/// Owns the sender side of the queue channel and capacity metadata.
pub(crate) struct FinalizerHandle {
    sender: mpsc::Sender<QueuedCloseJob>,
    configured_capacity: usize,
    in_flight: Arc<AtomicUsize>,
}

/// Handle for joining the finalizer worker task on shutdown.
///
/// Does NOT use AbortOnDrop — the worker is drained gracefully by dropping
/// the FinalizerHandle (sender), which causes the receiver to return None.
#[must_use = "Finalizer task handle must be joined for clean shutdown"]
pub(crate) struct FinalizerTaskHandle {
    join_handle: tokio::task::JoinHandle<Result<()>>,
}

impl FinalizerHandle {
    /// Spawn the finalizer worker and return the handle pair.
    ///
    /// The worker processes close jobs serially in FIFO order.
    /// Shutdown: drop the FinalizerHandle, then await FinalizerTaskHandle::join().
    pub fn spawn<F, Fut>(
        capacity: usize,
        metrics: Arc<BlockProductionMetrics>,
        execute_fn: F,
    ) -> (Self, FinalizerTaskHandle)
    where
        F: Fn(Arc<BlockProductionMetrics>, QueuedClosePayload) -> Fut + Send + 'static,
        Fut: Future<Output = Result<CloseJobCompletion>> + Send + 'static,
    {
        let capacity = capacity.max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let in_flight_worker = Arc::clone(&in_flight);

        let join_handle = tokio::spawn(async move {
            let mut receiver: mpsc::Receiver<QueuedCloseJob> = receiver;
            while let Some(job) = receiver.recv().await {
                let _in_flight_guard = InFlightGaugeGuard::new(metrics.clone(), Arc::clone(&in_flight_worker));
                let block_n = job.payload.db_payload.block_n;
                let queue_wait = job.payload.enqueued_at.elapsed();
                metrics.close_queue_wait_duration.record(queue_wait.as_secs_f64(), &[]);
                tracing::info!(
                    "close_job_processing_started block_number={} queue_wait_ms={} in_flight={}",
                    block_n,
                    queue_wait.as_secs_f64() * 1000.0,
                    in_flight_worker.load(Ordering::Relaxed)
                );

                let execute_start = std::time::Instant::now();
                let result = execute_fn(metrics.clone(), job.payload).await;
                tracing::info!(
                    "close_job_processing_finished block_number={} execute_duration_ms={} success={} in_flight={}",
                    block_n,
                    execute_start.elapsed().as_secs_f64() * 1000.0,
                    result.is_ok(),
                    in_flight_worker.load(Ordering::Relaxed)
                );

                if let Err(_send_err) = job.completion.send(result) {
                    tracing::warn!("Close job completion receiver dropped before finalizer send");
                }
            }

            Ok(())
        });

        let handle = Self { sender, configured_capacity: capacity, in_flight };
        let task_handle = FinalizerTaskHandle { join_handle };
        (handle, task_handle)
    }

    pub fn configured_capacity(&self) -> usize {
        self.configured_capacity
    }

    /// Current number of jobs in the queue.
    pub fn current_depth(&self) -> usize {
        self.configured_capacity.saturating_sub(self.sender.capacity())
    }

    pub fn current_in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }

    /// Enqueue a close job, waiting for capacity when the queue is full.
    pub async fn enqueue(
        &self,
        payload: QueuedClosePayload,
    ) -> Result<(ClosePreconfirmedResult, oneshot::Receiver<Result<CloseJobCompletion>>)> {
        let block_n = payload.db_payload.block_n;
        let (sender, receiver) = oneshot::channel();
        let job = QueuedCloseJob { payload, completion: sender };

        self.sender.send(job).await.map_err(|_| anyhow!("Close queue is closed"))?;
        let queued = QueuedMeta { block_n, queue_depth: self.current_depth() };
        Ok((ClosePreconfirmedResult::Queued(queued), receiver))
    }
}

impl FinalizerTaskHandle {
    /// Await worker completion. Call after dropping FinalizerHandle to drain.
    pub async fn join(self) -> Result<()> {
        self.join_handle.await.context("Finalizer worker task panicked")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CurrentBlockState;
    use blockifier::bouncer::BouncerWeights;
    use mc_db::close_pipeline_contract::CloseJobPayload as DbCloseJobPayload;
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_state_update::StateDiff;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn test_payload(block_n: u64) -> QueuedClosePayload {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        QueuedClosePayload {
            db_payload: DbCloseJobPayload { block_n },
            state: CurrentBlockState::new(backend, block_n),
            canonical_bouncer_weights: BouncerWeights::empty(),
            state_diff: StateDiff {
                storage_diffs: vec![],
                old_declared_contracts: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                replaced_classes: vec![],
                nonces: vec![],
                migrated_compiled_classes: vec![],
            },
            canonical_executed_rows: vec![],
            canonical_header: Default::default(),
            enqueued_at: Instant::now(),
        }
    }

    async fn test_execute(
        _metrics: Arc<BlockProductionMetrics>,
        payload: QueuedClosePayload,
    ) -> Result<CloseJobCompletion> {
        Ok(CloseJobCompletion { block_n: payload.db_payload.block_n })
    }

    #[tokio::test]
    async fn saturated_queue_waits_for_capacity() {
        let first_started = Arc::new(tokio::sync::Notify::new());
        let release_first = Arc::new(tokio::sync::Notify::new());
        let first_started_worker = first_started.clone();
        let release_first_worker = release_first.clone();
        let execute_fn = move |_metrics: Arc<BlockProductionMetrics>,
                               payload: QueuedClosePayload|
              -> std::pin::Pin<Box<dyn Future<Output = Result<CloseJobCompletion>> + Send>> {
            let first_started = first_started_worker.clone();
            let release_first = release_first_worker.clone();
            Box::pin(async move {
                if payload.db_payload.block_n == 0 {
                    first_started.notify_one();
                    release_first.notified().await;
                }
                Ok(CloseJobCompletion { block_n: payload.db_payload.block_n })
            })
        };

        let metrics = Arc::new(BlockProductionMetrics::register());
        let (handle, task_handle) = FinalizerHandle::spawn(1, metrics, execute_fn);
        let (_, first_completion) = handle.enqueue(test_payload(0)).await.expect("first enqueue should succeed");
        first_started.notified().await;
        let (_, second_completion) = handle.enqueue(test_payload(1)).await.expect("second enqueue should fill queue");

        let third_completion = {
            let third_enqueue = handle.enqueue(test_payload(2));
            tokio::pin!(third_enqueue);
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut third_enqueue).await.is_err(),
                "enqueue should wait while the bounded queue is saturated"
            );

            release_first.notify_one();
            let (_, completion) = tokio::time::timeout(Duration::from_secs(1), &mut third_enqueue)
                .await
                .expect("enqueue should resume after capacity is released")
                .expect("third enqueue should succeed");
            completion
        };

        for completion in [first_completion, second_completion, third_completion] {
            completion.await.expect("completion channel should stay open").expect("close should succeed");
        }

        drop(handle);
        task_handle.join().await.expect("worker should complete cleanly");
    }

    #[tokio::test]
    async fn ordered_completion() {
        let metrics = Arc::new(BlockProductionMetrics::register());
        let (handle, task_handle) = FinalizerHandle::spawn(8, metrics, test_execute);

        let mut receivers = Vec::new();
        for i in 0..5u64 {
            let (_, recv) = handle.enqueue(test_payload(i)).await.expect("enqueue should succeed");
            receivers.push(recv);
        }

        for (i, recv) in receivers.into_iter().enumerate() {
            let completion = recv.await.expect("channel open").expect("close ok");
            assert_eq!(completion.block_n, i as u64, "completion order must match enqueue order");
        }

        drop(handle);
        task_handle.join().await.expect("worker should complete cleanly");
    }

    #[tokio::test]
    async fn drain_shutdown_completes_in_flight_job() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_clone = gate.clone();

        let execute_fn = move |_metrics: Arc<BlockProductionMetrics>,
                               payload: QueuedClosePayload|
              -> std::pin::Pin<Box<dyn Future<Output = Result<CloseJobCompletion>> + Send>> {
            let gate = gate_clone.clone();
            Box::pin(async move {
                if payload.db_payload.block_n == 0 {
                    // Block until gate is released, simulating in-flight work during shutdown.
                    gate.notified().await;
                }
                Ok(CloseJobCompletion { block_n: payload.db_payload.block_n })
            })
        };

        let metrics = Arc::new(BlockProductionMetrics::register());
        let (handle, task_handle) = FinalizerHandle::spawn(4, metrics, execute_fn);

        let (_, recv) = handle.enqueue(test_payload(0)).await.expect("enqueue should succeed");

        // Yield to let the worker pick up the job before we drop the handle.
        tokio::task::yield_now().await;

        // Drop sender to initiate shutdown (worker will drain after current job).
        drop(handle);

        // Release the gate so the in-flight job can complete.
        gate.notify_one();

        // The job must complete even though we dropped the handle.
        let completion = recv.await.expect("channel open").expect("close ok");
        assert_eq!(completion.block_n, 0, "in-flight job must complete during drain");

        task_handle.join().await.expect("worker should complete cleanly after drain");
    }
}
