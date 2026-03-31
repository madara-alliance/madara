use mc_db::subscription::SubscribeNewBlocksTag;
use mp_rpc::v0_8_1::{BlockId, BlockTag};

use crate::errors::{ErrorExtWs, OptionExtWs, StarknetWsApiError};

use super::BLOCK_PAST_LIMIT;

pub async fn subscribe_new_heads(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    block_id: BlockId,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let mut block_n = match block_id {
        BlockId::Number(block_n) => {
            let block_latest = starknet.backend.latest_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?;

            if block_n < block_latest.saturating_sub(BLOCK_PAST_LIMIT) {
                return Err(StarknetWsApiError::TooManyBlocksBack);
            }

            block_n
        }
        BlockId::Hash(block_hash) => {
            let block_latest = starknet.backend.latest_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?;
            let block_n = starknet
                .backend
                .view_on_latest_confirmed()
                .find_block_by_hash(&block_hash)
                .or_else_internal_server_error(|| format!("Failed to retrieve block info at hash {block_hash:#x}"))?
                .ok_or(StarknetWsApiError::BlockNotFound)?;

            if block_n < block_latest.saturating_sub(BLOCK_PAST_LIMIT) {
                return Err(StarknetWsApiError::TooManyBlocksBack);
            }

            block_n
        }
        BlockId::Tag(BlockTag::Latest) => {
            starknet.backend.latest_confirmed_block_n().ok_or(StarknetWsApiError::NoBlocks)?
        }
        BlockId::Tag(BlockTag::Pending) => {
            return Err(StarknetWsApiError::Pending);
        }
    };

    for n in block_n.. {
        if sink.is_closed() {
            return Ok(());
        }

        let Some(block_view) = starknet.backend.block_view_on_confirmed(n) else {
            break;
        };
        let block_info = block_view
            .get_block_info()
            .or_else_internal_server_error(|| format!("Failed to retrieve block info for block {n}"))?;

        if block_info.header.block_number != n {
            let err = format!("Retrieved mismatched block {}, expected {n}", block_info.header.block_number);
            return Err(StarknetWsApiError::internal_server_error(err));
        };

        send_block_header(&sink, block_info, n).await?;
        block_n = block_n.saturating_add(1);
    }

    let mut heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Confirmed);
    heads.set_start_from(block_n);
    let mut reorgs = starknet.backend.subscribe_reorgs();

    loop {
        let next_block_n = tokio::select! {
            head = heads.next_head() => head.latest_confirmed_block_n(),
            reorg = reorgs.recv() => {
                match reorg {
                    Ok(reorg) => {
                        super::send_reorg_notification(&sink, &reorg).await?;
                        heads = starknet.backend.subscribe_new_heads(SubscribeNewBlocksTag::Confirmed);
                        heads.set_start_from(reorg.first_reverted_block_n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        return Err(crate::errors::StarknetWsApiError::Internal);
                    }
                }
            },
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
        };

        let next_block_n =
            next_block_n.expect("Confirmed block subscription should always yield a confirmed block number");
        let block_view = starknet
            .backend
            .block_view_on_confirmed(next_block_n)
            .ok_or_else_internal_server_error(|| format!("Failed to retrieve block info for block {next_block_n}"))?;
        let block_info = block_view
            .get_block_info()
            .or_else_internal_server_error(|| format!("Failed to retrieve block info for block {next_block_n}"))?;
        send_block_header(&sink, block_info, next_block_n).await?;
    }
}

