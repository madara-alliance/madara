//! Starknet event streaming implementation.
//!
//! This module provides functionality to stream Starknet events, particularly focused on
//! L1-to-L2 messaging events. It implements a Stream that continuously polls for new events
//! from a Starknet provider and processes them to extract L1-to-L2 messaging data.
//!
//! The main component is the [`StarknetEventStream`] which:
//! - Connects to a Starknet node via JSON-RPC
//! - Periodically polls for new events matching a specified filter
//! - Processes and deduplicates events based on their nonce
//! - Converts Starknet events into a more usable [`L1toL2MessagingEventData`] structure
//!
//! This is primarily used for cross-chain messaging between Starknet (L2) and Madara-Appchain (L3),
//! allowing the client to track messages sent from L2 that need to be processed on L3.
//!
//! PS: the naming might be a bit confusing because even though the events are coming from L2 and yet
//! we are using the term "L1-to-L2 messaging" in the name. This is temporary and will be changed
//! in the future.

use crate::error::SettlementClientError;
use crate::messaging::L1toL2MessagingEventData;
use crate::starknet::error::StarknetClientError;
use futures::Stream;
use starknet_core::types::{BlockId, EmittedEvent, EventFilter, EventsPage};
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

/// Type alias for the future returned by the event fetching function.
type FutureType = Pin<Box<dyn Future<Output = anyhow::Result<(Option<EmittedEvent>, EventFilter)>> + Send>>;

/// Stream implementation for retrieving Starknet events.
///
/// `StarknetEventStream` connects to a Starknet node and continuously polls for new events
/// matching the provided filter. It keeps track of processed events to avoid duplication
/// and implements the `Stream` trait to provide an asynchronous stream of events.
///
/// The stream will:
/// 1. Fetch events from the Starknet node based on the provided filter
/// 2. Process each event and check if it has already been processed (using the nonce)
/// 3. Convert valid events to [`L1toL2MessagingEventData`]
/// 4. Update the filter to look for newer blocks in subsequent requests
///
/// PS: As of now the event stream is for L1-to-L2 messaging events.
pub struct StarknetEventStream {
    /// The Starknet JSON-RPC client used to fetch events.
    provider: Arc<JsonRpcClient<HttpTransport>>,

    /// The filter used to specify which events to retrieve.
    filter: EventFilter,

    /// Set of event nonces that have already been processed, used for deduplication.
    processed_events: HashSet<Felt>,

    /// The current future being polled (if any).
    future: Option<FutureType>,

    /// Time interval between consecutive polling attempts.
    polling_interval: Duration,
}

impl StarknetEventStream {
    /// Creates a new `StarknetEventStream` with the specified provider, filter, and polling interval.
    ///
    /// # Arguments
    ///
    /// * `provider` - Arc-wrapped Starknet JSON-RPC client to use for fetching events
    /// * `filter` - Event filter that specifies starting block and ending block, event keys are fixed (Because we are fetching L1-to-L2 messaging events only)
    /// * `polling_interval` - Time interval between consecutive polling attempts
    ///
    /// # Returns
    ///
    /// A new `StarknetEventStream` instance configured with the provided parameters.
    pub fn new(provider: Arc<JsonRpcClient<HttpTransport>>, filter: EventFilter, polling_interval: Duration) -> Self {
        Self { provider, filter, processed_events: HashSet::new(), future: None, polling_interval }
    }

    async fn fetch_events_with_retry(
        provider: &Arc<JsonRpcClient<HttpTransport>>,
        filter: EventFilter,
        continuation_token: Option<String>,
    ) -> anyhow::Result<EventsPage> {
        const MAX_RETRIES: u8 = 3;
        const RETRY_TIMEOUT: u64 = 5;
        let mut retries = 0;
        loop {
            match provider
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
                .await
            {
                Ok(res) => {
                    return Ok(res);
                }
                Err(e) => {
                    if retries < MAX_RETRIES {
                        retries += 1;
                        tracing::warn!("Error in fetch_events, retrying: {:?}", e);
                        sleep(Duration::from_secs(RETRY_TIMEOUT)).await;
                        continue;
                    } else {
                        tracing::error!("Error in fetch_events, {:?}", e);
                        return Err(anyhow::anyhow!("Starknet error: {}", e));
                    }
                }
            }
        }
    }

