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
    (tx, AdditionalTxInfo { declared_class, arrived_at: ts, from_mempool: false })
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
    (tx, AdditionalTxInfo { declared_class, arrived_at: Default::default(), from_mempool: false })
}

#[rstest::rstest]
#[case::rejected_predecessor(true)]
#[case::reverted_predecessor(false)]
#[tokio::test]
async fn contiguous_nonce_execution_preserves_successors(#[case] reject: bool) {
    use crate::tests::make_invoke_tx;
    use crate::CurrentBlockState;
    use futures::FutureExt;
    use mc_db::preconfirmed::PreconfirmedBlock;
    use mc_devnet::{Call, Multicall, Selector};

    let setup = devnet_setup(Duration::from_secs(30000), false, true).await;
    let sender = &setup.contracts.0[0];
    let mut first = make_invoke_tx(
        sender,
        Multicall::default().with(Call {
            to: sender.address,
            selector: Selector::from("deliberately_missing_entrypoint"),
            calldata: vec![],
        }),
        &setup.backend,
        Felt::ZERO,
    );
    if reject {
        // Fault injection after admission: model validation becoming invalid between
        // admission and execution. This is NOT a production admission bypass.
        let mp_rpc::v0_9_0::BroadcastedInvokeTxn::V3(tx) = &mut first else { unreachable!() };
        tx.signature = vec![Felt::ZERO, Felt::ZERO].into();
    }
    let next = make_invoke_tx(sender, Multicall::default(), &setup.backend, Felt::ONE);
    let to_mempool_tx = |tx| {
        BroadcastedTxn::Invoke(tx)
            .into_validated_tx(
                setup.backend.chain_config().chain_id.to_felt(),
                StarknetVersion::LATEST,
                mp_transactions::validated::TxTimestamp::now(),
            )
            .unwrap()
    };
    setup.mempool.accept_tx(to_mempool_tx(first)).await.unwrap();
    let successor = to_mempool_tx(next);
    setup.mempool.accept_tx(successor.clone()).await.unwrap();
    let take_batch = |mut consumer: mc_mempool::MempoolConsumer| {
        std::iter::from_fn(|| consumer.next_contiguous(300))
            .map(|tx| {
                let (tx, arrived_at, declared_class) = tx.into_blockifier_for_sequencing().unwrap();
                (tx, AdditionalTxInfo { arrived_at, declared_class, from_mempool: true })
            })
            .collect::<BatchToExecute>()
    };
    let batch = take_batch(setup.mempool.get_consumer().await);
    assert_eq!(batch.len(), 2);
    let (_commands_sender, commands) = mpsc::unbounded_channel();
    let mut handle = start_executor_thread(setup.backend.clone(), commands, setup.metrics.clone(), false).unwrap();
    handle.send_batch.as_ref().unwrap().send(batch).await.unwrap();
    let Some(ExecutorMessage::StartNewBlock { exec_ctx }) = handle.replies.recv().await else {
        panic!("expected block start")
    };
    let block_n = exec_ctx.block_number;
    setup.backend.write_access().new_preconfirmed(PreconfirmedBlock::new(exec_ctx.into_header())).unwrap();
    let Some(ExecutorMessage::BatchExecuted(result)) = handle.replies.recv().await else { panic!("expected results") };
    assert_eq!(result.blockifier_results.len(), 2);
    if reject {
        assert!(result.blockifier_results.iter().all(Result::is_err));
    } else {
        assert!(result.blockifier_results[0].as_ref().unwrap().0.is_reverted());
        assert!(result.blockifier_results[1].is_ok(), "revert must consume the predecessor nonce");
    }
    let mut current = CurrentBlockState::new(setup.backend.clone(), block_n);
    let deferred = current.append_batch(result).await.unwrap();
    if reject {
        assert_eq!(deferred, vec![successor]);
        for tx in deferred {
            setup.mempool.requeue_tx(tx).await.unwrap();
        }
        assert!(setup.mempool.get_consumer().now_or_never().is_none());
        let replacement = make_invoke_tx(sender, Multicall::default(), &setup.backend, Felt::ZERO);
        setup.mempool.accept_tx(to_mempool_tx(replacement)).await.unwrap();
        let batch = take_batch(setup.mempool.get_consumer().await);
        assert_eq!(batch.len(), 2);
        handle.send_batch.as_ref().unwrap().send(batch).await.unwrap();
        let Some(ExecutorMessage::BatchExecuted(result)) = handle.replies.recv().await else {
            panic!("expected retry results")
        };
        assert!(result.blockifier_results.iter().all(Result::is_ok));
        assert!(current.append_batch(result).await.unwrap().is_empty());
    } else {
        assert!(deferred.is_empty());
    }
    assert_eq!(setup.backend.block_view_on_preconfirmed(block_n).unwrap().num_executed_transactions(), 2);
    handle.send_batch.take();
    while tokio::time::timeout(Duration::from_secs(30), handle.replies.recv()).await.unwrap().is_some() {}
    tokio::time::timeout(Duration::from_secs(30), handle.stop.recv()).await.unwrap().unwrap();
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

#[tokio::test]
async fn block_full_tail_precedes_fresh_work_and_drains_on_shutdown() {
    use blockifier::bouncer::{BouncerConfig, BouncerWeights};
    use mc_devnet::{ChainGenesisDescription, Multicall};
    use mp_chain_config::{BlockProductionConfig, ChainConfig};

    let mut genesis = ChainGenesisDescription::base_config().unwrap();
    let contracts = genesis.add_devnet_contracts(1).unwrap();
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig {
        block_time: Duration::from_secs(30000),
        block_production_concurrency: BlockProductionConfig {
            batch_size: 100,
            disable_concurrency: true,
            ..Default::default()
        },
        bouncer_config: BouncerConfig {
            block_max_capacity: BouncerWeights { n_txs: 170, ..BouncerWeights::max() },
            builtin_weights: Default::default(),
        },
        ..ChainConfig::madara_devnet()
    }));
    backend.set_l1_gas_quote_for_testing();
    genesis.build_and_store(&backend).await.unwrap();
    let (sender, batches) = mpsc::channel(3);
    let (replies, mut received) = mpsc::channel(100);
    let (_commands_sender, commands) = mpsc::unbounded_channel();
    let mut expected_hashes = Vec::new();
    for start in [0u64, 100, 200] {
        let mut batch = BatchToExecute::default();
        for nonce in start..start + 100 {
            let tx = crate::tests::make_invoke_tx(&contracts.0[0], Multicall::default(), &backend, Felt::from(nonce));
            let (tx, mut info) = make_tx(&backend, BroadcastedTxn::Invoke(tx));
            expected_hashes.push(tx.tx_hash().to_felt());
            info.from_mempool = true;
            info.arrived_at = mp_transactions::validated::TxTimestamp(nonce);
            batch.push(tx, info);
        }
        sender.send(batch).await.unwrap();
    }
    // Close intake before execution: even the retained tail must finish before shutdown.
    drop(sender);
    let worker = tokio::task::spawn_blocking(move || {
        thread::ExecutorThread::new(
            backend,
            batches,
            replies,
            commands,
            Arc::new(BlockProductionMetrics::register()),
            false,
        )
        .unwrap()
        .run()
    });
    let mut lengths = Vec::new();
    let mut actual_hashes = Vec::new();
    let mut timestamps = Vec::new();
    let mut final_block = false;
    while let Some(reply) = tokio::time::timeout(Duration::from_secs(60), received.recv()).await.unwrap() {
        match reply {
            ExecutorMessage::BatchExecuted(result) => {
                assert!(result.blockifier_results.iter().all(Result::is_ok));
                lengths.push(result.executed_txs.len());
                for (tx, info) in result.executed_txs {
                    actual_hashes.push(tx.tx_hash().to_felt());
                    timestamps.push(info.arrived_at.0);
                    assert!(info.from_mempool);
                }
            }
            ExecutorMessage::EndFinalBlock(Some(_)) => final_block = true,
            _ => {}
        }
    }
    worker.await.unwrap().unwrap();
    // The first 100 leave room for only 70 of the next 100 in block zero.
    // Block one must begin with the remaining 30 plus 70 fresh, then the last 30.
    assert_eq!(lengths, [100, 70, 100, 30]);
    assert_eq!(actual_hashes, expected_hashes);
    assert_eq!(timestamps, (0..300).collect::<Vec<_>>());
    assert!(final_block);
}

