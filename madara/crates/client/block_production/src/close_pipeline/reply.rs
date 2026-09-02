//! Executor reply handling and close-queue payload construction.

use super::super::*;
use super::parallel::{active_parallel_root_jobs, collect_diffs_for_root_from_base};
use mc_db::rocksdb::SnapshotRef;
use mp_chain_config::StarknetVersion;
use tokio::sync::oneshot;

/// Snapshot base and cumulative diffs required by an optional parallel root job.
struct ParallelRootInputs {
    base_block_n: Option<u64>,
    snapshot: Option<SnapshotRef>,
    state_diffs: Vec<StateDiff>,
}

impl ParallelRootInputs {
    /// Returns an empty root input for the sequential close path.
    fn sequential() -> Self {
        Self { base_block_n: None, snapshot: None, state_diffs: Vec::new() }
    }
}

/// Timing values captured before ownership moves into the close queue.
struct CloseEnqueueTiming {
    last_execution_finished_at: Option<Instant>,
    close_block_received_at: Instant,
    enqueued_at: Instant,
    executor_to_close_queue: Option<Duration>,
    enqueue_duration: Duration,
}

/// Executor-owned values that become the immutable body of one close job.
struct ClosePayloadData {
    state: CurrentBlockState,
    block_exec_summary: Box<BlockExecutionSummary>,
    state_diff: StateDiff,
    protocol_version: StarknetVersion,
    is_boundary: bool,
}

impl BlockProductionTask {
    /// Applies one executor message to the block-production state machine.
    pub(crate) async fn process_reply(
        &mut self,
        reply: ExecutorMessage,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        match reply {
            ExecutorMessage::StartNewBlock { exec_ctx } => self.start_preconfirmed_block(exec_ctx).await?,
            ExecutorMessage::BatchExecuted(batch) => self.append_executed_batch(batch, close_queue).await?,
            ExecutorMessage::EndBlock(summary) => {
                tracing::debug!("received_executor_end_block");
                self.close_block(summary, close_queue).await?;
            }
            ExecutorMessage::EndFinalBlock(Some(summary)) => {
                tracing::debug!("received_executor_end_final_block");
                self.close_block(summary, close_queue).await?;
            }
            ExecutorMessage::EndFinalBlock(None) => {
                tracing::debug!("EndFinalBlock(None) received - executor completed without block");
            }
        }
        Ok(())
    }

    /// Creates the block-keyed preconfirmed row after validating the executor cursor.
    async fn start_preconfirmed_block(&mut self, exec_ctx: BlockExecutionContext) -> anyhow::Result<()> {
        tracing::debug!("received_executor_start_new_block block_n={}", exec_ctx.block_number);
        let current_state = self.current_state.take().context("No current state")?;
        let TaskState::NotExecuting { latest_block_n } = current_state else {
            anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
        };
        let expected = latest_block_n
            .map(|number| number.checked_add(1).context("Block number overflow while starting new block"))
            .transpose()?
            .unwrap_or(0);
        anyhow::ensure!(
            expected == exec_ctx.block_number,
            "Received new block_n={} from executor, expected block_n={expected}",
            exec_ctx.block_number
        );

        let backend = Arc::clone(&self.backend);
        global_spawn_rayon_task(move || {
            backend.write_access().new_preconfirmed(PreconfirmedBlock::new(exec_ctx.into_header()))
        })
        .await?;
        self.current_state = Some(TaskState::Executing(CurrentBlockState::new(Arc::clone(&self.backend), expected)));
        self.record_block_stage_metrics();
        Ok(())
    }

    /// Persists one executor batch and folds its statistics into the open block.
    async fn append_executed_batch(
        &mut self,
        batch: BatchExecutionResult,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let current_state = self.current_state.as_mut().context("No current state")?;
        let TaskState::Executing(state) = current_state else {
            anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
        };
        let batch_execution = batch.stats.exec_duration.as_secs_f64();
        let delivery = batch.emitted_at.elapsed().as_secs_f64();
        self.metrics.executor_batch_execution_duration.record(batch_execution, &[]);
        self.metrics.executor_batch_execution_last.record(batch_execution, &[]);
        self.metrics.executor_to_main_delivery_duration.record(delivery, &[]);
        self.metrics.executor_to_main_delivery_last.record(delivery, &[]);
        tracing::debug!(
            "received_executor_batch_executed block_number={} txs_executed_in_batch={} txs_added_to_block={} txs_reverted={} txs_rejected={} batch_exec_duration_ms={} executor_to_main_delivery_ms={} close_queue_depth={} close_queue_in_flight={} pending_close_completions={}",
            state.block_number,
            batch.stats.n_executed,
            batch.stats.n_added_to_block,
            batch.stats.n_reverted,
            batch.stats.n_rejected,
            batch_execution * 1000.0,
            delivery * 1000.0,
            close_queue.current_depth(),
            close_queue.current_in_flight(),
            self.pending_completions.len()
        );

        self.metrics.record_execution_stats(&batch.stats);
        state.accumulated_stats = state.accumulated_stats.clone() + batch.stats.clone();
        state.last_execution_finished_at = Some(batch.emitted_at);
        state.append_batch(batch).await?;
        self.send_state_notification(BlockProductionStateNotification::BatchExecuted);
        Ok(())
    }

