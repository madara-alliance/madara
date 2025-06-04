#![cfg(test)]
use super::*;
use crate::tests::{make_declare_tx, make_invoke_tx};
use crate::BlockProductionTask;
use crate::{metrics::BlockProductionMetrics, tests::devnet_setup, util::AdditionalTxInfo};
use assert_matches::assert_matches;
use blockifier::transaction::transaction_execution::Transaction;
use mc_db::MadaraBackend;
use mc_devnet::{Call, DevnetKeys, Multicall, Selector, UDC_CONTRACT_ADDRESS};
use mc_exec::execution::TxInfo;
use mc_mempool::{Mempool, MempoolConfig, MockL1DataProvider};
use mc_submit_tx::TransactionValidator;
use mp_chain_config::StarknetVersion;
use mp_convert::ToFelt;
use mp_rpc::BroadcastedTxn;
use mp_transactions::IntoBlockifierExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rstest::fixture;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

fn make_tx(backend: &MadaraBackend, tx: impl IntoBlockifierExt) -> (Transaction, AdditionalTxInfo) {
    let (tx, declared_class) =
        tx.into_blockifier(backend.chain_config().chain_id.to_felt(), StarknetVersion::LATEST).unwrap();
    (tx, AdditionalTxInfo { declared_class })
}

fn make_handler_tx(
    backend: &MadaraBackend,
    contract_address: Felt,
    nonce: u64,
    from_l1_address: Felt,
    arg1: Felt,
    arg2: Felt,
) -> (Transaction, AdditionalTxInfo) {
    make_tx(
        &backend,
        L1HandlerTransactionWithFee::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce,
                contract_address,
                entry_point_selector: Felt::from_bytes_be_slice(b"l1_handler_entrypoint"),
                calldata: vec![from_l1_address, arg1, arg2],
            },
            /* paid_fee_on_l1 */ 128328392,
        ),
    )
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
    #[with(Duration::from_secs(30000), None)]
    #[future]
    devnet_setup: (
        Arc<MadaraBackend>,
        Arc<BlockProductionMetrics>,
        Arc<MockL1DataProvider>,
        Arc<Mempool>,
        Arc<TransactionValidator>,
        DevnetKeys,
    ),
) -> L1HandlerSetup {
    let (backend, _metrics, l1_data_provider, _mempool, _tx_validator, contracts) = devnet_setup.await;

    let (commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread(backend.clone(), l1_data_provider, commands).unwrap();

    // Send declare tx.
    handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_tx(&backend, BroadcastedTxn::Declare(make_declare_tx(&contracts.0[0], &backend, Felt::ZERO)))]
                .into_iter()
                .collect(),
        )
        .await;

    // Close block.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender));
    recv.await;
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        println!("{res:?}");
        assert_eq!(res.executed_txs.len(), 1);
        assert_eq!(res.blockifier_results[0].as_ref().unwrap().0.is_reverted(), false);
    });
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock));

    let class_hash = Felt::ONE;
    let contract_address = Felt::ONE;

    // Deploy account using udc.

    handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_tx(
                &backend,
                BroadcastedTxn::Invoke(make_invoke_tx(
                    &contracts.0[0],
                    Multicall::default().with(Call {
                        to: UDC_CONTRACT_ADDRESS,
                        selector: Selector::from("deployContract"),
                        calldata: vec![
                            class_hash,
                            /* salt */ Felt::ZERO,
                            /* unique */ Felt::ONE,
                            /* call_data.len */ Felt::ONE,
                            /* calldata (pubkey) */ Felt::TWO,
                        ],
                    }),
                    &backend,
                    Felt::ONE,
                )),
            )]
            .into_iter()
            .collect(),
        )
        .await;

    // Close block.
    let (sender, recv) = oneshot::channel();
    commands_sender.send(ExecutorCommand::CloseBlock(sender));
    recv.await;
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        println!("{res:?}");
        assert_eq!(res.executed_txs.len(), 1);
        assert_eq!(res.blockifier_results[0].as_ref().unwrap().0.is_reverted(), false);
    });
    assert_matches!(handle.replies.recv().await, Some(ExecutorMessage::EndBlock));

    L1HandlerSetup { backend, handle, commands_sender, contract_address }
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
                make_handler_tx(
                    &setup.backend,
                    setup.contract_address,
                    /* nonce */ 55,
                    Felt::from_hex_unchecked("asjdjkasd"),
                    Felt::ONE,
                    Felt::TWO,
                ),
                make_handler_tx(
                    &setup.backend,
                    setup.contract_address,
                    /* nonce */ 55,
                    Felt::from_hex_unchecked("eeee"),
                    Felt::ONE,
                    Felt::TWO,
                ),
            ]
            .into_iter()
            .collect(),
        )
        .await;

    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender));
    recv.await;
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1); // only one transaction! not two
        assert_eq!(res.blockifier_results[0].as_ref().unwrap().0.is_reverted(), false);
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), Felt::from_hex_unchecked("asjdjkasd"));
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock));
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
            [make_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("asjdjkasd"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await;

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("eeee"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await;

    // Close block.
    let (sender, recv) = oneshot::channel();
    setup.commands_sender.send(ExecutorCommand::CloseBlock(sender));
    recv.await;
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::StartNewBlock { .. }));
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1); // only one transaction! not two
        assert_eq!(res.blockifier_results[0].as_ref().unwrap().0.is_reverted(), false);
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), Felt::from_hex_unchecked("asjdjkasd"));
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    assert_matches!(setup.handle.replies.recv().await, Some(ExecutorMessage::EndBlock));
}

