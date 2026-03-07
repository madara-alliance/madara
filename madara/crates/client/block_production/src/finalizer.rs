use crate::close_queue::{CloseJobCompletion, QueuedCloseJob, QueuedClosePayload};
use crate::metrics::BlockProductionMetrics;
use anyhow::{anyhow, bail, Context, Result};
use mc_db::close_pipeline_contract::{ClosePreconfirmedResult, QueuedMeta};
use opentelemetry::KeyValue;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

struct InFlightGaugeGuard {
    metrics: Arc<BlockProductionMetrics>,
    in_flight: Arc<AtomicUsize>,
    job_count: usize,
}

impl InFlightGaugeGuard {
    fn new(metrics: Arc<BlockProductionMetrics>, in_flight: Arc<AtomicUsize>, job_count: usize) -> Self {
        let current = in_flight.fetch_add(job_count, Ordering::Relaxed) + job_count;
        metrics.close_queue_in_flight.record(current as u64, &[]);
        Self { metrics, in_flight, job_count }
    }
}

impl Drop for InFlightGaugeGuard {
    fn drop(&mut self) {
        let current = self.in_flight.fetch_sub(self.job_count, Ordering::Relaxed).saturating_sub(self.job_count);
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
    metrics: Arc<BlockProductionMetrics>,
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
    /// The worker processes close jobs in FIFO order, batching contiguous jobs up to
    /// the next boundary while preserving serial completion order.
    /// Shutdown: drop the FinalizerHandle, then await FinalizerTaskHandle::join().
    pub fn spawn<F, Fut>(
        capacity: usize,
        metrics: Arc<BlockProductionMetrics>,
        execute_fn: F,
    ) -> (Self, FinalizerTaskHandle)
    where
        F: Fn(Arc<BlockProductionMetrics>, Vec<QueuedClosePayload>) -> Fut + Send + 'static,
        Fut: Future<Output = Vec<Result<CloseJobCompletion>>> + Send + 'static,
    {
        let capacity = capacity.max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        let in_flight = Arc::new(AtomicUsize::new(0));
        let in_flight_worker = Arc::clone(&in_flight);
        let handle_metrics = Arc::clone(&metrics);

        let join_handle = tokio::spawn(async move {
            let mut receiver: mpsc::Receiver<QueuedCloseJob> = receiver;
            while let Some(first_job) = receiver.recv().await {
                let mut jobs = vec![first_job];
                while !jobs.last().is_some_and(|job| job.payload.is_boundary) {
                    match receiver.try_recv() {
                        Ok(job) => jobs.push(job),
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                let batch_len = jobs.len();
                let _in_flight_guard =
                    InFlightGaugeGuard::new(metrics.clone(), Arc::clone(&in_flight_worker), batch_len);

                let first_block_n = jobs.first().expect("close batch has first job").payload.db_payload.block_n;
                let last_block_n = jobs.last().expect("close batch has last job").payload.db_payload.block_n;
                let block_numbers: Vec<_> = jobs.iter().map(|job| job.payload.db_payload.block_n).collect();
                for job in &jobs {
                    let queue_wait = job.payload.enqueued_at.elapsed();
                    metrics.close_queue_wait_duration.record(queue_wait.as_secs_f64(), &[]);
                }
                tracing::info!(
                    "close_job_batch_processing_started start_block={} end_block={} batch_size={} in_flight={}",
                    first_block_n,
                    last_block_n,
                    batch_len,
                    in_flight_worker.load(Ordering::Relaxed)
                );

                let mut payloads = Vec::with_capacity(batch_len);
                let mut completions = Vec::with_capacity(batch_len);
                for job in jobs {
                    completions.push(job.completion);
                    payloads.push(job.payload);
                }

                let execute_start = std::time::Instant::now();
                let mut results = execute_fn(metrics.clone(), payloads).await;
                if results.len() != batch_len {
                    let message = format!(
                        "Finalizer batch executor returned {} results for {} queued jobs",
                        results.len(),
                        batch_len
                    );
                    tracing::error!(
                        start_block = first_block_n,
                        end_block = last_block_n,
                        returned_results = results.len(),
                        expected_results = batch_len,
                        "close_job_batch_result_mismatch"
                    );
                    results = (0..batch_len).map(|_| Err(anyhow!(message.clone()))).collect();
                }

                for (block_n, result) in block_numbers.into_iter().zip(results.iter()) {
                    if let Err(error) = result {
                        metrics.close_job_failures_total.add(1, &[]);
                        tracing::error!(block_number = block_n, error = ?error, "close_job_processing_failed");
                    }
                }

                let successful = results.iter().filter(|result| result.is_ok()).count();
                tracing::info!(
                    "close_job_batch_processing_finished start_block={} end_block={} batch_size={} successful={} execute_duration_ms={} in_flight={}",
                    first_block_n,
                    last_block_n,
                    batch_len,
                    successful,
                    execute_start.elapsed().as_secs_f64() * 1000.0,
                    in_flight_worker.load(Ordering::Relaxed)
                );

                for (completion, result) in completions.into_iter().zip(results.into_iter()) {
                    if let Err(_send_err) = completion.send(result) {
                        tracing::debug!("Close job completion receiver dropped before finalizer send");
                    }
                }
            }

            Ok(())
        });

        let handle = Self { sender, configured_capacity: capacity, in_flight, metrics: handle_metrics };
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

    /// Try to enqueue a close job. Returns backpressure error if the queue is full.
    pub fn try_enqueue(
        &self,
        payload: QueuedClosePayload,
    ) -> Result<(ClosePreconfirmedResult, oneshot::Receiver<Result<CloseJobCompletion>>)> {
        let block_n = payload.db_payload.block_n;
        let (sender, receiver) = oneshot::channel();
        let job = QueuedCloseJob { payload, completion: sender };

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
    /// Await worker completion. Call after dropping FinalizerHandle to drain.
    pub async fn join(self) -> Result<()> {
        self.join_handle.await.context("Finalizer worker task panicked")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CurrentBlockState;
    use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
    use blockifier::bouncer::{BouncerWeights, CasmHashComputationData};
    use blockifier::state::cached_state::CommitmentStateDiff;
    use mc_db::close_pipeline_contract::CloseJobPayload as DbCloseJobPayload;
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_state_update::StateDiff;
    use rstest::rstest;
    use std::sync::Arc;
    use std::time::Instant;

    fn empty_block_exec_summary() -> BlockExecutionSummary {
        BlockExecutionSummary {
            state_diff: CommitmentStateDiff {
                address_to_class_hash: Default::default(),
                address_to_nonce: Default::default(),
                storage_updates: Default::default(),
                class_hash_to_compiled_class_hash: Default::default(),
            },
            compressed_state_diff: None,
            bouncer_weights: BouncerWeights::empty(),
            casm_hash_computation_data_sierra_gas: CasmHashComputationData {
                class_hash_to_casm_hash_computation_gas: Default::default(),
                gas_without_casm_hash_computation: Default::default(),
            },
            casm_hash_computation_data_proving_gas: CasmHashComputationData {
                class_hash_to_casm_hash_computation_gas: Default::default(),
                gas_without_casm_hash_computation: Default::default(),
            },
            compiled_class_hashes_for_migration: vec![],
        }
    }

    fn test_payload(block_n: u64) -> QueuedClosePayload {
        test_payload_with_boundary(block_n, false)
    }

    fn test_payload_with_boundary(block_n: u64, is_boundary: bool) -> QueuedClosePayload {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        QueuedClosePayload {
            db_payload: DbCloseJobPayload { block_n },
            state: CurrentBlockState::new(backend.into(), block_n),
            block_exec_summary: Box::new(empty_block_exec_summary()),
            state_diff: StateDiff {
                storage_diffs: vec![],
                old_declared_contracts: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                replaced_classes: vec![],
                nonces: vec![],
                migrated_compiled_classes: vec![],
            },
            is_boundary,
            trie_log_mode: mc_db::rocksdb::global_trie::in_memory::TrieLogMode::Checkpoint,
            trie_batch_handle: None,
            trie_batch_index: None,
            enqueued_at: Instant::now(),
        }
    }

    async fn test_execute(
        _metrics: Arc<BlockProductionMetrics>,
        payloads: Vec<QueuedClosePayload>,
    ) -> Vec<Result<CloseJobCompletion>> {
        payloads.into_iter().map(|payload| Ok(CloseJobCompletion { block_n: payload.db_payload.block_n })).collect()
    }

    #[rstest]
    #[case::cap1_enqueue1(1, 1, true)]
    #[case::cap1_enqueue2(1, 2, false)]
    #[case::cap2_enqueue2(2, 2, true)]
    #[case::cap3_enqueue1(3, 1, true)]
    #[tokio::test]
    async fn backpressure_matrix(#[case] capacity: usize, #[case] enqueue_count: usize, #[case] all_succeed: bool) {
        let metrics = Arc::new(BlockProductionMetrics::register());
        let (handle, task_handle) = FinalizerHandle::spawn(capacity, metrics, test_execute);

        let mut receivers = Vec::new();
        let mut enqueue_failures = 0;

        for i in 0..enqueue_count {
            match handle.try_enqueue(test_payload(i as u64)) {
                Ok((ClosePreconfirmedResult::Queued(_), recv)) => receivers.push(recv),
                Err(_) => enqueue_failures += 1,
            }
        }

        if all_succeed {
            assert_eq!(enqueue_failures, 0, "all enqueues should succeed with capacity={capacity}");
        } else {
            assert!(enqueue_failures > 0, "some enqueues should fail with capacity={capacity}");
        }

        for recv in receivers {
            recv.await.expect("completion channel should not be dropped").expect("close should succeed");
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
            let (_, recv) = handle.try_enqueue(test_payload(i)).expect("enqueue should succeed");
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
    async fn batches_stop_at_boundary() {
        let seen_batches: Arc<std::sync::Mutex<Vec<Vec<u64>>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_batches_clone = Arc::clone(&seen_batches);
        let execute_fn =
            move |_metrics: Arc<BlockProductionMetrics>,
                  payloads: Vec<QueuedClosePayload>|
                  -> std::pin::Pin<Box<dyn Future<Output = Vec<Result<CloseJobCompletion>>> + Send>> {
                let seen_batches = Arc::clone(&seen_batches_clone);
                Box::pin(async move {
                    let block_numbers: Vec<_> = payloads.iter().map(|payload| payload.db_payload.block_n).collect();
                    seen_batches.lock().expect("batch log mutex").push(block_numbers.clone());
                    payloads
                        .into_iter()
                        .map(|payload| Ok(CloseJobCompletion { block_n: payload.db_payload.block_n }))
                        .collect()
                })
            };

        let metrics = Arc::new(BlockProductionMetrics::register());
        let (handle, task_handle) = FinalizerHandle::spawn(8, metrics, execute_fn);

        let mut receivers = Vec::new();
        for (block_n, is_boundary) in [(0_u64, false), (1, true), (2, false), (3, false)] {
            let (_, recv) =
                handle.try_enqueue(test_payload_with_boundary(block_n, is_boundary)).expect("enqueue should succeed");
            receivers.push(recv);
        }

        for recv in receivers {
            recv.await.expect("channel open").expect("close ok");
        }

        drop(handle);
        task_handle.join().await.expect("worker should complete cleanly");

        assert_eq!(
            *seen_batches.lock().expect("batch log mutex"),
            vec![vec![0, 1], vec![2, 3]],
            "worker should batch up to the first boundary, then start a new batch"
        );
    }

    #[tokio::test]
    async fn drain_shutdown_completes_in_flight_job() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let gate_clone = gate.clone();

        let execute_fn =
            move |_metrics: Arc<BlockProductionMetrics>,
                  payloads: Vec<QueuedClosePayload>|
                  -> std::pin::Pin<Box<dyn Future<Output = Vec<Result<CloseJobCompletion>>> + Send>> {
                let gate = gate_clone.clone();
                Box::pin(async move {
                    if payloads.iter().any(|payload| payload.db_payload.block_n == 0) {
                        // Block until gate is released, simulating in-flight work during shutdown.
                        gate.notified().await;
                    }
                    payloads
                        .into_iter()
                        .map(|payload| Ok(CloseJobCompletion { block_n: payload.db_payload.block_n }))
                        .collect()
                })
            };

        let metrics = Arc::new(BlockProductionMetrics::register());
        let (handle, task_handle) = FinalizerHandle::spawn(4, metrics, execute_fn);

        let (_, recv) = handle.try_enqueue(test_payload(0)).expect("enqueue should succeed");

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
