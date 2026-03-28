use crate::errors::ErrorExtWs;
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_9_0::{TxnStatusWithoutL1, TxnWithHash, TxnWithHashAndStatus};
use mp_transactions::{validated::ValidatedTransaction, Transaction};
use starknet_types_core::felt::Felt;
use std::{collections::HashSet, future::pending, sync::Arc};

pub async fn subscribe_new_transactions(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<TxnStatusWithoutL1>>,
    sender_address: Option<Vec<Felt>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    subscribe_new_transactions_inner(starknet, subscription_sink, finality_status, sender_address, false).await
}

pub async fn subscribe_new_transactions_with_reorg(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<TxnStatusWithoutL1>>,
    sender_address: Option<Vec<Felt>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    subscribe_new_transactions_inner(starknet, subscription_sink, finality_status, sender_address, true).await
}

async fn subscribe_new_transactions_inner(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<TxnStatusWithoutL1>>,
    sender_address: Option<Vec<Felt>>,
    emit_reorg_notifications: bool,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = if sender_address.as_ref().map_or(0, Vec::len) as u64 <= super::ADDRESS_FILTER_LIMIT {
        subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?
    } else {
        subscription_sink.reject(crate::errors::StarknetWsApiError::TooManyAddressesInFilter).await;
        return Ok(());
    };

    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;
    let allowed_statuses =
        finality_status.unwrap_or_else(|| vec![TxnStatusWithoutL1::AcceptedOnL2]).into_iter().collect::<HashSet<_>>();
    let sender_address = sender_address.map(|addresses| addresses.into_iter().collect::<HashSet<_>>());
    let mut emitted = HashSet::<(Felt, TxnStatusWithoutL1)>::new();

    let mut received_watch = if allowed_statuses.contains(&TxnStatusWithoutL1::Received) {
        Some(
            starknet
                .new_transactions_watcher
                .as_ref()
                .ok_or_else(|| {
                    crate::errors::StarknetWsApiError::internal_server_error(
                        "SubscribeNewTransactions failed: new-transactions watcher is not configured",
                    )
                })?
                .watch_new_transactions()
                .ok_or_else(|| {
                    crate::errors::StarknetWsApiError::internal_server_error(
                        "SubscribeNewTransactions failed to create new-transactions watcher",
                    )
                })?,
        )
    } else {
        None
    };

    let mut heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
    let mut current_preconfirmed = starknet
        .backend
        .block_view_on_preconfirmed_or_fake()
        .or_internal_server_error("SubscribeNewTransactions failed to create preconfirmed block view")?;
    current_preconfirmed.refresh_with_candidates();
    let mut reorgs = emit_reorg_notifications.then(|| starknet.backend.subscribe_reorgs());

    loop {
        tokio::select! {
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
            received = async {
                match &mut received_watch {
                    Some(watch) => watch.recv().await,
                    None => pending::<Option<Arc<ValidatedTransaction>>>().await,
                }
            } => {
                if let Some(tx) = received {
                    send_validated_transaction(
                        &sink,
                        tx.as_ref(),
                        TxnStatusWithoutL1::Received,
                        sender_address.as_ref(),
                        &allowed_statuses,
                        &mut emitted,
                    ).await?;
                }
            }
            reorg = async {
                match &mut reorgs {
                    Some(reorgs) => reorgs.recv().await,
                    None => pending::<Result<mc_db::ReorgNotification, tokio::sync::broadcast::error::RecvError>>().await,
                }
            } => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        emitted.clear();
                        heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                        heads.set_start_from(reorg.first_reverted_block_n);
                        current_preconfirmed = starknet
                            .backend
                            .block_view_on_preconfirmed_or_fake()
                            .or_internal_server_error("SubscribeNewTransactions failed to refresh preconfirmed block view after reorg")?;
                        current_preconfirmed.refresh_with_candidates();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            }
            _ = current_preconfirmed.wait_until_outdated() => {
                current_preconfirmed.refresh_with_candidates();
                send_preconfirmed_view_transactions(
                    &sink,
                    &current_preconfirmed,
                    sender_address.as_ref(),
                    &allowed_statuses,
                    &mut emitted,
                ).await?;
            }
            block_view = heads.next_block_view() => {
                if block_view.is_confirmed() {
                    send_confirmed_block_transactions(
                        &sink,
                        &block_view,
                        sender_address.as_ref(),
                        &allowed_statuses,
                        &mut emitted,
                    ).await?;
                    current_preconfirmed = starknet
                        .backend
                        .block_view_on_preconfirmed_or_fake()
                        .or_internal_server_error("SubscribeNewTransactions failed to refresh preconfirmed block view")?;
                    current_preconfirmed.refresh_with_candidates();
                } else {
                    let mut preconfirmed = block_view
                        .into_preconfirmed()
                        .expect("Preconfirmed block subscription should yield a preconfirmed block view");
                    preconfirmed.refresh_with_candidates();
                    send_preconfirmed_view_transactions(
                        &sink,
                        &preconfirmed,
                        sender_address.as_ref(),
                        &allowed_statuses,
                        &mut emitted,
                    ).await?;
                    current_preconfirmed = preconfirmed;
                }
            }
        }
    }
}

