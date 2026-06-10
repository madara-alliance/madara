use crate::errors::ErrorExtWs;

pub async fn subscribe_transaction_status(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    transaction_hash: mp_convert::Felt,
) -> Result<(), crate::errors::StarknetWsApiError> {
    // Resolve the watcher before accepting the subscription, so configurations without a
    // tx-status watcher reject the subscribe call instead of accepting the connection and then
    // closing it with an opaque Internal error.
    let watch =
        starknet.tx_status_watcher.as_ref().and_then(|watcher| watcher.watch_transaction_status(transaction_hash));
    let Some(mut watch) = watch else {
        subscription_sink
            .reject(crate::errors::StarknetWsApiError::internal_server_error(
                "SubscribeTransactionStatus is not available: tx-status watcher is not configured",
            ))
            .await;
        return Ok(());
    };

    let sink = subscription_sink
        .accept()
        .await
        .or_internal_server_error("SubscribeTransactionStatus failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;
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
                    crate::close_ws_subscription(starknet, sink.subscription_id()).await?;
                    return Ok(());
                }
            }
            SubscriptionUpdate::Reorg(reorg) => super::send_reorg_notification(&sink, &reorg).await?,
            SubscriptionUpdate::WatcherClosed => {
                crate::close_ws_subscription(starknet, sink.subscription_id()).await?;
                return Err(crate::errors::StarknetWsApiError::Internal);
            }
        }
    }
}

enum SubscriptionUpdate {
    Snapshot(crate::TxStatusSnapshot),
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
        _ = ctx.cancelled() => Err(crate::errors::StarknetWsApiError::Internal),
        reorg = reorgs.recv() => match reorg {
            Ok(reorg) => Ok(Some(SubscriptionUpdate::Reorg(reorg))),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                Err(super::missed_reorg_notifications_error())
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                Err(crate::errors::StarknetWsApiError::Internal)
            }
        },
        next = watch.recv() => {
            Ok(Some(next.map(SubscriptionUpdate::Snapshot).unwrap_or(SubscriptionUpdate::WatcherClosed)))
        },
    }
}

async fn send_txn_status(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    snapshot: crate::TxStatusSnapshot,
) -> Result<(), crate::errors::StarknetWsApiError> {
    let status = match snapshot {
        crate::TxStatusSnapshot::Received => mp_rpc::v0_9_0::TxnStatus::Received,
        crate::TxStatusSnapshot::Candidate => mp_rpc::v0_9_0::TxnStatus::Candidate,
        crate::TxStatusSnapshot::PreConfirmed => mp_rpc::v0_9_0::TxnStatus::PreConfirmed,
        crate::TxStatusSnapshot::AcceptedOnL2 => mp_rpc::v0_9_0::TxnStatus::AcceptedOnL2,
        crate::TxStatusSnapshot::AcceptedOnL1 => mp_rpc::v0_9_0::TxnStatus::AcceptedOnL1,
    };

    let item = super::SubscriptionItem::new(sink.subscription_id(), status);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_else_internal_server_error(|| "SubscribeTransactionStatus failed to create response".to_owned())?;

    sink.send(msg).await.or_internal_server_error("SubscribeTransactionStatus failed to respond to websocket request")
}

#[cfg(test)]
mod test {
    use crate::{
        test_utils::{TestTransactionProvider, TestTxStatusWatcher},
        versions::user::v0_9_0::{
            methods::ws::SubscriptionItem, StarknetWsRpcApiV0_9_0Client, StarknetWsRpcApiV0_9_0Server,
        },
        Starknet,
    };
    use assert_matches::assert_matches;
    use jsonrpsee::{
        core::{client::SubscriptionClientT, params::ObjectParams},
        ws_client::WsClientBuilder,
    };
    use mp_chain_config::ChainConfig;
    use mp_utils::service::ServiceContext;
    use serde_json::Value;
    use std::{sync::Arc, time::Duration};

    const SERVER_ADDR: &str = "127.0.0.1:0";
    const TX_HASH: starknet_types_core::felt::Felt = starknet_types_core::felt::Felt::from_hex_unchecked(
        "0x3ccaabf599097d1965e1ef8317b830e76eb681016722c9364ed6e59f3252908",
    );

