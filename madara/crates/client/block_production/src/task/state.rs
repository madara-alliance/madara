use super::*;

const DEFAULT_CLOSE_QUEUE_CAPACITY: usize = 10;
const MAX_CLOSE_QUEUE_CAPACITY: usize = 10;

/// Used for listening to state changes in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProductionStateNotification {
    ClosedBlock { block_n: u64 },
    BatchExecuted,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OptimisticPipelineNotification {
    BlockStarted { block_n: u64 },
    BatchExecuted { block_n: u64 },
    ComparatorStarted { block_n: u64 },
    ComparatorFinished { block_n: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MempoolIntakeMode {
    Running,
    Paused,
    FlushOnce,
}

#[derive(Debug)]
pub(super) struct BlockExecutionSnapshot {
    pub(super) execution_mode: ExecutionMode,
}

#[derive(Debug)]
pub(crate) struct CurrentBlockState {
    pub(super) backend: Arc<MadaraBackend>,
    pub(super) block_number: u64,
    pub(super) execution_snapshot: BlockExecutionSnapshot,
    pub(super) consumed_core_contract_nonces: HashSet<u64>,
    /// We need to keep track of deployed contracts, because blockifier can't make the difference between replaced class / deployed contract :/
    pub(super) deployed_contracts: HashSet<Felt>,
    /// Track when block production started for metrics
    pub(super) block_start_time: Instant,
    /// Accumulated execution stats across all batches for this block
    pub(super) accumulated_stats: util::ExecutionStats,
    /// In mixed mode, per-tx artifacts are accumulated here for comparator use.
    /// The runtime preconfirmed block also receives the content (C-011A) for
    /// internal speculative frontier / mempool nonce visibility. Canonical DB
    /// persistence happens after comparator selection in close_block().
    pub(super) speculative_executed_txs: Vec<PreconfirmedExecutedTransaction>,
}

#[derive(Debug, Clone)]
pub(super) struct ApprovedExternalContent {
    pub(super) source: CanonicalBlockSource,
    pub(super) header: PreconfirmedHeader,
    pub(super) executed_rows: Vec<PreconfirmedExecutedTransaction>,
}

#[derive(Debug)]
pub(super) struct PendingCanonicalizationInput {
    pub(super) state: CurrentBlockState,
    pub(super) block_exec_summary: Box<BlockExecutionSummary>,
    // C-018: parent_overlays removed. Overlays are recomputed at canonicalization start
    // (in maybe_start_canonicalization_task) to avoid stale overlays when a parent stop
    // invalidates descendant assumptions.
}

#[derive(Debug)]
pub(super) struct CanonicalizationTaskCanonical {
    pub(super) canonical: CanonicalizedBlockOutput,
    pub(super) stop_reason: Option<crate::comparator::StopReason>,
}

#[derive(Debug)]
pub(super) struct CanonicalizationTaskResult {
    pub(super) state: CurrentBlockState,
    pub(super) canonical_result: anyhow::Result<CanonicalizationTaskCanonical>,
    pub(super) dispatcher: Option<ReexecDispatcherHandle>,
}

#[derive(Debug)]
pub(super) struct StartupRecoveredBlockPayload {
    pub(super) header: PreconfirmedHeader,
    pub(super) canonical_rows: Vec<PreconfirmedExecutedTransaction>,
    pub(super) state_diff: StateDiff,
    pub(super) bouncer_weights: blockifier::bouncer::BouncerWeights,
    pub(super) consumed_core_contract_nonces: HashSet<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct TaintedRebuildSourceTx {
    pub(super) validated: ValidatedTransaction,
    pub(super) source_block_n: Option<u64>,
    pub(super) force_charge_fee: Option<bool>,
}

impl TaintedRebuildSourceTx {
    pub(super) fn effective_charge_fee(&self) -> bool {
        self.force_charge_fee.unwrap_or(self.validated.charge_fee)
    }
}

#[derive(Debug)]
pub(super) struct TaintedRebuildClosePayload {
    pub(super) state: CurrentBlockState,
    pub(super) canonical_bouncer_weights: blockifier::bouncer::BouncerWeights,
    pub(super) state_diff: StateDiff,
    pub(super) canonical_executed_rows: Vec<PreconfirmedExecutedTransaction>,
    pub(super) canonical_header: PreconfirmedHeader,
}

#[derive(Debug)]
pub(super) struct TaintedRebuildStepResult {
    pub(super) live_session_after_step: Option<mc_db::StoredTaintedRebuildSession>,
    pub(super) close_payload: TaintedRebuildClosePayload,
}

pub(super) struct PendingTaintedRebuildCarry {
    pub(super) carry_task: JoinHandle<anyhow::Result<Vec<mc_db::StoredTaintedRebuildCarryRow>>>,
}

pub(super) struct PendingStopFallbackHandoff {
    pub(super) block_n: u64,
    pub(super) state: CurrentBlockState,
    pub(super) canonical_bouncer_weights: blockifier::bouncer::BouncerWeights,
    pub(super) canonical_state_diff: StateDiff,
    pub(super) canonical_rows: Vec<PreconfirmedExecutedTransaction>,
    pub(super) canonical_header: PreconfirmedHeader,
    pub(super) replace_internal_preconfirmed_rows: Option<Vec<PreconfirmedExecutedTransaction>>,
    pub(super) had_speculative: bool,
    pub(super) carry: PendingTaintedRebuildCarry,
}

impl CurrentBlockState {
    #[cfg(test)]
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64) -> Self {
        Self::with_execution_mode(backend, block_number, ExecutionMode::BlockifierOnly)
    }

    pub(crate) fn with_execution_mode(
        backend: Arc<MadaraBackend>,
        block_number: u64,
        execution_mode: ExecutionMode,
    ) -> Self {
        Self {
            backend,
            block_number,
            execution_snapshot: BlockExecutionSnapshot { execution_mode },
            consumed_core_contract_nonces: Default::default(),
            deployed_contracts: Default::default(),
            block_start_time: Instant::now(),
            accumulated_stats: Default::default(),
            speculative_executed_txs: Vec::new(),
        }
    }

    /// Get old declared contracts (Cairo 0 legacy classes) from the in-memory speculative buffer.
    /// Used in mixed mode where executed txs are not yet written to the preconfirmed DB.
    pub(super) fn get_old_declared_contracts_from_speculative(&self) -> Vec<Felt> {
        let mut old_declared_contracts: Vec<Felt> = Vec::new();
        for tx in &self.speculative_executed_txs {
            for (&class_hash, &compiled_class) in &tx.state_diff.declared_classes {
                if matches!(compiled_class, DeclaredClassCompiledClass::Legacy) {
                    old_declared_contracts.push(class_hash);
                }
            }
        }
        old_declared_contracts.sort();
        old_declared_contracts
    }

    /// Process the execution result, merging it with the current pending state.
    ///
    /// In mixed mode (C-011A), per-tx artifacts are:
    /// 1. Appended to the internal runtime preconfirmed block (for mempool nonce visibility),
    /// 2. Persisted to block-scoped preconfirmed DB storage (for crash recovery),
    /// 3. Accumulated in `speculative_executed_txs` (for comparator use).
    ///    This per-batch DB persistence does NOT make the rows canonical — external preconfirmed
    ///    remains comparator-gated, and the canonical close payload is chosen later by the
    ///    comparator/close path.
    pub(super) async fn append_batch(&mut self, mut batch: BatchExecutionResult) -> anyhow::Result<()> {
        let mode = self.execution_snapshot.execution_mode;
        anyhow::ensure!(
            batch.execution_mode == mode,
            "Batch execution mode {:?} does not match frozen block mode {:?} for block #{}",
            batch.execution_mode,
            mode,
            self.block_number
        );
        let mut executed = vec![];

        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            if let Some(core_contract_nonce) = blockifier_tx.l1_handler_tx_nonce() {
                // Even when the l1 handler tx is reverted, we mark the nonce as consumed.
                self.consumed_core_contract_nonces
                    .insert(core_contract_nonce.to_felt().try_into().context("Invalid nonce while appending batch")?);
            }

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                let declared_class = additional_info.declared_class.take().filter(|_| !execution_info.is_reverted());

                let receipt = from_blockifier_execution_info(&execution_info, &blockifier_tx);
                let converted_tx = TransactionWithHash::from(blockifier_tx.clone());

                // Extract paid_fee_on_l1 from L1 handler transactions
                let paid_fee_on_l1 = match &blockifier_tx {
                    blockifier::transaction::transaction_execution::Transaction::L1Handler(l1_tx) => {
                        Some(l1_tx.paid_fee_on_l1.0)
                    }
                    _ => None,
                };

                executed.push(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt { transaction: converted_tx.transaction, receipt },
                    state_diff: TransactionStateUpdate {
                        nonces: state_diff
                            .nonces
                            .into_iter()
                            .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
                            .collect(),
                        contract_class_hashes: state_diff
                            .class_hashes
                            .into_iter()
                            .map(|(contract_addr, class_hash)| {
                                let addr_felt = contract_addr.to_felt();
                                let entry = if !self.deployed_contracts.contains(&addr_felt)
                                    && !self.backend.view_on_latest_confirmed().is_contract_deployed(&addr_felt)?
                                {
                                    self.deployed_contracts.insert(addr_felt);
                                    ClassUpdateItem::DeployedContract(class_hash.to_felt())
                                } else {
                                    ClassUpdateItem::ReplacedClass(class_hash.to_felt())
                                };

                                Ok((addr_felt, entry))
                            })
                            .collect::<anyhow::Result<_>>()?,
                        storage_diffs: state_diff
                            .storage
                            .into_iter()
                            .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
                            .collect(),
                        declared_classes: declared_class
                            .iter()
                            .map(|class| {
                                (
                                    *class.class_hash(),
                                    class
                                        .as_sierra()
                                        .and_then(|class| {
                                            // Use canonical hash (v2 if present, else v1)
                                            let hash =
                                                class.info.compiled_class_hash_v2.or(class.info.compiled_class_hash)?;
                                            Some(DeclaredClassCompiledClass::Sierra(hash))
                                        })
                                        .unwrap_or(DeclaredClassCompiledClass::Legacy),
                                )
                            })
                            .collect(),
                    },
                    declared_class,
                    arrived_at: additional_info.arrived_at,
                    paid_fee_on_l1,
                })
            }
        }

        let executed_count = executed.len();
        let first_tx_hash = executed
            .first()
            .map(|tx| format!("{:#x}", tx.transaction.receipt.transaction_hash()))
            .unwrap_or_else(|| "none".to_string());
        let last_tx_hash = executed
            .last()
            .map(|tx| format!("{:#x}", tx.transaction.receipt.transaction_hash()))
            .unwrap_or_else(|| "none".to_string());

        if mode == ExecutionMode::Mixed {
            // C-011A: Update internal speculative frontier for mempool nonce visibility.
            // The runtime preconfirmed block gets content updates so chain watcher can
            // observe nonce progression and make successor same-account txs ready.
            // This does NOT advance external preconfirmed — that remains comparator-gated.
            // Per-batch DB persistence ensures crash recovery can reconstruct the internal tip.
            self.backend
                .append_to_internal_preconfirmed_and_persist(&executed)
                .context("Updating internal speculative frontier with DB persistence")?;

            // Keep speculative buffer for comparator use and canonical row selection.
            // Note: DB persistence above is for crash recovery only — canonical authority
            // is determined later by the comparator and carried in the close payload.
            self.speculative_executed_txs.extend(executed);
        } else {
            // BlockifierOnly: write directly to preconfirmed DB (no comparator gate needed).
            let backend = self.backend.clone();
            global_spawn_rayon_task(move || {
                backend
                    .write_access()
                    .append_to_preconfirmed(&executed, /* candidates */ [])
                    .context("Appending to preconfirmed block")
            })
            .await?;
        }

        let stats = mem::take(&mut batch.stats);
        if stats.n_added_to_block > 0 {
            tracing::debug!(
                target: "mc_block_production",
                event = "append_batch_tx_hash_span",
                block_number = self.block_number,
                tx_count = executed_count,
                first_tx_hash,
                last_tx_hash,
                execution_mode = ?mode,
                "Accepted batch into preconfirmed block"
            );
            tracing::info!(
                "🧮 Executed and added {} transaction(s) to the preconfirmed block at height {} - {:.3?}",
                stats.n_added_to_block,
                self.block_number,
                stats.exec_duration,
            );
            tracing::debug!("Tick stats {:?}", stats);
        }
        Ok(())
    }
}