    /// Converts a finalized executor block into one bounded close-queue job.
    async fn close_block(
        &mut self,
        block_exec_summary: Box<BlockExecutionSummary>,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let state = self.take_executing_state()?;
        let block_n = state.block_number;
        let (last_execution_finished_at, close_block_received_at) = Self::start_close_timing(&state);
        let (state_diff, protocol_version) = Self::derive_close_state_diff(&state, &block_exec_summary)?;
        let is_boundary = self.is_boundary_block(block_n);
        let root_inputs = self.prepare_parallel_root_inputs(block_n, &state_diff, is_boundary)?;
        let timing = self.capture_close_enqueue_timing(block_n, last_execution_finished_at, close_block_received_at);
        let data = ClosePayloadData { state, block_exec_summary, state_diff, protocol_version, is_boundary };
        let payload = self.build_close_payload(data, root_inputs, &timing);
        let (ClosePreconfirmedResult::Queued(queued), completion) = close_queue.try_enqueue(payload)?;
        self.record_close_enqueued(close_queue, &queued, &timing);

        if self.parallel_merkle_enabled {
            self.defer_parallel_completion(block_n, completion, close_queue);
            Ok(())
        } else {
            self.await_serial_completion(completion, close_queue).await
        }
    }

    /// Takes ownership of the currently executing block or rejects an invalid transition.
    fn take_executing_state(&mut self) -> anyhow::Result<CurrentBlockState> {
        let state = self.current_state.take().context("No current state")?;
        let TaskState::Executing(state) = state else {
            anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
        };
        Ok(state)
    }

    /// Normalizes Blockifier's execution diff and preserves the block protocol version.
    fn derive_close_state_diff(
        state: &CurrentBlockState,
        summary: &BlockExecutionSummary,
    ) -> anyhow::Result<(StateDiff, StarknetVersion)> {
        let view = state
            .backend
            .block_view_on_preconfirmed(state.block_number)
            .with_context(|| format!("No pre-confirmed block #{}", state.block_number))?;
        let migration_v2_hashes =
            summary.compiled_class_hashes_for_migration.iter().map(|(v2_hash, _)| v2_hash.0).collect::<HashSet<_>>();
        let diff = StateDiff::from_blockifier(
            summary.state_diff.clone(),
            &migration_v2_hashes,
            &state.deployed_contracts,
            view.get_old_declared_contracts(),
        );
        Ok((diff, view.block().header.protocol_version))
    }

    /// Marks receipt of a finalized executor block before payload preparation.
    fn start_close_timing(state: &CurrentBlockState) -> (Option<Instant>, Instant) {
        let close_block_received_at = Instant::now();
        let last_execution_finished_at = state.last_execution_finished_at;
        let executor_to_close_queue =
            last_execution_finished_at.map(|finished| close_block_received_at.duration_since(finished));
        tracing::debug!(
            "close_block_received_from_executor block_number={} executor_to_close_queue_ms={:?}",
            state.block_number,
            executor_to_close_queue.map(|duration| duration.as_secs_f64() * 1000.0)
        );
        (last_execution_finished_at, close_block_received_at)
    }

    /// Captures queue-entry latency after state diff and root inputs are ready.
    fn capture_close_enqueue_timing(
        &self,
        block_n: u64,
        last_execution_finished_at: Option<Instant>,
        close_block_received_at: Instant,
    ) -> CloseEnqueueTiming {
        let executor_to_close_queue =
            last_execution_finished_at.map(|finished| close_block_received_at.duration_since(finished));
        let enqueued_at = Instant::now();
        let enqueue_duration = enqueued_at.duration_since(close_block_received_at);

        if let Some(duration) = executor_to_close_queue {
            self.metrics.executor_to_close_queue_duration.record(duration.as_secs_f64(), &[]);
            self.metrics.executor_to_close_queue_last.record(duration.as_secs_f64(), &[]);
        }
        self.metrics.close_block_enqueue_duration.record(enqueue_duration.as_secs_f64(), &[]);
        self.metrics.close_block_enqueue_last.record(enqueue_duration.as_secs_f64(), &[]);
        tracing::debug!(block_number = block_n, "close_payload_ready_for_enqueue");
        CloseEnqueueTiming {
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
            executor_to_close_queue,
            enqueue_duration,
        }
    }

