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
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let allowed_finality_status =
        finality_status.unwrap_or_else(|| vec![FinalityStatus::AcceptedOnL2]).into_iter().collect::<HashSet<_>>();
    let sender_address = sender_address.map(|addresses| addresses.into_iter().collect::<HashSet<_>>());

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
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            },
            block_view = block_stream.next_block_view() => block_view,
        };

        send_block_receipts(&sink, &allowed_finality_status, sender_address.as_ref(), block_view).await?;
    }
}

async fn send_block_receipts(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    allowed_finality_status: &HashSet<FinalityStatus>,
    sender_address: Option<&HashSet<Felt>>,
    block_view: mc_db::MadaraBlockView,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let (finality_status, block_hash) = match block_view.as_confirmed() {
        Some(confirmed) if confirmed.is_on_l1() => return Ok(()),
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
        return Ok(());
    }

    let block_number = block_view.block_number();
    for tx in block_view
        .get_executed_transactions(..)
        .or_internal_server_error("SubscribeNewTransactionReceipts failed to retrieve block transactions")?
    {
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

    Ok(())
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
