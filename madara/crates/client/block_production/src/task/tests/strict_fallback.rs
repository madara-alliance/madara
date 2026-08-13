use super::*;
use mc_exec::execution::TxInfo;

pub(super) fn make_preconfirmed_tx_with_hash(tx_hash: Felt) -> PreconfirmedExecutedTransaction {
    use mp_transactions::validated::TxTimestamp;
    PreconfirmedExecutedTransaction {
        transaction: mp_block::TransactionWithReceipt {
            transaction: Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0::default(),
            )),
            receipt: mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                transaction_hash: tx_hash,
                ..Default::default()
            }),
        },
        state_diff: mp_state_update::TransactionStateUpdate::default(),
        declared_class: None,
        arrived_at: TxTimestamp(0),
        paid_fee_on_l1: None,
    }
}

pub(super) fn carry_row_from_validated(
    seq_no: u64,
    validated: mp_transactions::validated::ValidatedTransaction,
    source_block_n: Option<u64>,
) -> mc_db::StoredTaintedRebuildCarryRow {
    carry_row_from_validated_with_effective_charge_fee(seq_no, validated, source_block_n, None)
}

fn carry_row_from_validated_with_effective_charge_fee(
    seq_no: u64,
    validated: mp_transactions::validated::ValidatedTransaction,
    source_block_n: Option<u64>,
    effective_charge_fee: Option<bool>,
) -> mc_db::StoredTaintedRebuildCarryRow {
    let declared_class = validated.declared_class.clone();
    let arrived_at = validated.arrived_at;
    let effective_charge_fee = effective_charge_fee.unwrap_or(validated.charge_fee);
    mc_db::StoredTaintedRebuildCarryRow {
        seq_no,
        tx: validated,
        declared_class,
        arrived_at,
        source_block_n,
        effective_charge_fee,
    }
}

pub(super) fn persist_preconfirmed_bucket(
    backend: &Arc<MadaraBackend>,
    header: PreconfirmedHeader,
    rows: Vec<PreconfirmedExecutedTransaction>,
) {
    backend.db.write_preconfirmed_header(&header).expect("persist preconfirmed header");
    backend.db.replace_preconfirmed_content_for_block(header.block_number, &rows).expect("persist preconfirmed rows");
}

pub(super) fn tainted_rebuild_control_plane_test_task(
    startup_mode: crate::fallback::types::StartupExecutionMode,
) -> BlockProductionTask {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    BlockProductionTask::new(
        backend,
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    )
    .with_startup_execution_mode(startup_mode)
}

pub(super) fn spawn_test_finalizer(
    task: &BlockProductionTask,
) -> (crate::finalizer::FinalizerHandle, crate::finalizer::FinalizerTaskHandle) {
    crate::finalizer::FinalizerHandle::spawn(
        task.close_queue_capacity(),
        task.metrics.clone(),
        BlockProductionTask::execute_close_payload,
    )
}

pub(super) async fn drain_next_pending_close_completion(
    task: &mut BlockProductionTask,
    close_queue_handle: &crate::finalizer::FinalizerHandle,
) {
    let (expected_block_n, rx) = task.pending_completions.pop_front().expect("pending completion");
    let completion = rx.await.expect("close completion channel").expect("close payload should complete successfully");
    task.handle_close_completion(close_queue_handle, expected_block_n, completion).expect("process close completion");
}

fn strict_stop_mismatch_summary() -> crate::comparator::state_diff::StateDiffMismatchSummary {
    crate::comparator::state_diff::StateDiffMismatchSummary {
        storage_diffs_match: false,
        declared_classes_match: true,
        old_declared_classes_match: true,
        deployed_contracts_match: true,
        replaced_classes_match: true,
        nonces_match: true,
        migrated_compiled_classes_match: true,
        diff_count: 1,
    }
}

fn make_strict_stop_canonicalization_result(backend: Arc<MadaraBackend>, block_n: u64) -> CanonicalizationTaskResult {
    CanonicalizationTaskResult {
        state: CurrentBlockState::with_execution_mode(backend, block_n, crate::fallback::types::ExecutionMode::Mixed),
        canonical_result: Ok(CanonicalizationTaskCanonical {
            canonical: crate::comparator::CanonicalizedBlockOutput {
                source: CanonicalBlockSource::BlockifierReexec,
                state_diff: StateDiff::default(),
                bouncer_weights: BouncerWeights::empty(),
                bre_per_tx: None,
            },
            stop_reason: Some(crate::comparator::StopReason::StateDiffMismatch {
                summary: strict_stop_mismatch_summary(),
            }),
        }),
        dispatcher: None,
    }
}

