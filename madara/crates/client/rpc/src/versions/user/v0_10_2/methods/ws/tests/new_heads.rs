use super::*;

#[tokio::test]
async fn subscribe_new_heads_defaults_to_latest_when_block_id_missing_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let expected = add_block_at(&backend, 0);
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub =
        StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, None).await.expect("starknet_subscribeNewHeads");

    let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for block header")
        .expect("Waiting for block header")
        .expect("Waiting for block header");

    assert_eq!(next, expected);
}

#[tokio::test]
async fn subscribe_new_heads_defaults_to_latest_when_block_id_missing_v0_10_0() {
    let (backend, starknet) = rpc_test_setup();
    let expected = add_block_at(&backend, 0);
    let (_handle, server_url) = start_server_v0_10_0(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub =
        StarknetWsRpcApiV0_10_0Client::subscribe_new_heads(&client, None).await.expect("starknet_subscribeNewHeads");

    let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for block header")
        .expect("Waiting for block header")
        .expect("Waiting for block header");

    let item = serde_json::to_value(next).expect("Serializing v0.10.0 header item");
    assert_eq!(item, serde_json::to_value(expected).expect("Serializing expected header"));
}

#[tokio::test]
async fn subscribe_new_heads_future_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Number(1)))
        .await
        .expect("starknet_subscribeNewHeads");

    let expected = add_block_at(&backend, 1);

    let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for future block header")
        .expect("Waiting for block header")
        .expect("Waiting for block header");

    assert_eq!(next, expected);
}

#[tokio::test]
async fn subscribe_new_heads_err_pending_v0_10_2() {
    let (_backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let err =
        match StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Tag(BlockTag::PreConfirmed)))
            .await
        {
            Ok(_) => panic!("starknet_subscribeNewHeads should reject preconfirmed before accepting"),
            Err(err) => err,
        };

    assert_matches!(
        err,
        jsonrpsee::core::client::error::Error::Call(err) => {
            assert_eq!(err, crate::errors::StarknetWsApiError::Pending.into());
        }
    );
}

#[tokio::test]
async fn subscribe_new_heads_err_l1_accepted_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    add_block_at(&backend, 0);
    backend.set_latest_l1_confirmed(Some(0)).expect("Failed to set L1 confirmed block");
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let err =
        match StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Tag(BlockTag::L1Accepted)))
            .await
        {
            Ok(_) => panic!("starknet_subscribeNewHeads should reject l1 accepted before accepting"),
            Err(err) => err,
        };

    assert_matches!(
        err,
        jsonrpsee::core::client::error::Error::Call(err) => {
            assert_eq!(err, crate::errors::StarknetWsApiError::Pending.into());
        }
    );
}

#[tokio::test]
async fn subscribe_new_heads_unsubscribe_uses_string_id_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let _header = add_block_at(&backend, 0);
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut sub =
        StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, None).await.expect("starknet_subscribeNewHeads");
    tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for block header")
        .expect("Waiting for block header")
        .expect("Waiting for block header");

    StarknetWsRpcApiV0_10_2Client::starknet_unsubscribe(&client, "0".into())
        .await
        .expect("Failed to close subscription");

    assert!(sub.next().await.is_none());
}

#[test]
fn subscription_id_provider_returns_strings() {
    use jsonrpsee::server::IdProvider;

    let id = crate::StarknetSubscriptionIdProvider::default().next_id();
    assert_matches!(id, jsonrpsee::types::SubscriptionId::Str(id) if id.parse::<u64>().is_ok());
}

#[tokio::test]
async fn subscribe_new_heads_reorg_then_resume_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let (block_0_hash, _block_0) = add_block_at_with_hash(&backend, 0);
    let (block_1_hash, _block_1) = add_block_at_with_hash(&backend, 1);
    let (block_2_hash, _block_2) = add_block_at_with_hash(&backend, 2);

    let mut sub = raw_subscribe_new_heads(&client, BlockId::Number(3)).await;

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
            ending_block_hash: block_2_hash,
            ending_block_number: 2,
        })
        .expect("Failed to serialize expected reorg notification")
    );

    let (_new_block_1_hash, new_block_1) = add_block_at_with_hash(&backend, 1);

    let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for replacement head")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve replacement head");
    let item: mp_rpc::v0_10_2::BlockHeader =
        serde_json::from_value(next).expect("Failed to deserialize block header item");

    assert_eq!(item, new_block_1);
}

