use mp_rpc::v0_9_0::BlockId;

use super::BLOCK_PAST_LIMIT;
use crate::errors::{ErrorExtWs, OptionExtWs, StarknetWsApiError};
use std::sync::Arc;

pub async fn subscribe_new_heads(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    block_id: BlockId,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let mut block_n = match block_id {
        BlockId::Number(block_n) => {
            let err = || format!("Failed to retrieve block info for block {block_n}");
            let block_latest = starknet
                .backend
                .get_block_n(&BlockId::Tag(BlockTag::Latest))
                .or_else_internal_server_error(err)?
                .ok_or(StarknetWsApiError::NoBlocks)?;

            if block_n < block_latest.saturating_sub(BLOCK_PAST_LIMIT) {
                return Err(StarknetWsApiError::TooManyBlocksBack);
            }

            block_n
        }
        BlockId::Hash(block_hash) => {
            let err = || format!("Failed to retrieve block info at hash {block_hash:#x}");
            let block_latest = starknet
                .backend
                .get_block_n(&BlockId::Tag(BlockTag::Latest))
                .or_else_internal_server_error(err)?
                .ok_or(StarknetWsApiError::NoBlocks)?;

            let block_n = starknet
                .backend
                .get_block_n(&block_id)
                .or_else_internal_server_error(err)?
                .ok_or(StarknetWsApiError::BlockNotFound)?;

            if block_n < block_latest.saturating_sub(BLOCK_PAST_LIMIT) {
                return Err(StarknetWsApiError::TooManyBlocksBack);
            }

            block_n
        }
        BlockId::Tag(BlockTag::Latest) => starknet
            .backend
            .get_latest_block_n()
            .or_internal_server_error("Failed to retrieve block info for latest block")?
            .ok_or(StarknetWsApiError::NoBlocks)?,
        BlockId::Tag(BlockTag::Pending) => {
            return Err(StarknetWsApiError::Pending);
        }
    };

    let mut rx = starknet.backend.subscribe_closed_blocks();
    for n in block_n.. {
        if sink.is_closed() {
            return Ok(());
        }

        let block_info = match starknet.backend.get_block_info(&BlockId::Number(n)) {
            Ok(Some(block_info)) => {
                let err = || format!("Failed to retrieve block info for block {n}");
                block_info.into_closed().ok_or_else_internal_server_error(err)?
            }
            Ok(None) => break,
            Err(e) => {
                let err = format!("Failed to retrieve block info for block {n}: {e}");
                return Err(StarknetWsApiError::internal_server_error(err));
            }
        };

        send_block_header(&sink, block_info, block_n).await?;
        block_n = block_n.saturating_add(1);
    }

    // Catching up with the backend
    loop {
        let block_info = tokio::select! {
            block_info = rx.recv() => block_info.or_internal_server_error("Failed to retrieve block info")?,
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
        };

        if block_info.header.block_number == block_n {
            break send_block_header(&sink, Arc::unwrap_or_clone(block_info), block_n).await?;
        }
    }

    // New block headers
    loop {
        let block_info = tokio::select! {
            block_info = rx.recv() => block_info.or_internal_server_error("Failed to retrieve block info")?,
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal),
        };

        if block_info.header.block_number == block_n + 1 {
            send_block_header(&sink, Arc::unwrap_or_clone(block_info), block_n).await?;
        } else {
            let err =
                format!("Received non-sequential block {}, expected {}", block_info.header.block_number, block_n + 1);
            return Err(StarknetWsApiError::internal_server_error(err));
        }
        block_n = block_n.saturating_add(1);
    }
}

async fn send_block_header(
    sink: &jsonrpsee::core::server::SubscriptionSink,
    block_info: mp_block::MadaraBlockInfo,
    block_n: u64,
) -> Result<(), StarknetWsApiError> {
    let header = mp_rpc::v0_8_1::BlockHeader::from(block_info);
    let item = super::SubscriptionItem::new(sink.subscription_id(), header);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_else_internal_server_error(|| format!("Failed to create response message for block {block_n}"))?;

    sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    use jsonrpsee::ws_client::WsClientBuilder;
    use mp_rpc::v0_8_1::BlockHeader;
    use starknet_types_core::felt::Felt;

    use crate::{
        test_utils::rpc_test_setup,
        versions::user::v0_8_1::{StarknetWsRpcApiV0_8_1Client, StarknetWsRpcApiV0_8_1Server},
        Starknet,
    };

    fn block_generator(backend: &mc_db::MadaraBackend) -> impl Iterator<Item = BlockHeader> + '_ {
        (0..).map(|n| {
            backend
                .store_block(
                    mp_block::MadaraMaybePendingBlock {
                        info: mp_block::MadaraMaybePreconfirmedBlockInfo::Confirmed(mp_block::MadaraBlockInfo {
                            header: mp_block::Header {
                                parent_block_hash: Felt::from(n),
                                block_number: n,
                                ..Default::default()
                            },
                            block_hash: Felt::from(n),
                            tx_hashes: vec![],
                        }),
                        inner: mp_block::MadaraBlockInner { transactions: vec![], receipts: vec![] },
                    },
                    mp_state_update::StateDiff::default(),
                    vec![],
                )
                .expect("Storing block");

            let block_info = backend
                .get_block_info(&BlockId::Number(n))
                .expect("Retrieving block info")
                .expect("Retrieving block info")
                .into_closed()
                .expect("Retrieving block info");

            BlockHeader::from(block_info)
        })
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
}