fn make_accepted_blockifier_canonicalization_result(
    backend: Arc<MadaraBackend>,
    block_n: u64,
) -> CanonicalizationTaskResult {
    let mut result = make_strict_stop_canonicalization_result(backend, block_n);
    result.canonical_result.as_mut().expect("canonical result").stop_reason = None;
    result
}

pub(super) async fn apply_tainted_rebuild_step_result(
    task: &mut BlockProductionTask,
    result: TaintedRebuildStepResult,
) {
    task.tainted_rebuild_session = result.live_session_after_step;
    let (close_queue_handle, close_queue_task) = spawn_test_finalizer(task);
    let TaintedRebuildClosePayload {
        state,
        canonical_bouncer_weights,
        state_diff,
        canonical_executed_rows,
        canonical_header,
    } = result.close_payload;
    task.enqueue_canonical_close_payload(
        &close_queue_handle,
        state,
        canonical_bouncer_weights,
        state_diff,
        canonical_executed_rows,
        canonical_header,
    )
    .await
    .expect("enqueue tainted rebuild close");
    drain_next_pending_close_completion(task, &close_queue_handle).await;
    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn strict_stop_handoff_returns_before_executor_carry_reply_arrives() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("seed preconfirmed block");

    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let metrics = Arc::new(BlockProductionMetrics::register());
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        metrics.clone(),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );
    task.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(0) });

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);

    task.handle_canonicalization_result(
        make_strict_stop_canonicalization_result(backend.clone(), 0),
        &close_queue_handle,
    )
    .await
    .expect("strict stop handoff should queue carry request without blocking");

    assert!(task.pending_stop_fallback_handoff.is_some(), "strict stop must retain a pending handoff");
    assert!(task.pending_completions.is_empty(), "close flow must wait until carry handoff completes");
    assert!(task.tainted_rebuild_handoff_pending, "handoff gate must stay closed while carry is pending");

    let command = task
        .executor_commands_recv
        .as_mut()
        .expect("executor command receiver still owned by test task")
        .try_recv()
        .expect("strict stop should have queued a carry request");
    match command {
        crate::executor::ExecutorCommand::PrepareTaintedRebuildFallback { block_n, execution_epoch, .. } => {
            assert_eq!(block_n, 0);
            assert_eq!(execution_epoch, 1, "strict stop must advance the execution epoch before the carry handoff");
        }
        other => panic!("unexpected executor command during pending strict stop handoff: {other:?}"),
    }

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn strict_stop_handoff_can_drain_stale_replies_before_carry_completion() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("seed preconfirmed block");

    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let metrics = Arc::new(BlockProductionMetrics::register());
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        metrics.clone(),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );
    task.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(0) });

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);

    task.handle_canonicalization_result(
        make_strict_stop_canonicalization_result(backend.clone(), 0),
        &close_queue_handle,
    )
    .await
    .expect("strict stop handoff should queue carry request without blocking");

    task.process_reply(
        ExecutorMessage::StartNewBlock {
            exec_ctx: crate::util::BlockExecutionContext {
                block_number: 1,
                sequencer_address: Default::default(),
                block_timestamp: std::time::UNIX_EPOCH,
                protocol_version: Default::default(),
                gas_prices: Default::default(),
                l1_da_mode: Default::default(),
            },
            execution_mode: crate::fallback::types::ExecutionMode::Mixed,
            execution_epoch: 0,
        },
        &close_queue_handle,
    )
    .await
    .expect("stale forward reply should be drainable while carry handoff is pending");
    assert!(
        backend.block_view_on_preconfirmed(1).is_none(),
        "stale StartNewBlock must still be dropped while the strict stop carry handoff is pending"
    );

    let command = task
        .executor_commands_recv
        .as_mut()
        .expect("executor command receiver still owned by test task")
        .try_recv()
        .expect("strict stop should have queued a carry request");
    let crate::executor::ExecutorCommand::PrepareTaintedRebuildFallback { reply, .. } = command else {
        panic!("unexpected executor command during strict stop handoff");
    };
    reply.send(Ok(Vec::new())).expect("send carry reply");

    task.complete_pending_stop_fallback_handoff(&close_queue_handle)
        .await
        .expect("carry completion should resume strict stop close flow");

    assert!(task.pending_stop_fallback_handoff.is_none(), "carry completion must clear the pending handoff");
    assert_eq!(task.pending_completions.len(), 1, "strict stop close payload should enqueue after carry completion");

    drain_next_pending_close_completion(&mut task, &close_queue_handle).await;
    assert_eq!(
        backend.latest_confirmed_block_n(),
        Some(0),
        "strict stop close flow should still complete after draining stale executor replies first"
    );

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn ready_strict_stop_canonicalization_preempts_forward_progress_before_more_reply_drain() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("seed preconfirmed block");

    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let metrics = Arc::new(BlockProductionMetrics::register());
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        metrics.clone(),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );
    task.current_state = Some(TaskState::Executing(CurrentBlockState::with_execution_mode(
        backend.clone(),
        1,
        crate::fallback::types::ExecutionMode::Mixed,
    )));

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);

    let strict_stop = make_strict_stop_canonicalization_result(backend.clone(), 0);
    task.canonicalization_task = Some(tokio::spawn(async move { Ok(strict_stop) }));
    tokio::task::yield_now().await;

    let processed = task
        .maybe_process_ready_priority_work(&close_queue_handle)
        .await
        .expect("ready strict-stop canonicalization should be handled");
    assert!(processed, "ready canonicalization must be consumed before more forward reply work");
    assert!(task.pending_stop_fallback_handoff.is_some(), "strict stop should install a pending handoff");
    assert_eq!(task.execution_epoch, 1, "strict stop should advance the execution epoch");
    assert!(
        matches!(task.current_state, Some(TaskState::NotExecuting { latest_block_n: Some(0) })),
        "strict stop should clamp stale forward execution state before more reply drain"
    );

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn accepted_blockifier_canonical_handoff_fences_late_successor_batch() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    for block_number in 0..=1 {
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number, ..Default::default() }))
            .expect("seed speculative block");
    }

    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let metrics = Arc::new(BlockProductionMetrics::register());
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        metrics.clone(),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );
    task.current_state = Some(TaskState::Executing(CurrentBlockState::with_execution_mode(
        backend.clone(),
        1,
        crate::fallback::types::ExecutionMode::Mixed,
    )));

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);

    task.handle_canonicalization_result(
        make_accepted_blockifier_canonicalization_result(backend.clone(), 0),
        &close_queue_handle,
    )
    .await
    .expect("accepted Blockifier canonical output should start a safe handoff");

    assert_eq!(task.execution_epoch, 1, "canonical substitution must fence the old executor epoch");
    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(0));
    assert!(matches!(task.current_state, Some(TaskState::NotExecuting { latest_block_n: Some(0) })));
    assert_eq!(task.fallback.reason, Some(crate::fallback::types::FallbackReason::BlockifierCanonicalSubstitution));

    task.process_reply(
        ExecutorMessage::BatchExecuted(BatchExecutionResult {
            executed_txs: crate::util::BatchToExecute::default(),
            blockifier_results: vec![],
            original_tx_hashes: vec![],
            stats: crate::util::ExecutionStats { n_executed: 1, n_added_to_block: 1, ..Default::default() },
            execution_mode: crate::fallback::types::ExecutionMode::Mixed,
            execution_epoch: 0,
            emitted_at: std::time::Instant::now(),
        }),
        &close_queue_handle,
    )
    .await
    .expect("late successor BatchExecuted must be dropped instead of touching the rewound frontier");

    assert_eq!(backend.chain_head_state().internal_preconfirmed_tip, Some(0));
    assert!(task.pending_stop_fallback_handoff.is_some());

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

