use super::*;

#[tokio::test]
async fn tainted_rebuild_waits_for_anchor_confirm() {
    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend,
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    );

    task.current_state = Some(super::TaskState::NotExecuting { latest_block_n: Some(0) });
    task.tainted_rebuild_session = Some(mc_db::StoredTaintedRebuildSession {
        execution_epoch: 0,
        anchor_block_n: 1,
        next_block_n: 2,
        tail_block_n: 2,
        active: true,
    });
    task.publish_tainted_rebuild_gate();

    task.maybe_start_tainted_rebuild_task().await.expect("anchor-confirm gate should not error");
    assert!(task.tainted_rebuild_task.is_none(), "rebuild must not start before the anchor is confirmed");
}

#[test]
fn executionbox_status_and_enable_remain_safe_while_tainted_rebuild_is_active() {
    use crate::fallback::manager::EnableError;
    use crate::fallback::types::{ExecutionMode, StartupExecutionMode};

    let mut task = tainted_rebuild_control_plane_test_task(StartupExecutionMode::Mixed);
    task.fallback.on_startup_recovery_complete();
    task.tainted_rebuild_session = Some(mc_db::StoredTaintedRebuildSession {
        execution_epoch: task.execution_epoch,
        anchor_block_n: 22,
        next_block_n: 23,
        tail_block_n: 25,
        active: true,
    });
    task.publish_tainted_rebuild_gate();

    let status = task.executionbox_status_snapshot();
    assert_eq!(status.mode, ExecutionMode::Mixed, "manager-owned desired mode should be preserved");
    assert!(!status.replay_backlog_empty, "active tainted rebuild must surface as in-progress replay");
    assert!(status.replay_supported);
    assert_eq!(
        task.executionbox_enable_from_control_plane(),
        Err(EnableError::ReplayInProgress),
        "manual enable must stay blocked while tainted rebuild is active"
    );
}

#[test]
fn final_overflow_rebuild_step_advances_durable_resume_cursor() {
    let session = mc_db::StoredTaintedRebuildSession {
        execution_epoch: 7,
        anchor_block_n: 10,
        next_block_n: 12,
        tail_block_n: 11,
        active: true,
    };

    let drained_session = BlockProductionTask::tainted_rebuild_live_session_after_step(&session, None)
        .expect("drained session marker must survive until the final close is acknowledged");

    assert_eq!(drained_session.next_block_n, 13, "cursor must advance past the overflow block that just closed");
    assert_eq!(
        drained_session.next_block_n - 1,
        session.next_block_n,
        "restart must expect the final overflow block as the confirmed head"
    );
}