/// Little state machine that helps us following the state transitions the executor thread sends us.
#[allow(clippy::large_enum_variant)]
pub(crate) enum TaskState {
    NotExecuting {
        /// [`None`] when the next block to execute is genesis.
        latest_block_n: Option<u64>,
    },
    Executing(CurrentBlockState),
}

/// The block production task consumes transactions from the mempool in batches.
///
/// This is to allow optimistic concurrency. However, the block may get full during batch execution,
/// so the executor and batcher keep any unexecuted suffix local for the next cycle.
///
/// To understand block production in madara, you should probably start with the [`mp_chain_config::ChainConfig`]
/// documentation.
pub struct BlockProductionTask {
    pub(super) backend: Arc<MadaraBackend>,
    pub(super) mempool: Arc<Mempool>,
    pub(super) close_queue_capacity: usize,
    pub(super) current_state: Option<TaskState>,
    pub(super) metrics: Arc<BlockProductionMetrics>,
    pub(super) state_notifications: Option<mpsc::UnboundedSender<BlockProductionStateNotification>>,
    #[cfg(test)]
    pub(super) optimistic_pipeline_notifications: Option<mpsc::UnboundedSender<OptimisticPipelineNotification>>,
    #[cfg(test)]
    pub(super) comparator_gates: BTreeMap<u64, Arc<tokio::sync::Semaphore>>,
    pub(super) handle: BlockProductionHandle,
    pub(super) executor_commands_recv: Option<mpsc::UnboundedReceiver<executor::ExecutorCommand>>,
    pub(super) fallback_commands_recv: Option<mpsc::UnboundedReceiver<executor::FallbackCommand>>,
    pub(super) l1_client: Arc<dyn SettlementClient>,
    pub(super) bypass_tx_input: Option<mpsc::Receiver<ValidatedTransaction>>,
    pub(super) mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
    pub(super) tainted_rebuild_active_tx: watch::Sender<bool>,
    pub(super) tainted_rebuild_active_rx: watch::Receiver<bool>,
    pub(super) executor_replay_status_tx: watch::Sender<crate::fallback::types::RuntimeReplayStatus>,
    pub(super) executor_replay_status_rx: watch::Receiver<crate::fallback::types::RuntimeReplayStatus>,
    pub(super) execution_mode_tx: watch::Sender<ExecutionMode>,
    pub(super) execution_mode_rx: watch::Receiver<ExecutionMode>,
    pub(super) execution_epoch_tx: watch::Sender<u64>,
    pub(super) execution_epoch_rx: watch::Receiver<u64>,
    pub(super) routing_cfg: util::RustExecRoutingConfig,
    pub(super) no_charge_fee: bool,
    pub(super) replay_mode_enabled: bool,
    pub(super) diffs_since_snapshot: Vec<(u64, StateDiff)>,
    pub(super) approved_external_content: BTreeMap<u64, ApprovedExternalContent>,
    pub(super) pending_canonicalizations: VecDeque<PendingCanonicalizationInput>,
    pub(super) canonicalization_task: Option<JoinHandle<anyhow::Result<CanonicalizationTaskResult>>>,
    pub(super) pending_stop_fallback_handoff: Option<PendingStopFallbackHandoff>,
    pub(super) tainted_rebuild_task: Option<JoinHandle<anyhow::Result<TaintedRebuildStepResult>>>,
    pub(super) pending_completions: VecDeque<(u64, tokio::sync::oneshot::Receiver<anyhow::Result<CloseJobCompletion>>)>,
    pub(super) tainted_rebuild_session: Option<mc_db::StoredTaintedRebuildSession>,
    pub(super) tainted_rebuild_handoff_pending: bool,
    pub(super) fallback: FallbackManager,
    /// Re-execution dispatcher for Mixed-mode comparator.
    /// None in BlockifierOnly mode (or before first Mixed-mode block).
    pub(super) reexec_dispatcher: Option<ReexecDispatcherHandle>,
    /// Current re-execution epoch (incremented on fallback / epoch reset).
    pub(super) reexec_epoch: u64,
    /// Current execution epoch. Incremented on fallback so stale forward executor replies
    /// from the old speculative epoch can be discarded safely.
    pub(super) execution_epoch: u64,
    /// Test-only failpoint: if true, `run_comparator_for_block` will return an error
    /// immediately to exercise the fail-safe fallback path.
    #[cfg(test)]
    pub(super) force_comparator_error: bool,
}