#[tokio::test]
async fn subscribe_new_heads_reorg_during_backfill_v0_10_2() {
    const BACKFILL_BLOCKS: u64 = 1_025;
    let (backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let mut block_0_hash = Felt::ZERO;
    let mut block_1_hash = Felt::ZERO;
    let mut previous_head_hash = Felt::ZERO;
    for n in 0..BACKFILL_BLOCKS {
        let (block_hash, _header) = add_block_at_with_hash(&backend, n);
        if n == 0 {
            block_0_hash = block_hash;
        } else if n == 1 {
            block_1_hash = block_hash;
        }
        previous_head_hash = block_hash;
    }

    let mut sub = raw_subscribe_new_heads(&client, BlockId::Number(0)).await;
    backend.revert_to(&block_0_hash).expect("Revert should succeed");

    let expected_reorg = serde_json::to_value(mp_rpc::v0_10_2::ReorgData {
        starting_block_hash: block_1_hash,
        starting_block_number: 1,
        ending_block_hash: previous_head_hash,
        ending_block_number: BACKFILL_BLOCKS - 1,
    })
    .expect("Failed to serialize expected reorg notification");

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let next =
                sub.next().await.expect("Subscription closed unexpectedly").expect("Failed to retrieve backfill item");

            if next == expected_reorg {
                break;
            }
        }
    })
    .await
    .expect("Timed out waiting for reorg notification after replay backfill");

    let (_replacement_hash, replacement_head) = add_block_at_with_hash(&backend, 1);

    let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("Timed out waiting for replacement head")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve replacement head");
    let item: mp_rpc::v0_10_2::BlockHeader =
        serde_json::from_value(next).expect("Failed to deserialize replacement head");

    assert_eq!(item, replacement_head);
}

#[tokio::test]
async fn subscribe_new_heads_many_clients_slow_reader_and_cleanup_v0_10_2() {
    let (backend, starknet) = rpc_test_setup();
    let starknet_for_assert = starknet.clone();
    let (_handle, server_url) = start_server(starknet).await;
    add_block_at(&backend, 0);

    let mut next_block = 1;
    for count in [5, 50, 100, 500] {
        let mut subscribers = Vec::with_capacity(count);
        for _ in 0..count {
            let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");
            let sub = StarknetWsRpcApiV0_10_2Client::subscribe_new_heads(&client, Some(BlockId::Number(next_block)))
                .await
                .expect("starknet_subscribeNewHeads");
            subscribers.push((client, sub));
        }
        wait_for_active_subscriptions(&starknet_for_assert, count).await;

        let first = add_block_at(&backend, next_block);
        let second = add_block_at(&backend, next_block + 1);
        next_block += 2;

        for (_, sub) in subscribers.iter_mut().take(count - 1) {
            expect_next_head(sub, &first).await;
            let _ = expect_next_head(sub, &second).await;
        }
        expect_next_head(&mut subscribers.last_mut().expect("slow subscriber").1, &first).await;
        let _ = expect_next_head(&mut subscribers.last_mut().expect("slow subscriber").1, &second).await;

        let unsubscribe_count = count / 2;
        drop(subscribers.drain(..unsubscribe_count));
        wait_for_active_subscriptions(&starknet_for_assert, count - unsubscribe_count).await;

        let third = add_block_at(&backend, next_block);
        next_block += 1;
        for (_, sub) in subscribers.iter_mut().skip(unsubscribe_count) {
            let _ = expect_next_head(sub, &third).await;
        }

        drop(subscribers);
        wait_for_active_subscriptions(&starknet_for_assert, 0).await;
    }
}

async fn expect_next_head(
    sub: &mut jsonrpsee::core::client::Subscription<mp_rpc::v0_10_2::BlockHeader>,
    expected: &mp_rpc::v0_10_2::BlockHeader,
) {
    let item = tokio::time::timeout(Duration::from_secs(10), sub.next())
        .await
        .expect("Timed out waiting for block header")
        .expect("Subscription closed unexpectedly")
        .expect("Failed to retrieve block header");
    assert_eq!(item, *expected);
}

#[tokio::test]
async fn unsubscribe_rejects_invalid_string_id_v0_10_2() {
    let (_backend, starknet) = rpc_test_setup();
    let (_handle, server_url) = start_server(starknet).await;
    let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

    let err = StarknetWsRpcApiV0_10_2Client::starknet_unsubscribe(&client, "not-a-number".into())
        .await
        .expect_err("unsubscribe should fail");

    assert_matches!(
        err,
        jsonrpsee::core::client::error::Error::Call(err) => {
            assert_eq!(err, crate::StarknetRpcApiError::InvalidSubscriptionId.into());
        }
    );
}
