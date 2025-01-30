use crate::{
    errors::{ErrorExtWs, StarknetWsApiError},
    utils::get_filtered_pending_tx_with_hash,
};
use mp_transactions::TransactionWithHash;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{Txn, TxnHash};

pub async fn subscribe_pending_transaction(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    sender_addresses: Option<Vec<Felt>>,
    transaction_details: bool,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;

    let mut rx = starknet.backend.subscribe_pending_transaction();

    let latest_pending_block =
        starknet.backend.get_pending_block_inner().or_internal_server_error("Failed to retrieve latest block")?;

    let txs_with_hash = get_filtered_pending_tx_with_hash(latest_pending_block, &sender_addresses);

    // Send current pending block transactions
    for tx_with_hash in txs_with_hash {
        send_pending_transactions(&tx_with_hash, transaction_details, &sink).await?;
    }

    // New pending block transactions
    loop {
        tokio::select! {
            latest_pending_block = rx.recv() => {
                let latest_pending_block = latest_pending_block.or_internal_server_error("Failed to retrieve block info")?;
                let txs_with_hash = get_filtered_pending_tx_with_hash(latest_pending_block, &sender_addresses);
                for tx_with_hash in txs_with_hash {
                    send_pending_transactions(&tx_with_hash, transaction_details, &sink).await?;
                }

            },
            _ = sink.closed() => {
                return Ok(())
            }
        }
    }
}

pub async fn send_pending_transactions(
    tx_with_hash: &TransactionWithHash,
    transaction_details: bool,
    sink: &jsonrpsee::SubscriptionSink,
) -> Result<(), StarknetWsApiError> {
    if transaction_details {
        let tx_message = Txn::from(tx_with_hash.transaction.clone());
        let msg = jsonrpsee::SubscriptionMessage::from_json(&tx_message)
            .or_internal_server_error("Failed to create response message")?;
        sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")?;
    }
    let tx_message: Felt = TxnHash::from(tx_with_hash.hash);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&tx_message)
        .or_internal_server_error("Failed to create response message")?;
    sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")?;
    Ok(())
}
