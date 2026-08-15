#![cfg(test)]
use super::*;
use crate::fallback::types::{ExecutionMode, RuntimeReplayStatus};
use crate::metrics::BlockProductionMetrics;
use crate::tests::devnet_setup;
use crate::tests::{make_declare_tx, make_udc_call, DevnetSetup};
use crate::util::{AdditionalTxInfo, BatchRoute, BlockifierRouteCause, RouteFallbackReason, RoutedBatchToExecute};
use assert_matches::assert_matches;
use blockifier::transaction::transaction_execution::Transaction;
use mc_db::MadaraBackend;
use mc_devnet::{Call, Multicall, Selector, RUST_EXEC_TRANSFER_CONTRACT_ADDRESS};
use mc_exec::execution::TxInfo;
use mp_chain_config::StarknetVersion;
use mp_convert::{Felt, ToFelt};
use mp_rpc::v0_9_0::BroadcastedTxn;
use mp_transactions::IntoStarknetApiExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rstest::fixture;
use starknet_core::utils::get_selector_from_name;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, mpsc::UnboundedSender, oneshot, watch};

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

fn make_rust_transfer_tx(
    setup: &DevnetSetup,
    sender_index: usize,
    recipient_index: usize,
    nonce: Felt,
    amount: u64,
) -> (Transaction, AdditionalTxInfo) {
    make_tx(
        &setup.backend,
        BroadcastedTxn::Invoke(crate::task::tests::make_invoke_tx(
            &setup.contracts.0[sender_index],
            Multicall::default().with(Call {
                to: RUST_EXEC_TRANSFER_CONTRACT_ADDRESS,
                selector: Selector::from("transfer"),
                calldata: vec![setup.contracts.0[recipient_index].address, Felt::from(amount)],
            }),
            &setup.backend,
            nonce,
        )),
    )
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

fn routed_batch(
    transactions: impl IntoIterator<Item = (Transaction, AdditionalTxInfo)>,
    route: BatchRoute,
    original_tx_hashes: Vec<Felt>,
    block_n: u64,
    execution_mode: ExecutionMode,
) -> RoutedBatchToExecute {
    RoutedBatchToExecute {
        transactions: transactions.into_iter().collect(),
        route,
        original_tx_hashes,
        block_n,
        execution_mode,
        execution_epoch: 0,
    }
}

fn blockifier_only_batch(
    transactions: impl IntoIterator<Item = (Transaction, AdditionalTxInfo)>,
    block_n: u64,
) -> RoutedBatchToExecute {
    routed_batch(
        transactions,
        BatchRoute::BlockifierOnly { cause: BlockifierRouteCause::FrozenBlockMode },
        Vec::new(),
        block_n,
        ExecutionMode::BlockifierOnly,
    )
}

fn start_executor_thread_for_tests(
    backend: Arc<MadaraBackend>,
    commands: mpsc::UnboundedReceiver<ExecutorCommand>,
    mode: ExecutionMode,
) -> ExecutorThreadHandle {
    let (replay_status_tx, _replay_status_rx) = watch::channel(RuntimeReplayStatus::idle());
    let (mode_tx, mode_rx) = watch::channel(mode);
    let (execution_epoch_tx, execution_epoch_rx) = watch::channel(0u64);
    let _ = Box::leak(Box::new(execution_epoch_tx));
    start_executor_thread(
        backend,
        commands,
        Arc::new(BlockProductionMetrics::register()),
        false,
        replay_status_tx,
        mode_tx,
        mode_rx,
        execution_epoch_rx,
        false,
        crate::BlockPipelineMode::Optimistic,
        10,
        mc_rust_exec::RustExecRuntimeConfig::default(),
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
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::BlockifierOnly);

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
        assert!(!res.execution_results[0].as_ref().unwrap().0.is_reverted());
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));

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
        tracing::debug!("res = {:?}", res.execution_results[0].as_ref().unwrap());
        assert!(!res.execution_results[0].as_ref().unwrap().0.is_reverted());
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));

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
        assert!(!res.execution_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
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
        assert!(!res.execution_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
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
        assert!(!res.execution_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));

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
        assert!(!res.execution_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(56u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// A mixed logical batch executes its Rust prefix before its Blockifier suffix and reports
/// results in the exact same source order.
#[rstest::rstest]
#[tokio::test]
async fn test_mixed_mode_both_branches_produce_combined_result(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    // Different accounts keep this test focused on ordered engine handoff.
    let (blockifier_tx, blockifier_info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let (rust_tx, rust_info) = make_rust_transfer_tx(&setup, 1, 2, Felt::ZERO, 42);
    let blockifier_hash = blockifier_tx.tx_hash().to_felt();
    let rust_hash = rust_tx.tx_hash().to_felt();

    let routed = routed_batch(
        [(rust_tx, rust_info), (blockifier_tx, blockifier_info)],
        BatchRoute::RustThenBlockifier {
            split_at: NonZeroUsize::new(1).unwrap(),
            trigger: RouteFallbackReason::NotInvoke,
        },
        vec![rust_hash, blockifier_hash],
        0,
        ExecutionMode::Mixed,
    );
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 2, "both execution engines should contribute to one result");
        assert_eq!(
            res.executed_txs.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(),
            vec![rust_hash, blockifier_hash],
            "physical execution must preserve source order"
        );
        assert_eq!(res.original_tx_hashes, vec![rust_hash, blockifier_hash]);
        assert_eq!(res.execution_results.len(), 2, "results must align positionally with executed txs");
    });

    // Close block.
    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

#[rstest::rstest]
#[tokio::test]
async fn mixed_block_returns_to_rust_after_a_blockifier_batch(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    let (blockifier_tx, blockifier_info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let blockifier_hash = blockifier_tx.tx_hash().to_felt();
    handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(routed_batch(
            [(blockifier_tx, blockifier_info)],
            BatchRoute::BlockifierOnly { cause: BlockifierRouteCause::Classifier(RouteFallbackReason::NotInvoke) },
            vec![blockifier_hash],
            0,
            ExecutionMode::Mixed,
        ))
        .await
        .unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.stats.n_added_by_blockifier, 1);
    });

    let (rust_tx, rust_info) = make_rust_transfer_tx(&setup, 1, 2, Felt::ZERO, 42);
    let rust_hash = rust_tx.tx_hash().to_felt();
    handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(routed_batch([(rust_tx, rust_info)], BatchRoute::RustOnly, vec![rust_hash], 0, ExecutionMode::Mixed))
        .await
        .unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.txs[0].tx_hash().to_felt(), rust_hash);
        assert_eq!(res.stats.n_added_by_rust_exec, 1);
        assert_eq!(res.stats.n_added_by_blockifier, 0);
    });

    let (sender, receiver) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    receiver.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

