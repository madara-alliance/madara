use crate::error::SettlementClientError;
use crate::messaging::MessageToL2WithMetadata;
use crate::starknet::error::StarknetClientError;
use bigdecimal::ToPrimitive;
use futures::{stream, Stream, StreamExt, TryStreamExt};
use mp_convert::FeltExt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use starknet::core::types::{BlockId, EmittedEvent, EventFilter};
use starknet::providers::{Provider, ProviderError};
use starknet_types_core::felt::Felt;
use std::iter;
use std::sync::Arc;
use std::time::Duration;

// Starknet event conversion
impl TryFrom<EmittedEvent> for MessageToL2WithMetadata {
    type Error = SettlementClientError;

    fn try_from(event: EmittedEvent) -> Result<Self, Self::Error> {
        // https://github.com/keep-starknet-strange/piltover/blob/a7dc4141fd21300f6d7c23b87d496004a739f430/src/messaging/component.cairo#L86-L96
        // keys: ['MessageSent', message_hash, from_address, to_address]
        // data: [selector, nonce, payload_len, payload[]...]

        let block_number = event.block_number.ok_or_else(|| {
            SettlementClientError::Starknet(StarknetClientError::EventProcessing {
                message: "Unable to get block number from event".to_string(),
                event_id: "MessageSent".to_string(),
            })
        })?;

        let selector = event.data.first().ok_or_else(|| {
            SettlementClientError::Starknet(StarknetClientError::EventProcessing {
                message: "Missing selector in event data".to_string(),
                event_id: "MessageSent".to_string(),
            })
        })?;

        let nonce = event.data.get(1).ok_or_else(|| {
            SettlementClientError::Starknet(StarknetClientError::EventProcessing {
                message: "Missing nonce in event data".to_string(),
                event_id: "MessageSent".to_string(),
            })
        })?;

        let from = event.keys.get(2).ok_or_else(|| {
            SettlementClientError::Starknet(StarknetClientError::EventProcessing {
                message: "Missing from_address in event keys".to_string(),
                event_id: "MessageSent".to_string(),
            })
        })?;

        let to = event.keys.get(3).ok_or_else(|| {
            SettlementClientError::Starknet(StarknetClientError::EventProcessing {
                message: "Missing to_address in event keys".to_string(),
                event_id: "MessageSent".to_string(),
            })
        })?;

        Ok(Self {
            l1_transaction_hash: event.transaction_hash.to_u256(),
            l1_block_number: block_number,
            message: L1HandlerTransactionWithFee::new(
                L1HandlerTransaction {
                    version: Felt::ZERO,
                    nonce: nonce.to_u64().ok_or_else(|| {
                        SettlementClientError::Starknet(StarknetClientError::EventProcessing {
                            message: "Nonce overflows u64".to_string(),
                            event_id: "MessageSent".to_string(),
                        })
                    })?,
                    contract_address: *to,
                    entry_point_selector: *selector,
                    calldata: iter::once(*from).chain(event.data.iter().skip(3).copied()).collect::<Vec<_>>().into(),
                },
                /* paid_fee_on_l1 */ 1, // TODO: we need the paid fee here.
            ),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WatchBlockNEvent {
    pub new: u64,
    pub previous: Option<u64>,
}

/// Watch the latest block_n. Everytime a change is detected, a `WatchBlockNEvent` is emitted with the previous and new block_n value.
pub fn watch_latest_block_n(
    provider: Arc<impl Provider + 'static>,
    from_block_n: Option<u64>,
    polling_interval: Duration,
) -> impl Stream<Item = Result<WatchBlockNEvent, ProviderError>> {
    let mut interval = tokio::time::interval(polling_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    stream::try_unfold((provider, interval, from_block_n), |(provider, mut interval, previous)| async move {
        interval.tick().await; // first tick is immediate

        let new = provider.block_number().await?;
        Ok(Some((WatchBlockNEvent {
            new,
            previous,
        }, (provider, interval, Some(new)))))
    })
    // only return events where the block_n changed
    .try_filter(|&ev| async move {
        ev.previous != Some(ev.new)
    })
}

/// Get all of the events matching a given filter.
pub fn get_events(
    provider: Arc<impl Provider + 'static>,
    filter: EventFilter,
    chunk_size: u64,
) -> impl Stream<Item = Result<EmittedEvent, ProviderError>> {
    stream::try_unfold((provider, filter, chunk_size,  None, false), |(provider, filter, chunk_size, continuation_token, stop)| async move {
        if stop {
            return Ok(None);
        }
        let res = provider.get_events(filter.clone(), continuation_token, chunk_size).await?;

        let stop = res.continuation_token.is_none(); // stop just after this chunk when no continuation token.
        Ok::<_, ProviderError>(Some((stream::iter(res.events).map(Ok), (provider, filter, chunk_size, res.continuation_token, stop))))
    })
    // flatten: convert stream of `Vec<EmittedEvent>` chunks into individual `EmittedEvent`s.
    .try_flatten()
}

pub struct WatchEventFilter {
    pub address: Option<Felt>,
    pub keys: Option<Vec<Vec<Felt>>>,
}
pub fn watch_events(
    provider: Arc<impl Provider + 'static>,
    from_block_n: Option<u64>,
    filter: WatchEventFilter,
    polling_interval: Duration,
    chunk_size: u64,
) -> impl Stream<Item = Result<EmittedEvent, ProviderError>> {
    // Watch the latest block_n, and everytime it changes, get all the events in the change range.
    watch_latest_block_n(provider.clone(), from_block_n, polling_interval)
        .map_ok(move |ev| {
            get_events(
                provider.clone(),
                EventFilter {
                    // add 1 to the previous block_n to start returning events from the block we don't know about. (EventFilter range is inclusive)
                    from_block: Some(BlockId::Number(ev.previous.map(|n| n.saturating_add(1)).unwrap_or(0))),
                    to_block: Some(BlockId::Number(ev.new)),
                    address: filter.address,
                    keys: filter.keys.clone(),
                },
                chunk_size,
            )
        })
        .try_flatten()
}

#[cfg(test)]
mod starknet_event_stream_tests {
    use super::*;
    use assert_matches::assert_matches;
    use futures::StreamExt;
    use httpmock::prelude::*;
    use httpmock::Mock;
    use rstest::*;
    use serde_json::json;
    use starknet::providers::jsonrpc::HttpTransport;
    use starknet::providers::JsonRpcClient;
    use std::str::FromStr;
    use tokio_util::time::FutureExt as _;
    use url::Url;

    struct MockStarknetServer {
        server: MockServer,
    }

    impl MockStarknetServer {
        fn new() -> Self {
            Self { server: MockServer::start() }
        }

        fn url(&self) -> String {
            self.server.base_url()
        }

        fn mock_get_events(&self, events: Vec<EmittedEvent>, continuation_token: Option<&str>) -> Mock {
            self.server.mock(|when, then| {
                when.method(POST).path("/").header("Content-Type", "application/json").matches(|req| {
                    let body = req.body.clone().unwrap();
                    let body_str = std::str::from_utf8(body.as_slice()).unwrap_or_default();
                    tracing::error!("{}", body_str);
                    body_str.contains("starknet_getEvents")
                });

                then.status(200).json_body(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "events": events,
                        "continuation_token": continuation_token
                    }
                }));
            })
        }

        fn mock_block_number(&self, block_number: u64) -> Mock {
            self.server.mock(|when, then| {
                when.method(POST).path("/").matches(|req| {
                    let body = req.body.clone().unwrap();
                    let body_str = std::str::from_utf8(body.as_slice()).unwrap_or_default();
                    body_str.contains("starknet_blockNumber")
                });

                then.status(200).json_body(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": block_number
                }));
            })
        }

        fn mock_error_response(&self, error_code: i64, error_message: &str) -> Mock {
            self.server.mock(|when, then| {
                when.method(POST).path("/");

                then.status(200).json_body(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": {
                        "code": error_code,
                        "message": error_message
                    }
                }));
            })
        }
    }

    #[fixture]
    fn mock_server() -> MockStarknetServer {
        MockStarknetServer::new()
    }

    #[fixture]
    fn test_event(#[default(1)] nonce: u64, #[default(100)] block_number: u64) -> EmittedEvent {
        EmittedEvent {
            from_address: Default::default(),
            transaction_hash: Felt::from_hex("0x1234").unwrap(),
            block_number: Some(block_number),
            block_hash: Some(Felt::from_hex("0x5678").unwrap()),
            data: vec![
                Felt::from_hex("0x3333").unwrap(),                  // selector
                Felt::from_hex(&format!("0x{:x}", nonce)).unwrap(), // nonce
                Felt::from_hex("0x5555").unwrap(),                  // len
                Felt::from_hex("0x6666").unwrap(),                  // payload[0]
                Felt::from_hex("0x7777").unwrap(),                  // payload[1]
            ],
            keys: vec![
                Felt::from_hex("0x0001").unwrap(), // event key
                Felt::from_hex("0x0001").unwrap(), // message_hash
                Felt::from_hex("0x1111").unwrap(), // from
                Felt::from_hex("0x2222").unwrap(), // to
            ],
        }
    }

    fn create_stream(
        mock_server: &MockStarknetServer,
    ) -> impl Stream<Item = Result<MessageToL2WithMetadata, SettlementClientError>> + 'static {
        let provider =
            JsonRpcClient::new(HttpTransport::new(Url::from_str(&mock_server.url()).expect("Failed to parse URL")));

        watch_events(
            Arc::new(provider),
            Some(0),
            WatchEventFilter { address: Some(Felt::from_hex("0x1").unwrap()), keys: Some(vec![]) },
            Duration::from_millis(100),
            /* chunk size */ 5,
        )
        .map_err(|e| SettlementClientError::from(StarknetClientError::Provider(format!("Provider error: {e:#}"))))
        .map(|r| r.and_then(MessageToL2WithMetadata::try_from))
        .boxed()
    }

    #[tokio::test]
    #[rstest]
    async fn test_single_event(mock_server: MockStarknetServer, test_event: EmittedEvent) {
        let events_mock = mock_server.mock_get_events(vec![test_event.clone()], None);
        mock_server.mock_block_number(101);

        let mut stream = Box::pin(create_stream(&mock_server));

        if let Some(Ok(event_data)) = stream.next().await {
            assert_eq!(event_data.l1_block_number, 100);
            assert_eq!(event_data.message.tx.calldata.len(), 3);
        } else {
            panic!("Expected successful event");
        }
        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );

        events_mock.assert_hits(1);
    }

    #[tokio::test]
    #[rstest]
    async fn test_multiple_pages(mock_server: MockStarknetServer) {
        let mut events_mock = mock_server.mock_get_events(vec![test_event(1, 150); 100], None);
        let mut block_n_mock = mock_server.mock_block_number(101);

        let mut stream = Box::pin(create_stream(&mock_server));

        for _ in 0..100 {
            assert_matches!(stream.next().await, Some(Ok(event_data)) => {
                assert_eq!(event_data.l1_block_number, 150);
                assert_eq!(event_data.message.tx.calldata.len(), 3);
            })
        }
        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );

        events_mock.delete();
        block_n_mock.delete();
        mock_server.mock_get_events(vec![test_event(1, 150); 100], None);
        mock_server.mock_block_number(254);

        for _ in 0..100 {
            assert_matches!(stream.next().await, Some(Ok(event_data)) => {
                assert_eq!(event_data.l1_block_number, 150);
                assert_eq!(event_data.message.tx.calldata.len(), 3);
            })
        }
        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );
    }

    #[tokio::test]
    #[rstest]
    async fn test_multiple_events_in_single_block(mock_server: MockStarknetServer) {
        let event1 = test_event(1, 100);
        let event2 = test_event(2, 100);

        mock_server.mock_get_events(vec![event1.clone(), event2.clone()], None);
        mock_server.mock_block_number(101);

        let mut stream = Box::pin(create_stream(&mock_server));

        if let Some(Ok(event_data1)) = stream.next().await {
            assert_eq!(event_data1.l1_block_number, 100);
            assert_eq!(event_data1.message.tx.nonce, 1);
            assert_eq!(event_data1.message.tx.calldata.len(), 3);
            assert_eq!(event_data1.l1_transaction_hash, event1.transaction_hash.to_u256());
        } else {
            panic!("Expected first event");
        }

        if let Some(Ok(event_data2)) = stream.next().await {
            assert_eq!(event_data2.l1_block_number, 100);
            assert_eq!(event_data2.message.tx.nonce, 2);
            assert_eq!(event_data2.message.tx.calldata.len(), 3);
            assert_eq!(event_data2.l1_transaction_hash, event2.transaction_hash.to_u256());
        } else {
            panic!("Expected second event");
        }

        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );
    }

    #[tokio::test]
    #[rstest]
    async fn test_error_handling(mock_server: MockStarknetServer) {
        let error_mock = mock_server.mock_error_response(-32000, "Internal error");

        let mut stream = Box::pin(create_stream(&mock_server));

        assert_matches!(stream.next().await, Some(Err(_)), "Expected error");

        error_mock.assert();
    }

    #[tokio::test]
    #[rstest]
    async fn test_empty_events(mock_server: MockStarknetServer) {
        mock_server.mock_get_events(vec![], None);
        mock_server.mock_block_number(100);

        let mut stream = Box::pin(create_stream(&mock_server));

        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );
    }

    #[tokio::test]
    #[rstest]
    async fn test_follows_chain(mock_server: MockStarknetServer) {
        let mut events_mock = mock_server.mock_get_events(vec![test_event(1, 100); 5], None);
        let mut block_n_mock = mock_server.mock_block_number(101);

        let mut stream = Box::pin(create_stream(&mock_server));
        for _ in 0..5 {
            assert_matches!(stream.next().await, Some(Ok(event_data)) => {
                assert_eq!(event_data.l1_block_number, 100);
                assert_eq!(event_data.message.tx.calldata.len(), 3);
            })
        }
        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );

        block_n_mock.delete();
        events_mock.delete();
        mock_server.mock_block_number(254);
        mock_server.mock_get_events(vec![test_event(1, 150); 100], None);

        for _ in 0..100 {
            assert_matches!(stream.next().await, Some(Ok(event_data)) => {
                assert_eq!(event_data.l1_block_number, 150);
                assert_eq!(event_data.message.tx.calldata.len(), 3);
            })
        }
        // should not find any more events
        assert_matches!(
            stream.next().timeout(Duration::from_secs(3)).await,
            Err(_),
            "Expected waiting after processing all events"
        );
    }
}