    /// Selects a durable snapshot floor and contiguous diff span for root preparation.
    fn prepare_parallel_root_inputs(
        &mut self,
        block_n: u64,
        state_diff: &StateDiff,
        is_boundary: bool,
    ) -> anyhow::Result<ParallelRootInputs> {
        if !self.parallel_merkle_enabled {
            return Ok(ParallelRootInputs::sequential());
        }

        self.diffs_since_snapshot.push((block_n, state_diff.clone()));
        let generic_floor = self.backend.db.get_latest_snapshot_floor(block_n.checked_sub(1));
        let (base_block_n, snapshot) =
            self.backend.db.get_latest_durable_snapshot_floor(block_n.checked_sub(1)).ok_or_else(|| {
                anyhow::anyhow!("Missing durable snapshot floor for root computation of block #{block_n}")
            })?;
        if generic_floor.as_ref().is_some_and(|(generic, _)| *generic != base_block_n) {
            tracing::debug!(
                "parallel_root_non_durable_floor_ignored block_number={} generic_base_snapshot_block={:?} durable_base_snapshot_block={base_block_n:?}",
                block_n,
                generic_floor.map(|(block_n, _)| block_n)
            );
        }

        let state_diffs = collect_diffs_for_root_from_base(&self.diffs_since_snapshot, base_block_n, block_n)?;
        tracing::debug!(
            "parallel_root_job_enqueued block_number={} base_snapshot_block={base_block_n:?} diff_count={} squashed_block_count={} diff_start_block={} diff_end_block={} include_overlay={} durable_base=true active_parallel_root_jobs={}",
            block_n,
            state_diffs.len(),
            state_diffs.len(),
            base_block_n.map_or(0, |base| base.saturating_add(1)),
            block_n,
            is_boundary,
            active_parallel_root_jobs()
        );
        Ok(ParallelRootInputs { base_block_n, snapshot: Some(snapshot), state_diffs })
    }

    /// Couples execution output, root inputs, and timings into the queue data contract.
    fn build_close_payload(
        &self,
        data: ClosePayloadData,
        root_inputs: ParallelRootInputs,
        timing: &CloseEnqueueTiming,
    ) -> QueuedClosePayload {
        let ClosePayloadData { state, block_exec_summary, state_diff, protocol_version, is_boundary } = data;
        QueuedClosePayload {
            close_job_payload: mc_db::close_pipeline_contract::CloseJobPayload { block_n: state.block_number },
            state,
            block_exec_summary,
            state_diff,
            is_boundary,
            parallel_merkle_flush_interval: self.parallel_merkle_flush_interval,
            compare_parallel_with_sequential: self.parallel_merkle_compare_sequential,
            root_base_block_n: root_inputs.base_block_n,
            root_snapshot: root_inputs.snapshot,
            root_state_diffs: root_inputs.state_diffs,
            protocol_version,
            last_execution_finished_at: timing.last_execution_finished_at,
            close_block_received_at: timing.close_block_received_at,
            enqueued_at: timing.enqueued_at,
        }
    }

    /// Records the queue state immediately after a successful enqueue.
    fn record_close_enqueued(
        &self,
        close_queue: &FinalizerHandle,
        queued: &mc_db::close_pipeline_contract::QueuedMeta,
        timing: &CloseEnqueueTiming,
    ) {
        let pending = self.pending_completions.len() + usize::from(self.parallel_merkle_enabled);
        self.metrics.close_queue_enqueued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(close_queue.current_depth() as u64, &[]);
        tracing::debug!(
            "close_block_queued block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} parallel_merkle={} executor_to_close_queue_ms={:?} close_block_to_queue_enqueue_ms={}",
            queued.block_n,
            close_queue.current_depth(),
            close_queue.configured_capacity(),
            close_queue.current_in_flight(),
            pending,
            self.parallel_merkle_enabled,
            timing.executor_to_close_queue.map(|duration| duration.as_secs_f64() * 1000.0),
            timing.enqueue_duration.as_secs_f64() * 1000.0
        );
    }

    /// Keeps a parallel completion in block order while execution advances.
    fn defer_parallel_completion(
        &mut self,
        block_n: u64,
        completion: oneshot::Receiver<anyhow::Result<CloseJobCompletion>>,
        close_queue: &FinalizerHandle,
    ) {
        self.pending_completions.push_back((block_n, completion));
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(block_n) });
        self.record_block_stage_metrics();
        tracing::debug!(
            "parallel_merkle_close_deferred block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} root_compute_background=true",
            block_n,
            close_queue.current_depth(),
            close_queue.configured_capacity(),
            close_queue.current_in_flight(),
            self.pending_completions.len()
        );
    }

    /// Waits for the sequential DB close before permitting the next block.
    async fn await_serial_completion(
        &mut self,
        completion: oneshot::Receiver<anyhow::Result<CloseJobCompletion>>,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let completion = completion.await.context("Close queue worker dropped completion channel")??;
        self.metrics.close_queue_dequeued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(close_queue.current_depth() as u64, &[]);
        self.mark_replay_boundary_closed(completion.block_n);
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(completion.block_n) });
        self.send_state_notification(BlockProductionStateNotification::ClosedBlock { block_n: completion.block_n });
        self.record_block_stage_metrics();
        tracing::debug!(
            "close_block_complete block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} parallel_merkle=false",
            completion.block_n,
            close_queue.current_depth(),
            close_queue.configured_capacity(),
            close_queue.current_in_flight(),
            self.pending_completions.len()
        );
        Ok(())
    }
}
