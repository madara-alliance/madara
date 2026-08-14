use super::*;
use crate::executor::ExecutorCommand;
use crate::executor::{BatchExecutionResult, ExecutorMessage};
use crate::metrics::BlockProductionMetrics;
use blockifier::state::cached_state::StateMaps;
use blockifier::transaction::transaction_execution::Transaction;
use mp_block::{FullBlockWithoutCommitments, PreconfirmedHeader};
use mp_chain_config::{ChainConfig, StarknetVersion};
use mp_state_update::StateDiff;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use starknet_core::utils::get_selector_from_name;
use tokio::sync::{mpsc, watch};

#[allow(clippy::type_complexity)]
fn make_executor_thread() -> (
    ExecutorThread,
    mpsc::Sender<RoutedBatchToExecute>,
    mpsc::UnboundedSender<ExecutorCommand>,
    watch::Receiver<RuntimeReplayStatus>,
    mpsc::Receiver<ExecutorMessage>,
    watch::Sender<u64>,
) {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    backend.set_l1_gas_quote_for_testing();
    let (incoming_tx, incoming_rx) = mpsc::channel(1);
    let (replies_tx, replies_rx) = mpsc::channel(1);
    let (commands_tx, commands_rx) = mpsc::unbounded_channel();
    let (replay_status_tx, replay_status_rx) = watch::channel(RuntimeReplayStatus::idle());
    let (mode_tx, mode_rx) = watch::channel(ExecutionMode::Mixed);
    let (execution_epoch_tx, execution_epoch_rx) = watch::channel(0u64);
    let thread = ExecutorThread::new(
        backend,
        incoming_rx,
        replies_tx,
        commands_rx,
        Arc::new(BlockProductionMetrics::register()),
        false,
        replay_status_tx,
        mode_tx,
        mode_rx,
        execution_epoch_rx,
        false,
        crate::BlockPipelineMode::Optimistic,
        16,
        RustExecRuntimeConfig::default(),
    )
    .expect("executor thread");
    (thread, incoming_tx, commands_tx, replay_status_rx, replies_rx, execution_epoch_tx)
}

fn seed_confirmed_blocks(backend: &Arc<MadaraBackend>, confirmed_tip: u64) {
    for block_n in 0..=confirmed_tip {
        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: block_n, ..Default::default() },
                    state_diff: StateDiff::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .expect("seed confirmed block");
    }
}

fn make_fallback_carry_tx(backend: &MadaraBackend, nonce: u64) -> (Transaction, crate::util::AdditionalTxInfo) {
    let (tx, declared_class) = L1HandlerTransactionWithFee::new(
        L1HandlerTransaction {
            version: Felt::ZERO,
            nonce,
            contract_address: Felt::from(0x1234u64),
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").expect("selector"),
            calldata: vec![Felt::from(0x55u64)].into(),
        },
        1,
    )
    .into_blockifier(backend.chain_config().chain_id.to_felt(), StarknetVersion::LATEST)
    .expect("l1 handler tx");
    (tx, crate::util::AdditionalTxInfo { declared_class, arrived_at: Default::default() })
}

fn blockifier_only_batch(
    txs: impl IntoIterator<Item = (Transaction, crate::util::AdditionalTxInfo)>,
    block_n: u64,
    execution_mode: ExecutionMode,
    execution_epoch: u64,
) -> RoutedBatchToExecute {
    RoutedBatchToExecute {
        transactions: txs.into_iter().collect(),
        route: BatchRoute::BlockifierOnly { cause: BlockifierRouteCause::FrozenBlockMode },
        original_tx_hashes: Vec::new(),
        block_n,
        execution_mode,
        execution_epoch,
    }
}

