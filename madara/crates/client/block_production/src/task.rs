//! Runtime lifecycle for the batcher, executor, and finalizer workers.

use super::*;
use crate::close_pipeline::{prune_diffs_since_snapshot, validate_parallel_queue_invariant};
use crate::finalizer::{FinalizerConfig, FinalizerTaskHandle};

/// Tracks the independent shutdown signals observed by the runtime loop.
#[derive(Default)]
struct ShutdownProgress {
    batcher_completed: bool,
    end_final_block_received: bool,
    executor_stopped: bool,
    batcher_error: Option<anyhow::Error>,
}

impl ShutdownProgress {
    /// Records the batcher outcome while allowing the executor/finalizer to drain.
    fn record_batcher_result(&mut self, result: anyhow::Result<()>, has_preconfirmed_block: bool) {
        self.batcher_completed = true;
        match result {
            Ok(()) => tracing::debug!("Batcher task completed normally"),
            Err(error) => {
                let error = error.context("In batcher task");
                tracing::warn!("Batcher task errored: {error:?}");
                if has_preconfirmed_block {
                    tracing::warn!("Batcher errored with preconfirmed block, attempting graceful shutdown");
                }
                self.batcher_error = Some(error);
            }
        }
    }

    /// Returns true once producers stopped and the executor's final block was handled.
    fn is_complete(&self, pending_completions: usize) -> bool {
        self.batcher_completed && self.end_final_block_received && pending_completions == 0
    }

    /// Returns the deferred batcher error after an otherwise clean drain.
    fn finish(self) -> anyhow::Result<()> {
        self.batcher_error.map(Err).unwrap_or(Ok(()))
    }
}

impl BlockProductionTask {
    /// Validates recovery settings, repairs persisted preconfirmed work, and sets the confirmed cursor.
    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;
        if self.parallel_merkle_enabled {
            if !self.backend.saves_preconfirmed_blocks() {
                tracing::warn!(
                    parallel_merkle = true,
                    preconfirmed_persistence = false,
                    "Parallel Merkle will recover the canonical trie to the confirmed head after a crash, but unfinished preconfirmed transactions above it cannot be restored and may need resubmission"
                );
            }
            self.backend
                .db
                .ensure_parallel_merkle_recovery_config()
                .context("Validating parallel Merkle recovery configuration")?;
        }