async fn send_block_header(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    block_info: mp_block::MadaraBlockInfo,
    block_n: u64,
) -> Result<(), StarknetWsApiError> {
    let header = block_info.to_rpc_v0_8();
    let item = super::SubscriptionItem::new(sink.subscription_id(), header);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_else_internal_server_error(|| format!("Failed to create response message for block {block_n}"))?;

    sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use jsonrpsee::{
        core::{client::SubscriptionClientT, params::ObjectParams},
        ws_client::WsClientBuilder,
    };
    use mp_rpc::v0_8_1::BlockHeader;
    use serde_json::Value;
    use starknet_types_core::felt::Felt;
    use std::time::Duration;

    use crate::{
        test_utils::rpc_test_setup,
        versions::user::v0_8_1::{StarknetWsRpcApiV0_8_1Client, StarknetWsRpcApiV0_8_1Server},
        Starknet,
    };

    fn block_generator(backend: &std::sync::Arc<mc_db::MadaraBackend>) -> impl Iterator<Item = BlockHeader> + '_ {
        (0..).map(|n| {
            backend
                .write_access()
                .add_full_block_with_classes(
                    &mp_block::FullBlockWithoutCommitments {
                        header: mp_block::PreconfirmedHeader { block_number: n, ..Default::default() },
                        state_diff: mp_state_update::StateDiff::default(),
                        transactions: vec![],
                        events: vec![],
                    },
                    &[],
                    false,
                )
                .expect("Storing block");

            backend
                .block_view_on_confirmed(n)
                .expect("Retrieving block view")
                .get_block_info()
                .expect("Retrieving block info")
                .to_rpc_v0_8()
        })
    }

    fn add_block_at(backend: &std::sync::Arc<mc_db::MadaraBackend>, n: u64) -> (Felt, BlockHeader) {
        let block_hash = backend
            .write_access()
            .add_full_block_with_classes(
                &mp_block::FullBlockWithoutCommitments {
                    header: mp_block::PreconfirmedHeader { block_number: n, ..Default::default() },
                    state_diff: mp_state_update::StateDiff::default(),
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .expect("Storing block")
            .block_hash;

        let header = backend
            .block_view_on_confirmed(n)
            .expect("Retrieving block view")
            .get_block_info()
            .expect("Retrieving block info")
            .to_rpc_v0_8();

        (block_hash, header)
    }

    async fn raw_subscribe_new_heads(
        client: &jsonrpsee::ws_client::WsClient,
        block: BlockId,
    ) -> jsonrpsee::core::client::Subscription<Value> {
        let mut params = ObjectParams::new();
        params.insert("block", block).expect("Building subscribeNewHeads params");
        SubscriptionClientT::subscribe(
            client,
            "starknet_V0_8_1_subscribeNewHeads",
            params,
            "starknet_V0_8_1_unsubscribe",
        )
        .await
        .expect("starknet_V0_8_1_subscribeNewHeads")
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);
        let expected = generator.next().expect("Retrieving block from backend");

        let mut sub =
            client.subscribe_new_heads(BlockId::Tag(BlockTag::Latest)).await.expect("starknet_subscribeNewHeads");

        let next = sub.next().await;
        let header = next.expect("Waiting for block header").expect("Waiting for block header").result;

        assert_eq!(
            header,
            expected,
            "actual: {}\nexpect: {}",
            serde_json::to_string_pretty(&header).unwrap_or_default(),
            serde_json::to_string_pretty(&expected).unwrap_or_default()
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_many(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let generator = block_generator(&backend);
        let expected: Vec<_> = generator.take(BLOCK_PAST_LIMIT as usize).collect();

        let mut sub = client.subscribe_new_heads(BlockId::Number(0)).await.expect("starknet_subscribeNewHeads");

        for e in expected {
            let next = sub.next().await;
            let header = next.expect("Waiting for block header").expect("Waiting for block header").result;

            assert_eq!(
                header,
                e,
                "actual: {}\nexpect: {}",
                serde_json::to_string_pretty(&header).unwrap_or_default(),
                serde_json::to_string_pretty(&e).unwrap_or_default()
            );
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_disconnect(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);
        let expected = generator.next().expect("Retrieving block from backend");

        let mut sub = client.subscribe_new_heads(BlockId::Number(0)).await.expect("starknet_subscribeNewHeads");

        let next = sub.next().await;
        let header = next.expect("Waiting for block header").expect("Waiting for block header").result;

        assert_eq!(
            header,
            expected,
            "actual: {}\nexpect: {}",
            serde_json::to_string_pretty(&header).unwrap_or_default(),
            serde_json::to_string_pretty(&expected).unwrap_or_default()
        );

        let next = sub.unsubscribe().await;
        assert!(next.is_ok());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_future(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);
        let _block_0 = generator.next().expect("Retrieving block from backend");

        let mut sub = client.subscribe_new_heads(BlockId::Number(1)).await.expect("starknet_subscribeNewHeads");

        let block_1 = generator.next().expect("Retrieving block from backend");

        let next = sub.next().await;
        let header = next.expect("Waiting for block header").expect("Waiting for block header").result;

        // Note that `sub` does not yield block 0. This is because it starts
        // from block 1, ignoring any block before. This can server to notify
        // when a block is ready
        assert_eq!(
            header,
            block_1,
            "actual: {}\nexpect: {}",
            serde_json::to_string_pretty(&header).unwrap_or_default(),
            serde_json::to_string_pretty(&block_1).unwrap_or_default()
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_unsubscribe(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);
        let _block_0 = generator.next().expect("Retrieving block from backend");

        let mut sub = client.subscribe_new_heads(BlockId::Number(1)).await.expect("starknet_subscribeNewHeads");

        let _block_1 = generator.next().expect("Retrieving block from backend");

        let next = sub.next().await;
        let subscription_id = next.unwrap().unwrap().subscription_id;
        client.starknet_unsubscribe(subscription_id).await.expect("Failed to close subscription");

        assert!(sub.next().await.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_err_too_far_back_block_n(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        // We generate BLOCK_PAST_LIMIT + 2 because genesis is block 0
        let generator = block_generator(&backend);
        let _expected: Vec<_> = generator.take(BLOCK_PAST_LIMIT as usize + 2).collect();

        let mut sub = client.subscribe_new_heads(BlockId::Number(0)).await.expect("starknet_subscribeNewHeads");

        // Jsonrsee seems to just close the connection and not return the error
        // to the client so this is the best we can do :/
        let next = sub.next().await;
        assert!(next.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_err_too_far_back_block_hash(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        // We generate BLOCK_PAST_LIMIT + 2 because genesis is block 0
        let generator = block_generator(&backend);
        let _expected: Vec<_> = generator.take(BLOCK_PAST_LIMIT as usize + 2).collect();

        let mut sub =
            client.subscribe_new_heads(BlockId::Hash(Felt::from(0))).await.expect("starknet_subscribeNewHeads");

        // Jsonrsee seems to just close the connection and not return the error
        // to the client so this is the best we can do :/
        let next = sub.next().await;
        assert!(next.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_err_pending(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let generator = block_generator(&backend);
        let _expected: Vec<_> = generator.take(BLOCK_PAST_LIMIT as usize + 2).collect();

        let mut sub =
            client.subscribe_new_heads(BlockId::Tag(BlockTag::Pending)).await.expect("starknet_subscribeNewHeads");

        // Jsonrsee seems to just close the connection and not return the error
        // to the client so this is the best we can do :/
        let next = sub.next().await;
        assert!(next.is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_new_heads_reorg_then_resume(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_1Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let (block_0_hash, _block_0) = add_block_at(&backend, 0);
        let (block_1_hash, _block_1) = add_block_at(&backend, 1);
        let (block_2_hash, _block_2) = add_block_at(&backend, 2);

        let mut sub = raw_subscribe_new_heads(&client, BlockId::Number(3)).await;

        backend.revert_to(&block_0_hash).expect("Revert should succeed");

        let reorg = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for reorg notification")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve reorg notification");

        assert_eq!(
            reorg,
            serde_json::to_value(mp_rpc::v0_8_1::ReorgData {
                starting_block_hash: block_1_hash,
                starting_block_number: 1,
                ending_block_hash: block_2_hash,
                ending_block_number: 2,
            })
            .expect("Failed to serialize expected reorg notification")
        );

        let (_new_block_1_hash, new_block_1) = add_block_at(&backend, 1);

        let next = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("Timed out waiting for new-fork head")
            .expect("Subscription closed unexpectedly")
            .expect("Failed to retrieve new-fork head");
        let item: super::super::SubscriptionItem<BlockHeader> =
            serde_json::from_value(next).expect("Failed to deserialize block header item");

        assert_eq!(item.result, new_block_1);
    }
}