#[tokio::test]
async fn drained_tainted_rebuild_waits_for_resume_ack_before_gate_reopens() {
    use crate::fallback::types::StartupExecutionMode;

    let mut task = tainted_rebuild_control_plane_test_task(StartupExecutionMode::BlockifierOnly);
    let mut executor_commands_recv = task.executor_commands_recv.take().expect("executor command receiver");
    task.backend
        .write_access()
        .add_full_block_with_classes(
            &mp_block::FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: StateDiff::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .expect("seed confirmed resume head");
    let session = mc_db::StoredTaintedRebuildSession {
        execution_epoch: task.execution_epoch,
        anchor_block_n: 0,
        next_block_n: 1,
        tail_block_n: 0,
        active: true,
    };
    task.backend.write_tainted_rebuild_session(&session).expect("persist drained rebuild session");
    task.tainted_rebuild_session = Some(session);
    task.publish_tainted_rebuild_gate();
    assert!(task.tainted_rebuild_active(), "rebuild gate should start closed");

    let execution_epoch = task.execution_epoch;
    let responder = tokio::spawn(async move {
        let command = executor_commands_recv.recv().await.expect("resume command");
        let crate::executor::ExecutorCommand::ResumeAfterTaintedRebuild {
            expected_confirmed_head,
            execution_epoch: command_epoch,
            reply,
        } = command
        else {
            panic!("expected acknowledged tainted rebuild resume");
        };
        assert_eq!(expected_confirmed_head, 0);
        assert_eq!(command_epoch, execution_epoch);
        reply
            .send(Ok(crate::executor::TaintedRebuildResumeAck { confirmed_head: 0, next_block_n: 1, execution_epoch }))
            .expect("send resume acknowledgement");
    });

    let drained =
        task.maybe_finish_tainted_rebuild_if_drained().await.expect("finishing drained tainted rebuild should succeed");
    responder.await.expect("resume responder");

    assert!(drained, "drained tainted rebuild should be detected");
    assert!(task.tainted_rebuild_session.is_none(), "drained rebuild must clear the in-memory session");
    assert!(task.backend.get_tainted_rebuild_session().expect("read rebuild session").is_none());
    assert!(!task.tainted_rebuild_active(), "rebuild gate should reopen only after resume is acknowledged");
}

#[rstest::rstest]
#[case(1)]
#[case(2)]
#[tokio::test]
async fn restart_after_final_overflow_close_reconciles_and_remains_blockifier_only(
    #[case] persisted_next_block_n: u64,
) {
    use crate::fallback::types::{ExecutionMode, StartupExecutionMode};

    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    for block_number in 0..=1 {
        backend
            .write_access()
            .add_full_block_with_classes(
                &mp_block::FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number, ..Default::default() },
                    state_diff: StateDiff::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .expect("seed confirmed overflow head");
    }
    let persisted_session = mc_db::StoredTaintedRebuildSession {
        execution_epoch: 9,
        anchor_block_n: 0,
        next_block_n: persisted_next_block_n,
        tail_block_n: 0,
        active: true,
    };
    backend.write_tainted_rebuild_session(&persisted_session).expect("persist rebuild cursor");
    backend.refresh_head_projection_from_db().expect("refresh confirmed head");

    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend,
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    )
    .with_startup_execution_mode(StartupExecutionMode::Mixed);
    task.setup_initial_state().await.expect("restore stale rebuild session");
    assert_eq!(task.fallback.mode, ExecutionMode::BlockifierOnly);

    let mut executor_commands_recv = task.executor_commands_recv.take().expect("executor command receiver");
    let responder = tokio::spawn(async move {
        let command = executor_commands_recv.recv().await.expect("resume command");
        let crate::executor::ExecutorCommand::ResumeAfterTaintedRebuild {
            expected_confirmed_head,
            execution_epoch,
            reply,
        } = command
        else {
            panic!("expected acknowledged tainted rebuild resume");
        };
        assert_eq!(expected_confirmed_head, 1);
        assert_eq!(execution_epoch, 9);
        reply
            .send(Ok(crate::executor::TaintedRebuildResumeAck { confirmed_head: 1, next_block_n: 2, execution_epoch }))
            .expect("send resume acknowledgement");
    });

    assert!(task.maybe_finish_tainted_rebuild_if_drained().await.expect("reconcile completed final overflow close"));
    responder.await.expect("resume responder");

    assert!(task.tainted_rebuild_session.is_none());
    assert!(task.backend.get_tainted_rebuild_session().expect("read rebuild session").is_none());
    assert!(!task.tainted_rebuild_active());
    assert_eq!(task.fallback.mode, ExecutionMode::BlockifierOnly);
}

