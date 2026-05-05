use crate::errors::ErrorExtWs;

pub async fn subscribe_transaction_status(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_hash: mp_convert::Felt,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let sink = subscription_sink
        .accept()
        .await
        .or_internal_server_error("SubscribeTransactionStatus failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let mut watch = starknet
        .tx_status_watcher
        .as_ref()
        .ok_or_else(|| {
            crate::errors::StarknetWsApiError::internal_server_error(
                "SubscribeTransactionStatus failed: tx-status watcher is not configured",
            )
        })?
        .watch_transaction_status(transaction_hash)
        .ok_or_else(|| {
            crate::errors::StarknetWsApiError::internal_server_error(
                "SubscribeTransactionStatus failed to create transaction status watcher",
            )
        })?;
    let mut reorgs = starknet.backend.subscribe_reorgs();

    let mut allow_current = true;
    loop {
        let Some(update) = next_update(&sink, &ctx, &mut watch, &mut reorgs, allow_current).await? else {
            return Ok(());
        };
        match update {
            SubscriptionUpdate::Snapshot(snapshot) => {
                allow_current = false;

                send_txn_status(&sink, snapshot).await?;
                if matches!(snapshot, crate::TxStatusSnapshot::AcceptedOnL1) {
                    let subscription_id = match sink.subscription_id() {
                        jsonrpsee::types::SubscriptionId::Num(id) => id,
                        jsonrpsee::types::SubscriptionId::Str(id) => {
                            id.parse().expect("string subscription ids should remain numeric internally")
                        }
                    };
                    let _ = starknet.ws_handles.subscription_close(subscription_id).await;
                    return Ok(());
                }
            }
            SubscriptionUpdate::Reorg(reorg) => super::send_reorg_notification(&sink, &reorg).await?,
        }
    }
}

enum SubscriptionUpdate {
    Snapshot(crate::TxStatusSnapshot),
    Reorg(mc_db::ReorgNotification),
}

async fn next_update(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    ctx: &crate::WsSubscriptionGuard,
    watch: &mut Box<dyn crate::TxStatusWatch + Send>,
    reorgs: &mut mc_db::subscription::SubscribeReorgs<mc_db::rocksdb::RocksDBStorage>,
    allow_current: bool,
) -> Result<Option<SubscriptionUpdate>, crate::errors::StarknetWsApiError> {
    if allow_current {
        if let Some(snapshot) = watch.take_current() {
            return Ok(Some(SubscriptionUpdate::Snapshot(snapshot)));
        }
    }

    loop {
        tokio::select! {
            _ = sink.closed() => return Ok(None),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
            reorg = reorgs.recv() => match reorg {
                Ok(reorg) => return Ok(Some(SubscriptionUpdate::Reorg(reorg))),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    return Err(super::missed_reorg_notifications_error());
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(crate::errors::StarknetWsApiError::Internal);
                }
            },
            next = watch.recv() => {
                if let Some(snapshot) = next {
                    return Ok(Some(SubscriptionUpdate::Snapshot(snapshot)));
                }
            },
        }
    }
}

async fn send_txn_status(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    snapshot: crate::TxStatusSnapshot,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let status = match snapshot {
        crate::TxStatusSnapshot::Received => mp_rpc::v0_10_2::TxnStatus::Received,
        crate::TxStatusSnapshot::Candidate => mp_rpc::v0_10_2::TxnStatus::Candidate,
        crate::TxStatusSnapshot::PreConfirmed => mp_rpc::v0_10_2::TxnStatus::PreConfirmed,
        crate::TxStatusSnapshot::AcceptedOnL2 => mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2,
        crate::TxStatusSnapshot::AcceptedOnL1 => mp_rpc::v0_10_2::TxnStatus::AcceptedOnL1,
    };

    let item = super::SubscriptionItem::new(sink.subscription_id(), status);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_else_internal_server_error(|| "SubscribeTransactionStatus failed to create response".to_owned())?;

    sink.send(msg).await.or_internal_server_error("SubscribeTransactionStatus failed to respond to websocket request")
}
