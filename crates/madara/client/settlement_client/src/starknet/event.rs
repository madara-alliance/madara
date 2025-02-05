use crate::error::SettlementClientError;
use crate::messaging::CommonMessagingEventData;
use futures::Stream;
use log::error;
use starknet_core::types::{BlockId, EmittedEvent, EventFilter};
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::{JsonRpcClient, Provider};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::sleep;

type FutureType = Pin<Box<dyn Future<Output = anyhow::Result<(Option<EmittedEvent>, EventFilter)>> + Send>>;

pub struct StarknetEventStream {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    filter: EventFilter,
    processed_events: HashSet<Felt>,
    future: Option<FutureType>,
    polling_interval: Duration,
}

impl StarknetEventStream {
    pub fn new(provider: Arc<JsonRpcClient<HttpTransport>>, filter: EventFilter, polling_interval: Duration) -> Self {
        Self { provider, filter, processed_events: HashSet::new(), future: None, polling_interval }
    }
}

impl Stream for StarknetEventStream {
    type Item = Option<Result<CommonMessagingEventData, SettlementClientError>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.future.is_none() {
            let provider = self.provider.clone();
            let filter = self.filter.clone();
            let processed_events = self.processed_events.clone();
            let polling_interval = self.polling_interval;

            async fn fetch_events(
                provider: Arc<JsonRpcClient<HttpTransport>>,
                mut filter: EventFilter,
                processed_events: HashSet<Felt>,
                polling_interval: Duration,
            ) -> anyhow::Result<(Option<EmittedEvent>, EventFilter)> {
                // Adding sleep to introduce delay
                sleep(polling_interval).await;

                let mut event_vec = Vec::new();
                let mut page_indicator = false;
                let mut continuation_token: Option<String> = None;

                while !page_indicator {
                    let events = provider
                        .get_events(
                            EventFilter {
                                from_block: filter.from_block,
                                to_block: filter.to_block,
                                address: filter.address,
                                keys: filter.keys.clone(),
                            },
                            continuation_token.clone(),
                            1000,
                        )
                        .await?;

                    event_vec.extend(events.events);
                    if let Some(token) = events.continuation_token {
                        continuation_token = Some(token);
                    } else {
                        page_indicator = true;
                    }
                }

                let latest_block = provider.block_number().await?;

                for event in event_vec.clone() {
                    let event_nonce = event.data[1];
                    if !processed_events.contains(&event_nonce) {
                        return Ok((Some(event), filter));
                    }
                }

                filter.from_block = filter.to_block;
                filter.to_block = Some(BlockId::Number(latest_block));

                Ok((None, filter))
            }

            let future = async move {
                let (event, updated_filter) =
                    fetch_events(provider, filter, processed_events, polling_interval).await?;
                Ok((event, updated_filter))
            };

            self.future = Some(Box::pin(future));
        }

        match self.future.as_mut() {
            Some(fut) => match fut.as_mut().poll(cx) {
                Poll::Ready(result) => {
                    self.future = None;
                    match result {
                        Ok((Some(event), updated_filter)) => {
                            // Update the filter
                            self.filter = updated_filter;
                            // Insert the event nonce before returning
                            self.processed_events.insert(event.data[1]);

                            let event_data = event
                                .block_number
                                .ok_or_else(|| anyhow::anyhow!("Unable to get block number from event"))
                                .map(|block_number| CommonMessagingEventData {
                                    from: event.keys[2],
                                    to: event.keys[3],
                                    selector: event.data[0],
                                    nonce: event.data[1],
                                    payload: {
                                        let mut payload_array = vec![];
                                        event.data.iter().skip(3).for_each(|data| {
                                            payload_array.push(*data);
                                        });
                                        payload_array
                                    },
                                    fee: None,
                                    transaction_hash: event.transaction_hash,
                                    message_hash: Some(event.keys[1]),
                                    block_number,
                                    event_index: None,
                                });

                            match event_data {
                                Ok(data) => Poll::Ready(Some(Some(Ok(data)))),
                                Err(e) => Poll::Ready(Some(Some(Err(SettlementClientError::Other(e))))),
                            }
                        }
                        Ok((None, updated_filter)) => {
                            // Update the filter even when no events are found
                            self.filter = updated_filter;
                            Poll::Ready(Some(None))
                        }
                        Err(e) => Poll::Ready(Some(Some(Err(SettlementClientError::Other(e))))),
                    }
                }
                Poll::Pending => Poll::Pending,
            },
            None => {
                // If the code comes here then this is an unexpected behaviour.
                // Following scenarios can lead to this:
                // - Not able to call the RPC and fetch events.
                // - Connection Issues.
                error!("Starknet Event Stream : Unable to fetch events from starknet stream. Restart Sequencer.");
                Poll::Ready(Some(None))
            }
        }
    }
}

