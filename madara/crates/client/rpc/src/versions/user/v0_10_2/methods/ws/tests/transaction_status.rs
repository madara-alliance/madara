use super::*;

#[tokio::test]
async fn subscribe_transaction_status_received_before_v0_10_2() {
    let (_backend, mut starknet) = rpc_test_setup();
    let watcher = TestTxStatusWatcher::new();
    watcher.set_status(Some(crate::TxStatusSnapshot::Received));
    starknet.set_tx_status_watcher(Some(watcher));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for status")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve status");

    assert_eq!(item, expected_txn_status(mp_rpc::v0_10_2::TxnStatus::Received));
}

#[tokio::test]
async fn subscribe_transaction_status_full_flow_v0_10_2() {
    let (_backend, mut starknet) = rpc_test_setup();
    let watcher = TestTxStatusWatcher::new();
    starknet.set_tx_status_watcher(Some(watcher.clone()));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");

    for (snapshot, expected) in [
        (crate::TxStatusSnapshot::Received, mp_rpc::v0_10_2::TxnStatus::Received),
        (crate::TxStatusSnapshot::Candidate, mp_rpc::v0_10_2::TxnStatus::Candidate),
        (crate::TxStatusSnapshot::PreConfirmed, mp_rpc::v0_10_2::TxnStatus::PreConfirmed),
        (crate::TxStatusSnapshot::AcceptedOnL2, mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2),
        (crate::TxStatusSnapshot::AcceptedOnL1, mp_rpc::v0_10_2::TxnStatus::AcceptedOnL1),
    ] {
        watcher.set_status(Some(snapshot));
        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for status")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve status");
        assert_eq!(item, expected_txn_status(expected));
    }
}

#[tokio::test]
async fn subscribe_transaction_status_includes_succeeded_execution_v0_10_2() {
    let (backend, mut starknet) = rpc_test_setup();
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![transaction_with_receipt(SENDER_ADDRESS, TX_HASH)],
                events: vec![],
            },
            &[],
            true,
        )
        .expect("Failed to store confirmed block");
    let watcher = TestTxStatusWatcher::new();
    watcher.set_status(Some(crate::TxStatusSnapshot::AcceptedOnL2));
    starknet.set_tx_status_watcher(Some(watcher));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");
    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for status")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve status");

    assert_eq!(
        item,
        expected_txn_status_with_execution(
            mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2,
            Some(mp_rpc::v0_10_2::TxnExecutionStatus::Succeeded),
            None,
        )
    );
}

#[tokio::test]
async fn subscribe_transaction_status_includes_reverted_execution_v0_10_2() {
    let (backend, mut starknet) = rpc_test_setup();
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                state_diff: Default::default(),
                transactions: vec![transaction_with_receipt_and_execution(
                    SENDER_ADDRESS,
                    TX_HASH,
                    ExecutionResult::Reverted { reason: "boom".into() },
                )],
                events: vec![],
            },
            &[],
            true,
        )
        .expect("Failed to store confirmed block");
    let watcher = TestTxStatusWatcher::new();
    watcher.set_status(Some(crate::TxStatusSnapshot::AcceptedOnL2));
    starknet.set_tx_status_watcher(Some(watcher));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");
    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for status")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve status");

    assert_eq!(
        item,
        expected_txn_status_with_execution(
            mp_rpc::v0_10_2::TxnStatus::AcceptedOnL2,
            Some(mp_rpc::v0_10_2::TxnExecutionStatus::Reverted),
            Some("boom".into()),
        )
    );
}

#[tokio::test]
async fn subscribe_transaction_status_none_status_keeps_subscription_open_v0_10_2() {
    let (_backend, mut starknet) = rpc_test_setup();
    let watcher = TestTxStatusWatcher::new();
    starknet.set_tx_status_watcher(Some(watcher.clone()));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");

    watcher.set_status(None);
    assert!(tokio::time::timeout(Duration::from_millis(100), sub.next()).await.is_err());

    watcher.set_status(Some(crate::TxStatusSnapshot::Received));
    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for status")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve status");

    assert_eq!(item, expected_txn_status(mp_rpc::v0_10_2::TxnStatus::Received));
}

#[tokio::test]
async fn subscribe_transaction_status_cleanup_on_drop_v0_10_2() {
    let (_backend, mut starknet) = rpc_test_setup();
    let watcher = TestTxStatusWatcher::new();
    starknet.set_tx_status_watcher(Some(watcher.clone()));
    let starknet_for_assert = starknet.clone();

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");
    watcher.set_status(Some(crate::TxStatusSnapshot::Received));

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for status")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve status");
    assert_eq!(item, expected_txn_status(mp_rpc::v0_10_2::TxnStatus::Received));

    drop(sub);
    drop(client);
    wait_for_active_subscriptions(&starknet_for_assert, 0).await;
}

#[tokio::test]
async fn subscribe_transaction_status_reorg_notification_v0_10_2() {
    let (backend, mut starknet) = rpc_test_setup();
    let watcher = TestTxStatusWatcher::new();
    starknet.set_tx_status_watcher(Some(watcher));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
    let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);

    let mut sub = raw_subscribe_transaction_status(&client, TX_HASH).await;

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
}

#[tokio::test]
async fn subscribe_transaction_status_watcher_close_does_not_emit_error_v0_10_2() {
    let (_backend, mut starknet) = rpc_test_setup();
    let watcher = TestTxStatusWatcher::new();
    starknet.set_tx_status_watcher(Some(watcher.clone()));

    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_transaction_status(&client, TX_HASH)
        .await
        .expect("Failed subscription");
    watcher.close();

    assert!(tokio::time::timeout(Duration::from_millis(100), sub.next()).await.is_err());
}
