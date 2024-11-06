use starknet_core::types::{BlockId, BlockTag, EmittedEvent, EventFilterWithPage, EventsPage, Felt};

use crate::constants::{MAX_EVENTS_CHUNK_SIZE, MAX_EVENTS_KEYS};
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::types::ContinuationToken;
use crate::Starknet;

/// Returns all events matching the given filter.
///
/// This function retrieves all event objects that match the conditions specified in the
/// provided event filter. The filter can include various criteria such as contract addresses,
/// event types, and block ranges. The function supports pagination through the result page
/// request schema.
///
/// ### Arguments
///
/// * `filter` - The conditions used to filter the returned events. The filter is a combination of
///   an event filter and a result page request, allowing for precise control over which events are
///   returned and in what quantity.
///
/// ### Returns
///
/// Returns a chunk of event objects that match the filter criteria, encapsulated in an
/// `EventsChunk` type. The chunk includes details about the events, such as their data, the
/// block in which they occurred, and the transaction that triggered them. In case of
/// errors, such as `PAGE_SIZE_TOO_BIG`, `INVALID_CONTINUATION_TOKEN`, `BLOCK_NOT_FOUND`, or
/// `TOO_MANY_KEYS_IN_FILTER`, returns a `StarknetRpcApiError` indicating the specific issue.
pub async fn get_events(starknet: &Starknet, filter: EventFilterWithPage) -> StarknetRpcResult<EventsPage> {
    // ===================================================================== //
    //                         Pre-search validation
    // ===================================================================== //

    let from_address = filter.event_filter.address;
    let keys = filter.event_filter.keys.unwrap_or_default();
    let chunk_size = filter.result_page_request.chunk_size;

    if keys.len() > MAX_EVENTS_KEYS {
        return Err(StarknetRpcApiError::TooManyKeysInFilter);
    }
    if chunk_size > MAX_EVENTS_CHUNK_SIZE {
        return Err(StarknetRpcApiError::PageSizeTooBig);
    }

    let block_latest = starknet.get_block_n(&BlockId::Tag(BlockTag::Latest))?;
    let (block_from, pending_from) = to_block_n(filter.event_filter.from_block, starknet, block_latest, 0)?;

    let (block_to, pending_to) = to_block_n(filter.event_filter.to_block, starknet, block_latest, block_latest)?;

    if block_from > block_to || (pending_from && !pending_to) {
        return Ok(EventsPage { events: vec![], continuation_token: None });
    }

    // ===================================================================== //
    //                           Continuation Token
    // ===================================================================== //

    let continuation_token = match filter.result_page_request.continuation_token {
        Some(token) => ContinuationToken::parse(token).map_err(|_| StarknetRpcApiError::InvalidContinuationToken)?,
        None => ContinuationToken { block_n: block_from, event_n: 0 },
    };
    let block_from = continuation_token.block_n;

    let mut events_filtered = Vec::with_capacity(16);

    if !pending_from {
        // ================================================================= //
        //                            Initial loop
        // ================================================================= //

        let mut block_stream = starknet.get_block_stream(block_from)?;
        let block = block_stream.next().ok_or(StarknetRpcApiError::BlockNotFound)?;
        let block_hash = block.info.block_hash;
        let block_n = block.info.header.block_number;
        let block_receipts = block.inner.receipts;

        let events_filtered_block =
            get_block_events(Some(block_hash), Some(block_n), block_receipts, from_address, &keys).collect::<Vec<_>>();
        let event_n = events_filtered_block.len() as u64;

        if event_n < continuation_token.event_n {
            return Err(StarknetRpcApiError::InvalidContinuationToken);
        }

        events_filtered_block
            .into_iter()
            .skip(continuation_token.event_n as usize)
            .take(chunk_size as usize)
            .collect_into(&mut events_filtered);

        if events_filtered.len() == chunk_size as usize {
            let event_n = continuation_token.event_n + chunk_size;
            let token = Some(ContinuationToken { block_n, event_n }.to_string());

            return Ok(EventsPage { events: events_filtered, continuation_token: token });
        }

        // ================================================================= //
        //                              Main loop
        // ================================================================= //

        // FIXME: block stream should return a `Result<MadaraBlock>` so we can
        // detect db errors and they are not silenced!
        for block in block_stream.take((block_to - block_from) as usize) {
            let block_hash = block.info.block_hash;
            let block_n = block.info.header.block_number;

            let event_n = events_filtered.len();
            get_block_events(Some(block_hash), Some(block_n), block.inner.receipts, from_address, &keys)
                .take(chunk_size as usize - events_filtered.len())
                .collect_into(&mut events_filtered);
            let event_n = (events_filtered.len() - event_n) as u64;

            if events_filtered.len() == chunk_size as usize {
                let token = Some(ContinuationToken { block_n, event_n }.to_string());

                return Ok(EventsPage { events: events_filtered, continuation_token: token });
            }
        }
    }

    if pending_to {
        // ================================================================= //
        //                            Pending loop
        // ================================================================= //

        let block_id = &starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending);
        let block = starknet.get_block(block_id)?;

        get_block_events(None, None, block.inner.receipts, from_address, &keys)
            .take(chunk_size as usize - events_filtered.len())
            .collect_into(&mut events_filtered);
    }

    Ok(EventsPage { events: events_filtered, continuation_token: None })
}