#[tokio::test]
async fn capped_batches_release_capacity_during_concurrent_admission() {
    use crate::{BlockProductionStateNotification, BlockProductionTask};
    use mc_devnet::{ChainGenesisDescription, Multicall};
    use mc_mempool::{Mempool, MempoolConfig, MempoolInsertionError};
    use mc_settlement_client::L1ClientMock;
    use mp_chain_config::{BlockProductionConfig, ChainConfig, MempoolFullPolicy};
    use mp_utils::{service::ServiceContext, AbortOnDrop};

    let mut genesis = ChainGenesisDescription::base_config().unwrap();
    let contracts = genesis.add_devnet_contracts(3).unwrap();
    let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig {
        block_time: Duration::from_secs(30000),
        mempool_max_transactions: 120,
        mempool_full_policy: MempoolFullPolicy::RejectNew,
        block_production_concurrency: BlockProductionConfig {
            batch_size: 100,
            max_txs_per_account_per_batch: 30,
            disable_concurrency: true,
            ..Default::default()
        },
        ..ChainConfig::madara_devnet()
    }));
    backend.set_l1_gas_quote_for_testing();
    genesis.build_and_store(&backend).await.unwrap();
    let pool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default().with_save_to_db(false)));
    let mut expected = Vec::new();
    let mut later = Vec::new();
    for account in &contracts.0 {
        for nonce in 0u64..60 {
            let tx = BroadcastedTxn::Invoke(crate::tests::make_invoke_tx(
                account,
                Multicall::default(),
                &backend,
                Felt::from(nonce),
            ))
            .into_validated_tx(
                backend.chain_config().chain_id.to_felt(),
                StarknetVersion::LATEST,
                mp_transactions::validated::TxTimestamp::now(),
            )
            .unwrap();
            expected.push(tx.hash);
            if nonce < 40 {
                pool.accept_tx(tx).await.unwrap();
            } else {
                later.push(tx);
            }
        }
    }
    let ctx = ServiceContext::new_for_testing();
    let watcher_ctx = ctx.clone();
    let watcher_pool = pool.clone();
    let watcher = AbortOnDrop::spawn(async move { watcher_pool.run_mempool_task(watcher_ctx).await });
    let mut producer = BlockProductionTask::new(
        backend.clone(),
        pool.clone(),
        Arc::new(BlockProductionMetrics::register()),
        Arc::new(L1ClientMock::new()),
        false,
        false,
        false,
    );
    let mut notifications = producer.subscribe_state_notifications();
    let producer_ctx = ctx.clone();
    let production = AbortOnDrop::spawn(async move { producer.run(producer_ctx).await });
    let admission_pool = pool.clone();
    let admission = AbortOnDrop::spawn(async move {
        for tx in later {
            loop {
                match admission_pool.accept_tx(tx.clone()).await {
                    Ok(()) => break,
                    Err(MempoolInsertionError::InnerMempool(mc_mempool::TxInsertionError::Limit(_))) => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("unexpected admission failure: {error}"),
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    tokio::time::timeout(Duration::from_secs(60), async {
        admission.await.unwrap();
        loop {
            if matches!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted)
                && backend.block_view_on_current_preconfirmed().unwrap().num_executed_transactions() == expected.len()
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    let mut actual = backend.block_view_on_current_preconfirmed().unwrap().get_block_info().tx_hashes.clone();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    assert!(pool.is_empty().await);
    ctx.cancel_global();
    tokio::time::timeout(Duration::from_secs(30), production).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(30), watcher).await.unwrap().unwrap();
}
