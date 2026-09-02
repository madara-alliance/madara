use super::*;
use crate::CurrentBlockState;
use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
use blockifier::bouncer::{BouncerWeights, CasmHashComputationData};
use blockifier::state::cached_state::CommitmentStateDiff;
use mc_db::close_pipeline_contract::CloseJobPayload;
use mc_db::MadaraBackend;
use mp_chain_config::ChainConfig;
use mp_state_update::StateDiff;
use rstest::rstest;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::time::{sleep, Duration};

/// Builds the smallest execution summary accepted by a close-queue payload.
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
        block_info: Default::default(),
    }
}

/// Builds a non-boundary queue payload for scheduler tests.
fn test_payload(block_n: u64) -> QueuedClosePayload {
    test_payload_with_boundary(block_n, false)
}

/// Builds a queue payload with an explicit boundary marker.
fn test_payload_with_boundary(block_n: u64, is_boundary: bool) -> QueuedClosePayload {
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
    QueuedClosePayload {
        close_job_payload: CloseJobPayload { block_n },
        state: CurrentBlockState::new(backend, block_n),
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
        parallel_merkle_flush_interval: 3,
        compare_parallel_with_sequential: false,
        root_base_block_n: None,
        root_snapshot: None,
        root_state_diffs: Vec::new(),
        protocol_version: mp_chain_config::StarknetVersion::LATEST,
        last_execution_finished_at: None,
        close_block_received_at: Instant::now(),
        enqueued_at: Instant::now(),
    }
}

/// Returns one successful completion per serial test payload.
fn test_execute(
    _metrics: Arc<BlockProductionMetrics>,
    payloads: Vec<QueuedClosePayload>,
) -> BoxFuture<'static, Vec<Result<CloseJobCompletion>>> {
    Box::pin(async move {
        payloads
            .into_iter()
            .map(|payload| Ok(CloseJobCompletion { block_n: payload.close_job_payload.block_n }))
            .collect()
    })
}

/// Spawns a serial finalizer with the deterministic test executor.
fn spawn_test_serial(capacity: usize) -> (FinalizerHandle, FinalizerTaskHandle) {
    let metrics = Arc::new(BlockProductionMetrics::register());
    FinalizerHandle::spawn_with_workers(capacity, metrics, FinalizerWorkers::Serial { execute: Arc::new(test_execute) })
}

#[rstest]
#[case::cap1_enqueue1(1, 1, true)]
#[case::cap1_enqueue2(1, 2, false)]
#[case::cap2_enqueue2(2, 2, true)]
#[case::cap3_enqueue1(3, 1, true)]
#[tokio::test]
async fn backpressure_matrix(#[case] capacity: usize, #[case] enqueue_count: usize, #[case] all_succeed: bool) {
    let (handle, task_handle) = spawn_test_serial(capacity);
    let mut receivers = Vec::new();
    let mut enqueue_failures = 0;

    for block_n in 0..enqueue_count {
        match handle.try_enqueue(test_payload(block_n as u64)) {
            Ok((ClosePreconfirmedResult::Queued(_), receiver)) => receivers.push(receiver),
            Err(_) => enqueue_failures += 1,
        }
    }

    if all_succeed {
        assert_eq!(enqueue_failures, 0, "all enqueues should succeed with capacity={capacity}");
    } else {
        assert!(enqueue_failures > 0, "some enqueues should fail with capacity={capacity}");
    }
    for receiver in receivers {
        receiver.await.expect("completion channel should remain open").expect("close should succeed");
    }
    drop(handle);
    task_handle.join().await.expect("worker should complete cleanly");
}

#[tokio::test]
async fn ordered_completion() {
    let (handle, task_handle) = spawn_test_serial(8);
    let mut receivers = Vec::new();
    for block_n in 0..5 {
        let (_, receiver) = handle.try_enqueue(test_payload(block_n)).expect("enqueue should succeed");
        receivers.push(receiver);
    }
    for (expected, receiver) in receivers.into_iter().enumerate() {
        let completion = receiver.await.expect("channel open").expect("close ok");
        assert_eq!(completion.block_n, expected as u64);
    }
    drop(handle);
    task_handle.join().await.expect("worker should complete cleanly");
}