#[tokio::test]
async fn drained_tainted_rebuild_keeps_durable_gate_when_resume_cannot_be_queued() {
    use crate::fallback::types::StartupExecutionMode;

    let mut task = tainted_rebuild_control_plane_test_task(StartupExecutionMode::BlockifierOnly);
    drop(task.executor_commands_recv.take().expect("executor command receiver"));
    task.backend
        .write_access()
        .add_full_block_with_classes(
            &mp_block::FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: StateDiff::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .expect("seed confirmed resume head");
    let session = mc_db::StoredTaintedRebuildSession {
        execution_epoch: task.execution_epoch,
        anchor_block_n: 0,
        next_block_n: 1,
        tail_block_n: 0,
        active: true,
    };
    task.backend.write_tainted_rebuild_session(&session).expect("persist drained rebuild session");
    task.tainted_rebuild_session = Some(session);
    task.publish_tainted_rebuild_gate();

    let err = task
        .maybe_finish_tainted_rebuild_if_drained()
        .await
        .expect_err("resume must fail when the executor command channel is closed");

    assert!(format!("{err:#}").contains("Requesting executor resume"));
    assert!(task.tainted_rebuild_session.is_some(), "failed resume must preserve the durable session");
    assert!(task.backend.get_tainted_rebuild_session().expect("read rebuild session").is_some());
    assert!(task.tainted_rebuild_active(), "failed resume must keep the rebuild gate closed");
}

#[tokio::test]
async fn active_tainted_rebuild_close_completion_keeps_executor_parked() {
    use crate::fallback::types::StartupExecutionMode;

    let mut task = tainted_rebuild_control_plane_test_task(StartupExecutionMode::BlockifierOnly);
    let mut executor_commands_recv = task.executor_commands_recv.take().expect("executor command receiver");
    task.tainted_rebuild_session = Some(mc_db::StoredTaintedRebuildSession {
        execution_epoch: task.execution_epoch,
        anchor_block_n: 22,
        next_block_n: 26,
        tail_block_n: 30,
        active: true,
    });
    task.publish_tainted_rebuild_gate();

    let (close_queue_handle, close_queue_task) = spawn_test_finalizer(&task);
    task.handle_close_completion(&close_queue_handle, 25, crate::close_queue::CloseJobCompletion { block_n: 25 })
        .await
        .expect("processing active tainted rebuild close completion");

    assert!(matches!(executor_commands_recv.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
    assert!(task.tainted_rebuild_active(), "active rebuild must keep the executor and batcher parked");

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn active_tainted_rebuild_close_completion_needs_no_executor_command() {
    use crate::fallback::types::StartupExecutionMode;

    let mut task = tainted_rebuild_control_plane_test_task(StartupExecutionMode::BlockifierOnly);
    drop(task.executor_commands_recv.take().expect("executor command receiver"));
    task.tainted_rebuild_session = Some(mc_db::StoredTaintedRebuildSession {
        execution_epoch: task.execution_epoch,
        anchor_block_n: 22,
        next_block_n: 26,
        tail_block_n: 30,
        active: true,
    });
    task.publish_tainted_rebuild_gate();

    let (close_queue_handle, close_queue_task) = spawn_test_finalizer(&task);
    task.handle_close_completion(&close_queue_handle, 25, crate::close_queue::CloseJobCompletion { block_n: 25 })
        .await
        .expect("processing active tainted rebuild close completion should tolerate executor shutdown");

    assert_eq!(
        task.tainted_rebuild_session.as_ref().map(|session| session.next_block_n),
        Some(26),
        "active session should remain intact without per-close executor resync"
    );

    drop(close_queue_handle);
    close_queue_task.join().await.expect("finalizer should shut down cleanly");
}

#[tokio::test]
async fn startup_resume_restores_tainted_rebuild_session_and_blocks_mixed_enable() {
    use crate::fallback::manager::EnableError;
    use crate::fallback::types::{ExecutionMode, StartupExecutionMode};
    use mp_block::FullBlockWithoutCommitments;

    let backend = MadaraBackend::open_for_testing_with_config(
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
    );
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: StateDiff::default(),
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .expect("seed confirmed anchor block");

    persist_preconfirmed_bucket(
        &backend,
        PreconfirmedHeader { block_number: 1, ..Default::default() },
        vec![make_preconfirmed_tx_with_hash(Felt::from(0x11u64))],
    );
    backend
        .write_tainted_rebuild_session(&mc_db::StoredTaintedRebuildSession {
            execution_epoch: 7,
            anchor_block_n: 0,
            next_block_n: 1,
            tail_block_n: 1,
            active: true,
        })
        .expect("persist tainted rebuild session");
    backend
        .replace_tainted_rebuild_carry_rows(&[carry_row_from_validated(
            0,
            make_preconfirmed_tx_with_hash(Felt::from(0x12u64)).to_validated(),
            None,
        )])
        .expect("persist tainted rebuild carry");
    backend.refresh_head_projection_from_db().expect("refresh head projection");

    let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
    let mut task = BlockProductionTask::new(
        backend.clone(),
        mempool,
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
    )
    .with_startup_execution_mode(StartupExecutionMode::Mixed);

    task.setup_initial_state().await.expect("setup initial state");

    assert_eq!(task.fallback.mode, ExecutionMode::BlockifierOnly);
    assert!(!task.fallback.comparator_enabled, "resume must keep comparator disabled during rebuild");
    assert!(task.fallback.startup_recovery_active, "startup recovery stays active until rebuild drains");
    assert!(task.tainted_rebuild_active(), "rebuild gate must remain active on resume");
    assert!(
        matches!(task.current_state, Some(super::TaskState::NotExecuting { latest_block_n: Some(0) })),
        "setup should restore the task cursor at the confirmed anchor"
    );
    assert_eq!(
        task.executionbox_enable_from_control_plane(),
        Err(EnableError::ReplayInProgress),
        "manual Mixed enable must remain blocked while rebuild session is active"
    );
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn all_rust_traffic_respects_rust_batch_size(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::types::ExecutionMode;

    let devnet_setup = devnet_setup.await;
    let routing_cfg = rust_transfer_routing_cfg(&devnet_setup.backend, &devnet_setup.contracts.0[0], 2, 10);
    let tx0 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    let tx1 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        Felt::ONE,
    );
    let overflow_tx = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[3],
        &devnet_setup.backend,
        Felt::TWO,
    );
    let (ctx, batcher_task, mut out_rx) = spawn_batcher_with_bypass_txs(
        &devnet_setup,
        ExecutionMode::Mixed,
        routing_cfg,
        vec![tx0, tx1, overflow_tx.clone()],
        crate::MempoolIntakeMode::Paused,
    );

    let routed = recv_routed_batch(&mut out_rx).await;
    assert_eq!(routed.rust_batch.len(), 2, "rust branch cap must stop all-Rust traffic");
    assert_eq!(routed.blockifier_batch.len(), 0);
    assert_tx_not_in_mempool(&devnet_setup.mempool, &overflow_tx).await;
    let replayed = recv_routed_batch(&mut out_rx).await;
    let (blockifier_hashes, rust_hashes) = routed_batch_hashes(&replayed);
    assert!(blockifier_hashes.is_empty(), "all-Rust overflow must stay on the rust branch");
    assert_eq!(rust_hashes, vec![overflow_tx.hash], "local carry must replay the overflow tx next cycle");

    ctx.cancel_global();
    batcher_task.await.expect("batcher join");
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn all_blockifier_traffic_respects_blockifier_batch_size(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::types::ExecutionMode;

    let devnet_setup = devnet_setup.await;
    let routing_cfg = crate::util::RustExecRoutingConfig {
        rust_batch_size: 10,
        blockifier_batch_size: 2,
        ..crate::util::RustExecRoutingConfig::default()
    };
    let tx0 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    let tx1 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[3],
        &devnet_setup.backend,
        Felt::ONE,
    );
    let overflow_tx = make_validated_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[4],
        &devnet_setup.backend,
        Felt::TWO,
    );
    let (ctx, batcher_task, mut out_rx) = spawn_batcher_with_bypass_txs(
        &devnet_setup,
        ExecutionMode::Mixed,
        routing_cfg,
        vec![tx0, tx1, overflow_tx.clone()],
        crate::MempoolIntakeMode::Paused,
    );

    let routed = recv_routed_batch(&mut out_rx).await;
    assert_eq!(routed.rust_batch.len(), 0);
    assert_eq!(routed.blockifier_batch.len(), 2, "blockifier branch cap must stop all-Blockifier traffic");
    assert_tx_not_in_mempool(&devnet_setup.mempool, &overflow_tx).await;
    let replayed = recv_routed_batch(&mut out_rx).await;
    let (blockifier_hashes, rust_hashes) = routed_batch_hashes(&replayed);
    assert_eq!(
        blockifier_hashes,
        vec![overflow_tx.hash],
        "local carry must replay the overflow tx on the next batcher cycle"
    );
    assert!(rust_hashes.is_empty(), "all-Blockifier overflow must stay on the Blockifier branch");

    ctx.cancel_global();
    batcher_task.await.expect("batcher join");
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn mixed_traffic_stops_at_first_tx_whose_branch_is_full(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::types::ExecutionMode;

    let devnet_setup = devnet_setup.await;
    let routing_cfg = rust_transfer_routing_cfg(&devnet_setup.backend, &devnet_setup.contracts.0[0], 2, 2);
    let rust_0 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    let blockifier_0 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    let rust_1 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[3],
        &devnet_setup.backend,
        Felt::ONE,
    );
    let overflow_rust = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[4],
        &devnet_setup.backend,
        Felt::TWO,
    );
    let tail_blockifier = make_validated_invoke_tx(
        &devnet_setup.contracts.0[1],
        &devnet_setup.contracts.0[5],
        &devnet_setup.backend,
        Felt::ONE,
    );
    let (ctx, batcher_task, mut out_rx) = spawn_batcher_with_bypass_txs(
        &devnet_setup,
        ExecutionMode::Mixed,
        routing_cfg,
        vec![rust_0, blockifier_0, rust_1, overflow_rust.clone(), tail_blockifier.clone()],
        crate::MempoolIntakeMode::Paused,
    );

    let routed = recv_routed_batch(&mut out_rx).await;
    assert_eq!(routed.rust_batch.len(), 2, "the first full branch must stop the cycle");
    assert_eq!(routed.blockifier_batch.len(), 1, "later txs after the cap hit must stay deferred");
    assert_eq!(routed.total_len(), 3);
    assert_tx_not_in_mempool(&devnet_setup.mempool, &overflow_rust).await;
    assert_tx_not_in_mempool(&devnet_setup.mempool, &tail_blockifier).await;
    let replayed = recv_routed_batch(&mut out_rx).await;
    let (blockifier_hashes, rust_hashes) = routed_batch_hashes(&replayed);
    assert_eq!(
        rust_hashes,
        vec![overflow_rust.hash],
        "the first picked overflow tx should stay on the rust branch next cycle"
    );
    assert_eq!(
        blockifier_hashes,
        Vec::<Felt>::new(),
        "unpicked suffix must remain in the ingress queue rather than being folded into local carry"
    );
    let source_tail = recv_routed_batch(&mut out_rx).await;
    let (blockifier_hashes, rust_hashes) = routed_batch_hashes(&source_tail);
    assert_eq!(
        blockifier_hashes,
        vec![tail_blockifier.hash],
        "txs beyond the one-cycle pick limit should remain in the source queue and arrive after carry drains"
    );
    assert_eq!(rust_hashes, Vec::<Felt>::new());

    ctx.cancel_global();
    batcher_task.await.expect("batcher join");
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn blockifier_only_mode_respects_blockifier_batch_size(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::fallback::types::ExecutionMode;

    let devnet_setup = devnet_setup.await;
    let routing_cfg = rust_transfer_routing_cfg(&devnet_setup.backend, &devnet_setup.contracts.0[0], 10, 2);
    let tx0 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        Felt::ZERO,
    );
    let tx1 = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[2],
        &devnet_setup.backend,
        Felt::ONE,
    );
    let overflow_tx = make_validated_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[3],
        &devnet_setup.backend,
        Felt::TWO,
    );
    let (ctx, batcher_task, mut out_rx) = spawn_batcher_with_bypass_txs(
        &devnet_setup,
        ExecutionMode::BlockifierOnly,
        routing_cfg,
        vec![tx0, tx1, overflow_tx.clone()],
        crate::MempoolIntakeMode::Paused,
    );

    let routed = recv_routed_batch(&mut out_rx).await;
    assert_eq!(routed.rust_batch.len(), 0, "BlockifierOnly mode must not emit a rust branch");
    assert_eq!(routed.blockifier_batch.len(), 2, "BlockifierOnly mode must honor the Blockifier cap");
    assert_tx_not_in_mempool(&devnet_setup.mempool, &overflow_tx).await;
    let replayed = recv_routed_batch(&mut out_rx).await;
    let (blockifier_hashes, rust_hashes) = routed_batch_hashes(&replayed);
    assert_eq!(
        blockifier_hashes,
        vec![overflow_tx.hash],
        "BlockifierOnly overflow must stay local and arrive on the next cycle"
    );
    assert!(rust_hashes.is_empty(), "BlockifierOnly replay must not emit a rust branch");

    ctx.cancel_global();
    batcher_task.await.expect("batcher join");
}

