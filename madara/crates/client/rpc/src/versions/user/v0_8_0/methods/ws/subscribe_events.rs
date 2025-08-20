use crate::errors::{ErrorExtWs, StarknetWsApiError};
use mp_block::{
    event_with_info::{drain_block_events, event_match_filter},
    BlockId,
};
use mp_rpc::EmittedEvent;
use starknet_types_core::felt::Felt;

use super::BLOCK_PAST_LIMIT;

pub async fn subscribe_events(
    starknet: &crate::Starknet,
    subscription_sink: jsonrpsee::PendingSubscriptionSink,
    from_address: Option<Felt>,
    keys: Option<Vec<Vec<Felt>>>,
    block_id: Option<BlockId>,
) -> Result<(), StarknetWsApiError> {
    let sink = subscription_sink.accept().await.or_internal_server_error("Failed to establish websocket connection")?;
    let ctx = starknet.ws_handles.subscription_register(sink.subscription_id()).await;

    let mut rx = starknet.backend.subscribe_events(from_address);

    if let Some(block_id) = block_id {
        let latest_block = starknet
            .backend
            .get_latest_block_n()
            .or_internal_server_error("Failed to retrieve latest block")?
            .ok_or(StarknetWsApiError::NoBlocks)?;

        let block_n = starknet
            .backend
            .resolve_block_id(&block_id)
            .or_internal_server_error("Failed to resolve block id")?
            .ok_or(StarknetWsApiError::BlockNotFound)?
            .block_n()
            .ok_or(StarknetWsApiError::Pending)?;

        if block_n < latest_block.saturating_sub(BLOCK_PAST_LIMIT) {
            return Err(StarknetWsApiError::TooManyBlocksBack);
        }
        for block_number in block_n..=latest_block {
            let block = starknet
                .get_block(&BlockId::Number(block_number))
                .or_internal_server_error("Failed to retrieve block")?;
            for event in drain_block_events(block)
                .filter(|event| event_match_filter(&event.event, from_address.as_ref(), keys.as_deref()))
            {
                send_event(event, &sink).await?;
            }
        }
    }

    loop {
        let event = tokio::select! {
            event = rx.recv() => event.or_internal_server_error("Failed to retrieve event")?,
            _ = sink.closed() => return Ok(()),
            _ = ctx.cancelled() => return Err(crate::errors::StarknetWsApiError::Internal)
        };

        if event_match_filter(&event.event, from_address.as_ref(), keys.as_deref()) {
            send_event(event, &sink).await?;
        }
    }
}

async fn send_event(
    event: mp_block::EventWithInfo,
    sink: &jsonrpsee::server::SubscriptionSink,
) -> Result<(), StarknetWsApiError> {
    let event = EmittedEvent::from(event);
    let item = super::SubscriptionItem::new(sink.subscription_id(), event);
    let msg = jsonrpsee::SubscriptionMessage::from_json(&item)
        .or_internal_server_error("Failed to create response message")?;
    sink.send(msg).await.or_internal_server_error("Failed to respond to websocket request")
}

#[cfg(test)]
mod test {
    use crate::{
        versions::user::v0_8_0::{StarknetWsRpcApiV0_8_0Client, StarknetWsRpcApiV0_8_0Server},
        Starknet,
    };

    use super::*;
    use crate::test_utils::rpc_test_setup;
    use jsonrpsee::ws_client::WsClientBuilder;
    use mp_receipt::{InvokeTransactionReceipt, TransactionReceipt};
    use mp_rpc::{EmittedEvent, Event, EventContent};

