#![cfg(test)]
use super::*;
use crate::metrics::BlockProductionMetrics;
use crate::tests::{make_declare_tx, make_udc_call, DevnetSetup};
use crate::{tests::devnet_setup, util::AdditionalTxInfo};
use assert_matches::assert_matches;
use blockifier::transaction::transaction_execution::Transaction;
use mc_db::MadaraBackend;
use mc_exec::execution::TxInfo;
use mp_chain_config::StarknetVersion;
use mp_convert::{Felt, ToFelt};
use mp_rpc::admin::ReplayBlockBoundary;
use mp_rpc::v0_9_0::BroadcastedTxn;
use mp_transactions::IntoStarknetApiExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rstest::fixture;
use starknet_core::utils::get_selector_from_name;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

pub(super) fn make_tx(backend: &MadaraBackend, tx: impl IntoStarknetApiExt) -> (Transaction, AdditionalTxInfo) {
    let (tx, ts, declared_class) = tx
        .into_validated_tx(
            backend.chain_config().chain_id.to_felt(),
            StarknetVersion::LATEST,
            mp_transactions::validated::TxTimestamp::UNIX_EPOCH,
        )
        .unwrap()
        .into_blockifier_for_sequencing()
        .unwrap();
    (tx, AdditionalTxInfo { declared_class, arrived_at: ts })
}

fn make_l1_handler_tx(
    backend: &MadaraBackend,
    contract_address: Felt,
    nonce: u64,
    from_l1_address: Felt,
    arg1: Felt,
    arg2: Felt,
) -> (Transaction, AdditionalTxInfo) {
    let (tx, declared_class) = L1HandlerTransactionWithFee::new(
        L1HandlerTransaction {
            version: Felt::ZERO,
            nonce,
            contract_address,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            calldata: vec![from_l1_address, arg1, arg2].into(),
        },
        /* paid_fee_on_l1 */ 128328,
    )
    .into_blockifier(backend.chain_config().chain_id.to_felt(), StarknetVersion::LATEST)
    .unwrap();
    (tx, AdditionalTxInfo { declared_class, arrived_at: Default::default() })
}

fn make_executor_thread_for_tests(backend: Arc<MadaraBackend>, replay_mode_enabled: bool) -> thread::ExecutorThread {
    let (_send_batch, incoming_batches) = mpsc::channel(1);
    let (replies_sender, _replies_recv) = mpsc::channel(1);
    let (_commands_sender, commands) = mpsc::unbounded_channel();
    thread::ExecutorThread::new(
        backend,
        incoming_batches,
        replies_sender,
        commands,
        Arc::new(BlockProductionMetrics::register()),
        replay_mode_enabled,
    )
    .unwrap()
}

struct L1HandlerSetup {
    backend: Arc<MadaraBackend>,
    handle: ExecutorThreadHandle,
    commands_sender: UnboundedSender<ExecutorCommand>,
    contract_address: Felt,
}

#[fixture]
async fn l1_handler_setup(
    // long block time, no pending tick
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) -> L1HandlerSetup {
    let setup = devnet_setup.await;

    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle =
        start_executor_thread(setup.backend.clone(), commands, Arc::new(BlockProductionMetrics::register()), false)
            .unwrap();

    let (tx, additional_info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let class_hash = tx.declared_class_hash().unwrap().to_felt();
    // Send declare tx.
    handle.send_batch.as_mut().unwrap().send([(tx, additional_info)].into_iter().collect()).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1);
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));

    // Deploy account using udc.

    let (contract_address, tx) = make_udc_call(
        &setup.contracts.0[0],
        &setup.backend,
        /* nonce */ Felt::ONE,
        class_hash,
        /* calldata (pubkey) */ &[Felt::TWO],
    );
    handle
        .send_batch
        .as_mut()
        .unwrap()
        .send([make_tx(&setup.backend, BroadcastedTxn::Invoke(tx))].into_iter().collect())
        .await
        .unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1);
        tracing::debug!("res = {:?}", res.blockifier_results[0].as_ref().unwrap());
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));

    L1HandlerSetup { backend: setup.backend.clone(), handle, commands_sender, contract_address }
}

// we test 4 cases:
// * the two l1handlertx are in the same batch
// * the two l1handlertx are at the same height different batch
// * the two l1handlertx are at different heights but still in state adaptor cache
// * the nonce is in db.

#[rstest::rstest]
#[tokio::test]
// Case 1: two in same batch.
async fn test_duplicate_l1_handler_same_batch(#[future] l1_handler_setup: L1HandlerSetup) {
    let mut setup = l1_handler_setup.await;

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [
                make_l1_handler_tx(
                    &setup.backend,
                    setup.contract_address,
                    /* nonce */ 55,
                    Felt::from_hex_unchecked("0x10101010"),
                    Felt::ONE,
                    Felt::TWO,
                ),
                make_l1_handler_tx(
                    &setup.backend,
                    setup.contract_address,
                    /* nonce */ 55,
                    Felt::from_hex_unchecked("0x102222"),
                    Felt::ONE,
                    Felt::TWO,
                ),
            ]
            .into_iter()
            .collect(),
        )
        .await
        .unwrap();

    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1); // only one transaction! not two
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));
}