fn to_block_n(
    id: Option<starknet_core::types::BlockId>,
    starknet: &Starknet,
    latest: u64,
    default: u64,
) -> Result<(u64, bool), StarknetRpcApiError> {
    match id {
        Some(BlockId::Tag(BlockTag::Pending)) => Ok((latest, true)),
        Some(block_id) => starknet.get_block_n(&block_id).and_then(|block_n| {
            if block_n > latest {
                Err(StarknetRpcApiError::BlockNotFound)
            } else {
                Ok((block_n, false))
            }
        }),
        None => Ok((default, false)),
    }
}

fn get_block_events(
    block_hash: Option<Felt>,
    block_number: Option<u64>,
    receipts: Vec<mp_receipt::TransactionReceipt>,
    address: Option<Felt>,
    keys: &[Vec<Felt>],
) -> impl Iterator<Item = starknet_core::types::EmittedEvent> + '_ {
    receipts.into_iter().flat_map(move |receipt| {
        let transaction_hash = receipt.transaction_hash();

        receipt.events_owned().into_iter().filter_map(move |event| {
            if address.is_some() && address.unwrap() != event.from_address {
                return None;
            }

            // Keys are matched as follows:
            //
            // - `keys` is an array of Felt
            // - `keys[n]` represents allowed value for event key at index n
            // - so `event.keys[n]` needs to match any value in `keys[n]`
            let match_keys = keys
                .iter()
                .enumerate()
                .all(|(i, keys)| event.keys.len() > i && (keys.is_empty() || keys.contains(&event.keys[i])));

            if !match_keys {
                None
            } else {
                Some(EmittedEvent {
                    from_address: event.from_address,
                    keys: event.keys,
                    data: event.data,
                    block_hash,
                    block_number,
                    transaction_hash,
                })
            }
        })
    })
}

#[cfg(test)]
mod test {
    use jsonrpsee::http_client::HttpClientBuilder;

    use crate::{
        test_utils::rpc_test_setup,
        versions::v0_7_1::{StarknetReadRpcApiV0_7_1Client, StarknetReadRpcApiV0_7_1Server},
    };

    fn block_info(n: u64) -> mp_block::MadaraMaybePendingBlockInfo {
        mp_block::MadaraMaybePendingBlockInfo::NotPending(mp_block::MadaraBlockInfo {
            header: mp_block::Header {
                parent_block_hash: starknet_core::types::Felt::from(n),
                block_number: n,
                ..Default::default()
            },
            block_hash: starknet_core::types::Felt::from(n),
            tx_hashes: vec![],
        })
    }