#[rstest::rstest]
#[tokio::test]
async fn same_sender_nonce_chain_crosses_the_engine_boundary_in_source_order(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    let (rust_0, rust_0_info) = make_rust_transfer_tx(&setup, 0, 2, Felt::ZERO, 41);
    let (cairo_1, cairo_1_info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ONE)),
    );
    let (rust_capable_2, rust_capable_2_info) = make_rust_transfer_tx(&setup, 0, 3, Felt::TWO, 43);
    let expected_hashes = [&rust_0, &cairo_1, &rust_capable_2].map(|tx| tx.tx_hash().to_felt()).to_vec();
    let routed = routed_batch(
        [(rust_0, rust_0_info), (cairo_1, cairo_1_info), (rust_capable_2, rust_capable_2_info)],
        BatchRoute::RustThenBlockifier {
            split_at: NonZeroUsize::new(1).unwrap(),
            trigger: RouteFallbackReason::NotInvoke,
        },
        expected_hashes.clone(),
        0,
        ExecutionMode::Mixed,
    );
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(
            res.executed_txs.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(),
            expected_hashes,
            "Rust(n), Cairo(n+1), and the barrier-amplified Cairo(n+2) must execute in source order"
        );
        assert_eq!(res.stats.n_added_by_rust_exec, 1);
        assert_eq!(res.stats.n_added_by_blockifier, 2);
        assert!(res.execution_results.iter().all(Result::is_ok));
        let final_state_diff = &res.execution_results.last().unwrap().as_ref().unwrap().1;
        assert!(
            final_state_diff.nonces.values().any(|nonce| nonce.to_felt() == Felt::from(3u64)),
            "the final Blockifier transaction must observe both earlier nonce increments"
        );
    });

    let (sender, receiver) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    receiver.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// The global bouncer transaction cap applies across both engines. A Blockifier suffix must
/// observe the Rust prefix already packed into the block and roll over to the next one.
#[rstest::rstest]
#[tokio::test]
async fn test_mixed_mode_uses_shared_global_transaction_cap(
    #[with(Duration::from_secs(30000), true)]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    let (blockifier_tx, blockifier_info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let blockifier_hash = blockifier_tx.tx_hash().to_felt();
    let (rust_tx, rust_info) = make_rust_transfer_tx(&setup, 1, 2, Felt::ZERO, 42);
    let rust_hash = rust_tx.tx_hash().to_felt();
    let routed = routed_batch(
        [(rust_tx, rust_info), (blockifier_tx, blockifier_info)],
        BatchRoute::RustThenBlockifier {
            split_at: NonZeroUsize::new(1).unwrap(),
            trigger: RouteFallbackReason::NotInvoke,
        },
        vec![rust_hash, blockifier_hash],
        1,
        ExecutionMode::Mixed,
    );
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(), vec![rust_hash]);
    });
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { block_exec_summary, .. }) => {
        assert_eq!(block_exec_summary.bouncer_weights.n_txs, 1);
    });

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(), vec![blockifier_hash]);
    });
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { block_exec_summary, .. }) => {
        assert_eq!(block_exec_summary.bouncer_weights.n_txs, 1);
    });
}

/// Blockifier-only mode normalizes and executes the complete ordered payload in place.
#[rstest::rstest]
#[tokio::test]
async fn test_blockifier_only_mode_executes_the_complete_ordered_payload(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::BlockifierOnly);

    let (tx1, info1) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let (tx2, info2) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[1], &setup.backend, Felt::ZERO)),
    );

    let routed = blockifier_only_batch([(tx1, info1), (tx2, info2)], 0);
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 2, "the complete ordered payload should execute in one iteration");
    });

    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// In Mixed mode a homogeneous Rust payload executes through Rust.
