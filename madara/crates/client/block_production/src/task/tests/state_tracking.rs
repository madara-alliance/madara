use super::*;

#[rstest::rstest]
#[case::parallel_below_minimum(true, 9, false)]
#[case::parallel_minimum(true, 10, true)]
#[case::serial_any_capacity(false, 1, true)]
fn queue_invariant_matrix(#[case] parallel: bool, #[case] capacity: usize, #[case] expect_ok: bool) {
    let result = validate_parallel_queue_invariant(parallel, capacity);
    assert_eq!(result.is_ok(), expect_ok);
    if !expect_ok {
        let msg = format!("{:#}", result.expect_err("must fail"));
        assert!(msg.contains("QueueInvariantViolated"));
    }
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

#[rstest::rstest]
#[case::prune_nothing(vec![(11, empty_state_diff()), (12, empty_state_diff())], 10, vec![11, 12])]
#[case::prune_prefix(vec![(10, empty_state_diff()), (11, empty_state_diff()), (12, empty_state_diff())], 10, vec![11, 12])]
#[case::prune_all(vec![(10, empty_state_diff())], 10, vec![])]
fn boundary_prune_matrix(
    #[case] mut input: Vec<(u64, StateDiff)>,
    #[case] completed_block_n: u64,
    #[case] expected_blocks: Vec<u64>,
) {
    prune_diffs_since_snapshot(&mut input, completed_block_n);
    let remaining_blocks = input.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
    assert_eq!(remaining_blocks, expected_blocks);
}

#[rstest::rstest]
#[case::from_empty_base(vec![(0, empty_state_diff()), (1, empty_state_diff()), (2, empty_state_diff())], None, 2, 3)]
#[case::from_snapshot_floor(vec![(90, empty_state_diff()), (91, empty_state_diff()), (92, empty_state_diff())], Some(89), 92, 3)]
#[case::skip_pruned_prefix(vec![(90, empty_state_diff()), (91, empty_state_diff()), (92, empty_state_diff())], Some(90), 92, 2)]
fn collect_diffs_for_root_from_base_ok(
    #[case] input: Vec<(u64, StateDiff)>,
    #[case] base_block_n: Option<u64>,
    #[case] target_block_n: u64,
    #[case] expected_len: usize,
) {
    let collected =
        collect_diffs_for_root_from_base(&input, base_block_n, target_block_n).expect("diff span should be contiguous");
    assert_eq!(collected.len(), expected_len);
}

#[test]
fn collect_diffs_for_root_from_base_rejects_gap() {
    let input = vec![(90, empty_state_diff()), (92, empty_state_diff())];
    let err = collect_diffs_for_root_from_base(&input, Some(89), 92).expect_err("gap must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("Missing tracked state diff for block #91"));
}
