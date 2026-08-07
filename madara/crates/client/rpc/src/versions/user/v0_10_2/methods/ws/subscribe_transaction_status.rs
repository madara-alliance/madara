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
    let ctx = starknet
        .ws_handles
        .subscription_register(sink.subscription_id(), crate::metrics::SUBSCRIBE_TRANSACTION_STATUS)
        .await;

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

                send_txn_status(&sink, transaction_hash, snapshot).await?;
                if matches!(snapshot, crate::TxStatusSnapshot::AcceptedOnL1) {
                    crate::close_ws_subscription(
                        starknet,
                        sink.subscription_id(),
                        "SubscribeTransactionStatus failed to parse string subscription id",
                    )
                    .await?;
                    return Ok(());
                }
            }
            SubscriptionUpdate::Reorg(reorg) => super::send_reorg_notification(&sink, &reorg).await?,
            SubscriptionUpdate::NoStatus => {}
            SubscriptionUpdate::WatcherClosed => {
                crate::close_ws_subscription(
                    starknet,
                    sink.subscription_id(),
                    "SubscribeTransactionStatus failed to parse string subscription id",
                )
                .await?;
                return Ok(());
            }
        }
    }
}

enum SubscriptionUpdate {
    Snapshot(crate::TxStatusSnapshot),
    NoStatus,
    Reorg(mc_db::ReorgNotification),
    WatcherClosed,
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

    tokio::select! {
        _ = sink.closed() => Ok(None),
        _ = ctx.cancelled() => Err(crate::errors::StarknetWsApiError::SubscriptionClosed),
        reorg = reorgs.recv() => match reorg {
            Ok(reorg) => Ok(Some(SubscriptionUpdate::Reorg(reorg))),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                crate::metrics::record_lagged_reorg(crate::metrics::SUBSCRIBE_TRANSACTION_STATUS);
                Err(super::missed_reorg_notifications_error())
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                Err(crate::errors::StarknetWsApiError::Internal)
            }
        },
        next = watch.recv() => {
            Ok(Some(match next {
                crate::TxStatusWatchUpdate::Status(Some(snapshot)) => SubscriptionUpdate::Snapshot(snapshot),
                crate::TxStatusWatchUpdate::Status(None) => SubscriptionUpdate::NoStatus,
                crate::TxStatusWatchUpdate::Closed => SubscriptionUpdate::WatcherClosed,
            }))
        },
    }
}

async fn send_txn_status(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    transaction_hash: mp_convert::Felt,
    snapshot: crate::TxStatusSnapshot,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let finality_status = match snapshot {
        crate::TxStatusSnapshot::Received => mp_rpc::v0_10_2::TxnStatus::Received,
        crate::TxStatusSnapshot::Candidate => mp_rpc::v0_10_2::TxnStatus::Candidate,
        crate::TxStatusSnapshot::PreConfirmed => mp_rpc::v0_10_2::TxnStatus::PreConfirmed,
        crate::TxStatusSnapshot::AcceptedOnL2 => mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2,
        crate::TxStatusSnapshot::AcceptedOnL1 => mp_rpc::v0_10_2::TxnStatus::AcceptedOnL1,
    };
    let status = mp_rpc::v0_10_2::WsTxnStatusResult { execution_status: None, finality_status, failure_reason: None };
    let item = mp_rpc::v0_10_2::NewTxnStatus { transaction_hash, status };
    super::send_starknet_subscription(sink, super::TRANSACTION_STATUS_NOTIFICATION_METHOD, &item).await
}
