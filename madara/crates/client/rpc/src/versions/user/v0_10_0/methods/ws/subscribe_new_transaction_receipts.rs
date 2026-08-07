use std::collections::HashSet;

use crate::errors::ErrorExtWs;
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_10_0::{FinalityStatus, TxnFinalityStatus, TxnReceiptWithBlockInfo};
use starknet_types_core::felt::Felt;

pub async fn subscribe_new_transaction_receipts_with_reorg(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    finality_status: Option<Vec<FinalityStatus>>,
    sender_address: Option<Vec<Felt>>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    if sender_address.as_ref().map_or(0, Vec::len) as u64 > super::ADDRESS_FILTER_LIMIT {
        subscription_sink.reject(crate::errors::StarknetWsApiError::TooManyAddressesInFilter).await;
        return Ok(());
    }

    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let allowed_finality_status =
        finality_status.unwrap_or_else(|| vec![FinalityStatus::AcceptedOnL2]).into_iter().collect::<HashSet<_>>();
    let sender_address = crate::normalize_sender_address_filter(sender_address);

    let mut block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
    let mut current_preconfirmed = starknet
        .backend
        .block_view_on_preconfirmed_or_fake()
        .or_internal_server_error("SubscribeNewTransactionReceipts failed to create preconfirmed block view")?;
    let mut reorgs = starknet.backend.subscribe_reorgs();
    let mut emitted = HashSet::<(Felt, FinalityStatus)>::new();

    loop {
        let block_view = tokio::select! {
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::SubscriptionClosed),
            reorg = reorgs.recv() => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        emitted.clear();
                        block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                        block_stream.set_start_from(reorg.first_reverted_block_n);
                        current_preconfirmed = starknet
                            .backend
                            .block_view_on_preconfirmed_or_fake()
                            .or_internal_server_error("SubscribeNewTransactionReceipts failed to refresh preconfirmed block view after reorg")?;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        return Err(super::missed_reorg_notifications_error());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            },
            _ = current_preconfirmed.wait_until_outdated() => {
                current_preconfirmed.refresh();
                if let Some(reorg) = send_block_receipts(
                    &sink,
                    &allowed_finality_status,
                    sender_address.as_ref(),
                    &mut reorgs,
                    current_preconfirmed.clone().into(),
                    &mut emitted,
                ).await? {
                    super::send_reorg_notification(&sink, &reorg).await?;
                    emitted.clear();
                    block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                    block_stream.set_start_from(reorg.first_reverted_block_n);
                    current_preconfirmed = starknet
                        .backend
                        .block_view_on_preconfirmed_or_fake()
                        .or_internal_server_error("SubscribeNewTransactionReceipts failed to refresh preconfirmed block view after reorg")?;
                }
                continue;
            },
            block_view = block_stream.next_block_view() => block_view,
        };

        if block_view.is_confirmed() {
            let block_number = block_view.block_number();
            match crate::resolve_live_confirmed_head(
                &starknet.backend,
                &mut reorgs,
                block_number,
                super::missed_reorg_notifications_error,
            )? {
                crate::LiveConfirmedHeadResolution::Block(_) => {}
                crate::LiveConfirmedHeadResolution::Reorg(reorg) => {
                    super::send_reorg_notification(&sink, &reorg).await?;
                    emitted.clear();
                    block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                    block_stream.set_start_from(reorg.first_reverted_block_n);
                    current_preconfirmed =
                        starknet.backend.block_view_on_preconfirmed_or_fake().or_internal_server_error(
                            "SubscribeNewTransactionReceipts failed to refresh preconfirmed block view after reorg",
                        )?;
                    continue;
                }
                crate::LiveConfirmedHeadResolution::RetryBackfill => {
                    block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                    block_stream.set_start_from(block_number);
                    continue;
                }
            }
        }

        if let Some(reorg) = send_block_receipts(
            &sink,
            &allowed_finality_status,
            sender_address.as_ref(),
            &mut reorgs,
            block_view.clone(),
            &mut emitted,
        )
        .await?
        {
            super::send_reorg_notification(&sink, &reorg).await?;
            emitted.clear();
            block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
            block_stream.set_start_from(reorg.first_reverted_block_n);
            current_preconfirmed = starknet.backend.block_view_on_preconfirmed_or_fake().or_internal_server_error(
                "SubscribeNewTransactionReceipts failed to refresh preconfirmed block view after reorg",
            )?;
            continue;
        }

        if let Some(mut preconfirmed) = block_view.into_preconfirmed() {
            preconfirmed.refresh();
            current_preconfirmed = preconfirmed;
        } else {
            current_preconfirmed = starknet.backend.block_view_on_preconfirmed_or_fake().or_internal_server_error(
                "SubscribeNewTransactionReceipts failed to refresh preconfirmed block view",
            )?;
        }
    }
}

async fn send_block_receipts(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    allowed_finality_status: &HashSet<FinalityStatus>,
    sender_address: Option<&HashSet<Felt>>,
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
    block_view: mc_db::MadaraBlockView,
    emitted: &mut HashSet<(Felt, FinalityStatus)>,
) -> Result<Option<mc_db::ReorgNotification>, crate::errors::StarknetWsApiError> {
    let (finality_status, receipt_finality_status, block_hash) = match block_view.as_confirmed() {
        Some(confirmed) => {
            let block_hash = confirmed
                .get_block_info()
                .or_internal_server_error("SubscribeNewTransactionReceipts failed to retrieve confirmed block info")?
                .block_hash;
            let receipt_finality_status =
                if confirmed.is_on_l1() { TxnFinalityStatus::L1 } else { TxnFinalityStatus::L2 };

            (FinalityStatus::AcceptedOnL2, receipt_finality_status, Some(block_hash))
        }
        None => (FinalityStatus::PreConfirmed, TxnFinalityStatus::PreConfirmed, None),
    };

    if !allowed_finality_status.contains(&finality_status) {
        return Ok(None);
    }

    let block_number = block_view.block_number();
    let transactions = block_view
        .get_executed_transactions(..)
        .or_internal_server_error("SubscribeNewTransactionReceipts failed to retrieve block transactions")?;

    if let Some(reorg) = take_pending_reorg(reorgs)? {
        return Ok(Some(reorg));
    }

    for tx in transactions {
        if let Some(reorg) = take_pending_reorg(reorgs)? {
            return Ok(Some(reorg));
        }
        if !crate::transaction_matches_sender(&tx.transaction, sender_address) {
            continue;
        }

        let tx_hash = *tx.receipt.transaction_hash();
        if !emitted.insert((tx_hash, finality_status.clone())) {
            continue;
        }

        let item = TxnReceiptWithBlockInfo {
            transaction_receipt: tx.receipt.to_rpc_v0_10(receipt_finality_status),
            block_hash,
            block_number,
        };
        crate::versions::user::v0_10_2::methods::ws::send_starknet_subscription(
            sink,
            super::NEW_TRANSACTION_RECEIPTS_NOTIFICATION_METHOD,
            &item,
        )
        .await?;
    }

    Ok(None)
}

fn take_pending_reorg(
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
) -> Result<Option<mc_db::ReorgNotification>, crate::errors::StarknetWsApiError> {
    crate::try_recv_live_reorg(reorgs, super::missed_reorg_notifications_error)
}