#[cfg(test)]
mod starknet_event_stream_tests {
    use super::*;
    use futures::StreamExt;
    use httpmock::prelude::*;
    use httpmock::Mock;
    use rstest::rstest;
    use serde_json::json;
    use std::str::FromStr;
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

    fn create_test_event(nonce: u64, block_number: u64) -> EmittedEvent {
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

    fn setup_stream(mock_server: &MockStarknetServer) -> StarknetEventStream {
        let provider = JsonRpcClient::new(HttpTransport::new(Url::from_str(&mock_server.url()).unwrap()));

        StarknetEventStream::new(
            Arc::new(provider),
            EventFilter {
                from_block: Some(BlockId::Number(0)),
                to_block: Some(BlockId::Number(100)),
                address: Some(Felt::from_hex("0x1").unwrap()),
                keys: Some(vec![]),
            },
            Duration::from_secs(1),
        )
    }

    #[tokio::test]
    #[rstest]
    async fn test_single_event() {
        let mock_server = MockStarknetServer::new();

        // Setup mocks
        let test_event = create_test_event(1, 100);
        let events_mock = mock_server.mock_get_events(vec![test_event], None);
        let block_mock = mock_server.mock_block_number(101);

        let mut stream = Box::pin(setup_stream(&mock_server));

        if let Some(Some(Ok(event_data))) = stream.next().await {
            assert_eq!(event_data.block_number, 100);
            assert!(event_data.message_hash.is_some());
            assert_eq!(event_data.payload.len(), 2);
        } else {
            panic!("Expected successful event");
        }

        // Verify mocks were called
        events_mock.assert();
        events_mock.assert();
        block_mock.assert();
    }

    #[tokio::test]
    #[rstest]
    async fn test_error_handling() {
        let mock_server = MockStarknetServer::new();

        let error_mock = mock_server.mock_error_response(-32000, "Internal error");

        let mut stream = Box::pin(setup_stream(&mock_server));

        match stream.next().await {
            Some(Some(Err(e))) => {
                assert!(e.to_string().contains("Internal error"));
            }
            _ => panic!("Expected error"),
        }

        error_mock.assert();
    }

    #[tokio::test]
    #[rstest]
    async fn test_empty_events() {
        let mock_server = MockStarknetServer::new();

        let events_mock = mock_server.mock_get_events(vec![], None);
        let block_mock = mock_server.mock_block_number(100);

        let mut stream = Box::pin(setup_stream(&mock_server));

        match stream.next().await {
            Some(None) => { /* Expected */ }
            _ => panic!("Expected None for empty events"),
        }

        events_mock.assert();
        block_mock.assert();
    }

    #[tokio::test]
    #[rstest]
    async fn test_multiple_events_in_single_block() {
        let mock_server = MockStarknetServer::new();

        // Create two events with different nonces but same block number
        let test_event1 = create_test_event(1, 100);
        let test_event2 = create_test_event(2, 100);

        // Setup mocks for events and block number
        let events_mock = mock_server.mock_get_events(vec![test_event1.clone(), test_event2.clone()], None);
        let block_mock = mock_server.mock_block_number(101);

        let mut stream = Box::pin(setup_stream(&mock_server));

        if let Some(Some(Ok(event_data1))) = stream.next().await {
            assert_eq!(event_data1.block_number, 100);
            assert_eq!(event_data1.nonce, Felt::from_hex("0x1").unwrap());
            assert_eq!(event_data1.payload.len(), 2);
            assert_eq!(event_data1.transaction_hash, test_event1.transaction_hash);
        } else {
            panic!("Expected first event");
        }

        if let Some(Some(Ok(event_data2))) = stream.next().await {
            assert_eq!(event_data2.block_number, 100);
            assert_eq!(event_data2.nonce, Felt::from_hex("0x2").unwrap());
            assert_eq!(event_data2.payload.len(), 2);
            assert_eq!(event_data2.transaction_hash, test_event2.transaction_hash);
        } else {
            panic!("Expected second event");
        }

        // Verify that there are no more events
        match stream.next().await {
            Some(None) => { /* Expected */ }
            _ => panic!("Expected None after processing all events"),
        }

        // Verify mocks were called
        events_mock.assert();
        block_mock.assert();
    }
}