#[rstest::rstest]
#[timeout(Duration::from_secs(30))]
#[tokio::test]
async fn fresh_mempool_txs_are_not_consumed_while_tainted_rebuild_is_active(
    #[future]
    #[from(devnet_setup)]
    devnet_setup: DevnetSetup,
) {
    use crate::batcher::Batcher;
    use crate::fallback::types::ExecutionMode;

    let devnet_setup = devnet_setup.await;
    sign_and_add_invoke_tx(
        &devnet_setup.contracts.0[0],
        &devnet_setup.contracts.0[1],
        &devnet_setup.backend,
        &devnet_setup.tx_validator,
        Felt::ZERO,
    )
    .await;

    let (out_tx, mut out_rx) = mpsc::channel(1);
    let (_bypass_tx, bypass_rx) = mpsc::channel(1);
    let (mempool_intake_tx, mempool_intake_rx) = watch::channel(crate::MempoolIntakeMode::Running);
    let (tainted_rebuild_active_tx, tainted_rebuild_active_rx) = watch::channel(true);
    let (_execution_mode_tx, execution_mode_rx) = watch::channel(ExecutionMode::BlockifierOnly);
    let (_execution_epoch_tx, execution_epoch_rx) = watch::channel(0u64);
    let ctx = ServiceContext::new_for_testing();
    let ctx_clone = ctx.clone();

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
        crate::util::RustExecRoutingConfig::default(),
        devnet_setup.metrics.clone(),
        false,
    );
    let batcher_task = tokio::spawn(async move { batcher.run().await.unwrap() });

    assert!(
        tokio::time::timeout(Duration::from_millis(250), out_rx.recv()).await.is_err(),
        "batcher must not consume fresh txs while rebuild gate is active"
    );
    let _ = tainted_rebuild_active_tx.send(false);

    let routed = tokio::time::timeout(Duration::from_secs(5), out_rx.recv())
        .await
        .expect("batcher should emit once rebuild gate opens")
        .expect("routed batch should exist");
    assert_eq!(routed.total_len(), 1, "fresh tx should be routed after rebuild gate opens");

    ctx_clone.cancel_global();
    batcher_task.await.expect("batcher join");
}

