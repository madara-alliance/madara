use std::collections::HashSet;

use crate::errors::ErrorExtWs;
use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_10_0::{FinalityStatus, TxnFinalityStatus, TxnReceiptWithBlockInfo};
use mp_transactions::Transaction;
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
    let mut reorgs = starknet.backend.subscribe_reorgs();

    loop {
        let block_view = tokio::select! {
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
            reorg = reorgs.recv() => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                        block_stream.set_start_from(reorg.first_reverted_block_n);
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
            block_view = block_stream.next_block_view() => block_view,
        };

        if block_view.is_confirmed() {
            let block_number = block_view.block_number();
            match crate::resolve_live_confirmed_head(
                &starknet.backend,
                &mut reorgs,
                block_number,
                super::missed_reorg_notifications_error(),
            )? {
                crate::LiveConfirmedHeadResolution::Block(_) => {}
                crate::LiveConfirmedHeadResolution::Reorg(reorg) => {
                    super::send_reorg_notification(&sink, &reorg).await?;
                    block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                    block_stream.set_start_from(reorg.first_reverted_block_n);
                    continue;
                }
                crate::LiveConfirmedHeadResolution::RetryBackfill => {
                    block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
                    block_stream.set_start_from(block_number);
                    continue;
                }
            }
        }

        let mut live_reorgs = block_view.is_confirmed().then_some(&mut reorgs);
        if let Some(reorg) =
            send_block_receipts(&sink, &allowed_finality_status, sender_address.as_ref(), &mut live_reorgs, block_view)
                .await?
        {
            super::send_reorg_notification(&sink, &reorg).await?;
            block_stream = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Preconfirmed);
            block_stream.set_start_from(reorg.first_reverted_block_n);
            continue;
        }
    }
}

async fn send_block_receipts(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    allowed_finality_status: &HashSet<FinalityStatus>,
    sender_address: Option<&HashSet<Felt>>,
    reorgs: &mut Option<&mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>>,
    block_view: mc_db::MadaraBlockView,
) -> Result<Option<mc_db::ReorgNotification>, crate::errors::StarknetWsApiError> {
    let (finality_status, block_hash) = match block_view.as_confirmed() {
        Some(confirmed) if confirmed.is_on_l1() => return Ok(None),
        Some(confirmed) => (
            FinalityStatus::AcceptedOnL2,
            Some(
                confirmed
                    .get_block_info()
                    .or_internal_server_error(
                        "SubscribeNewTransactionReceipts failed to retrieve confirmed block info",
                    )?
                    .block_hash,
            ),
        ),
        None => (FinalityStatus::PreConfirmed, None),
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
        if !transaction_matches_sender(&tx.transaction, sender_address) {
            continue;
        }

        let tx_hash = *tx.receipt.transaction_hash();
        let transaction_receipt = tx.receipt.to_rpc_v0_10(match finality_status {
            FinalityStatus::PreConfirmed => TxnFinalityStatus::PreConfirmed,
            FinalityStatus::AcceptedOnL2 => TxnFinalityStatus::L2,
        });
        let item = super::SubscriptionItem::new(
            sink.subscription_id(),
            TxnReceiptWithBlockInfo { transaction_receipt, block_hash, block_number },
        );
        let msg = jsonrpsee::SubscriptionMessage::from_json(&item).or_else_internal_server_error(|| {
            format!("SubscribeNewTransactionReceipts failed to create response for tx hash {tx_hash:#x}")
        })?;

        sink.send(msg)
            .await
            .or_internal_server_error("SubscribeNewTransactionReceipts failed to respond to websocket request")?;
    }

    Ok(None)
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