impl BlockProductionTask {
    pub(super) fn record_block_stage_metrics(&self) {
        let executing = u64::from(matches!(self.current_state.as_ref(), Some(TaskState::Executing(_))));
        let pending_close = self.pending_completions.len() as u64;
        let pending_diffs = self.diffs_since_snapshot.len() as u64;
        let tracked_total = executing.saturating_add(pending_close).saturating_add(pending_diffs);

        self.metrics.stage_executing_blocks.record(executing, &[]);
        self.metrics.stage_pending_close_completions.record(pending_close, &[]);
        self.metrics.stage_diffs_since_snapshot.record(pending_diffs, &[]);
        self.metrics.stage_tracked_blocks_total.record(tracked_total, &[]);
    }

    pub(super) fn close_queue_capacity(&self) -> usize {
        self.close_queue_capacity
    }

    pub(super) fn next_internal_preconfirmed_block_n_from_backend(&self) -> anyhow::Result<u64> {
        let head = self.backend.chain_head_state();
        let next = head
            .internal_preconfirmed_tip
            .or(head.confirmed_tip)
            .map(|n| n.checked_add(1).context("Block number overflow while computing next block from backend head"))
            .transpose()?
            .unwrap_or(/* genesis */ 0);
        Ok(next)
    }

    pub(super) fn comparator_ignored_storage_addresses(
        chain_config: &mp_chain_config::ChainConfig,
        ignore_fee_token_mismatch: bool,
    ) -> std::collections::BTreeSet<Felt> {
        if ignore_fee_token_mismatch {
            std::collections::BTreeSet::from([
                chain_config.parent_fee_token_address.to_felt(),
                chain_config.native_fee_token_address.to_felt(),
            ])
        } else {
            std::collections::BTreeSet::new()
        }
    }

