use crate::errors::{ErrorExtWs, OptionExtWs};

#[cfg(test)]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(not(test))]
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300); // 5min

pub async fn subscribe_pending_transactions(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_details: bool,
    sender_address: std::collections::HashSet<starknet_types_core::felt::Felt>,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let mut channel = starknet.backend.subscribe_pending_block();

    loop {
        tokio::time::timeout(TIMEOUT, async {
            channel.changed().await.or_internal_server_error("Error waiting for watch channel update")?;
            let pending_info = channel.borrow_and_update();
            let mut pending_txs = pending_info.tx_hashes.iter().peekable();
            let (block, _) = match pending_txs.peek() {
                Some(tx_hash) => starknet
                    .backend
                    .find_tx_hash_block(tx_hash)
                    .or_else_internal_server_error(|| {
                        format!("SubscribePendingTransactions failed to retrieve block at tx {tx_hash:#x}")
                    })?
                    .ok_or_else_internal_server_error(|| {
                        format!("SubscribePendingTransactions failed to retrieve block at tx {tx_hash:#x}")
                    })?,
                None => return Ok(()),
            };

            for (tx, hash) in block.inner.transactions.into_iter().zip(pending_txs) {
                let tx = match tx {
                    mp_transactions::Transaction::Invoke(ref inner)
                        if sender_address.contains(inner.sender_address()) =>
                    {
                        tx
                    }
                    mp_transactions::Transaction::Declare(ref inner)
                        if sender_address.contains(inner.sender_address()) =>
                    {
                        tx
                    }
                    mp_transactions::Transaction::DeployAccount(ref inner)
                        if sender_address.contains(inner.sender_address()) =>
                    {
                        tx
                    }
                    _ => continue,
                };

                let tx_info = if transaction_details {
                    mp_rpc::v0_8_1::PendingTxnInfo::Full(tx.into())
                } else {
                    mp_rpc::v0_8_1::PendingTxnInfo::Hash(*hash)
                };

                let msg = jsonrpsee::SubscriptionMessage::from_json(&tx_info).or_else_internal_server_error(|| {
                    format!("SubscribePendingTransactions failed to create response message at tx {hash:#x}")
                })?;

                sink.send(msg).await.or_else_internal_server_error(|| {
                    format!("SubscribePendingTransactions failed to respond to websocket request at tx {hash:#x}")
                })?;
            }

            Ok(())
        })
        .await
        .or_internal_server_error("SubscribePendingTransactions timed out")??;
    }
}
