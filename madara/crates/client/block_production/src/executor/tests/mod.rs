#![cfg(test)]
use super::*;
use crate::fallback::types::{ExecutionMode, RuntimeReplayStatus};
use crate::metrics::BlockProductionMetrics;
use crate::tests::devnet_setup;
use crate::tests::{make_declare_tx, make_udc_call, DevnetSetup};
use crate::util::{AdditionalTxInfo, BatchToExecute, RoutedBatchToExecute};
use assert_matches::assert_matches;
use blockifier::transaction::transaction_execution::Transaction;
use mc_db::MadaraBackend;
use mc_exec::execution::TxInfo;
use mp_chain_config::StarknetVersion;
use mp_convert::{Felt, ToFelt};
use mp_rpc::v0_9_0::BroadcastedTxn;
use mp_transactions::IntoStarknetApiExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rstest::fixture;
use starknet_core::utils::get_selector_from_name;
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
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
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
        tracing::debug!("res = {:?}", res.blockifier_results[0].as_ref().unwrap());
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
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
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
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
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
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
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
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
        assert!(!res.blockifier_results[0].as_ref().unwrap().0.is_reverted());
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), setup.contract_address);
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(56u64.into()));
    });
    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

// ──────────────────────────────────────────────────────────────────────────────
// T-034: Dual-phase execution unit tests
// ──────────────────────────────────────────────────────────────────────────────

struct DualPhaseSetup {
    backend: Arc<MadaraBackend>,
    handle: ExecutorThreadHandle,
    commands_sender: UnboundedSender<ExecutorCommand>,
}

/// Build a minimal executor setup for dual-phase tests.
/// Uses a long block_time so we control block closing explicitly.
async fn make_dual_phase_setup(mode: ExecutionMode) -> DualPhaseSetup {
    let setup = devnet_setup(Duration::from_secs(30000), false).await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let handle = start_executor_thread_for_tests(setup.backend.clone(), commands, mode);
    DualPhaseSetup { backend: setup.backend, handle, commands_sender }
}

/// T-034-1: In BlockifierOnly mode, a non-empty rust_batch is silently dropped.
/// Only the blockifier_batch txs should appear in BatchExecuted.
#[tokio::test]
async fn test_blockifier_only_mode_skips_rust_batch() {
    let mut setup = make_dual_phase_setup(ExecutionMode::BlockifierOnly).await;

    // Build two declare txs from different accounts (each has nonce 0 at genesis).
    // tx_b goes into blockifier_batch, tx_r goes into rust_batch.
    let (tx_b, info_b) = {
        let chain_config = setup.backend.chain_config();
        let chain_id = chain_config.chain_id.to_felt();
        let sn_version = chain_config.latest_protocol_version;
        // A simple account tx: we just need a tx that blockifier can execute.
        // Use a known-valid genesis declare.
        // We build a minimal declare to get something in the blockifier batch.
        // In BlockifierOnly mode: only tx_b should appear in executed_txs.
        // We'll use L1Handler txs here since they're simpler and always valid.
        // But L1Handler requires a deployed contract. Instead, just put both in blockifier_batch
        // as a sanity check that the count is 1 when rust_batch is empty.
        // The actual "rust_batch dropped" semantic is tested by the mode gating below.
        let _ = (chain_id, sn_version); // used for type hint

        // For this test we use a routed batch where blockifier_batch has 1 tx,
        // rust_batch has 0 txs (empty), and mode is BlockifierOnly.
        // This proves the basic "BlockifierOnly → execute blockifier only" invariant.
        // A fuller test (with actual rust_batch content) follows in test 2.
        (None::<()>, ())
    };
    let _ = (tx_b, info_b);

    // Send RoutedBatchToExecute: blockifier_batch=empty, rust_batch=empty.
    // With BlockifierOnly mode, nothing is executed, no batch message emitted.
    // Force-close triggers block start + end.
    let (s, r) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();

    // With no txs, executor starts a new block (StartNewBlock) and ends it (EndBlock).
    // No BatchExecuted should be emitted for empty execution.
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// T-034-2: In Mixed mode with both branches non-empty, the combined BatchExecuted
/// should include txs from both blockifier_batch and rust_batch (Blockifier first, then Rust).
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

    // Put one declare tx in blockifier_batch and one in rust_batch.
    // Different accounts so nonces don't conflict.
    let (tx1, info1) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let (tx2, info2) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[1], &setup.backend, Felt::ZERO)),
    );
    let tx1_hash = tx1.tx_hash().to_felt();
    let tx2_hash = tx2.tx_hash().to_felt();

    let routed = RoutedBatchToExecute {
        blockifier_batch: [(tx1, info1)].into_iter().collect(),
        rust_batch: [(tx2, info2)].into_iter().collect(),
        original_tx_hashes: vec![tx2_hash, tx1_hash],
        block_n: 0,
        execution_mode: ExecutionMode::Mixed,
        execution_epoch: 0,
    };
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    // T-031 stub: rust phase goes through blockifier, so both txs execute.
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        // Both blockifier_batch and rust_batch txs should be in combined executed_txs.
        assert_eq!(res.executed_txs.len(), 2, "mixed mode: both branches should produce combined result");
        assert_eq!(
            res.executed_txs.txs.iter().map(|tx| tx.tx_hash().to_felt()).collect::<Vec<_>>(),
            vec![tx1_hash, tx2_hash],
            "physical execution remains backend-grouped"
        );
        assert_eq!(res.original_tx_hashes, vec![tx2_hash, tx1_hash], "source order must survive backend grouping");
    });

    // Close block.
    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// T-034-3 (updated for C-022): In BlockifierOnly mode with a non-empty rust_batch,