#[tokio::test]
async fn batches_stop_at_boundary() {
    let seen_batches = Arc::new(Mutex::new(Vec::<Vec<u64>>::new()));
    let execute: SerialExecute = {
        let seen_batches = Arc::clone(&seen_batches);
        Arc::new(move |_metrics, payloads| {
            let seen_batches = Arc::clone(&seen_batches);
            Box::pin(async move {
                let blocks = payloads.iter().map(|payload| payload.close_job_payload.block_n).collect::<Vec<_>>();
                seen_batches.lock().expect("batch log mutex").push(blocks);
                payloads
                    .into_iter()
                    .map(|payload| Ok(CloseJobCompletion { block_n: payload.close_job_payload.block_n }))
                    .collect()
            })
        })
    };
    let metrics = Arc::new(BlockProductionMetrics::register());
    let (handle, task_handle) = FinalizerHandle::spawn_with_workers(8, metrics, FinalizerWorkers::Serial { execute });

    let mut receivers = Vec::new();
    for (block_n, is_boundary) in [(0, false), (1, true), (2, false), (3, false)] {
        receivers.push(handle.try_enqueue(test_payload_with_boundary(block_n, is_boundary)).unwrap().1);
    }
    for receiver in receivers {
        receiver.await.unwrap().unwrap();
    }
    drop(handle);
    task_handle.join().await.unwrap();
    assert_eq!(*seen_batches.lock().unwrap(), vec![vec![0, 1], vec![2, 3]]);
}

#[tokio::test]
async fn drain_shutdown_completes_in_flight_job() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let execute: SerialExecute = {
        let gate = Arc::clone(&gate);
        Arc::new(move |_metrics, payloads| {
            let gate = Arc::clone(&gate);
            Box::pin(async move {
                gate.notified().await;
                payloads
                    .into_iter()
                    .map(|payload| Ok(CloseJobCompletion { block_n: payload.close_job_payload.block_n }))
                    .collect()
            })
        })
    };
    let metrics = Arc::new(BlockProductionMetrics::register());
    let (handle, task_handle) = FinalizerHandle::spawn_with_workers(4, metrics, FinalizerWorkers::Serial { execute });
    let (_, receiver) = handle.try_enqueue(test_payload(0)).unwrap();
    tokio::task::yield_now().await;
    drop(handle);
    gate.notify_one();
    assert_eq!(receiver.await.unwrap().unwrap().block_n, 0);
    task_handle.join().await.unwrap();
}

#[tokio::test]
async fn parallel_roots_can_finish_out_of_order_but_commit_in_order() {
    let metrics = Arc::new(BlockProductionMetrics::register());
    let commit_order = Arc::new(Mutex::new(Vec::new()));
    let max_active_roots = Arc::new(AtomicUsize::new(0));
    let active_roots = Arc::new(AtomicUsize::new(0));

    let prepare: ParallelPrepare = {
        let active_roots = Arc::clone(&active_roots);
        let max_active_roots = Arc::clone(&max_active_roots);
        Arc::new(move |_metrics, payload| {
            let active_roots = Arc::clone(&active_roots);
            let max_active_roots = Arc::clone(&max_active_roots);
            Box::pin(async move {
                let block_n = payload.close_job_payload.block_n;
                let now_active = active_roots.fetch_add(1, Ordering::Relaxed) + 1;
                max_active_roots.fetch_max(now_active, Ordering::Relaxed);
                sleep(if block_n == 0 { Duration::from_millis(40) } else { Duration::from_millis(5) }).await;
                active_roots.fetch_sub(1, Ordering::Relaxed);
                Ok(crate::close_pipeline::parallel_computed_payload_for_test(payload))
            })
        })
    };
    let commit: ParallelCommit = {
        let commit_order = Arc::clone(&commit_order);
        Arc::new(move |_metrics, computed| {
            let commit_order = Arc::clone(&commit_order);
            Box::pin(async move {
                let block_n = computed.block_n();
                commit_order.lock().unwrap().push(block_n);
                Ok(CloseJobCompletion { block_n })
            })
        })
    };
    let workers = FinalizerWorkers::Parallel { root_workers: 2, prepare, commit };
    let (handle, task_handle) = FinalizerHandle::spawn_with_workers(8, metrics, workers);

    let mut receivers = Vec::new();
    for block_n in 0..4 {
        receivers.push(handle.try_enqueue(test_payload(block_n)).unwrap().1);
    }
    for (expected, receiver) in receivers.into_iter().enumerate() {
        assert_eq!(receiver.await.unwrap().unwrap().block_n, expected as u64);
    }
    drop(handle);
    task_handle.join().await.unwrap();

    assert_eq!(*commit_order.lock().unwrap(), vec![0, 1, 2, 3]);
    assert!(max_active_roots.load(Ordering::Relaxed) >= 2);
}