#[tokio::test]
async fn carry_spills_into_next_persisted_block() {
    let mut source_devnet_setup = tainted_rebuild_spill_devnet_setup().await;
    let mut target_devnet_setup = tainted_rebuild_spill_devnet_setup().await;

    let (header_1, rows_1) = seed_real_preconfirmed_block(&mut source_devnet_setup, 0, 1, Felt::ZERO).await;
    persist_preconfirmed_bucket(&target_devnet_setup.backend, header_1.clone(), rows_1);

    let mut header_2 = header_1.clone();
    header_2.block_number = 2;
    persist_preconfirmed_bucket(&target_devnet_setup.backend, header_2, vec![]);

    let carry_rows = vec![
        carry_row_from_validated(
            0,
            make_validated_invoke_tx(
                &target_devnet_setup.contracts.0[2],
                &target_devnet_setup.contracts.0[3],
                &target_devnet_setup.backend,
                Felt::ZERO,
            ),
            None,
        ),
        carry_row_from_validated(
            1,
            make_validated_invoke_tx(
                &target_devnet_setup.contracts.0[4],
                &target_devnet_setup.contracts.0[5],
                &target_devnet_setup.backend,
                Felt::ZERO,
            ),
            None,
        ),
        carry_row_from_validated(
            2,
            make_validated_invoke_tx(
                &target_devnet_setup.contracts.0[6],
                &target_devnet_setup.contracts.0[7],
                &target_devnet_setup.backend,
                Felt::ZERO,
            ),
            None,
        ),
    ];
    target_devnet_setup.backend.replace_tainted_rebuild_carry_rows(&carry_rows).expect("persist carry");

    let session = mc_db::StoredTaintedRebuildSession {
        execution_epoch: 3,
        anchor_block_n: 0,
        next_block_n: 1,
        tail_block_n: 2,
        active: true,
    };
    target_devnet_setup.backend.write_tainted_rebuild_session(&session).expect("persist session");

    let result =
        BlockProductionTask::run_tainted_rebuild_step_task(target_devnet_setup.backend.clone(), session.clone(), false)
            .await
            .expect("run tainted rebuild step");
    let live_session_after_step = result.live_session_after_step.clone();

    assert_eq!(
        live_session_after_step,
        Some(mc_db::StoredTaintedRebuildSession { next_block_n: 2, ..session.clone() }),
        "leftover carry must advance into the next saved persisted descendant block"
    );
    assert!(
        !target_devnet_setup.backend.get_tainted_rebuild_carry_rows().expect("read carry rows after step").is_empty(),
        "spill into the next persisted block must leave carry rows for the next step"
    );
    let mut task = target_devnet_setup.block_prod_task();
    apply_tainted_rebuild_step_result(&mut task, result).await;
    assert_eq!(task.tainted_rebuild_session, live_session_after_step);
    assert_eq!(target_devnet_setup.backend.latest_confirmed_block_n(), Some(1));
}