        self.close_preconfirmed_block_if_exists().await.context("Cannot close preconfirmed block on startup")?;
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: self.backend.latest_confirmed_block_n() });
        self.record_block_stage_metrics();
        Ok(())
    }

    /// Starts the dedicated executor thread with the task's one-shot command receiver.
    fn start_executor(&mut self) -> anyhow::Result<executor::ExecutorThreadHandle> {
        executor::start_executor_thread(
            Arc::clone(&self.backend),
            self.executor_commands_recv.take().context("Task already started")?,
            Arc::clone(&self.metrics),
            self.replay_mode_enabled,
        )
        .context("Starting executor thread")
    }

    /// Builds and starts the selected close worker after validating queue limits.
    fn start_finalizer(&self) -> anyhow::Result<(FinalizerHandle, FinalizerTaskHandle)> {
        let capacity = self.close_queue_capacity();
        validate_parallel_queue_invariant(self.parallel_merkle_enabled, capacity)?;
        let config = if self.parallel_merkle_enabled {
            FinalizerConfig::parallel(capacity, self.parallel_merkle_root_workers)
        } else {
            FinalizerConfig::serial(capacity)
        };
        let handles = FinalizerHandle::spawn(config, Arc::clone(&self.metrics));
        self.log_finalizer_configuration(&handles.0);
        Ok(handles)
    }

    /// Logs the effective queue and root-worker settings used by the finalizer.
    fn log_finalizer_configuration(&self, handle: &FinalizerHandle) {
        if self.parallel_merkle_enabled {
            tracing::info!(
                "initialized_finalizer_runtime mode=parallel_merkle queue_capacity={} configured_max_inflight={} configured_capacity={} parallel_merkle={} parallel_merkle_root_workers={} parallel_merkle_compare_sequential={}",
                self.close_queue_capacity(),
                self.close_queue_capacity,
                handle.configured_capacity(),
                true,
                self.parallel_merkle_root_workers,
                self.parallel_merkle_compare_sequential
            );
        } else {
            tracing::info!(
                "initialized_finalizer_runtime mode=serial queue_capacity={} configured_max_inflight={} configured_capacity={}",
                self.close_queue_capacity(),
                self.close_queue_capacity,
                handle.configured_capacity()
            );
        }
    }

    /// Starts the batcher with ownership of the executor's single-slot batch sender.
    fn start_batcher(
        &mut self,
        ctx: ServiceContext,
        executor: &mut executor::ExecutorThreadHandle,
    ) -> anyhow::Result<AbortOnDrop<anyhow::Result<()>>> {
        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let bypass_tx_input = self.bypass_tx_input.take().context("Bypass tx channel already taken")?;
        Ok(AbortOnDrop::spawn(
            Batcher::new(
                Arc::clone(&self.backend),
                Arc::clone(&self.mempool),
                Arc::clone(&self.metrics),
                Arc::clone(&self.l1_client),
                ctx,
                batch_sender,
                bypass_tx_input,
                self.mempool_intake_rx.clone(),
            )
            .run(),
        ))
    }

    /// Waits for executor replies, ordered close completions, and shutdown signals.
    async fn run_event_loop(
        &mut self,
        executor: &mut executor::ExecutorThreadHandle,
        batcher: &mut AbortOnDrop<anyhow::Result<()>>,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let mut shutdown = ShutdownProgress::default();

        loop {
            tokio::select! {
                result = &mut *batcher, if !shutdown.batcher_completed => {
                    shutdown.record_batcher_result(result, self.backend.has_preconfirmed_block());
                }
                Some(reply) = executor.replies.recv() => {
                    let is_final = matches!(reply, ExecutorMessage::EndFinalBlock(_));
                    self.process_reply(reply, close_queue)
                        .await
                        .context("Processing reply from executor thread")?;
                    if is_final {
                        shutdown.end_final_block_received = true;
                        tracing::debug!("EndFinalBlock processed, executor completed");
                    }
                }
                result = Self::front_completion(&mut self.pending_completions),
                    if !self.pending_completions.is_empty() =>
                {
                    let (expected_block_n, _) =
                        self.pending_completions.pop_front().expect("pending completion exists");
                    let completion = result.context("Close queue worker dropped completion channel")??;
                    self.accept_ordered_completion(expected_block_n, completion, close_queue)?;
                }
                result = executor.stop.recv(), if !shutdown.executor_stopped => {
                    shutdown.executor_stopped = true;
                    result.context("In executor thread")?;
                }
            }

            if shutdown.is_complete(self.pending_completions.len()) {
                tracing::debug!("Shutdown complete: batcher completed, EndFinalBlock processed");
                return shutdown.finish();
            }
        }
    }

    /// Borrows only the first completion so parallel closes are observed in block order.
    async fn front_completion(
        pending: &mut VecDeque<(u64, tokio::sync::oneshot::Receiver<anyhow::Result<CloseJobCompletion>>)>,
    ) -> Result<anyhow::Result<CloseJobCompletion>, tokio::sync::oneshot::error::RecvError> {
        let (_, receiver) = pending.front_mut().expect("checked non-empty");
        receiver.await
    }

    /// Validates and publishes one ordered finalizer completion.
    fn accept_ordered_completion(
        &mut self,
        expected_block_n: u64,
        completion: CloseJobCompletion,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            completion.block_n == expected_block_n,
            "Out-of-order close completion: expected #{expected_block_n}, got #{}",
            completion.block_n
        );
        self.metrics.close_queue_dequeued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(close_queue.current_depth() as u64, &[]);
        tracing::debug!(
            "close_block_complete block_number={} expected_block_n={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} parallel_merkle={}",
            completion.block_n,
            expected_block_n,
            close_queue.current_depth(),
            close_queue.configured_capacity(),
            close_queue.current_in_flight(),
            self.pending_completions.len(),
            self.parallel_merkle_enabled
        );

        self.prune_completed_boundary(completion);
        self.mark_replay_boundary_closed(completion.block_n);
        self.send_state_notification(BlockProductionStateNotification::ClosedBlock { block_n: completion.block_n });
        self.record_block_stage_metrics();
        Ok(())
    }

    /// Drops tracked state diffs once their durable boundary has committed.
    fn prune_completed_boundary(&mut self, completion: CloseJobCompletion) {
        if self.parallel_merkle_enabled && completion.durable_checkpoint_committed {
            prune_diffs_since_snapshot(&mut self.diffs_since_snapshot, completion.block_n);
        }
    }

    /// Completes replay bookkeeping and warns if the closed block missed its requested boundary.
    pub(crate) fn mark_replay_boundary_closed(&self, block_n: u64) {
        let Some(status) = self.backend.replay_boundary_mark_closed(block_n) else {
            return;
        };
        if !status.boundary_met {
            tracing::warn!(
                "replay_boundary_closed_without_match block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} mismatch={:?}",
                status.block_n,
                status.expected_tx_count,
                status.executed_tx_count,
                status.dispatched_tx_count,
                status.reached_last_tx_hash,
                status.mismatch
            );
        }
    }

    /// Drains completion receivers left behind by an error-path loop exit.
    async fn drain_pending_completions(&mut self) {
        while let Some((expected_block_n, receiver)) = self.pending_completions.pop_front() {
            match receiver.await {
                Ok(Ok(completion)) if completion.block_n == expected_block_n => {
                    self.prune_completed_boundary(completion);
                    self.mark_replay_boundary_closed(completion.block_n);
                }
                Ok(Ok(completion)) => tracing::warn!(
                    "Shutdown drain received out-of-order completion: expected #{expected_block_n}, got #{}",
                    completion.block_n
                ),
                Ok(Err(error)) => {
                    tracing::warn!("Shutdown drain: close completion for block #{expected_block_n} failed: {error:#}")
                }
                Err(error) => {
                    tracing::warn!("Shutdown drain: completion channel dropped for block #{expected_block_n}: {error}")
                }
            }
            self.record_block_stage_metrics();
        }
    }

    /// Clears transient pipeline state after the finalizer has stopped.
    fn reset_after_shutdown(&mut self) {
        self.pending_completions.clear();
        self.diffs_since_snapshot.clear();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: self.backend.latest_confirmed_block_n() });
        self.record_block_stage_metrics();
    }

    /// Preserves both the event-loop and finalizer error when shutdown reports both.
    fn combine_runtime_results(
        loop_result: anyhow::Result<()>,
        finalizer_result: anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        match (loop_result, finalizer_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(finalizer_error)) => Err(finalizer_error.context("In finalizer worker")),
            (Err(primary_error), Ok(())) => Err(primary_error),
            (Err(primary_error), Err(finalizer_error)) => {
                tracing::warn!("Finalizer worker also errored during shutdown: {finalizer_error:?}");
                Err(primary_error.context(format!("Additionally, finalizer worker errored: {finalizer_error:#}")))
            }
        }
    }

    /// Runs block production and always drains the finalizer before returning.
    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn run(mut self, ctx: ServiceContext) -> Result<(), anyhow::Error> {
        self.setup_initial_state().await?;
        self.metrics.close_queue_depth.record(0, &[]);
        self.metrics.close_queue_in_flight.record(0, &[]);
        self.record_block_stage_metrics();

        let mut executor = self.start_executor()?;
        let (close_queue, finalizer_task) = self.start_finalizer()?;
        let mut batcher = self.start_batcher(ctx, &mut executor)?;
        let loop_result = self.run_event_loop(&mut executor, &mut batcher, &close_queue).await;

        drop(close_queue);
        let finalizer_result = finalizer_task.join().await;
        self.drain_pending_completions().await;
        self.reset_after_shutdown();

        Self::combine_runtime_results(loop_result, finalizer_result)
    }
}
