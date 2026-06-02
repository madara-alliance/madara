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
use mp_rpc::v0_9_0::BroadcastedTxn;
use mp_transactions::IntoStarknetApiExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rstest::fixture;
use starknet_core::utils::get_selector_from_name;
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
        start_executor_thread(setup.backend.clone(), commands, Arc::new(BlockProductionMetrics::register())).unwrap();

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

/// The `ExecuteBatch` command executes the whole batch as one contiguous unit and reports each
/// transaction's outcome, in submission order, on the response channel.
#[rstest::rstest]
#[tokio::test]
async fn test_execute_batch_command(
    // long block time, no pending tick: keep the block open so the whole batch lands together.
    #[with(Duration::from_secs(30000))]
    #[future]
    devnet_setup: DevnetSetup,
) {
    use crate::tests::make_invoke_tx;
    use mc_devnet::{Call, Multicall, Selector};

    let setup = devnet_setup.await;
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle =
        start_executor_thread(setup.backend.clone(), commands, Arc::new(BlockProductionMetrics::register())).unwrap();

    let erc20 = Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
    let sender = &setup.contracts.0[0];
    let receiver = &setup.contracts.0[1];
    // Two transfers from the same account with sequential nonces (a dependent ordered batch).
    let transfer = |nonce: Felt| {
        make_invoke_tx(
            sender,
            Multicall::default().with(Call {
                to: erc20,
                selector: Selector::from("transfer"),
                calldata: vec![receiver.address, Felt::from(1_000u128), Felt::ZERO],
            }),
            &setup.backend,
            nonce,
        )
    };

    let batch: BatchToExecute = [
        make_tx(&setup.backend, BroadcastedTxn::Invoke(transfer(Felt::ZERO))),
        make_tx(&setup.backend, BroadcastedTxn::Invoke(transfer(Felt::ONE))),
    ]
    .into_iter()
    .collect();
    let hashes: Vec<Felt> = batch.txs.iter().map(|t| t.tx_hash().to_felt()).collect();

    let (response, response_rx) = oneshot::channel();
    commands_sender.send(ExecutorCommand::ExecuteBatch { batch, response }).unwrap();

    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 2);
    });

    // The response reports both transactions, in submission order, both succeeded.
    let outcomes = response_rx.await.expect("batch response should be sent");
    assert_eq!(outcomes.iter().map(|(h, _)| *h).collect::<Vec<_>>(), hashes);
    for (_, outcome) in &outcomes {
        assert_matches!(outcome, super::TxExecutionOutcome::Succeeded);
    }

    // Close the block to tear down the executor's open block state cleanly.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender)).unwrap();
    recv.await.unwrap().unwrap();
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock(_)));
}