#[tokio::test]
async fn carry_overflow_creates_session_beyond_original_tail_and_resumes_after_restart() {
    use crate::fallback::manager::EnableError;
    use crate::fallback::types::{ExecutionMode, StartupExecutionMode};

    let mut source_devnet_setup = tainted_rebuild_spill_devnet_setup().await;
    let mut target_devnet_setup = tainted_rebuild_spill_devnet_setup().await;

    let (header_1, rows_1) = seed_real_preconfirmed_block(&mut source_devnet_setup, 0, 1, Felt::ZERO).await;
    persist_preconfirmed_bucket(&target_devnet_setup.backend, header_1.clone(), rows_1);

    let carry_rows = vec![
        carry_row_from_validated(
            0,
            make_validated_invoke_tx(
                &target_devnet_setup.contracts.0[2],
                &target_devnet_setup.contracts.0[3],
                &target_devnet_setup.backend,
                Felt::ZERO,
            ),
            None,
        ),
        carry_row_from_validated(
            1,
            make_validated_invoke_tx(
                &target_devnet_setup.contracts.0[4],
                &target_devnet_setup.contracts.0[5],
                &target_devnet_setup.backend,
                Felt::ZERO,
            ),
            None,
        ),
        carry_row_from_validated(
            2,
            make_validated_invoke_tx(
                &target_devnet_setup.contracts.0[6],
                &target_devnet_setup.contracts.0[7],
                &target_devnet_setup.backend,
                Felt::ZERO,
            ),
            None,
        ),
    ];
    target_devnet_setup.backend.replace_tainted_rebuild_carry_rows(&carry_rows).expect("persist carry");

    let session = mc_db::StoredTaintedRebuildSession {
        execution_epoch: 5,
        anchor_block_n: 0,
        next_block_n: 1,
        tail_block_n: 1,
        active: true,
    };
    target_devnet_setup.backend.write_tainted_rebuild_session(&session).expect("persist session");

    let result =
        BlockProductionTask::run_tainted_rebuild_step_task(target_devnet_setup.backend.clone(), session.clone(), false)
            .await
            .expect("run tainted rebuild step");
    let overflow_session = result.live_session_after_step.clone().expect("overflow carry should require another block");

    assert_eq!(overflow_session.next_block_n, 2, "overflow step must advance beyond the original tainted tail");
    assert_eq!(overflow_session.tail_block_n, 1, "original tail remains recorded while overflow is pending");
    let mut task = target_devnet_setup.block_prod_task();
    apply_tainted_rebuild_step_result(&mut task, result).await;
    assert_eq!(target_devnet_setup.backend.latest_confirmed_block_n(), Some(1));

    target_devnet_setup.backend.refresh_head_projection_from_db().expect("refresh head projection");
    let mempool = Arc::new(Mempool::new(target_devnet_setup.backend.clone(), MempoolConfig::default()));
    let mut restart_task = BlockProductionTask::new(
        target_devnet_setup.backend.clone(),
        mempool,
        target_devnet_setup.metrics.clone(),
        Arc::new(target_devnet_setup.l1_client.clone()),
        false,
        false,
    )
    .with_startup_execution_mode(StartupExecutionMode::Mixed);

    restart_task.setup_initial_state().await.expect("resume task setup");

    assert_eq!(restart_task.tainted_rebuild_session, Some(overflow_session));
    assert_eq!(restart_task.fallback.mode, ExecutionMode::BlockifierOnly);
    assert!(
        !restart_task.executionbox_status_snapshot().replay_backlog_empty,
        "restarted task must continue to report rebuild in progress"
    );
    assert_eq!(
        restart_task.executionbox_enable_from_control_plane(),
        Err(EnableError::ReplayInProgress),
        "manual Mixed enable must remain blocked after restart while overflow rebuild is pending"
    );
}