async fn send_preconfirmed_view_transactions(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    preconfirmed: &mc_db::view::MadaraPreconfirmedBlockView,
    sender_address: Option<&HashSet<Felt>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
    emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    if allowed_statuses.contains(&TxnStatusWithoutL1::PreConfirmed) {
        for tx in preconfirmed.get_executed_transactions(..) {
            send_executed_transaction(
                sink,
                &tx,
                TxnStatusWithoutL1::PreConfirmed,
                sender_address,
                allowed_statuses,
                emitted,
            )
            .await?;
        }
    }

    if allowed_statuses.contains(&TxnStatusWithoutL1::Candidate) {
        for tx in preconfirmed.candidate_transactions() {
            send_validated_transaction(
                sink,
                tx.as_ref(),
                TxnStatusWithoutL1::Candidate,
                sender_address,
                allowed_statuses,
                emitted,
            )
            .await?;
        }
    }

    Ok(())
}

async fn send_confirmed_block_transactions(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    block_view: &mc_db::MadaraBlockView,
    sender_address: Option<&HashSet<Felt>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
    emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    if !allowed_statuses.contains(&TxnStatusWithoutL1::AcceptedOnL2) {
        return Ok(());
    }

    for tx in block_view
        .get_executed_transactions(..)
        .or_internal_server_error("SubscribeNewTransactions failed to retrieve confirmed block transactions")?
    {
        send_executed_transaction(
            sink,
            &tx,
            TxnStatusWithoutL1::AcceptedOnL2,
            sender_address,
            allowed_statuses,
            emitted,
        )
        .await?;
    }

    Ok(())
}

async fn send_validated_transaction(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    tx: &ValidatedTransaction,
    status: TxnStatusWithoutL1,
    sender_address: Option<&HashSet<Felt>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
    emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    if !allowed_statuses.contains(&status)
        || !sender_address.map_or(true, |addresses| addresses.contains(&tx.contract_address))
        || !mark_emitted(emitted, tx.hash, &status)
    {
        return Ok(());
    }

    send_transaction_item(
        sink,
        TxnWithHashAndStatus {
            transaction: TxnWithHash { transaction: tx.transaction.clone().to_rpc_v0_8(), transaction_hash: tx.hash },
            finality_status: status,
        },
    )
    .await
}

async fn send_executed_transaction(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    tx: &mp_block::TransactionWithReceipt,
    status: TxnStatusWithoutL1,
    sender_address: Option<&HashSet<Felt>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
    emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let tx_hash = *tx.receipt.transaction_hash();
    if !allowed_statuses.contains(&status)
        || !transaction_matches_sender(&tx.transaction, sender_address)
        || !mark_emitted(emitted, tx_hash, &status)
    {
        return Ok(());
    }

    send_transaction_item(
        sink,
        TxnWithHashAndStatus {
            transaction: TxnWithHash { transaction: tx.transaction.clone().to_rpc_v0_8(), transaction_hash: tx_hash },
            finality_status: status,
        },
    )
    .await
}

async fn send_transaction_item(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    item: TxnWithHashAndStatus,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let tx_hash = item.transaction.transaction_hash;
    let item = super::SubscriptionItem::new(sink.subscription_id(), item);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item).or_else_internal_server_error(|| {
        format!("SubscribeNewTransactions failed to create response for tx hash {tx_hash:#x}")
    })?;

    sink.send(msg).await.or_internal_server_error("SubscribeNewTransactions failed to respond to websocket request")
}

fn mark_emitted(emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>, tx_hash: Felt, status: &TxnStatusWithoutL1) -> bool {
    emitted.insert((tx_hash, status.clone()))
}