    fn starknet_with_status_watcher() -> (Arc<mc_db::MadaraBackend>, Starknet, Arc<TestTxStatusWatcher>) {
        let chain_config = Arc::new(ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let backend_for_rpc = backend.clone();
        let watcher = TestTxStatusWatcher::new();
        let provider = Arc::new(TestTransactionProvider);
        let mut starknet = Starknet::new(
            backend_for_rpc,
            Arc::clone(&provider) as _,
            provider,
            Default::default(),
            None,
            ServiceContext::new_for_testing(),
        );
        starknet.set_tx_status_watcher(Some(watcher.clone()));
        (backend, starknet, watcher)
    }

    fn add_empty_block(backend: &Arc<mc_db::MadaraBackend>, block_number: u64) -> starknet_types_core::felt::Felt {
        backend
            .write_access()
            .add_full_block_with_classes(
                &mp_block::FullBlockWithoutCommitments {
                    header: mp_block::PreconfirmedHeader { block_number, ..Default::default() },
                    state_diff: Default::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .expect("Storing block")
            .block_hash
    }

    async fn raw_subscribe_transaction_status(
        client: &jsonrpsee::ws_client::WsClient,
        transaction_hash: starknet_types_core::felt::Felt,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        params.insert("transaction_hash", transaction_hash).expect("Building subscribeTransactionStatus params");
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_9_0_subscribeTransactionStatus",
            params,
            "starknet_V0_9_0_unsubscribe",
        )
        .await
        .expect("starknet_V0_9_0_subscribeTransactionStatus")
    }

    #[tokio::test]
    async fn subscribe_transaction_status_received_before() {
        let (_backend, starknet, watcher) = starknet_with_status_watcher();
        watcher.set_status(Some(crate::TxStatusSnapshot::Received));

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status"),
            Some(Ok(SubscriptionItem { result: status, .. })) => {
                assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::Received);
            }
        );
    }

    #[tokio::test]
    async fn subscribe_transaction_status_full_flow() {
        let (_backend, starknet, watcher) = starknet_with_status_watcher();

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");

        watcher.set_status(Some(crate::TxStatusSnapshot::Received));
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status"),
            Some(Ok(SubscriptionItem { result: status, .. })) => {
                assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::Received);
            }
        );

        watcher.set_status(Some(crate::TxStatusSnapshot::Candidate));
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status"),
            Some(Ok(SubscriptionItem { result: status, .. })) => {
                assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::Candidate);
            }
        );

        watcher.set_status(Some(crate::TxStatusSnapshot::PreConfirmed));
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status"),
            Some(Ok(SubscriptionItem { result: status, .. })) => {
                assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::PreConfirmed);
            }
        );

        watcher.set_status(Some(crate::TxStatusSnapshot::AcceptedOnL2));
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status"),
            Some(Ok(SubscriptionItem { result: status, .. })) => {
                assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::AcceptedOnL2);
            }
        );

        watcher.set_status(Some(crate::TxStatusSnapshot::AcceptedOnL1));
        assert_matches!(
            tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status"),
            Some(Ok(SubscriptionItem { result: status, .. })) => {
                assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::AcceptedOnL1);
            }
        );
    }

    #[tokio::test]
    async fn subscribe_transaction_status_unsubscribe() {
        let (_backend, starknet, watcher) = starknet_with_status_watcher();

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");
        watcher.set_status(Some(crate::TxStatusSnapshot::Received));

        let subscription_id =
            match tokio::time::timeout(Duration::from_secs(5), sub.next()).await.expect("Timed out waiting for status")
            {
                Some(Ok(SubscriptionItem { subscription_id, result: status })) => {
                    assert_eq!(status, mp_rpc::v0_9_0::TxnStatus::Received);
                    subscription_id
                }
                other => panic!("Unexpected subscription result: {other:?}"),
            };

        client.starknet_unsubscribe(subscription_id).await.expect("Failed to close subscription");
        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    async fn subscribe_transaction_status_reorg_notification() {
        let (backend, starknet, _watcher) = starknet_with_status_watcher();
        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        let block_0_hash = add_empty_block(&backend, 0);
        let block_1_hash = add_empty_block(&backend, 1);

        let mut sub = raw_subscribe_transaction_status(&client, TX_HASH).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_9_0::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_1_hash,
                ending_block_number: 1,
            })
            .expect("Failed to serialize expected reorg notification")
        );
    }

    #[tokio::test]
    async fn subscribe_transaction_status_watcher_close_ends_subscription() {
        let (_backend, starknet, watcher) = starknet_with_status_watcher();

        let builder = jsonrpsee::server::Server::builder();
        let server = builder.build(SERVER_ADDR).await.expect("Failed to start jsonrpsee server");
        let server_url = format!("ws://{}", server.local_addr().expect("Failed to retrieve server local addr"));
        let _server_handle = server.start(StarknetWsRpcApiV0_9_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Failed to start jsonrpsee ws client");

        let mut sub = client.subscribe_transaction_status(TX_HASH).await.expect("Failed subscription");
        watcher.close();

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for watcher-close stream termination");
        assert!(next.is_none());
    }
}