#[test]
fn spill_carry_preserves_saved_charge_fee_semantics_across_the_next_rebuild_step() {
    let saved_rows = vec![
        make_preconfirmed_tx_with_hash(Felt::from(0x100u64)),
        make_preconfirmed_tx_with_hash(Felt::from(0x101u64)),
        make_preconfirmed_tx_with_hash(Felt::from(0x102u64)),
    ];

    let mut source_txs = BlockProductionTask::tainted_rebuild_sources_from_preconfirmed_rows(9, saved_rows, true);
    let spilled_to_carry = source_txs.split_off(1);
    let carry_rows = BlockProductionTask::tainted_rebuild_carry_rows_from_sources_for_next_block(10, spilled_to_carry);

    assert!(
        carry_rows.iter().all(|row| !row.effective_charge_fee),
        "spilled carry rows must durably preserve the saved no-charge-fee execution mode"
    );
    assert!(
        carry_rows.iter().all(|row| row.source_block_n == Some(10)),
        "spilled carry rows must restart from the next rebuild block, not keep the stale source block"
    );

    let next_step_sources = BlockProductionTask::tainted_rebuild_sources_from_carry_rows(carry_rows);
    assert!(
        next_step_sources.iter().all(|source| source.force_charge_fee == Some(false)),
        "the next rebuild step must replay spilled rows with the same effective charge_fee=false semantics"
    );
}