fn transaction_matches_sender(transaction: &Transaction, sender_address: Option<&HashSet<Felt>>) -> bool {
    let Some(sender_address) = sender_address else {
        return true;
    };

    match transaction {
        Transaction::Invoke(inner) => sender_address.contains(inner.sender_address()),
        Transaction::L1Handler(inner) => sender_address.contains(&inner.contract_address),
        Transaction::Declare(inner) => sender_address.contains(inner.sender_address()),
        Transaction::Deploy(inner) => sender_address.contains(&inner.calculate_contract_address()),
        Transaction::DeployAccount(inner) => sender_address.contains(&inner.calculate_contract_address()),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        test_utils::{rpc_test_setup, TestNewTransactionsWatcher},
        versions::user::{
            v0_10_0::{StarknetWsRpcApiV0_10_0Client, StarknetWsRpcApiV0_10_0Server},
            v0_9_0::{StarknetWsRpcApiV0_9_0Client, StarknetWsRpcApiV0_9_0Server},
        },
        Starknet,
    };
    use assert_matches::assert_matches;
    use jsonrpsee::{
        core::{client::SubscriptionClientT, params::ObjectParams},
        ws_client::WsClientBuilder,
    };
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
    use mp_chain_config::{ChainConfig, StarknetVersion};
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_transactions::{
        validated::{TxTimestamp, ValidatedTransaction},
        InvokeTransaction, InvokeTransactionV0, Transaction as MpTransaction,
    };
    use mp_utils::service::ServiceContext;
    use serde_json::Value;
    use std::{
        sync::{
            atomic::{AtomicU64, Ordering::Relaxed},
            Arc,
        },
        time::Duration,
    };

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("0x1234");
    const OTHER_SENDER_ADDRESS: Felt = Felt::from_hex_unchecked("0x5678");

    fn next_hash() -> Felt {
        static HASH: AtomicU64 = AtomicU64::new(1);
        HASH.fetch_add(1, Relaxed).into()
    }

    fn transaction_with_receipt(sender_address: Felt, transaction_hash: Felt) -> TransactionWithReceipt {
        TransactionWithReceipt {
            transaction: MpTransaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                contract_address: sender_address,
                ..Default::default()
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                actual_fee: FeePayment { amount: Felt::from_hex_unchecked("0x9"), unit: PriceUnit::Wei },
                messages_sent: vec![],
                events: vec![],
                execution_resources: ExecutionResources::default(),
                execution_result: ExecutionResult::Succeeded,
            }),
        }
    }

    fn validated_tx(sender_address: Felt) -> ValidatedTransaction {
        ValidatedTransaction {
            transaction: MpTransaction::Invoke(InvokeTransaction::V0(InvokeTransactionV0 {
                contract_address: sender_address,
                ..Default::default()
            })),
            paid_fee_on_l1: None,
            contract_address: sender_address,
            arrived_at: TxTimestamp::now(),
            declared_class: None,
            hash: next_hash(),
            charge_fee: true,
        }
    }

    async fn start_v0_9_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder().build(SERVER_ADDR).await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn start_v0_10_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder().build(SERVER_ADDR).await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_10_0Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_new_transactions(
        client: &jsonrpsee::ws_client::WsClient,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_0_subscribeNewTransactions",
            ObjectParams::new(),
            "starknet_V0_10_0_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_0_subscribeNewTransactions")
    }

    #[tokio::test]
    async fn subscribe_new_transactions_default_finality_emits_confirmed_transactions_v0_10() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_v0_10_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_0Client::subscribe_new_transactions(&client, None, None)
            .await
            .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x5151");
        let tx = transaction_with_receipt(OTHER_SENDER_ADDRESS, transaction_hash);
        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![tx.clone()],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block");

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for transaction")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve transaction");

        assert_eq!(
            serde_json::to_value(&item).expect("Failed to serialize transaction item")["result"],
            serde_json::to_value(TxnWithHashAndStatus {
                transaction: TxnWithHash { transaction: tx.transaction.to_rpc_v0_8(), transaction_hash },
                finality_status: TxnStatusWithoutL1::AcceptedOnL2,
            })
            .expect("Failed to serialize expected transaction")
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_received_filter_and_sender_v0_9() {
        let (backend, mut starknet) = rpc_test_setup();
        let watcher = TestNewTransactionsWatcher::new();
        starknet.set_new_transactions_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_v0_9_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_9_0Client::subscribe_new_transactions(
            &client,
            Some(vec![TxnStatusWithoutL1::Received]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let tx_1 = validated_tx(SENDER_ADDRESS);
        let tx_2 = validated_tx(OTHER_SENDER_ADDRESS);
        let _ = backend; // keep backend alive with the server state

        watcher.send_transaction(tx_2);
        watcher.send_transaction(tx_1.clone());

        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for transaction")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve transaction");

        assert_eq!(
            item.result,
            TxnWithHashAndStatus {
                transaction: TxnWithHash { transaction: tx_1.transaction.to_rpc_v0_8(), transaction_hash: tx_1.hash },
                finality_status: TxnStatusWithoutL1::Received,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_preconfirmed_and_candidate_v0_9() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_v0_9_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_9_0Client::subscribe_new_transactions(
            &client,
            Some(vec![TxnStatusWithoutL1::PreConfirmed, TxnStatusWithoutL1::Candidate]),
            Some(vec![SENDER_ADDRESS]),
        )
        .await
        .expect("Failed subscription");

        let preconfirmed_hash = Felt::from_hex_unchecked("0x6262");
        let candidate = Arc::new(validated_tx(SENDER_ADDRESS));
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(
                PreconfirmedHeader {
                    block_number: 0,
                    protocol_version: StarknetVersion::V0_13_2,
                    ..Default::default()
                },
                vec![PreconfirmedExecutedTransaction {
                    transaction: transaction_with_receipt(SENDER_ADDRESS, preconfirmed_hash),
                    state_diff: Default::default(),
                    declared_class: None,
                    arrived_at: Default::default(),
                    paid_fee_on_l1: None,
                }],
                vec![candidate.clone()],
            ))
            .expect("Failed to store preconfirmed block");

        let first = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for first transaction")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve first transaction");
        let second = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for second transaction")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve second transaction");

        assert_eq!(
            first.result,
            TxnWithHashAndStatus {
                transaction: TxnWithHash {
                    transaction: transaction_with_receipt(SENDER_ADDRESS, preconfirmed_hash).transaction.to_rpc_v0_8(),
                    transaction_hash: preconfirmed_hash,
                },
                finality_status: TxnStatusWithoutL1::PreConfirmed,
            }
        );
        assert_eq!(
            second.result,
            TxnWithHashAndStatus {
                transaction: TxnWithHash {
                    transaction: candidate.transaction.clone().to_rpc_v0_8(),
                    transaction_hash: candidate.hash,
                },
                finality_status: TxnStatusWithoutL1::Candidate,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_rejects_too_many_sender_addresses_v0_9() {
        let chain_config = Arc::new(ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let mut starknet = Starknet::new(
            backend,
            Arc::new(crate::test_utils::TestTransactionProvider),
            Default::default(),
            None,
            ServiceContext::new_for_testing(),
        );
        starknet.set_new_transactions_watcher(Some(TestNewTransactionsWatcher::new()));

        let (_handle, server_url) = start_v0_9_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let size = super::super::ADDRESS_FILTER_LIMIT as usize + 1;
        let err = StarknetWsRpcApiV0_9_0Client::subscribe_new_transactions(
            &client,
            Some(vec![TxnStatusWithoutL1::Received]),
            Some(vec![SENDER_ADDRESS; size]),
        )
        .await
        .expect_err("Subscription should fail");

        assert_matches!(
            err,
            jsonrpsee::core::client::error::Error::Call(err) => {
                assert_eq!(err, crate::errors::StarknetWsApiError::TooManyAddressesInFilter.into());
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_reorg_then_resume_v0_10() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_v0_10_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let block_0_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 0,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block 0")
            .block_hash;
        let block_1_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 1,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store confirmed block 1")
            .block_hash;

        let mut sub = raw_subscribe_new_transactions(&client).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_10_0::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_1_hash,
                ending_block_number: 1,
            })
            .expect("Failed to serialize expected reorg notification")
        );

        let transaction_hash = Felt::from_hex_unchecked("0x9898");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader {
                        block_number: 1,
                        protocol_version: StarknetVersion::V0_13_2,
                        ..Default::default()
                    },
                    state_diff: Default::default(),
                    transactions: vec![tx.clone()],
                    events: vec![],
                },
                &[],
                true,
            )
            .expect("Failed to store replacement confirmed block 1");

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for replacement transaction")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve replacement transaction");
        let item: super::super::SubscriptionItem<TxnWithHashAndStatus> =
            serde_json::from_value(next).expect("Failed to deserialize replacement transaction item");

        assert_eq!(
            item.result,
            TxnWithHashAndStatus {
                transaction: TxnWithHash { transaction: tx.transaction.to_rpc_v0_8(), transaction_hash },
                finality_status: TxnStatusWithoutL1::AcceptedOnL2,
            }
        );
    }
}