    /// Generates a transaction receipt with predictable event values for testing purposes.
    /// Values are generated using binary patterns for easy verification.
    ///
    /// # Values Pattern (in binary)
    /// For a given base B:
    /// - Transaction hash    = B << 32
    /// - For each event i:
    ///   - from_address     = (B << 32) | (i << 16) | 1
    ///   - keys[j]          = (B << 32) | (i << 16) | (2 + j)
    ///
    /// This means:
    /// - Top 32 bits: base value
    /// - Next 16 bits: event index
    /// - Last 16 bits: value type (1 for address, 2+ for keys)
    ///
    /// # Arguments
    /// * `base` - Base number used as prefix for all values
    /// * `num_events` - Number of events to generate
    /// * `keys_per_event` - Number of keys per event
    fn generate_receipt(base: u64, num_events: usize, keys_per_event: usize) -> TransactionReceipt {
        // Transaction hash is just the base shifted
        let tx_hash = Felt::from(base << 32);

        let events = (0..num_events)
            .map(|event_idx| {
                // Base pattern for this event: (base << 32) | (event_idx << 16)
                let event_pattern = (base << 32) | ((event_idx as u64) << 16);

                // from_address adds 1 to the pattern
                let from_address = Felt::from(event_pattern | 1);

                // Keys add 2+ to the pattern
                let keys =
                    (0..keys_per_event).map(|key_idx| Felt::from(event_pattern | (2 + key_idx as u64))).collect();

                mp_receipt::Event { from_address, keys, data: vec![] }
            })
            .collect();

        TransactionReceipt::Invoke(InvokeTransactionReceipt { transaction_hash: tx_hash, events, ..Default::default() })
    }