#[test]
fn spill_carry_advances_leftover_sources_to_the_next_rebuild_step_in_order() {
    let stale_block_n = 8;
    let next_block_n = 9;
    let future_hashes = vec![Felt::from(0x110u64), Felt::from(0x111u64), Felt::from(0x112u64)];
    let stale_sources = future_hashes
        .iter()
        .copied()
        .map(|hash| super::TaintedRebuildSourceTx {
            validated: make_preconfirmed_tx_with_hash(hash).to_validated(),
            source_block_n: Some(stale_block_n),
            force_charge_fee: None,
        })
        .collect();

    let carry_rows =
        BlockProductionTask::tainted_rebuild_carry_rows_from_sources_for_next_block(next_block_n, stale_sources);
    assert!(
        carry_rows.iter().all(|row| row.source_block_n == Some(next_block_n)),
        "overflow that survives block K must resume at block K+1"
    );

    let (source_txs, future_carry) =
        BlockProductionTask::tainted_rebuild_sources_for_saved_block(next_block_n, carry_rows, vec![], false);
    let source_hashes: Vec<_> = source_txs.iter().map(|source| source.validated.hash).collect();

    assert_eq!(
        source_hashes, future_hashes,
        "the next rebuild step must replay the leftover suffix in the same order after the spill is rebound"
    );
    assert!(future_carry.is_empty(), "rebound spill should be ready for the immediate next rebuild step");
}

#[test]
fn same_block_carry_overlap_is_deduped_with_carry_precedence() {
    let overlap_hash = Felt::from(0x110u64);
    let carry_only_hash = Felt::from(0x111u64);
    let saved_only_hash = Felt::from(0x112u64);

    let carry_rows = vec![
        carry_row_from_validated(0, make_preconfirmed_tx_with_hash(overlap_hash).to_validated(), Some(9)),
        carry_row_from_validated(1, make_preconfirmed_tx_with_hash(carry_only_hash).to_validated(), Some(9)),
    ];
    let saved_rows =
        vec![make_preconfirmed_tx_with_hash(overlap_hash), make_preconfirmed_tx_with_hash(saved_only_hash)];

    let (source_txs, future_carry) =
        BlockProductionTask::tainted_rebuild_sources_for_saved_block(9, carry_rows, saved_rows, false);
    let source_hashes: Vec<_> = source_txs.iter().map(|source| source.validated.hash).collect();

    assert_eq!(
        source_hashes,
        vec![overlap_hash, carry_only_hash, saved_only_hash],
        "same-block carry must win overlap dedupe without replaying the saved row twice"
    );
    assert!(future_carry.is_empty(), "same-block overlap should not defer any rows to future steps");
}

#[test]
fn future_block_carry_is_deferred_until_its_saved_block() {
    let saved_hash = Felt::from(0x120u64);
    let future_hash = Felt::from(0x121u64);

    let carry_rows =
        vec![carry_row_from_validated(0, make_preconfirmed_tx_with_hash(future_hash).to_validated(), Some(7))];
    let saved_rows = vec![make_preconfirmed_tx_with_hash(saved_hash)];

    let (source_txs, future_carry) =
        BlockProductionTask::tainted_rebuild_sources_for_saved_block(5, carry_rows, saved_rows, false);
    let source_hashes: Vec<_> = source_txs.iter().map(|source| source.validated.hash).collect();

    assert_eq!(
        source_hashes,
        vec![saved_hash],
        "rebuild step K must not consume carry that belongs to a later persisted block"
    );
    assert_eq!(future_carry.len(), 1, "future carry must remain persisted for later steps");
    assert_eq!(future_carry[0].tx.hash, future_hash);
    assert_eq!(future_carry[0].source_block_n, Some(7));
}

#[test]
fn anchor_suffix_carry_executes_before_next_saved_descendant_rows() {
    let carry_hash = Felt::from(0x130u64);
    let saved_hash = Felt::from(0x131u64);

    let carry_rows =
        vec![carry_row_from_validated(0, make_preconfirmed_tx_with_hash(carry_hash).to_validated(), Some(4))];
    let saved_rows = vec![make_preconfirmed_tx_with_hash(saved_hash)];

    let (source_txs, future_carry) =
        BlockProductionTask::tainted_rebuild_sources_for_saved_block(5, carry_rows, saved_rows, false);
    let source_hashes: Vec<_> = source_txs.iter().map(|source| source.validated.hash).collect();

    assert_eq!(
        source_hashes,
        vec![carry_hash, saved_hash],
        "anchor suffix carry must still run before the next descendant bucket"
    );
    assert!(future_carry.is_empty());
}