    /// Fetches events from the Starknet provider based on the provided filter.
    ///
    /// This method implements a stateful polling mechanism that:
    /// 1. Sleeps for the configured polling interval to avoid overwhelming the Starknet node
    /// 2. Fetches events from the Starknet provider using pagination (up to 1000 events per request)
    /// 3. Processes events to find unprocessed ones (based on nonce)
    /// 4. Updates the filter to implement a sliding window approach for continuous monitoring
    ///
    /// ## Detailed Logic
    ///
    /// The function follows these steps:
    /// - Introduces a delay based on the polling interval to avoid overwhelming the Starknet node
    /// - Retrieves all events matching the filter using pagination (handling the continuation token)
    /// - Fetches the latest block number from the Starknet node
    /// - Iterates through the retrieved events looking for ones with unprocessed nonces
    /// - For the first unprocessed event found, marks its nonce as processed and returns it
    /// - If no unprocessed events are found, updates the filter window to move forward
    ///   (from_block becomes the previous to_block, to_block becomes the latest block)
    ///
    /// This sliding window approach ensures we continuously monitor new blocks while avoiding
    /// reprocessing old events, even if the function is called repeatedly.
    ///
    /// ## Deduplication Strategy
    ///
    /// Events are deduplicated based on their nonce (second element in the event data).
    /// The function keeps track of processed nonces in a HashSet to ensure each message
    /// is processed exactly once, which is critical for cross-chain messaging systems.
    ///
    /// # Arguments
    ///
    /// * `provider` - Arc-wrapped Starknet JSON-RPC client
    /// * `filter` - Event filter specifying the block range to query
    /// * `processed_events` - Set of already processed event nonces for deduplication
    /// * `polling_interval` - Time interval between consecutive polling attempts
    ///
    /// # Returns
    ///
    /// A tuple containing:
    /// - An `Option<EmittedEvent>` with the first unprocessed event (if any)
    /// - The updated `EventFilter` for the next polling attempt, with an adjusted block range
    ///   that slides forward to capture new blocks
    async fn fetch_l1_to_l2_messaging_events(
        provider: Arc<JsonRpcClient<HttpTransport>>,
        mut filter: EventFilter,
        mut processed_events: HashSet<Felt>,
        polling_interval: Duration,
    ) -> anyhow::Result<(Option<EmittedEvent>, EventFilter)> {
        // Adding sleep to introduce delay
        sleep(polling_interval).await;

        let mut page_indicator = false;
        let mut continuation_token: Option<String> = None;

        while !page_indicator {
            let events = Self::fetch_events_with_retry(&provider, filter.clone(), continuation_token.clone()).await?;

            // Process this page of events immediately
            for event in &events.events {
                if let Some(nonce) = event.data.get(1) {
                    if !processed_events.contains(nonce) {
                        processed_events.insert(*nonce);
                        return Ok((Some(event.clone()), filter));
                    }
                }
            }

            // Continue pagination logic
            if let Some(token) = events.continuation_token {
                continuation_token = Some(token);
            } else {
                page_indicator = true;
            }
        }

        // If we get here, we didn't find any unprocessed events
        // So we update the filter and return None
        let latest_block = provider.block_number().await?;
        filter.from_block = filter.to_block;
        filter.to_block = Some(BlockId::Number(latest_block));

        Ok((None, filter))
    }
}

/// Implementation of the `Stream` trait for `StarknetEventStream`.
///
/// This implementation allows `StarknetEventStream` to be used as an asynchronous
/// stream of events. It handles polling for new events, processing them, and
/// converting them to `L1toL2MessagingEventData`.
///
/// The stream yields `Result<L1toL2MessagingEventData, SettlementClientError>` items,
/// where:
/// - `Ok(data)` represents a successfully processed event
/// - `Err(error)` represents an error that occurred during event fetching or processing
impl Stream for StarknetEventStream {
    type Item = Result<L1toL2MessagingEventData, SettlementClientError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.future.is_none() {
            let provider = self.provider.clone();
            let filter = self.filter.clone();
            let processed_events = self.processed_events.clone();
            let polling_interval = self.polling_interval;

            let future = async move {
                Self::fetch_l1_to_l2_messaging_events(provider, filter, processed_events, polling_interval).await
            };

            self.future = Some(Box::pin(future));
        }