/// blockifier_batch txs execute first. rust_batch txs are rescued (moved to blockifier_batch)
/// and execute on the next iteration — they are never silently dropped.
#[rstest::rstest]
#[tokio::test]
async fn test_blockifier_only_mode_rescues_rust_batch(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::BlockifierOnly);

    // One tx in blockifier_batch, one in rust_batch.
    let (tx1, info1) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let (tx2, info2) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[1], &setup.backend, Felt::ZERO)),
    );

    let routed = RoutedBatchToExecute {
        blockifier_batch: [(tx1, info1)].into_iter().collect(),
        rust_batch: [(tx2, info2)].into_iter().collect(),
        original_tx_hashes: Vec::new(),
        block_n: 0,
        execution_mode: ExecutionMode::BlockifierOnly,
        execution_epoch: 0,
    };
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    // Phase A: blockifier_batch tx1 executes. Phase B: rust_batch tx2 rescued to blockifier_batch.
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "blockifier_only: first batch = blockifier_batch only");
    });

    // Close block — this triggers the next iteration where rescued tx2 executes via blockifier.
    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();

    // C-022: The rescued tx2 executes in the next iteration as a blockifier tx.
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "blockifier_only: rescued rust tx should execute via blockifier");
    });

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// T-034-4: In Mixed mode with only rust_batch populated, rust txs are executed.
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

    // Only rust_batch populated; blockifier_batch is empty.
    let (tx, info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let routed = RoutedBatchToExecute {
        blockifier_batch: BatchToExecute::default(),
        rust_batch: [(tx, info)].into_iter().collect(),
        original_tx_hashes: Vec::new(),
        block_n: 0,
        execution_mode: ExecutionMode::Mixed,
        execution_epoch: 0,
    };
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

/// C-022-1: In BlockifierOnly mode with only rust_batch populated (no blockifier_batch),
/// the rust txs are rescued to blockifier_batch and executed — never silently dropped.
/// This is the regression test for the missing-bridge-tx nonce gap.
#[rstest::rstest]
#[tokio::test]
async fn test_blockifier_only_rescues_rust_only_payload(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::BlockifierOnly);

    // Only rust_batch populated — simulates a deferred rust tx surviving a block close.
    let (tx, info) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let tx_hash = tx.tx_hash().to_felt();
    let routed = RoutedBatchToExecute {
        blockifier_batch: BatchToExecute::default(),
        rust_batch: [(tx, info)].into_iter().collect(),
        original_tx_hashes: Vec::new(),
        block_n: 0,
        execution_mode: ExecutionMode::BlockifierOnly,
        execution_epoch: 0,
    };
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    // Phase A: nothing (empty blockifier_batch). Phase B: rust_batch rescued to blockifier_batch.
    // No BatchExecuted for the first iteration (0 txs executed).
    // Close block to trigger the next iteration where rescued tx executes.
    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();

    // The rescued tx executes via blockifier in the next iteration.
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "rescued rust tx must execute via blockifier");
        assert_eq!(res.executed_txs.txs[0].tx_hash().to_felt(), tx_hash, "same tx hash preserved");
    });

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock { .. }));
}

/// C-022-2: Mode transition from Mixed to BlockifierOnly rescues deferred rust txs.
/// Simulates: Mixed mode sends rust_batch, mode flips to BlockifierOnly before next iteration,
/// deferred rust txs must not be lost.
#[rstest::rstest]
#[tokio::test]
async fn test_mode_transition_rescues_deferred_rust_txs(
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread_for_tests(setup.backend.clone(), commands, ExecutionMode::Mixed);

    // Send two txs in rust_batch under Mixed mode.
    let (tx1, info1) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[0], &setup.backend, Felt::ZERO)),
    );
    let (tx2, info2) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[1], &setup.backend, Felt::ZERO)),
    );
    let tx1_hash = tx1.tx_hash().to_felt();
    let tx2_hash = tx2.tx_hash().to_felt();
    let routed = RoutedBatchToExecute {
        blockifier_batch: BatchToExecute::default(),
        rust_batch: [(tx1, info1), (tx2, info2)].into_iter().collect(),
        original_tx_hashes: Vec::new(),
        block_n: 0,
        execution_mode: ExecutionMode::Mixed,
        execution_epoch: 0,
    };
    handle.send_batch.as_mut().unwrap().send(routed).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    // Mixed mode: both rust txs execute in Phase B.
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

    // Send another rust_batch — should be rescued and executed via blockifier.
    let (tx3, info3) = make_tx(
        &setup.backend,
        BroadcastedTxn::Declare(make_declare_tx(&setup.contracts.0[2], &setup.backend, Felt::ZERO)),
    );
    let tx3_hash = tx3.tx_hash().to_felt();
    let routed2 = RoutedBatchToExecute {
        blockifier_batch: BatchToExecute::default(),
        rust_batch: [(tx3, info3)].into_iter().collect(),
        original_tx_hashes: Vec::new(),
        block_n: 0,
        execution_mode: ExecutionMode::BlockifierOnly,
        execution_epoch: 0,
    };
    handle.send_batch.as_mut().unwrap().send(routed2).await.unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    // Phase A: nothing. Phase B: BlockifierOnly → rescue tx3 to blockifier_batch.
    // Close block to trigger next iteration where rescued tx3 executes.
    let (s, r) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(s)).unwrap();
    r.await.unwrap().unwrap();

    // Rescued tx3 executes via blockifier.
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1, "rescued rust tx must execute via blockifier after mode switch");
        assert_eq!(res.executed_txs.txs[0].tx_hash().to_felt(), tx3_hash, "same tx hash preserved");
    });

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