#[rstest::rstest]
#[tokio::test]
// Case 4: the l1handlertx is already in db.
// We use BlockProductionTask here a bit to help us process the messages and save to db.
// TODO: move this test out of executor tests then?
async fn test_duplicate_l1_handler_in_db(#[future] l1_handler_setup: L1HandlerSetup) {
    let mut setup = l1_handler_setup.await;

    let mut task = BlockProductionTask::new(
        setup.backend.clone(),
        Mempool::new(setup.backend.clone(), MempoolConfig::for_testing()).into(),
        BlockProductionMetrics::register().into(),
        Arc::new(MockL1DataProvider::new()) as _,
    );

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("asjdjkasd"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await;

    // Close block.
    task.handle.close_block().await.unwrap();

    let r = setup.handle.replies.recv().await;
    assert_matches!(&r, Some(ExecutorMessage::StartNewBlock { .. }));
    task.process_reply(r.unwrap()).await;
    let r = setup.handle.replies.recv().await;
    assert_matches!(&r, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 1);
        assert_eq!(res.blockifier_results[0].as_ref().unwrap().0.is_reverted(), false);
        assert_eq!(res.executed_txs.txs[0].contract_address().to_felt(), Felt::from_hex_unchecked("asjdjkasd"));
        assert_eq!(res.executed_txs.txs[0].l1_handler_tx_nonce().map(ToFelt::to_felt), Some(55u64.into()));
    });
    task.process_reply(r.unwrap()).await;
    let r = setup.handle.replies.recv().await;
    assert_matches!(&r, Some(ExecutorMessage::EndBlock));
    task.process_reply(r.unwrap()).await;

    setup
        .handle
        .send_batch
        .as_mut()
        .unwrap()
        .send(
            [make_handler_tx(
                &setup.backend,
                setup.contract_address,
                /* nonce */ 55,
                Felt::from_hex_unchecked("eeee"),
                Felt::ONE,
                Felt::TWO,
            )]
            .into_iter()
            .collect(),
        )
        .await;

    let r = setup.handle.replies.recv().await;
    assert_matches!(&r, Some(ExecutorMessage::StartNewBlock { .. }));
    task.process_reply(r.unwrap()).await;
    let r = setup.handle.replies.recv().await;
    assert_matches!(&r, Some(ExecutorMessage::BatchExecuted(res)) => {
        assert_eq!(res.executed_txs.len(), 0); // zero
    });
    task.process_reply(r.unwrap()).await;
    let r = setup.handle.replies.recv().await;
    assert_matches!(&r, Some(ExecutorMessage::EndBlock));
    task.process_reply(r.unwrap()).await;
}
