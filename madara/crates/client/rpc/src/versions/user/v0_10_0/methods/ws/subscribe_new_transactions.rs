use crate::errors::ErrorExtWs;
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_10_0::TxnStatusWithoutL1;
use mp_rpc::v0_9_0::{TxnWithHash, TxnWithHashAndStatus};
use mp_transactions::validated::ValidatedTransaction;
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
    let sender_address = crate::normalize_sender_address_filter(sender_address);
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
            _ = ctx.cancelled() => return Ok(()),
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
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
    allowed_statuses: &HashSet<TxnStatusWithoutL1>,
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
        || !crate::transaction_matches_sender(&tx.transaction, sender_address)
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

fn take_pending_reorg(
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
) -> Result<Option<mc_db::ReorgNotification>, crate::errors::StarknetWsApiError> {
    match reorgs.as_deref_mut() {
        Some(reorgs) => crate::try_recv_live_reorg(reorgs, super::missed_reorg_notifications_error()),
        None => Ok(None),
    }
}