pub(super) fn make_validated_invoke_tx(
    sender: &DevnetPredeployedContract,
    receiver: &DevnetPredeployedContract,
    backend: &Arc<MadaraBackend>,
    nonce: Felt,
) -> mp_transactions::validated::ValidatedTransaction {
    let arrived_at = mp_transactions::validated::TxTimestamp::now();
    let (api_tx, converted_class) = BroadcastedTxn::Invoke(make_invoke_tx(
        sender,
        Multicall::default().with(Call {
            to: Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
            selector: Selector::from("transfer"),
            calldata: vec![receiver.address, (9_999u128 * 1_000_000_000_000_000_000).into(), Felt::ZERO],
        }),
        backend,
        nonce,
    ))
    .into_starknet_api(backend.chain_config().chain_id.to_felt(), backend.chain_config().latest_protocol_version)
    .expect("convert invoke to starknet api");

    mp_transactions::validated::ValidatedTransaction::from_starknet_api(api_tx, arrived_at, converted_class, true)
}

pub(super) fn rust_transfer_routing_cfg(
    backend: &Arc<MadaraBackend>,
    executor: &DevnetPredeployedContract,
    rust_batch_size: usize,
    blockifier_batch_size: usize,
) -> crate::util::RustExecRoutingConfig {
    let transfer_selector = get_selector_from_name("transfer").expect("transfer selector");
    let fee_token_address =
        Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
    let fee_token_class_hash = backend
        .view_on_latest_confirmed()
        .get_contract_class_hash(&fee_token_address)
        .expect("fee token class hash lookup")
        .expect("fee token must be deployed in devnet genesis");

    crate::util::RustExecRoutingConfig {
        executor_addresses: HashSet::from([executor.address]),
        supported_selectors: HashSet::from([transfer_selector]),
        supported_class_hashes: HashSet::from([fee_token_class_hash]),
        rust_batch_size,
        blockifier_batch_size,
        runtime_options: Default::default(),
    }
}