#[test]
fn block_local_batch_number_resets_for_each_rebound_block() {
    let mut sequence = None;

    assert_eq!(next_block_local_batch_number(&mut sequence, 41), 1);
    assert_eq!(next_block_local_batch_number(&mut sequence, 41), 2);
    assert_eq!(next_block_local_batch_number(&mut sequence, 42), 1);
    assert_eq!(next_block_local_batch_number(&mut sequence, 42), 2);
    assert_eq!(next_block_local_batch_number(&mut sequence, 41), 1);
}

#[test]
fn wait_take_tx_batch_prefers_commands_for_new_block_boundary() {
    let (mut thread, incoming_tx, commands_tx, _replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    incoming_tx
        .try_send(RoutedBatchToExecute {
            execution_mode: ExecutionMode::Mixed,
            execution_epoch: 7,
            ..Default::default()
        })
        .expect("enqueue routed batch");
    commands_tx
        .send(ExecutorCommand::SetDesiredExecutionMode { mode: ExecutionMode::BlockifierOnly })
        .expect("enqueue mode command");

    let outcome = thread.wait_take_tx_batch(Some(0), None, true, true);
    match outcome {
        WaitTxBatchOutcome::Command(ExecutorCommand::SetDesiredExecutionMode { mode }) => {
            assert_eq!(mode, ExecutionMode::BlockifierOnly);
        }
        _ => panic!("expected queued mode command to win at idle boundary"),
    }
}

#[test]
fn parked_executor_still_shuts_down_when_batch_channel_closes() {
    let (mut thread, incoming_tx, _commands_tx, _replay_status_rx, mut replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    thread.start_tainted_rebuild_parked = true;
    drop(incoming_tx);

    let executor = std::thread::spawn(move || thread.run());
    assert!(matches!(
        replies_rx.blocking_recv(),
        Some(ExecutorMessage::EndFinalBlock { block_exec_summary: None, block_number: None, execution_epoch: 0 })
    ));
    executor.join().expect("executor thread should join").expect("parked executor should shut down cleanly");
}

#[test]
fn sync_new_block_boundary_commands_rechecks_mode_after_batch_dequeue() {
    let (mut thread, _incoming_tx, commands_tx, _replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    let mut state = thread.initial_state().expect("initial executor state");
    let mut pending_routed =
        RoutedBatchToExecute { execution_mode: ExecutionMode::Mixed, execution_epoch: 7, ..Default::default() };
    let mut desired_execution_mode = ExecutionMode::Mixed;
    let mut execution_epoch = 7;
    let mut tainted_rebuild_parked = false;
    let mut runtime_replay_active = false;
    let mut replay_current_block_active = false;
    let mut next_block_deadline = Instant::now();
    let mut force_close = false;
    let mut block_empty = true;
    let mut l2_gas_consumed_block = 0;

    // Simulate the post-dequeue boundary: the first routed batch is already pending,
    // and the desired-mode command arrives just before NewBlock -> Executing.
    commands_tx
        .send(ExecutorCommand::SetDesiredExecutionMode { mode: ExecutionMode::BlockifierOnly })
        .expect("enqueue mode command");

    let outcome = thread
        .sync_new_block_boundary_commands(
            &mut state,
            &mut pending_routed,
            &mut desired_execution_mode,
            &mut execution_epoch,
            &mut tainted_rebuild_parked,
            &mut runtime_replay_active,
            &mut replay_current_block_active,
            &mut next_block_deadline,
            &mut force_close,
            &mut block_empty,
            &mut l2_gas_consumed_block,
            std::time::Duration::from_secs(30_000),
        )
        .expect("sync new-block boundary commands");

    assert!(matches!(outcome, NewBlockBoundarySyncOutcome::Proceed));
    assert!(matches!(state, ExecutorThreadState::NewBlock(_)));
    assert_eq!(desired_execution_mode, ExecutionMode::BlockifierOnly);
    assert_eq!(execution_epoch, 7);
    assert!(!tainted_rebuild_parked);
    assert!(!runtime_replay_active);
    assert!(!replay_current_block_active);
    assert!(!force_close);
    assert!(block_empty);
    assert_eq!(l2_gas_consumed_block, 0);
    assert_eq!(*thread.execution_mode_rx.borrow(), ExecutionMode::BlockifierOnly);

    ExecutorThread::normalize_routed_batch_for_execution_mode(&mut pending_routed, desired_execution_mode);
    assert_eq!(
        pending_routed.execution_mode,
        ExecutionMode::BlockifierOnly,
        "final NewBlock sync must retarget the pending first batch before the block mode is frozen"
    );
}

#[test]
fn deferred_suffix_rebinds_to_next_block_after_rollover() {
    let (mut thread, _incoming_tx, _commands_tx, _replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    let initial_state = thread.initial_state().expect("initial executor state");
    let ExecutorThreadState::NewBlock(state_new_block) = initial_state else {
        panic!("initial executor state must start as NewBlock");
    };
    let mut execution_state =
        thread.create_execution_state(state_new_block, 0, ExecutionMode::Mixed, None).expect("create execution state");
    let current_block_n = execution_state.exec_ctx.block_number;
    assert_eq!(current_block_n, 0, "executor test thread starts from block 0");

    let backend = thread.backend.clone();
    let (tx, info) = make_fallback_carry_tx(&backend, 1);
    let mut pending_routed = blockifier_only_batch([(tx, info)], 999, ExecutionMode::Mixed, 0);

    ExecutorThread::rebind_routed_batch_to_block(&mut pending_routed, current_block_n);
    assert_eq!(pending_routed.block_n, 0, "accepted routed work must bind to the current executing block");

    let next_state = thread.end_block(&mut execution_state).expect("end block");
    ExecutorThread::rebind_routed_batch_to_block(
        &mut pending_routed,
        ExecutorThread::current_executor_block_n(&next_state),
    );
    assert_eq!(pending_routed.block_n, 1, "deferred suffix must rebind to the next block after rollover");
}

#[test]
fn prepare_tainted_rebuild_fallback_rebinds_unaccepted_queued_batches_to_the_future_frontier() {
    let (mut thread, incoming_tx, _commands_tx, _replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    let initial_state = thread.initial_state().expect("initial executor state");
    let ExecutorThreadState::NewBlock(state_new_block) = initial_state else {
        panic!("initial executor state must start as NewBlock");
    };
    let execution_state =
        thread.create_execution_state(state_new_block, 0, ExecutionMode::Mixed, None).expect("create execution state");
    let current_block_n = execution_state.exec_ctx.block_number;
    let mut state = ExecutorThreadState::Executing(execution_state);
    let backend = thread.backend.clone();
    let (pending_tx, pending_info) = make_fallback_carry_tx(&backend, 1);
    let (queued_tx, queued_info) = make_fallback_carry_tx(&backend, 2);
    let mut pending_routed =
        blockifier_only_batch([(pending_tx, pending_info)], current_block_n, ExecutionMode::Mixed, 7);
    incoming_tx
        .try_send(blockifier_only_batch([(queued_tx, queued_info)], 0, ExecutionMode::Mixed, 7))
        .expect("enqueue stale routed batch");

    let mut next_block_deadline = Instant::now();
    let mut force_close = false;
    let mut block_empty = true;
    let mut l2_gas_consumed_block = 0;
    let carry = thread
        .prepare_tainted_rebuild_fallback(
            &mut state,
            &mut pending_routed,
            40,
            7,
            &mut next_block_deadline,
            &mut force_close,
            &mut block_empty,
            &mut l2_gas_consumed_block,
            std::time::Duration::from_secs(30_000),
        )
        .expect("prepare fallback carry");

    let carry_provenance: Vec<_> =
        carry.into_iter().map(|carry_tx| (carry_tx.tx.tx_hash().to_felt(), carry_tx.source_block_n)).collect();
    assert_eq!(carry_provenance.len(), 2, "both pending and queued routed txs should become rebuild carry");
    assert_eq!(
        carry_provenance[0].1,
        Some(current_block_n),
        "pending routed work should stay bound to the current executor block"
    );
    assert_eq!(
        carry_provenance[1].1,
        Some(current_block_n.saturating_add(1)),
        "queued routed work that never reached the executor frontier must restart from the earliest future block"
    );
}

#[test]
fn publish_replay_status_updates_executor_snapshot() {
    let (thread, _incoming_tx, _commands_tx, mut replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    assert_eq!(*replay_status_rx.borrow(), RuntimeReplayStatus::idle());

    thread.publish_replay_status(true, 7);
    assert_eq!(*replay_status_rx.borrow_and_update(), RuntimeReplayStatus::in_progress_for_epoch(7));

    thread.publish_replay_status(false, 7);
    assert_eq!(*replay_status_rx.borrow_and_update(), RuntimeReplayStatus::idle_for_epoch(7));
}

#[test]
fn record_replay_executed_hashes_marks_boundary_met() {
    let (mut thread, _incoming_tx, _commands_tx, _replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    thread.replay_mode_enabled = true;

    let tx_hash = Felt::from_hex_unchecked("0x1234");
    let status = thread.backend.set_replay_boundary(mp_rpc::admin::ReplayBlockBoundary {
        block_n: 0,
        expected_tx_count: 1,
        last_tx_hash: tx_hash,
    });
    assert_eq!(status.executed_tx_count, 0);
    assert!(!status.boundary_met);

    thread.record_replay_executed_hashes(0, &[tx_hash]);

    let status = thread.backend.get_replay_boundary_status(0).expect("replay boundary status");
    assert_eq!(status.executed_tx_count, 1, "recorded tx hash should advance executed count");
    assert_eq!(status.dispatched_tx_count, 1, "executed count should backfill dispatched count when needed");
    assert!(status.reached_last_tx_hash, "recorded hash should satisfy the replay boundary");
    assert!(status.boundary_met, "replay boundary should become met once the last hash executes");
}

#[test]
fn resync_to_backend_head_reanchors_next_block_to_backend_tip() {
    let (mut thread, _incoming_tx, _commands_tx, mut replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    let backend = thread.backend.clone();
    let initial_state = thread.initial_state().expect("initial executor state");
    let ExecutorThreadState::NewBlock(state_new_block) = initial_state else {
        panic!("initial executor state must start as NewBlock");
    };
    let executing_state = thread
        .create_execution_state(state_new_block, 77, ExecutionMode::BlockifierOnly, None)
        .expect("create stale execution state");
    let mut state = ExecutorThreadState::Executing(executing_state);
    let (pending_tx, pending_info) = make_fallback_carry_tx(&backend, 9);
    let mut pending_routed = blockifier_only_batch([(pending_tx, pending_info)], 0, ExecutionMode::BlockifierOnly, 9);

    seed_confirmed_blocks(&backend, 3);

    let mut desired_execution_mode = ExecutionMode::Mixed;
    let mut runtime_replay_active = true;
    let mut replay_current_block_active = true;
    let mut next_block_deadline = Instant::now();
    let mut force_close = true;
    let mut block_empty = false;
    let mut l2_gas_consumed_block = 123;

    thread
        .resync_to_backend_head(
            &mut state,
            &mut pending_routed,
            &mut desired_execution_mode,
            9,
            &mut runtime_replay_active,
            &mut replay_current_block_active,
            &mut next_block_deadline,
            &mut force_close,
            &mut block_empty,
            &mut l2_gas_consumed_block,
            std::time::Duration::from_secs(30_000),
        )
        .expect("resync executor state");

    assert!(matches!(state, ExecutorThreadState::NewBlock(_)));
    assert!(pending_routed.is_empty(), "stale routed payload must be discarded on resync");
    assert_eq!(desired_execution_mode, ExecutionMode::Mixed, "resync must preserve desired execution mode");
    assert!(!runtime_replay_active);
    assert!(!replay_current_block_active);
    assert!(!force_close);
    assert!(block_empty);
    assert_eq!(l2_gas_consumed_block, 0);
    assert_eq!(
        *replay_status_rx.borrow_and_update(),
        RuntimeReplayStatus::idle_for_epoch(9),
        "resync must publish idle replay status for the current epoch"
    );

    let ExecutorThreadState::NewBlock(state_new_block) = state else {
        panic!("resync must leave the executor at a NewBlock boundary");
    };
    assert_eq!(
        state_new_block.state_adaptor.block_n(),
        4,
        "resync must rebuild from the backend-confirmed head so the next block is confirmed_tip + 1"
    );
    let execution_state = thread
        .create_execution_state(state_new_block, 0, desired_execution_mode, None)
        .expect("create execution state after resync");
    assert_eq!(
        execution_state.exec_ctx.block_number, 4,
        "the next StartNewBlock after resync must use the backend-authoritative head"
    );
}

#[test]
fn stale_forward_reply_does_not_block_fallback_handoff() {
    let (mut thread, _incoming_tx, _commands_tx, _replay_status_rx, mut replies_rx, execution_epoch_tx) =
        make_executor_thread();
    let backend = thread.backend.clone();
    seed_confirmed_blocks(&backend, 0);
    let initial_state = thread.initial_state().expect("initial executor state");
    let ExecutorThreadState::NewBlock(state_new_block) = initial_state else {
        panic!("initial executor state must start as NewBlock");
    };
    let mut execution_state =
        thread.create_execution_state(state_new_block, 0, ExecutionMode::Mixed, None).expect("create execution state");
    let (carried_tx, carried_info) = make_fallback_carry_tx(&backend, 11);
    execution_state.executed_in_block.push(carried_tx, carried_info);
    let block_n = execution_state.exec_ctx.block_number;
    let mut state = ExecutorThreadState::Executing(execution_state);
    let mut pending_routed = RoutedBatchToExecute::default();
    let stale_exec_ctx = match &state {
        ExecutorThreadState::Executing(s) => s.exec_ctx.clone(),
        ExecutorThreadState::NewBlock(_) => unreachable!("test seeds executing state"),
    };

    thread
        .replies_sender
        .try_send(ExecutorMessage::StartNewBlock {
            exec_ctx: stale_exec_ctx,
            execution_mode: ExecutionMode::Mixed,
            execution_epoch: 0,
        })
        .expect("fill replies channel");

    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        execution_epoch_tx.send(1).expect("advance execution epoch");
    });

    let send_started = std::time::Instant::now();
    let outcome = thread.send_forward_reply(
        0,
        Some(block_n),
        "BatchExecuted",
        ExecutorMessage::BatchExecuted(BatchExecutionResult {
            executed_txs: BatchToExecute::default(),
            original_tx_hashes: vec![],
            execution_results: vec![],
            stats: ExecutionStats::default(),
            execution_mode: ExecutionMode::Mixed,
            execution_epoch: 0,
            emitted_at: StdInstant::now(),
        }),
    );
    assert!(matches!(outcome, ForwardReplySendOutcome::DroppedStale));
    assert!(
        send_started.elapsed() < std::time::Duration::from_secs(1),
        "stale forward replies must stop blocking once the execution epoch advances"
    );

    let mut next_block_deadline = Instant::now();
    let mut force_close = false;
    let mut block_empty = false;
    let mut l2_gas_consumed_block = 0;
    let carry = thread
        .prepare_tainted_rebuild_fallback(
            &mut state,
            &mut pending_routed,
            block_n.saturating_sub(1),
            1,
            &mut next_block_deadline,
            &mut force_close,
            &mut block_empty,
            &mut l2_gas_consumed_block,
            std::time::Duration::from_secs(30_000),
        )
        .expect("prepare fallback carry after stale reply drop");
    assert_eq!(carry.len(), 1, "fallback carry must still be recoverable after the stale reply is dropped");
    assert_eq!(carry[0].source_block_n, Some(block_n));
    assert!(matches!(replies_rx.try_recv(), Ok(ExecutorMessage::StartNewBlock { .. })));
    assert!(matches!(replies_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)));
}

#[test]
fn fallback_command_interrupts_wait_for_confirmed_hash() {
    let (mut thread, _incoming_tx, commands_tx, _replay_status_rx, _replies_rx, _execution_epoch_tx) =
        make_executor_thread();
    let backend = thread.backend.clone();
    seed_confirmed_blocks(&backend, 0);

    let mut state = thread.initial_state().expect("initial executor state");
    let current_block_n = match &mut state {
        ExecutorThreadState::NewBlock(state_new_block) => {
            for _ in 0..12 {
                state_new_block
                    .state_adaptor
                    .finish_block(StateMaps::default(), Default::default(), Default::default())
                    .expect("advance layered state adaptor without new confirmations");
            }
            state_new_block.state_adaptor.block_n()
        }
        ExecutorThreadState::Executing(_) => panic!("initial executor state must start as NewBlock"),
    };
    assert_eq!(current_block_n, 13, "seeded internal preconfirmed tip must advance the next executor block");

    let (carry_tx, carry_info) = make_fallback_carry_tx(&backend, 17);
    let mut pending_routed = blockifier_only_batch([(carry_tx, carry_info)], current_block_n, ExecutionMode::Mixed, 0);
    let mut desired_execution_mode = ExecutionMode::Mixed;
    let mut execution_epoch = 0;
    let mut tainted_rebuild_parked = false;
    let mut runtime_replay_active = false;
    let mut replay_current_block_active = false;
    let mut next_block_deadline = Instant::now();
    let mut force_close = false;
    let mut block_empty = true;
    let mut l2_gas_consumed_block = 0;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();

    commands_tx
        .send(ExecutorCommand::PrepareTaintedRebuildFallback {
            block_n: current_block_n.saturating_sub(1),
            execution_epoch: 1,
            reply: reply_tx,
        })
        .expect("enqueue fallback command");

    let outcome = thread
        .wait_for_hash_of_block_min_10_or_command(
            &mut state,
            &mut pending_routed,
            &mut desired_execution_mode,
            &mut execution_epoch,
            &mut tainted_rebuild_parked,
            &mut runtime_replay_active,
            &mut replay_current_block_active,
            &mut next_block_deadline,
            &mut force_close,
            &mut block_empty,
            &mut l2_gas_consumed_block,
            std::time::Duration::from_secs(30_000),
            current_block_n,
        )
        .expect("wait for hash or command");

    assert!(matches!(outcome, WaitForConfirmedHashOutcome::ContinueOuterLoop));
    assert_eq!(desired_execution_mode, ExecutionMode::BlockifierOnly);
    assert_eq!(execution_epoch, 1);
    assert!(tainted_rebuild_parked, "fallback must park the executor until the durable rebuild is acknowledged");
    assert_eq!(
        ExecutorThread::current_executor_block_n(&state),
        1,
        "fallback must discard a stale NewBlock frontier before replay resumes"
    );
    assert!(!runtime_replay_active);
    assert!(!replay_current_block_active);
    assert!(
        pending_routed.is_empty(),
        "fallback handoff must drain pending routed work even while hash wait is active"
    );

    let carry =
        thread.wait_rt.block_on(async { reply_rx.await.expect("fallback reply open").expect("fallback carry") });
    assert_eq!(carry.len(), 1, "queued fallback carry must still be returned while waiting for block_n-10 hash");
    assert_eq!(carry[0].source_block_n, Some(current_block_n));
}
