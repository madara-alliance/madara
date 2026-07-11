use crate::errors::ErrorExtWs;
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_10_2::{SubscriptionTag, TxnStatusWithoutL1, TxnWithHashAndProofFacts, TxnWithHashAndStatus};
use mp_transactions::{validated::ValidatedTransaction, Transaction};
use starknet_types_core::felt::Felt;
use std::{collections::HashSet, future::pending, sync::Arc};

pub async fn subscribe_new_transactions(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<TxnStatusWithoutL1>>,
    sender_address: Option<Vec<Felt>>,
    tags: Option<Vec<SubscriptionTag>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    subscribe_new_transactions_inner(starknet, subscription_sink, finality_status, sender_address, tags, false).await
}

pub async fn subscribe_new_transactions_with_reorg(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<TxnStatusWithoutL1>>,
    sender_address: Option<Vec<Felt>>,
    tags: Option<Vec<SubscriptionTag>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    subscribe_new_transactions_inner(starknet, subscription_sink, finality_status, sender_address, tags, true).await
}

async fn subscribe_new_transactions_inner(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<TxnStatusWithoutL1>>,
    sender_address: Option<Vec<Felt>>,
    tags: Option<Vec<SubscriptionTag>>,
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
    let sender_address = crate::normalize_sender_address_filter(sender_address);
    let include_proof_facts = tags.as_ref().is_some_and(|tags| tags.contains(&SubscriptionTag::IncludeProofFacts));
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
                    None => pending::<Result<Option<Arc<ValidatedTransaction>>, crate::NewTransactionsWatchError>>().await,
                }
            } => {
                match received {
                    Ok(Some(tx)) => {
                        send_validated_transaction(
                            &sink,
                            tx.as_ref(),
                            TxnStatusWithoutL1::Received,
                            sender_address.as_ref(),
                            &allowed_statuses,
                            include_proof_facts,
                            &mut emitted,
                        ).await?;
                    }
                    Ok(None) => {
                        received_watch = None;
                    }
                    Err(crate::NewTransactionsWatchError::Lagged) => {
                        return Err(super::missed_received_transaction_notifications_error());
                    }
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
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        return Err(super::missed_reorg_notifications_error());
                    }
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
                    include_proof_facts,
                    &mut emitted,
                ).await?;
            }
            block_view = heads.next_block_view() => {
                if block_view.is_confirmed() {
                    let block_number = block_view.block_number();
                    if let Some(reorgs) = reorgs.as_mut() {
                        match crate::resolve_live_confirmed_head(
                            &starknet.backend,
                            reorgs,
                            block_number,
                            super::missed_reorg_notifications_error(),
                        )? {
                            crate::LiveConfirmedHeadResolution::Block(_) => {}
                            crate::LiveConfirmedHeadResolution::Reorg(reorg) => {
                                super::send_reorg_notification(&sink, &reorg).await?;
                                emitted.clear();
                                heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                                heads.set_start_from(reorg.first_reverted_block_n);
                                current_preconfirmed = starknet
                                    .backend
                                    .block_view_on_preconfirmed_or_fake()
                                    .or_internal_server_error("SubscribeNewTransactions failed to refresh preconfirmed block view after reorg")?;
                                current_preconfirmed.refresh_with_candidates();
                                continue;
                            }
                            crate::LiveConfirmedHeadResolution::RetryBackfill => {
                                heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                                heads.set_start_from(block_number);
                                current_preconfirmed = starknet
                                    .backend
                                    .block_view_on_preconfirmed_or_fake()
                                    .or_internal_server_error("SubscribeNewTransactions failed to refresh preconfirmed block view")?;
                                current_preconfirmed.refresh_with_candidates();
                                continue;
                            }
                        }
                    }
                    let mut live_reorgs = reorgs.as_mut();
                    if let Some(reorg) = send_confirmed_block_transactions(
                        &sink,
                        &block_view,
                        sender_address.as_ref(),
                        &mut live_reorgs,
                        &allowed_statuses,
                        include_proof_facts,
                        &mut emitted,
                    ).await? {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        emitted.clear();
                        heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                        heads.set_start_from(reorg.first_reverted_block_n);
                        current_preconfirmed = starknet
                            .backend
                            .block_view_on_preconfirmed_or_fake()
                            .or_internal_server_error("SubscribeNewTransactions failed to refresh preconfirmed block view after reorg")?;
                        current_preconfirmed.refresh_with_candidates();
                        continue;
                    }
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
                        include_proof_facts,
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
    include_proof_facts: bool,
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
                include_proof_facts,
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
                include_proof_facts,
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
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
    include_proof_facts: bool,
    emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>,
) -> Result<Option<mc_db::ReorgNotification>, crate::errors::StarknetWsApiError> {
    if !allowed_statuses.contains(&TxnStatusWithoutL1::AcceptedOnL2) {
        return Ok(None);
    }

    let transactions = block_view
        .get_executed_transactions(..)
        .or_internal_server_error("SubscribeNewTransactions failed to retrieve confirmed block transactions")?;

    if let Some(reorg) = take_pending_reorg(reorgs)? {
        return Ok(Some(reorg));
    }

    for tx in transactions {
        if let Some(reorg) = take_pending_reorg(reorgs)? {
            return Ok(Some(reorg));
        }
        send_executed_transaction(
            sink,
            &tx,
            TxnStatusWithoutL1::AcceptedOnL2,
            sender_address,
            allowed_statuses,
            include_proof_facts,
            emitted,
        )
        .await?;
    }

    Ok(None)
}

async fn send_validated_transaction(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    tx: &ValidatedTransaction,
    status: TxnStatusWithoutL1,
    sender_address: Option<&HashSet<Felt>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
    include_proof_facts: bool,
    emitted: &mut HashSet<(Felt, TxnStatusWithoutL1)>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    if !allowed_statuses.contains(&status)
        || !sender_address.is_none_or(|addresses| addresses.is_empty() || addresses.contains(&tx.contract_address))
        || !mark_emitted(emitted, tx.hash, &status)
    {
        return Ok(());
    }

    send_transaction_item(
        sink,
        TxnWithHashAndStatus {
            transaction: TxnWithHashAndProofFacts {
                transaction: tx.transaction.clone().to_rpc_v0_10_2(include_proof_facts),
                transaction_hash: tx.hash,
            },
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
    include_proof_facts: bool,
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
            transaction: TxnWithHashAndProofFacts {
                transaction: tx.transaction.clone().to_rpc_v0_10_2(include_proof_facts),
                transaction_hash: tx_hash,
            },
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
    if sender_address.is_empty() {
        return true;
    }

    match transaction {
        Transaction::Invoke(inner) => sender_address.contains(inner.sender_address()),
        Transaction::L1Handler(inner) => sender_address.contains(&inner.contract_address),
        Transaction::Declare(inner) => sender_address.contains(inner.sender_address()),
        Transaction::Deploy(inner) => sender_address.contains(&inner.calculate_contract_address()),
        Transaction::DeployAccount(inner) => sender_address.contains(&inner.calculate_contract_address()),
    }
}

fn take_pending_reorg(
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
) -> Result<Option<mc_db::ReorgNotification>, crate::errors::StarknetWsApiError> {
    match reorgs.as_deref_mut() {
        Some(reorgs) => crate::try_recv_live_reorg(reorgs, super::missed_reorg_notifications_error()),
        None => Ok(None),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{
        test_utils::{rpc_test_setup, TestNewTransactionsWatcher},
        versions::user::v0_10_2::{StarknetWsRpcApiV0_10_2Client, StarknetWsRpcApiV0_10_2Server},
        Starknet,
    };
    use assert_matches::assert_matches;
    use jsonrpsee::{
        core::{client::SubscriptionClientT, params::ObjectParams},
        ws_client::WsClientBuilder,
    };
    use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
    use mp_chain_config::StarknetVersion;
    use mp_receipt::{
        ExecutionResources, ExecutionResult, FeePayment, InvokeTransactionReceipt, PriceUnit, TransactionReceipt,
    };
    use mp_transactions::{
        validated::{TxTimestamp, ValidatedTransaction},
        DataAvailabilityMode, InvokeTransaction, InvokeTransactionV0, InvokeTransactionV3, ResourceBoundsMapping,
        Transaction as MpTransaction,
    };
    use serde_json::Value;
    use std::{
        sync::atomic::{AtomicU64, Ordering::Relaxed},
        sync::Arc,
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

    fn invoke_v3_with_receipt(transaction_hash: Felt, proof_facts: Option<Vec<Felt>>) -> TransactionWithReceipt {
        TransactionWithReceipt {
            transaction: MpTransaction::Invoke(InvokeTransaction::V3(InvokeTransactionV3 {
                sender_address: SENDER_ADDRESS,
                calldata: vec![Felt::from_hex_unchecked("0x55")].into(),
                signature: vec![].into(),
                nonce: Felt::ZERO,
                resource_bounds: ResourceBoundsMapping::default(),
                tip: 0,
                paymaster_data: vec![],
                account_deployment_data: vec![],
                nonce_data_availability_mode: DataAvailabilityMode::L1,
                fee_data_availability_mode: DataAvailabilityMode::L1,
                proof_facts,
            })),
            receipt: TransactionReceipt::Invoke(InvokeTransactionReceipt {
                transaction_hash,
                actual_fee: FeePayment { amount: Felt::ONE, unit: PriceUnit::Wei },
                messages_sent: vec![],
                events: vec![],
                execution_resources: ExecutionResources::default(),
                execution_result: ExecutionResult::Succeeded,
            }),
        }
    }

    async fn start_server(starknet: Starknet) -> (jsonrpsee::server::ServerHandle, String) {
        let server = jsonrpsee::server::Server::builder().build(SERVER_ADDR).await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let handle = server.start(StarknetWsRpcApiV0_10_2Server::into_rpc(starknet));
        (handle, server_url)
    }

    async fn raw_subscribe_new_transactions(
        client: &jsonrpsee::ws_client::WsClient,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_10_2_subscribeNewTransactions",
            ObjectParams::new(),
            "starknet_V0_10_2_unsubscribe",
        )
        .await
        .expect("starknet_V0_10_2_subscribeNewTransactions")
    }

    #[tokio::test]
    async fn subscribe_new_transactions_default_finality_emits_confirmed_transactions_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transactions(&client, None, None, None)
            .await
            .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x5151");
        let tx = transaction_with_receipt(SENDER_ADDRESS, transaction_hash);
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
            item.result,
            TxnWithHashAndStatus {
                transaction: TxnWithHashAndProofFacts {
                    transaction: tx.transaction.to_rpc_v0_10_2(false),
                    transaction_hash,
                },
                finality_status: TxnStatusWithoutL1::AcceptedOnL2,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_received_filter_and_sender_v0_10_2() {
        let (_backend, mut starknet) = rpc_test_setup();
        let watcher = TestNewTransactionsWatcher::new();
        starknet.set_new_transactions_watcher(Some(watcher.clone()));

        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transactions(
            &client,
            Some(vec![TxnStatusWithoutL1::Received]),
            Some(vec![SENDER_ADDRESS]),
            None,
        )
        .await
        .expect("Failed subscription");

        let tx_1 = validated_tx(SENDER_ADDRESS);
        let tx_2 = validated_tx(OTHER_SENDER_ADDRESS);

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
                transaction: TxnWithHashAndProofFacts {
                    transaction: tx_1.transaction.to_rpc_v0_10_2(false),
                    transaction_hash: tx_1.hash,
                },
                finality_status: TxnStatusWithoutL1::Received,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_preconfirmed_and_candidate_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transactions(
            &client,
            Some(vec![TxnStatusWithoutL1::PreConfirmed, TxnStatusWithoutL1::Candidate]),
            Some(vec![SENDER_ADDRESS]),
            None,
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
                transaction: TxnWithHashAndProofFacts {
                    transaction: transaction_with_receipt(SENDER_ADDRESS, preconfirmed_hash)
                        .transaction
                        .to_rpc_v0_10_2(false),
                    transaction_hash: preconfirmed_hash,
                },
                finality_status: TxnStatusWithoutL1::PreConfirmed,
            }
        );
        assert_eq!(
            second.result,
            TxnWithHashAndStatus {
                transaction: TxnWithHashAndProofFacts {
                    transaction: candidate.transaction.clone().to_rpc_v0_10_2(false),
                    transaction_hash: candidate.hash,
                },
                finality_status: TxnStatusWithoutL1::Candidate,
            }
        );
    }

    #[tokio::test]
    async fn subscribe_new_transactions_include_proof_facts_tag_emits_v3_facts_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transactions(
            &client,
            None,
            None,
            Some(vec![SubscriptionTag::IncludeProofFacts]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x6161");
        let proof_facts = vec![Felt::from_hex_unchecked("0xabc"), Felt::from_hex_unchecked("0xdef")];
        let tx = invoke_v3_with_receipt(transaction_hash, Some(proof_facts.clone()));
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
                    transactions: vec![tx],
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

        let mp_rpc::v0_10_2::TxnWithProofFacts::Invoke(mp_rpc::v0_10_2::InvokeTxnWithProofFacts::V3(txn)) =
            item.result.transaction.transaction
        else {
            panic!("Expected invoke v3 transaction");
        };
        assert_eq!(txn.proof_facts, Some(proof_facts));
    }

    #[tokio::test]
    async fn subscribe_new_transactions_include_proof_facts_tag_emits_empty_array_when_missing_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_transactions(
            &client,
            None,
            None,
            Some(vec![SubscriptionTag::IncludeProofFacts]),
        )
        .await
        .expect("Failed subscription");

        let transaction_hash = Felt::from_hex_unchecked("0x7171");
        let tx = invoke_v3_with_receipt(transaction_hash, None);
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
                    transactions: vec![tx],
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

        let mp_rpc::v0_10_2::TxnWithProofFacts::Invoke(mp_rpc::v0_10_2::InvokeTxnWithProofFacts::V3(txn)) =
            item.result.transaction.transaction
        else {
            panic!("Expected invoke v3 transaction");
        };
        assert_eq!(txn.proof_facts, Some(vec![]));
    }

    #[tokio::test]
    async fn subscribe_new_transactions_rejects_too_many_sender_addresses_v0_10_2() {
        let (_backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start ws client");

        let size = super::super::ADDRESS_FILTER_LIMIT as usize + 1;
        let err = StarknetWsRpcApiV0_10_2Client::subscribe_new_transactions(
            &client,
            Some(vec![TxnStatusWithoutL1::Received]),
            Some(vec![SENDER_ADDRESS; size]),
            None,
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
    async fn subscribe_new_transactions_reorg_then_resume_v0_10_2() {
        let (backend, starknet) = rpc_test_setup();
        let (_handle, server_url) = start_server(starknet).await;
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
            serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
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
                transaction: TxnWithHashAndProofFacts {
                    transaction: tx.transaction.to_rpc_v0_10_2(false),
                    transaction_hash,
                },
                finality_status: TxnStatusWithoutL1::AcceptedOnL2,
            }
        );
    }
}