fn validated_tx_nonce(tx: &mp_transactions::validated::ValidatedTransaction) -> Felt {
    match &tx.transaction {
        Transaction::Invoke(tx) => *tx.nonce(),
        Transaction::Declare(tx) => *tx.nonce(),
        Transaction::DeployAccount(tx) => *tx.nonce(),
        Transaction::L1Handler(tx) => tx.nonce.into(),
        Transaction::Deploy(_) => panic!("deploy txs are not used in batcher routing cap tests"),
    }
}

pub(super) async fn assert_tx_not_in_mempool(
    mempool: &Arc<Mempool>,
    tx: &mp_transactions::validated::ValidatedTransaction,
) {
    let recovered_hash =
        mempool.get_transaction(tx.contract_address, validated_tx_nonce(tx), |stored| stored.hash).await;
    assert_eq!(
        recovered_hash, None,
        "tx {:#x} should stay in batcher-local carry instead of being reinserted into mempool",
        tx.hash
    );
}

pub(super) async fn recv_routed_batch(
    out_rx: &mut mpsc::Receiver<crate::util::RoutedBatchToExecute>,
) -> crate::util::RoutedBatchToExecute {
    tokio::time::timeout(Duration::from_secs(5), out_rx.recv())
        .await
        .expect("routed batch should arrive")
        .expect("routed batch should exist")
}

pub(super) fn routed_batch_hashes(batch: &crate::util::RoutedBatchToExecute) -> (Vec<Felt>, Vec<Felt>) {
    let blockifier_hashes = batch.blockifier_batch.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect();
    let rust_hashes = batch.rust_batch.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect();
    (blockifier_hashes, rust_hashes)
}

pub(super) fn spawn_batcher_with_bypass_txs(
    devnet_setup: &DevnetSetup,
    execution_mode: crate::fallback::types::ExecutionMode,
    routing_cfg: crate::util::RustExecRoutingConfig,
    bypass_txs: Vec<mp_transactions::validated::ValidatedTransaction>,
    mempool_mode: crate::MempoolIntakeMode,
) -> (ServiceContext, tokio::task::JoinHandle<()>, mpsc::Receiver<crate::util::RoutedBatchToExecute>) {
    use crate::batcher::Batcher;

    let (out_tx, out_rx) = mpsc::channel(1);
    let (bypass_tx, bypass_rx) = mpsc::channel(bypass_txs.len().max(1));
    for tx in bypass_txs {
        bypass_tx.try_send(tx).expect("queue bypass tx");
    }
    drop(bypass_tx);
    let (mempool_intake_tx, mempool_intake_rx) = watch::channel(mempool_mode);
    let (_tainted_rebuild_active_tx, tainted_rebuild_active_rx) = watch::channel(false);
    let (_execution_mode_tx, execution_mode_rx) = watch::channel(execution_mode);
    let (_execution_epoch_tx, execution_epoch_rx) = watch::channel(0u64);
    let ctx = ServiceContext::new_for_testing();
    let ctx_for_cancel = ctx.clone();

    let batcher = Batcher::new(
        devnet_setup.backend.clone(),
        devnet_setup.mempool.clone(),
        Arc::new(devnet_setup.l1_client.clone()),
        ctx,
        out_tx,
        bypass_rx,
        mempool_intake_rx,
        mempool_intake_tx,
        tainted_rebuild_active_rx,
        execution_mode_rx,
        execution_epoch_rx,
        routing_cfg,
        devnet_setup.metrics.clone(),
        false,
    );
    let batcher_task = tokio::spawn(async move { batcher.run().await.unwrap() });
    (ctx_for_cancel, batcher_task, out_rx)
}

