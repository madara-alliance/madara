use super::*;

#[test]
fn optimistic_window_configures_serial_close_queue_capacity() {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()));
    let metrics = Arc::new(BlockProductionMetrics::register());
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let l1_client = Arc::new(L1ClientMock::new());
    let new_task =
        || BlockProductionTask::new(backend.clone(), mempool.clone(), metrics.clone(), l1_client.clone(), false, false);

    assert_eq!(new_task().close_queue_capacity(), 10);
    assert_eq!(new_task().with_close_queue_capacity(4).expect("valid capacity").close_queue_capacity(), 4);
    assert!(new_task().with_close_queue_capacity(0).is_err());
    assert!(new_task().with_close_queue_capacity(11).is_err());
}

#[test]
fn internal_preconfirmed_window_reports_authoritative_depth() {
    let empty = InternalPreconfirmedWindowSnapshot::from_tips(None, None, None, 10);
    assert_eq!(empty.depth, 0);

    let full = InternalPreconfirmedWindowSnapshot::from_tips(Some(100), Some(101), Some(110), 10);
    assert_eq!(full.depth, 10);

    let draining = InternalPreconfirmedWindowSnapshot::from_tips(Some(105), Some(106), Some(110), 10);
    assert_eq!(draining.depth, 5);
    assert_eq!(draining.confirmed_advance_from(Some(104)), 1);

    let fallback_rewind = InternalPreconfirmedWindowSnapshot::from_tips(Some(105), Some(106), Some(106), 10);
    assert_eq!(fallback_rewind.depth, 1);
}

#[test]
fn preconfirmed_runahead_is_accepted_before_previous_close_completes() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block #0 should succeed");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating preconfirmed block #1 should succeed while #0 is still externally visible");

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, None);
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(1));
}