    fn block_info_pending(n: u64) -> mp_block::MadaraMaybePendingBlockInfo {
        mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
            header: mp_block::header::PendingHeader {
                parent_block_hash: starknet_core::types::Felt::from(n),
                ..Default::default()
            },
            tx_hashes: vec![],
        })
    }

    fn block_events(n: u64) -> Vec<mp_receipt::Event> {
        vec![
            mp_receipt::Event {
                from_address: starknet_core::types::Felt::from(n),
                keys: vec![
                    starknet_core::types::Felt::ZERO,
                    starknet_core::types::Felt::ONE,
                    starknet_core::types::Felt::from(n),
                ],
                data: vec![],
            },
            mp_receipt::Event {
                from_address: starknet_core::types::Felt::from(n.saturating_add(1)),
                keys: vec![
                    starknet_core::types::Felt::ZERO,
                    starknet_core::types::Felt::TWO,
                    starknet_core::types::Felt::from(n),
                ],
                data: vec![],
            },
            mp_receipt::Event {
                from_address: starknet_core::types::Felt::from(n.saturating_add(1)),
                keys: vec![],
                data: vec![],
            },
        ]
    }

    fn block_inner(n: u64) -> mp_block::MadaraBlockInner {
        mp_block::MadaraBlockInner {
            transactions: vec![],
            receipts: vec![
                mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                    events: block_events(n),
                    transaction_hash: starknet_core::types::Felt::from(n),
                    ..Default::default()
                }),
                mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt {
                    events: block_events(n),
                    transaction_hash: starknet_core::types::Felt::from(n),
                    ..Default::default()
                }),
            ],
        }
    }

    fn generator_events(
        backend: &mc_db::MadaraBackend,
    ) -> impl Iterator<Item = Vec<starknet_core::types::EmittedEvent>> + '_ {
        (0..).map(|n| {
            let info = block_info(n);
            let inner = block_inner(n);

            backend
                .store_block(
                    mp_block::MadaraMaybePendingBlock { info: info.clone(), inner: inner.clone() },
                    mp_state_update::StateDiff::default(),
                    vec![],
                )
                .expect("Storing block");

            inner
                .receipts
                .into_iter()
                .flat_map(move |receipt| {
                    let block_hash = info.block_hash();
                    let block_number = info.block_n();
                    let transaction_hash = receipt.transaction_hash();
                    receipt.events_owned().into_iter().map(move |event| starknet_core::types::EmittedEvent {
                        from_address: event.from_address,
                        keys: event.keys,
                        data: event.data,
                        block_hash,
                        block_number,
                        transaction_hash,
                    })
                })
                .collect()
        })
    }

    fn generator_events_pending(backend: &mc_db::MadaraBackend) -> Vec<starknet_core::types::EmittedEvent> {
        let info = block_info_pending(u64::MAX);
        let inner = block_inner(u64::MAX);

        backend
            .store_block(
                mp_block::MadaraMaybePendingBlock { info: info.clone(), inner: inner.clone() },
                mp_state_update::StateDiff::default(),
                vec![],
            )
            .expect("Storing block");

        inner
            .receipts
            .into_iter()
            .flat_map(move |receipt| {
                let block_hash = info.block_hash();
                let block_number = info.block_n();
                let transaction_hash = receipt.transaction_hash();
                receipt.events_owned().into_iter().map(move |event| starknet_core::types::EmittedEvent {
                    from_address: event.from_address,
                    keys: event.keys,
                    data: event.data,
                    block_hash,
                    block_number,
                    transaction_hash,
                })
            })
            .collect()
    }

    fn generator_blocks(backend: &mc_db::MadaraBackend) -> impl Iterator<Item = mp_block::MadaraBlock> + '_ {
        (0..).map(|n| {
            let block = mp_block::MadaraMaybePendingBlock { info: block_info(n), inner: block_inner(n) };

            backend.store_block(block.clone(), mp_state_update::StateDiff::default(), vec![]).expect("Storing block");

            mp_block::MadaraBlock::try_from(block).unwrap()
        })
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_block_stream(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;

        let generator = generator_blocks(&backend);
        let expected = Vec::from_iter(generator.take(10));

        let block_stream = starknet.get_block_stream(0).expect("Retrieving block stream");
        let blocks = block_stream.collect::<Vec<_>>();

        if blocks != expected {
            let file_blocks = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_blocks);
            serde_json::to_writer_pretty(writter, &blocks).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_block_stream2(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;

        let block_stream = starknet.get_block_stream(0).expect("Retrieving block stream");

        let generator = generator_blocks(&backend);
        let _ = generator.take(10).last();

        let blocks = block_stream.collect::<Vec<_>>();

        assert_eq!(blocks, []);
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let mut generator = generator_events(&backend);
        let expected = generator.next().expect("Retrieving event from backend");

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_block(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks =
            generator_events(&backend).take(3).flatten().filter(|event| event.block_number.is_some_and(|n| n >= 1));
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: Some(starknet_core::types::BlockId::Number(1)),
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_block_pending(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let _ = generator_events(&backend).take(3).last();
        let expected = generator_events_pending(&backend);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: Some(starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending)),
                    to_block: Some(starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending)),
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_block_invalid(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let _ = generator_events(&backend).next().expect("Retrieving event from backend");
        let expected = crate::StarknetRpcApiError::BlockNotFound;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: Some(starknet_core::types::BlockId::Number(1)),
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_block_out_of_order(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let _ = generator_events(&backend).take(3).last();
        let _ = generator_events_pending(&backend);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: Some(starknet_core::types::BlockId::Number(1)),
                    to_block: Some(starknet_core::types::BlockId::Number(0)),
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        assert_eq!(events, []);
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_block_out_of_order_pending(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let _ = generator_events(&backend).take(3).last();
        let _ = generator_events_pending(&backend);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: Some(starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending)),
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        assert_eq!(events, []);
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_to_block(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks =
            generator_events(&backend).take(3).flatten().filter(|event| event.block_number.is_some_and(|n| n <= 1));
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: Some(starknet_core::types::BlockId::Number(1)),
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_to_block_pending(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let mut expected = Vec::from_iter(generator_events(&backend).take(3).flatten());
        expected.extend(generator_events_pending(&backend));

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: Some(starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending)),
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_to_block_invalid(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let _ = generator_events(&backend).take(3).last();
        let expected = crate::StarknetRpcApiError::BlockNotFound;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: Some(starknet_core::types::BlockId::Number(3)),
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_address(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| event.from_address == starknet_core::types::Felt::TWO);
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: Some(starknet_core::types::Felt::TWO),
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_from_address_pending(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| event.from_address == starknet_core::types::Felt::from(u64::MAX));
        let mut expected = Vec::from_iter(blocks);

        generator_events_pending(&backend)
            .into_iter()
            .filter(|event| event.from_address == starknet_core::types::Felt::from(u64::MAX))
            .collect_into(&mut expected);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: Some(starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending)),
                    address: Some(starknet_core::types::Felt::from(u64::MAX)),
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_with_keys(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| !event.keys.is_empty() && event.keys[0] == starknet_core::types::Felt::ZERO);
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: Some(vec![vec![starknet_core::types::Felt::ZERO]]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_with_keys2(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| event.keys.len() > 1 && event.keys[1] == starknet_core::types::Felt::ONE);
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: Some(vec![vec![], vec![starknet_core::types::Felt::ONE]]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_with_keys_single(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| event.keys.len() > 2 && event.keys[2] == starknet_core::types::Felt::TWO);
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: Some(vec![vec![], vec![], vec![starknet_core::types::Felt::TWO]]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_with_keys_pending(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| !event.keys.is_empty() && event.keys[0] == starknet_core::types::Felt::ZERO);
        let mut expected = Vec::from_iter(blocks);

        generator_events_pending(&backend)
            .into_iter()
            .filter(|event| !event.keys.is_empty() && event.keys[0] == starknet_core::types::Felt::ZERO)
            .collect_into(&mut expected);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: Some(starknet_core::types::BlockId::Tag(starknet_core::types::BlockTag::Pending)),
                    address: None,
                    keys: Some(vec![vec![starknet_core::types::Felt::ZERO]]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .expect("starknet_getEvents")
            .events;

        if events != expected {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_with_continuation_token(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let blocks = generator_events(&backend)
            .take(3)
            .flatten()
            .filter(|event| !event.keys.is_empty() && event.keys[0] == starknet_core::types::Felt::ZERO);
        let expected = Vec::from_iter(blocks);

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: Some(vec![vec![starknet_core::types::Felt::ZERO]]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: 10,
                },
            })
            .await
            .expect("starknet_getEvents");

        if events.events != expected[..10] {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: Some(vec![vec![starknet_core::types::Felt::ZERO]]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: events.continuation_token,
                    chunk_size: 10,
                },
            })
            .await
            .expect("starknet_getEvents");

        if events.events != expected[10..] {
            let file_events = std::fs::File::create("./test_output_actual.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_events);
            serde_json::to_writer_pretty(writter, &events).unwrap_or_default();

            let file_expected = std::fs::File::create("./test_output_events.json").expect("Opening file");
            let writter = std::io::BufWriter::new(file_expected);
            serde_json::to_writer_pretty(writter, &expected).unwrap_or_default();

            panic!("Events do not match (see test output for more details)");
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_block_not_found(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (_, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");
        let expected = crate::StarknetRpcApiError::BlockNotFound;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_block_invalid_to(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let mut generator = generator_events(&backend);
        let _ = generator.next().expect("Retrieving event from backend");
        let expected = crate::StarknetRpcApiError::BlockNotFound;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: Some(starknet_core::types::BlockId::Number(1)),
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_page_size_too_big(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let mut generator = generator_events(&backend);
        let _ = generator.next().expect("Retrieving event from backend");
        let expected = crate::StarknetRpcApiError::PageSizeTooBig;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE + 1,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_invalid_continuation_token(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let mut generator = generator_events(&backend);
        let _ = generator.next().expect("Retrieving event from backend");
        let expected = crate::StarknetRpcApiError::InvalidContinuationToken;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: Some("".to_string()),
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_invalid_continuation_token2(
        rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet),
    ) {
        let (backend, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");

        let mut generator = generator_events(&backend);
        let _ = generator.next().expect("Retrieving event from backend");
        let expected = crate::StarknetRpcApiError::InvalidContinuationToken;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: None,
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: Some("0-100".to_string()),
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn get_events_too_many_keys(rpc_test_setup: (std::sync::Arc<mc_db::MadaraBackend>, crate::Starknet)) {
        let (_, starknet) = rpc_test_setup;
        let server = jsonrpsee::server::Server::builder().build("127.0.0.1:0").await.expect("Starting server");
        let server_url = format!("http://{}", server.local_addr().expect("Retrieving server local address"));

        // Server will be stopped once this is dropped
        let _server_handle = server.start(StarknetReadRpcApiV0_7_1Server::into_rpc(starknet));
        let client = HttpClientBuilder::default().build(&server_url).expect("Building client");
        let expected = crate::StarknetRpcApiError::TooManyKeysInFilter;

        let events = client
            .get_events(starknet_core::types::EventFilterWithPage {
                event_filter: starknet_core::types::EventFilter {
                    from_block: None,
                    to_block: None,
                    address: None,
                    keys: Some(vec![vec![]; crate::constants::MAX_EVENTS_KEYS + 1]),
                },
                result_page_request: starknet_core::types::ResultPageRequest {
                    continuation_token: None,
                    chunk_size: crate::constants::MAX_EVENTS_CHUNK_SIZE,
                },
            })
            .await
            .err()
            .expect("starknet_getEvents");

        let jsonrpsee::core::client::Error::Call(error_object) = events else {
            panic!("starknet_getEvents");
        };

        assert_eq!(error_object.code(), Into::<i32>::into(&expected));
        assert_eq!(error_object.message(), expected.to_string());
        assert!(error_object.data().is_none());
    }
}