pub(super) async fn seed_real_preconfirmed_block(
    setup: &mut DevnetSetup,
    sender_idx: usize,
    receiver_idx: usize,
    nonce: Felt,
) -> (PreconfirmedHeader, Vec<PreconfirmedExecutedTransaction>) {
    sign_and_add_invoke_tx(
        &setup.contracts.0[sender_idx],
        &setup.contracts.0[receiver_idx],
        &setup.backend,
        &setup.tx_validator,
        nonce,
    )
    .await;

    let mut block_production_task = setup.block_prod_task();
    let mut notifications = block_production_task.subscribe_state_notifications();
    let task =
        AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });

    assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

    let preconfirmed_view = setup.backend.block_view_on_current_preconfirmed().expect("preconfirmed block");
    let header = preconfirmed_view.block().header.clone();
    let rows = preconfirmed_view.borrow_content().executed_transactions().cloned().collect::<Vec<_>>();
    drop(task);
    tokio::time::sleep(Duration::from_millis(200)).await;

    (header, rows)
}

#[test]
fn stop_on_anchor_drops_stale_queue_entries_but_preserves_descendant_buckets() {
    use crate::fallback::types::ExecutionMode;

    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );

    let anchor_header = PreconfirmedHeader { block_number: 5, ..Default::default() };
    let anchor_rows = vec![make_preconfirmed_tx_with_hash(Felt::from(0x55u64))];
    persist_preconfirmed_bucket(&backend, anchor_header.clone(), anchor_rows.clone());
    persist_preconfirmed_bucket(
        &backend,
        PreconfirmedHeader { block_number: 6, ..Default::default() },
        vec![make_preconfirmed_tx_with_hash(Felt::from(0x66u64))],
    );
    persist_preconfirmed_bucket(
        &backend,
        PreconfirmedHeader { block_number: 7, ..Default::default() },
        vec![make_preconfirmed_tx_with_hash(Felt::from(0x77u64))],
    );

    for block_n in 5..=7 {
        task.pending_canonicalizations.push_back(PendingCanonicalizationInput {
            state: CurrentBlockState::with_execution_mode(backend.clone(), block_n, ExecutionMode::Mixed),
            block_exec_summary: Box::new(make_empty_block_exec_summary()),
        });
    }

    task.drop_descendant_pending_canonicalizations(5);
    task.install_tainted_rebuild_session(
        5,
        anchor_header,
        anchor_rows,
        vec![carry_row_from_validated(0, make_preconfirmed_tx_with_hash(Felt::from(0x88u64)).to_validated(), Some(5))],
    )
    .expect("install tainted rebuild session");

    assert_eq!(task.pending_canonicalizations.len(), 1, "only the anchor canonicalization should remain queued");
    assert_eq!(task.pending_canonicalizations.front().unwrap().state.block_number, 5);
    assert!(
        backend.db.get_preconfirmed_block_data(6).expect("read descendant block 6").is_some(),
        "persisted descendant block 6 must remain available as rebuild input"
    );
    assert!(
        backend.db.get_preconfirmed_block_data(7).expect("read descendant block 7").is_some(),
        "persisted descendant block 7 must remain available as rebuild input"
    );
    assert_eq!(
        task.tainted_rebuild_session,
        Some(mc_db::StoredTaintedRebuildSession {
            execution_epoch: 0,
            anchor_block_n: 5,
            next_block_n: 6,
            tail_block_n: 7,
            active: true,
        })
    );
}