    /// Creates a new BlockProductionTask.
    ///
    /// # Parameters
    ///
    /// * `mempool_paused`: If true, block production starts with mempool intake paused.
    /// * `no_charge_fee`: Determines whether fees are charged during transaction execution.
    ///
    /// # TODO(mohit 18/11/2025): Update the code to use config same as pre-close
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
        mempool_paused: bool,
        no_charge_fee: bool,
    ) -> Self {
        let (sender, recv) = mpsc::unbounded_channel();
        let (fallback_cmd_tx, fallback_cmd_rx) = mpsc::unbounded_channel();
        let (bypass_input_sender, bypass_tx_input) = mpsc::channel(16);
        let initial_intake = if mempool_paused { MempoolIntakeMode::Paused } else { MempoolIntakeMode::Running };
        let (mempool_intake_tx, mempool_intake_rx) = watch::channel(initial_intake);
        let (tainted_rebuild_active_tx, tainted_rebuild_active_rx) = watch::channel(false);
        let (executor_replay_status_tx, executor_replay_status_rx) =
            watch::channel(crate::fallback::types::RuntimeReplayStatus::idle());
        let (execution_mode_tx, execution_mode_rx) = watch::channel(ExecutionMode::BlockifierOnly);
        let (execution_epoch_tx, execution_epoch_rx) = watch::channel(0u64);
        Self {
            backend: backend.clone(),
            mempool,
            close_queue_capacity: DEFAULT_CLOSE_QUEUE_CAPACITY,
            current_state: None,
            metrics,
            handle: BlockProductionHandle::new(
                backend,
                sender,
                fallback_cmd_tx,
                bypass_input_sender,
                mempool_intake_tx.clone(),
                no_charge_fee,
            ),
            state_notifications: None,
            #[cfg(test)]
            optimistic_pipeline_notifications: None,
            #[cfg(test)]
            comparator_gates: BTreeMap::new(),
            executor_commands_recv: Some(recv),
            fallback_commands_recv: Some(fallback_cmd_rx),
            l1_client,
            bypass_tx_input: Some(bypass_tx_input),
            mempool_intake_rx,
            tainted_rebuild_active_tx,
            tainted_rebuild_active_rx,
            executor_replay_status_tx,
            executor_replay_status_rx,
            execution_mode_tx,
            execution_mode_rx,
            execution_epoch_tx,
            execution_epoch_rx,
            routing_cfg: util::RustExecRoutingConfig::default(),
            no_charge_fee,
            replay_mode_enabled: false,
            diffs_since_snapshot: Vec::new(),
            approved_external_content: BTreeMap::new(),
            pending_canonicalizations: VecDeque::new(),
            canonicalization_task: None,
            pending_stop_fallback_handoff: None,
            tainted_rebuild_task: None,
            pending_completions: VecDeque::new(),
            tainted_rebuild_session: None,
            tainted_rebuild_handoff_pending: false,
            fallback: FallbackManager::new(StartupExecutionMode::BlockifierOnly),
            reexec_dispatcher: None,
            reexec_epoch: 0,
            execution_epoch: 0,
            #[cfg(test)]
            force_comparator_error: false,
        }
    }

    pub fn with_startup_execution_mode(mut self, mode: StartupExecutionMode) -> Self {
        self.fallback = FallbackManager::new(mode);
        self
    }

    pub fn with_close_queue_capacity(mut self, close_queue_capacity: usize) -> anyhow::Result<Self> {
        anyhow::ensure!(
            (1..=MAX_CLOSE_QUEUE_CAPACITY).contains(&close_queue_capacity),
            "close queue capacity must be in 1..={MAX_CLOSE_QUEUE_CAPACITY}"
        );
        self.close_queue_capacity = close_queue_capacity;
        Ok(self)
    }

    pub fn with_replay_mode_enabled(mut self, enabled: bool) -> Self {
        self.replay_mode_enabled = enabled;
        self
    }

    pub fn with_rust_exec_executor_addresses<I>(mut self, executor_addresses: I) -> Self
    where
        I: IntoIterator<Item = Felt>,
    {
        self.routing_cfg.executor_addresses = executor_addresses.into_iter().collect();
        self
    }

    pub fn with_rust_exec_supported_selectors<I>(mut self, supported_selectors: I) -> Self
    where
        I: IntoIterator<Item = Felt>,
    {
        self.routing_cfg.supported_selectors = supported_selectors.into_iter().collect();
        self
    }

    pub fn with_rust_exec_supported_class_hashes<I>(mut self, supported_class_hashes: I) -> Self
    where
        I: IntoIterator<Item = Felt>,
    {
        self.routing_cfg.supported_class_hashes = supported_class_hashes.into_iter().collect();
        self
    }

    pub fn with_rust_exec_batch_size(mut self, rust_batch_size: usize) -> Self {
        self.routing_cfg.rust_batch_size = rust_batch_size.max(1);
        self
    }

    pub fn with_rust_exec_blockifier_batch_size(mut self, blockifier_batch_size: usize) -> Self {
        self.routing_cfg.blockifier_batch_size = blockifier_batch_size.max(1);
        self
    }

    pub fn with_rust_exec_runtime_options(mut self, runtime_options: util::RustExecRuntimeOptions) -> Self {
        self.routing_cfg.runtime_options = runtime_options;
        self
    }

    /// Test-only: force `run_comparator_for_block` to return an error on the next call.
    #[cfg(test)]
    pub fn with_test_force_comparator_error(mut self) -> Self {
        self.force_comparator_error = true;
        self
    }

    pub fn handle(&self) -> BlockProductionHandle {
        self.handle.clone()
    }

    /// This is a channel that helps the testing of the block production task. It is unused outside of tests.
    pub fn subscribe_state_notifications(&mut self) -> mpsc::UnboundedReceiver<BlockProductionStateNotification> {
        let (sender, recv) = mpsc::unbounded_channel();
        self.state_notifications = Some(sender);
        recv
    }

    pub(super) fn send_state_notification(&mut self, notification: BlockProductionStateNotification) {
        if let Some(sender) = self.state_notifications.as_mut() {
            let _ = sender.send(notification);
        }
    }

    #[cfg(test)]
    pub(super) fn subscribe_optimistic_pipeline_notifications(
        &mut self,
    ) -> mpsc::UnboundedReceiver<OptimisticPipelineNotification> {
        let (sender, recv) = mpsc::unbounded_channel();
        self.optimistic_pipeline_notifications = Some(sender);
        recv
    }

    #[cfg(test)]
    pub(super) fn gate_comparator_for_block(&mut self, block_n: u64) -> Arc<tokio::sync::Semaphore> {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        self.comparator_gates.insert(block_n, gate.clone());
        gate
    }

    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        self.close_preconfirmed_block_if_exists().await.context("Cannot close preconfirmed block on startup")?;

        self.tainted_rebuild_session =
            self.backend.get_tainted_rebuild_session().context("Reading tainted rebuild session on startup")?;
        self.tainted_rebuild_handoff_pending = false;
        self.publish_tainted_rebuild_gate();

        if self.tainted_rebuild_session.is_some() {
            self.fallback.mode = ExecutionMode::BlockifierOnly;
            self.fallback.comparator_enabled = false;
            let _ = self.execution_mode_tx.send(self.fallback.mode);
            tracing::info!("startup_tainted_rebuild_resume_pending mode={:?}", self.fallback.mode);
        } else {
            // Startup recovery (preconfirmed re-execution) is now complete.
            // Map startup config to runtime execution mode per design invariant:
            // - startup_mode Mixed  -> switch to Mixed now, enable comparator
            // - startup_mode BlockifierOnly -> remain BlockifierOnly until manual enable
            self.fallback.on_startup_recovery_complete();
            // Propagate post-startup mode to batcher watch channel.
            let _ = self.execution_mode_tx.send(self.fallback.mode);
            tracing::info!(
                "startup_recovery_complete mode={:?} comparator_enabled={}",
                self.fallback.mode,
                self.fallback.comparator_enabled
            );
        }

        // Record initial execution mode gauge.
        let mode_val: u64 = if self.fallback.mode == ExecutionMode::Mixed { 1 } else { 0 };
        self.metrics.execution_mode_current.record(mode_val, &[]);

        // initial state
        let latest_block_n = self.backend.latest_confirmed_block_n();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n });
        self.record_block_stage_metrics();

        Ok(())
    }
}