        match self.future.as_mut() {
            Some(fut) => match fut.as_mut().poll(cx) {
                Poll::Ready(result) => {
                    self.future = None;
                    match result {
                        Ok((Some(event), updated_filter)) => {
                            self.filter = updated_filter;
                            let nonce = match event.data.get(1) {
                                Some(nonce) => *nonce,
                                None => {
                                    return Poll::Ready(Some(Err(SettlementClientError::Starknet(
                                        StarknetClientError::EventProcessing {
                                            message: "Missing nonce in event data".to_string(),
                                            event_id: "MessageSent".to_string(),
                                        },
                                    ))))
                                }
                            };
                            self.processed_events.insert(nonce);

                            match L1toL2MessagingEventData::try_from(event) {
                                Ok(data) => Poll::Ready(Some(Ok(data))),
                                Err(e) => Poll::Ready(Some(Err(e))),
                            }
                        }
                        Ok((None, updated_filter)) => {
                            // Update the filter even when no events are found
                            self.filter = updated_filter;
                            Poll::Ready(None)
                        }
                        Err(e) => Poll::Ready(Some(Err(SettlementClientError::Starknet(
                            StarknetClientError::Provider(e.to_string()),
                        )))),
                    }
                }
                Poll::Pending => Poll::Pending,
            },
            None => {
                // If the code comes here then this is an unexpected behaviour.
                // Following scenarios can lead to this:
                // - Not able to call the RPC and fetch events.
                // - Connection Issues.
                tracing::error!(
                    "Starknet Event Stream : Unable to fetch events from starknet stream. Restart Sequencer."
                );
                Poll::Ready(None)
            }
        }
    }
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

    fn create_stream(mock_server: &MockStarknetServer) -> StarknetEventStream {
        let provider =
            JsonRpcClient::new(HttpTransport::new(Url::from_str(&mock_server.url()).expect("Failed to parse URL")));

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
    async fn test_single_event(mock_server: MockStarknetServer, test_event: EmittedEvent) {
        let events_mock = mock_server.mock_get_events(vec![test_event.clone()], None);
        let block_mock = mock_server.mock_block_number(101);

        let mut stream = Box::pin(create_stream(&mock_server));

        if let Some(Ok(event_data)) = stream.next().await {
            assert_eq!(event_data.block_number, 100);
            assert!(event_data.message_hash.is_some());
            assert_eq!(event_data.payload.len(), 2);
        } else {
            panic!("Expected successful event");
        }
        assert_matches!(stream.next().await, None, "Expected None for empty events");

        events_mock.assert_hits(2);
        block_mock.assert_hits(1);
    }

    #[tokio::test]
    #[rstest]
    async fn test_multiple_events_in_single_block(mock_server: MockStarknetServer) {
        let event1 = test_event(1, 100);
        let event2 = test_event(2, 100);

        let events_mock = mock_server.mock_get_events(vec![event1.clone(), event2.clone()], None);
        let block_mock = mock_server.mock_block_number(101);

        let mut stream = Box::pin(create_stream(&mock_server));

        if let Some(Ok(event_data1)) = stream.next().await {
            assert_eq!(event_data1.block_number, 100);
            assert_eq!(event_data1.nonce, Felt::from_hex("0x1").unwrap());
            assert_eq!(event_data1.payload.len(), 2);
            assert_eq!(event_data1.transaction_hash, event1.transaction_hash);
        } else {
            panic!("Expected first event");
        }

        if let Some(Ok(event_data2)) = stream.next().await {
            assert_eq!(event_data2.block_number, 100);
            assert_eq!(event_data2.nonce, Felt::from_hex("0x2").unwrap());
            assert_eq!(event_data2.payload.len(), 2);
            assert_eq!(event_data2.transaction_hash, event2.transaction_hash);
        } else {
            panic!("Expected second event");
        }

        assert_matches!(stream.next().await, None, "Expected None after processing all events");

        events_mock.assert_hits(3);
        block_mock.assert_hits(1);
    }

    #[tokio::test]
    #[rstest]
    async fn test_error_handling(mock_server: MockStarknetServer) {
        let error_mock = mock_server.mock_error_response(-32000, "Internal error");

        let mut stream = Box::pin(create_stream(&mock_server));

        // The stream should return an error after max retries
        assert_matches!(stream.next().await, Some(Err(_)), "Expected error after max retries");

        // Verify that the error mock was called (max retries exceeded)
        error_mock.assert_hits(4);
    }

    #[tokio::test]
    #[rstest]
    async fn test_empty_events(mock_server: MockStarknetServer) {
        let events_mock = mock_server.mock_get_events(vec![], None);
        let block_mock = mock_server.mock_block_number(100);

        let mut stream = Box::pin(create_stream(&mock_server));

        assert_matches!(stream.next().await, None, "Expected None for empty events");

        events_mock.assert();
        block_mock.assert();
    }
}