    // Generator function that produces a stream of blocks containing events
    // Each block contains two receipts:
    // 1. First receipt with 1 event and 1 key
    // 2. Second receipt with 2 events and 2 keys
    fn block_generator(backend: &mc_db::MadaraBackend) -> impl Iterator<Item = Vec<EmittedEvent>> + '_ {
        (0..).map(|n| {
            let block_info = mp_block::MadaraBlockInfo {
                header: mp_block::Header { parent_block_hash: Felt::from(n), block_number: n, ..Default::default() },
                block_hash: Felt::from(n),
                tx_hashes: vec![],
            };

            let receipts = vec![generate_receipt(n * 2, 1, 1), generate_receipt(n * 2 + 1, 2, 2)];

            let block_inner = mp_block::MadaraBlockInner { transactions: vec![], receipts };

            backend
                .store_block(
                    mp_block::MadaraMaybePendingBlock {
                        info: mp_block::MadaraMaybePreconfirmedBlockInfo::Confirmed(block_info.clone()),
                        inner: block_inner.clone(),
                    },
                    mp_state_update::StateDiff::default(),
                    vec![],
                )
                .expect("Storing block");

            block_inner
                .receipts
                .into_iter()
                .flat_map(|receipt| {
                    let tx_hash = receipt.transaction_hash();
                    receipt.into_events().into_iter().map(move |events| (tx_hash, events))
                })
                .map(|(transaction_hash, event)| EmittedEvent {
                    event: Event {
                        from_address: event.from_address,
                        event_content: EventContent { keys: event.keys, data: event.data },
                    },
                    block_hash: Some(block_info.block_hash),
                    block_number: Some(block_info.header.block_number),
                    transaction_hash,
                })
                .collect()
        })
    }

    // Test 1: Basic event subscription without any filters
    // - Creates 10 blocks with events
    // - Verifies that all 30 events are received (3 events per block * 10 blocks)
    // - Events should arrive in the same order they were generated
    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_events_no_filter(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);

        let mut sub = client.subscribe_events(None, None, None).await.expect("Subscribing to events");

        let mut nb_events = 0;
        for _ in 0..10 {
            let events = generator.next().expect("Retrieving block");
            for event in events {
                let received = sub.next().await.expect("Subscribing closed").expect("Failed to retrieve event");
                assert_eq!(received.result, event);
                nb_events += 1;
            }
        }
        assert_eq!(nb_events, 30);
    }

    // Test 2: Event subscription filtered by address
    // - Creates blocks and filters events by a specific from_address
    // - Only events from the specified address should be received
    // - Verifies that at least some events match the filter
    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_events_filter_address(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);

        let from_address = Felt::from(0x300000001u64);
        let mut sub = client.subscribe_events(Some(from_address), None, None).await.expect("Subscribing to events");

        let mut nb_events = 0;

        for _ in 0..10 {
            let events = generator.next().expect("Retrieving block");
            for event in events {
                if event.event.from_address == from_address {
                    let received = sub.next().await.expect("Subscribing closed").expect("Failed to retrieve event");
                    assert_eq!(received.result, event);
                    nb_events += 1;
                }
            }
        }
        assert_eq!(nb_events, 1);
    }

    // Test 3: Event subscription filtered by keys
    // - Creates blocks and filters events by specific key patterns
    // - Only events with matching keys should be received
    // - Verifies that exactly two specific events match the filter pattern
    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_events_filter_keys(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);

        let keys = vec![
            vec![Felt::from(0x300000002u64), Felt::from(0x500000002u64)],
            vec![Felt::from(0x300000003u64), Felt::from(0x500000003u64)],
        ];

        let mut sub = client.subscribe_events(None, Some(keys.clone()), None).await.expect("Subscribing to events");

        let expected_events = vec![
            EmittedEvent {
                event: Event {
                    from_address: Felt::from(0x300000001u64),
                    event_content: EventContent {
                        keys: vec![Felt::from(0x300000002u64), Felt::from(0x300000003u64)],
                        data: vec![],
                    },
                },
                block_hash: Some(Felt::from(1u64)),
                block_number: Some(1),
                transaction_hash: Felt::from(0x300000000u64),
            },
            EmittedEvent {
                event: Event {
                    from_address: Felt::from(0x500000001u64),
                    event_content: EventContent {
                        keys: vec![Felt::from(0x500000002u64), Felt::from(0x500000003u64)],
                        data: vec![],
                    },
                },
                block_hash: Some(Felt::from(2u64)),
                block_number: Some(2),
                transaction_hash: Felt::from(0x500000000u64),
            },
        ];

        for _ in 0..10 {
            let _ = generator.next().expect("Retrieving block");
        }

        for event in expected_events {
            let received = sub.next().await.expect("Subscribing closed").expect("Failed to retrieve event");
            assert_eq!(received.result, event);
        }
    }

    // Test 4: Event subscription starting from a past block
    // - Generates initial blocks (0-2)
    // - Starts subscription from block 3
    // - Verifies that only events from blocks 3-9 are received
    // - Events should arrive in the correct order
    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_events_past_block(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);

        // Generate first 3 blocks but ignore their events
        for _ in 0..3 {
            let _ = generator.next().expect("Retrieving block");
        }

        let mut expected_events = vec![];

        // Collect events from blocks 3-9
        for _ in 3..10 {
            let events = generator.next().expect("Retrieving block");
            for event in events {
                expected_events.push(event);
            }
        }

        let block_id = BlockId::Number(3);
        let mut sub = client.subscribe_events(None, None, Some(block_id)).await.expect("Subscribing to events");

        for event in expected_events {
            let received = sub.next().await.expect("Subscribing closed").expect("Failed to retrieve event");
            assert_eq!(received.result, event);
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn subscribe_events_unsubscribe(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("ws://{}", server.local_addr().expect("Retrieving server local address"));
        let _server_handle = server.start(StarknetWsRpcApiV0_8_0Server::into_rpc(starknet));
        let client = WsClientBuilder::default().build(&server_url).await.expect("Building client");

        let mut generator = block_generator(&backend);

        let mut sub = client.subscribe_events(None, None, None).await.expect("Subscribing to events");

        let events = generator.next().expect("Retrieving block");
        let subscription_id = sub.next().await.unwrap().unwrap().subscription_id;
        client.starknet_unsubscribe(subscription_id).await.expect("Failed to close subscription");

        let mut nb_events = 0;
        for event in events.into_iter().skip(1) {
            let received = sub.next().await.expect("Subscribing closed").expect("Failed to retrieve event");
            assert_eq!(received.result, event);
            nb_events += 1;
        }
        assert_eq!(nb_events, 2);

        assert!(sub.next().await.is_none());
    }
}