#[rstest::rstest]
#[tokio::test]
async fn test_mixed_mode_rust_only_payload_executes(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    let (tx, info) = make_rust_transfer_tx(&setup, 0, 1, Felt::ZERO, 42);
    let routed = routed_batch([(tx, info)], BatchRoute::RustOnly, Vec::new(), 0, ExecutionMode::Mixed);
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "mixed mode: rust-only payload should execute");
    });

    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// A payload that was classified as Rust is normalized at a Blockifier-only boundary without
/// losing or delaying the transaction.
#[rstest::rstest]
#[tokio::test]
async fn test_blockifier_only_normalizes_a_rust_routed_payload(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::BlockifierOnly);

    let (tx, info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let tx_hash = tx.tx_hash().to_felt();
    let routed = routed_batch([(tx, info)], BatchRoute::RustOnly, Vec::new(), 0, ExecutionMode::Mixed);
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "normalized Rust-routed tx must execute via Blockifier");
        assert_eq!(res.executed_txs.txs[0].tx_hash().to_felt(), tx_hash, "same tx hash preserved");
    });

    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// A mode transition from Mixed to BlockifierOnly normalizes a queued Rust route without loss.
#[rstest::rstest]
#[tokio::test]
async fn test_mode_transition_normalizes_queued_rust_txs(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    // Send two Rust-routed txs under Mixed mode.
    let (tx1, info1) = make_rust_transfer_tx(&setup, 0, 2, Felt::ZERO, 42);
    let (tx2, info2) = make_rust_transfer_tx(&setup, 1, 3, Felt::ZERO, 43);
    let tx1_hash = tx1.tx_hash().to_felt();
    let tx2_hash = tx2.tx_hash().to_felt();
    let routed = routed_batch([(tx1, info1), (tx2, info2)], BatchRoute::RustOnly, Vec::new(), 0, ExecutionMode::Mixed);
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    // Mixed mode: both Rust transactions execute in the Rust phase.
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 2, "mixed mode: both rust txs should execute");
    });

    // Close the block, then switch to BlockifierOnly before next batch.
    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));

    // Switch desired mode to BlockifierOnly for the next block.
    commands_sender.send(ExecutorCommand::SetDesiredExecutionMode { mode: ExecutionMode::BlockifierOnly }).unwrap();

    // Send another Rust-routed batch. The frozen mode normalizes it to Blockifier in place.
    let (tx3, info3) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[2], &setup.backend, Felt::ZERO)),
    );
    let tx3_hash = tx3.tx_hash().to_felt();
    let routed2 = routed_batch([(tx3, info3)], BatchRoute::RustOnly, Vec::new(), 0, ExecutionMode::Mixed);
    handle.send_batch.as_mut().unwrap().send(routed2).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "Rust-routed tx must execute via Blockifier after mode switch");
        assert_eq!(res.executed_txs.txs[0].tx_hash().to_felt(), tx3_hash, "same tx hash preserved");
    });

    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));

    let _ = (tx1_hash, tx2_hash); // suppress unused warnings
}

#[rstest::rstest]
#[tokio::test]
async fn desired_mode_change_mid_block_does_not_switch_later_batches_in_same_block(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    let batch1: RoutedBatchToExecute = [make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    )]
    .into_iter()
    .collect();
    handle.send_batch.as_mut().unwrap().send(batch1).await.unwrap();

    assert_matches!(
        handle.replies.recv().await,
        Some(ExecutorMessage::StartNewBlock { execution_mode, .. }) => {
            assert_eq!(execution_mode, ExecutionMode::Mixed, "first block should freeze Mixed mode");
        }
    );
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.execution_mode, ExecutionMode::Mixed, "first batch should execute in Mixed mode");
    });

    commands_sender.send(ExecutorCommand::SetDesiredExecutionMode { mode: ExecutionMode::BlockifierOnly }).unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let batch2: RoutedBatchToExecute = [make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[1], &setup.backend, Felt::ZERO)),
    )]
    .into_iter()
    .collect();
    handle.send_batch.as_mut().unwrap().send(batch2).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(
            res.execution_mode,
            ExecutionMode::Mixed,
            "later batches in the same block must keep the frozen Mixed mode"
        );
    });

    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));

    let batch3: RoutedBatchToExecute = [make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[2], &setup.backend, Felt::ZERO)),
    )]
    .into_iter()
    .collect();
    handle.send_batch.as_mut().unwrap().send(batch3).await.unwrap();

    assert_matches!(
        handle.replies.recv().await,
        Some(ExecutorMessage::StartNewBlock { execution_mode, .. }) => {
            assert_eq!(
                execution_mode,
                ExecutionMode::BlockifierOnly,
                "the next block should pick up the desired BlockifierOnly mode"
            );
        }
    );

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.execution_mode, ExecutionMode::BlockifierOnly);
    });
}