#[tokio::test]
async fn start_new_block_uses_backend_authoritative_internal_tip() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block #0 should succeed");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating preconfirmed block #1 should succeed while #0 is still externally visible");

    let metrics = Arc::new(BlockProductionMetrics::register());
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let l1_client = Arc::new(L1ClientMock::new());
    let mut task = BlockProductionTask::new(backend.clone(), mempool, metrics.clone(), l1_client, false, false);

    // Simulate a stale task-local cursor while backend has already advanced to internal tip #1.
    task.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(0) });

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);
    let header = PreconfirmedHeader { block_number: 2, ..Default::default() };
    let exec_ctx = crate::util::BlockExecutionContext {
        block_number: header.block_number,
        sequencer_address: header.sequencer_address,
        block_timestamp: std::time::UNIX_EPOCH + Duration::from_secs(header.block_timestamp.0),
        protocol_version: header.protocol_version,
        gas_prices: header.gas_prices.clone(),
        l1_da_mode: header.l1_da_mode,
    };

    task.process_reply(
        ExecutorMessage::StartNewBlock {
            exec_ctx: exec_ctx.clone(),
            execution_mode: crate::fallback::types::ExecutionMode::BlockifierOnly,
            execution_epoch: 0,
        },
        &close_queue_handle,
    )
    .await
    .expect("StartNewBlock should follow backend head, not stale task-local latest_block_n");

    let head = backend.chain_head_state();
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(2));

    let runtime_view = backend.block_view_on_preconfirmed(2).expect("runtime preconfirmed block #2 should exist");
    assert_eq!(
        runtime_view.block().header.block_number,
        2,
        "runtime preconfirmed block should advance to backend-authoritative next"
    );

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn stale_start_new_block_is_dropped_after_fallback() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    let metrics = Arc::new(BlockProductionMetrics::register());
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let l1_client = Arc::new(L1ClientMock::new());
    let mut task = BlockProductionTask::new(backend.clone(), mempool, metrics.clone(), l1_client, false, false);
    task.current_state = Some(super::TaskState::NotExecuting { latest_block_n: Some(0) });
    task.execution_epoch = 1;

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);
    let exec_ctx = crate::util::BlockExecutionContext {
        block_number: 1,
        sequencer_address: Default::default(),
        block_timestamp: std::time::UNIX_EPOCH,
        protocol_version: Default::default(),
        gas_prices: Default::default(),
        l1_da_mode: Default::default(),
    };

    task.process_reply(
        ExecutorMessage::StartNewBlock {
            exec_ctx,
            execution_mode: crate::fallback::types::ExecutionMode::BlockifierOnly,
            execution_epoch: 0,
        },
        &close_queue_handle,
    )
    .await
    .expect("stale StartNewBlock should be dropped");

    assert!(!backend.has_preconfirmed_block(), "stale StartNewBlock must not create a runtime preconfirmed block");
    assert!(matches!(task.current_state, Some(super::TaskState::NotExecuting { latest_block_n: Some(0) })));

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn redundant_start_new_block_is_dropped_after_head_advance() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
        .expect("creating preconfirmed block #0 should succeed");
    backend
        .write_access()
        .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
        .expect("creating preconfirmed block #1 should succeed");

    let metrics = Arc::new(BlockProductionMetrics::register());
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let l1_client = Arc::new(L1ClientMock::new());
    let mut task = BlockProductionTask::new(backend.clone(), mempool, metrics.clone(), l1_client, false, false);
    task.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(0) });
    task.execution_epoch = 1;

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);
    let exec_ctx = crate::util::BlockExecutionContext {
        block_number: 0,
        sequencer_address: Default::default(),
        block_timestamp: std::time::UNIX_EPOCH,
        protocol_version: Default::default(),
        gas_prices: Default::default(),
        l1_da_mode: Default::default(),
    };

    task.process_reply(
        ExecutorMessage::StartNewBlock {
            exec_ctx,
            execution_mode: crate::fallback::types::ExecutionMode::BlockifierOnly,
            execution_epoch: 1,
        },
        &close_queue_handle,
    )
    .await
    .expect("redundant StartNewBlock for an already-closed block should be dropped");

    let head = backend.chain_head_state();
    assert_eq!(head.confirmed_tip, None);
    assert_eq!(head.external_preconfirmed_tip, Some(0));
    assert_eq!(head.internal_preconfirmed_tip, Some(1));
    assert!(matches!(task.current_state, Some(TaskState::NotExecuting { latest_block_n: Some(0) })));

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn stale_batch_executed_is_dropped_after_fallback() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_devnet()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );

    let metrics = Arc::new(BlockProductionMetrics::register());
    let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
    let l1_client = Arc::new(L1ClientMock::new());
    let mut task = BlockProductionTask::new(backend.clone(), mempool, metrics.clone(), l1_client, false, false);
    task.current_state = Some(super::TaskState::Executing(super::CurrentBlockState::new(backend.clone(), 1)));
    task.execution_epoch = 1;

    let (close_queue_handle, close_queue_task) =
        crate::finalizer::FinalizerHandle::spawn(1, metrics, BlockProductionTask::execute_close_payload);

    task.process_reply(
        super::ExecutorMessage::BatchExecuted(super::BatchExecutionResult {
            executed_txs: crate::util::BatchToExecute::default(),
            original_tx_hashes: vec![],
            blockifier_results: vec![],
            stats: crate::util::ExecutionStats { n_executed: 1, n_added_to_block: 1, ..Default::default() },
            execution_mode: crate::fallback::types::ExecutionMode::Mixed,
            execution_epoch: 0,
            emitted_at: std::time::Instant::now(),
        }),
        &close_queue_handle,
    )
    .await
    .expect("stale BatchExecuted should be dropped");

    let Some(super::TaskState::Executing(state)) = task.current_state.as_ref() else {
        panic!("current state should remain executing");
    };
    assert_eq!(state.block_number, 1);
    assert!(state.speculative_executed_txs.is_empty(), "stale mixed batch must not be appended");
    assert_eq!(state.accumulated_stats.n_executed, 0, "stale mixed batch must not affect close stats");

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}