#[rstest::rstest]
#[tokio::test]
// Case 2: the two l1handlertx are at the same height different batch
async fn test_duplicate_l1_handler_same_height_different_batch(#[future] l1_handler_setup: L1HandlerSetup) {
    let mut setup = l1_handler_setup.await;

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_l1_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("0x10101010"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await
        .unwrap();

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_l1_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("0x191919"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await
        .unwrap();

    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1); // only one transaction! not two
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));
}

#[rstest::rstest]
#[tokio::test]
// Case 4: the l1handlertx is already in db.
async fn test_duplicate_l1_handler_in_db(#[future] l1_handler_setup: L1HandlerSetup) {
    let mut setup = l1_handler_setup.await;

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_l1_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("0x120101010"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await
        .unwrap();

    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1);
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));

    // Make another block.

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [
                make_l1_handler_tx(
                    &setup.backend,
                    setup.contract_address,
                    /* nonce */ 55, // Already used.
                    Felt::from_hex_unchecked("0x120101010"),
                    Felt::ONE,
                    Felt::TWO,
                ),
                make_l1_handler_tx(
                    &setup.backend,
                    setup.contract_address,
                    /* nonce */ 56, // another nonce, this one wasn't used.
                    Felt::from_hex_unchecked("0x120101010"),
                    Felt::ONE,
                    Felt::TWO,
                ),
            ]
            .into_iter()
            .collect(),
        )
        .await
        .unwrap();

    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1); // only one transaction! not two. Nonce 55 is already used.
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(56u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));
}

#[rstest::rstest]
#[tokio::test]
async fn replay_boundary_spillover_moves_overflow_suffix(#[future] devnet_setup: DevnetSetup) {
    let setup = devnet_setup.await;
    let thread = make_executor_thread_for_tests(setup.backend.clone(), true);
    let txs = [
        make_tx(
            &setup.backend,
            BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
        ),
        make_tx(
            &setup.backend,
            BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ONE)),
        ),
        make_tx(
            &setup.backend,
            BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::TWO)),
        ),
    ];
    let tx_hashes: Vec<_> = txs.iter().map(|(tx, _)| tx.tx_hash().to_felt()).collect();
    let block_n = 17;
    setup.backend.set_replay_boundary(ReplayBlockBoundary {
        block_n,
        expected_tx_count: 2,
        last_tx_hash: tx_hashes[1],
    });

    let mut to_exec: BatchToExecute = txs.into_iter().collect();
    let mut replay_next_block_buffer = BatchToExecute::default();
    thread.apply_replay_boundary_capacity(block_n, &mut to_exec, &mut replay_next_block_buffer);

    assert_eq!(to_exec.len(), 2);
    assert_eq!(replay_next_block_buffer.len(), 1);
    assert_eq!(to_exec.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(), tx_hashes[..2].to_vec());
    assert_eq!(replay_next_block_buffer.txs[0].tx_hash().to_felt(), tx_hashes[2]);
}

#[rstest::rstest]
#[tokio::test]
async fn replay_close_decision_waits_for_boundary_or_force_close(#[future] devnet_setup: DevnetSetup) {
    let setup = devnet_setup.await;
    let thread = make_executor_thread_for_tests(setup.backend.clone(), true);
    let last_tx_hash = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    )
    .0
    .tx_hash()
    .to_felt();
    let block_n = 23;
    setup.backend.set_replay_boundary(ReplayBlockBoundary { block_n, expected_tx_count: 1, last_tx_hash });

    let pending = thread.replay_close_decision(
        block_n, /* force_close */ false, /* block_full */ true, /* block_time_deadline_reached */ true,
    );
    assert!(!pending.should_close);
    assert!(pending.replay_boundary_exists);
    assert!(!pending.replay_boundary_met);
    assert_eq!(pending.reason, None);

    let forced = thread.replay_close_decision(
        block_n, /* force_close */ true, /* block_full */ false, /* block_time_deadline_reached */ false,
    );
    assert!(forced.should_close);
    assert_eq!(forced.reason, Some(thread::CloseReason::ForceClose));

    setup.backend.replay_boundary_record_executed_hashes(block_n, &[last_tx_hash]).unwrap();
    let met = thread.replay_close_decision(
        block_n, /* force_close */ false, /* block_full */ true, /* block_time_deadline_reached */ true,
    );
    assert!(met.should_close);
    assert!(met.replay_boundary_met);
    assert_eq!(met.reason, Some(thread::CloseReason::ReplayBoundaryMet));
}

#[test]
fn dashboard_uses_histograms_for_wait_metrics() {
    let dashboard_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../observability/grafana/dashboards/Madara/madara-overview.json");
    let dashboard = std::fs::read_to_string(&dashboard_path).unwrap();

    for removed_metric in [
        ["batcher", "batch", "wait", "last", "seconds"].join("_"),
        ["batcher", "output", "backpressure", "last", "seconds"].join("_"),
        ["executor", "inter", "batch", "wait", "last", "seconds"].join("_"),
    ] {
        assert!(
            !dashboard.contains(removed_metric.as_str()),
            "dashboard still references removed metric `{removed_metric}`"
        );
    }

    for histogram_metric in [
        "batcher_batch_wait_duration_seconds_bucket",
        "batcher_output_backpressure_duration_seconds_bucket",
        "executor_inter_batch_wait_duration_seconds_bucket",
    ] {
        assert!(
            dashboard.contains(histogram_metric),
            "dashboard should use histogram buckets for `{histogram_metric}`"
        );
    }
}
