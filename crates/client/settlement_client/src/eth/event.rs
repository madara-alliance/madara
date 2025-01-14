use crate::eth::StarknetCoreContract::LogMessageToL2;
use crate::messaging::CommonMessagingEventData;
use alloy::contract::EventPoller;
use alloy::rpc::types::Log;
use alloy::transports::http::{Client, Http};
use anyhow::Error;
use futures::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

type StreamItem = Result<(LogMessageToL2, Log), alloy::sol_types::Error>;
type StreamType = Pin<Box<dyn Stream<Item = StreamItem> + Send + 'static>>;

pub struct EthereumEventStream {
    stream: StreamType,
}

impl EthereumEventStream {
    pub fn new(watcher: EventPoller<Http<Client>, LogMessageToL2>) -> Self {
        let stream = watcher.into_stream();
        Self { stream: Box::pin(stream) }
    }
}

impl Stream for EthereumEventStream {
    type Item = Option<anyhow::Result<CommonMessagingEventData>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(result)) => match result {
                Ok((event, log)) => Poll::Ready(Some(Some(Ok(CommonMessagingEventData {
                    from: event.fromAddress.as_slice().into(),
                    to: event.toAddress.to_be_bytes_vec(),
                    selector: event.selector.to_be_bytes_vec(),
                    nonce: event.nonce.to_be_bytes_vec(),
                    payload: {
                        let mut payload_vec = vec![];
                        event.payload.iter().for_each(|ele| payload_vec.push(ele.to_be_bytes_vec()));
                        payload_vec
                    },
                    fee: Some(event.fee.to_be_bytes_vec()),
                    transaction_hash: log.transaction_hash.expect("Missing transaction hash").to_vec(),
                    message_hash: None,
                    block_number: log.block_number.expect("Missing block number"),
                    event_index: Some(log.log_index.expect("Missing log index")),
                })))),
                Err(e) => Poll::Ready(Some(Some(Err(Error::from(e))))),
            },
            Poll::Ready(None) => Poll::Ready(Some(None)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod eth_event_stream_tests {
    use super::*;
    use alloy::primitives::{Address, LogData, B256, U256};
    use futures::stream::iter;
    use futures::StreamExt;
    use rstest::rstest;
    use std::str::FromStr;

    // Helper function to create mock event
    fn create_mock_event() -> LogMessageToL2 {
        LogMessageToL2 {
            fromAddress: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            toAddress: U256::from(1u64),
            selector: U256::from(2u64),
            fee: U256::from(1000u64),
            nonce: U256::from(1u64),
            payload: vec![U256::from(1u64), U256::from(2u64)],
        }
    }

    // Helper function to create mock log
    fn create_mock_log() -> Log {
        Log {
            inner: alloy::primitives::Log {
                address: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
                data: LogData::default(),
            },
            block_hash: Some(
                B256::from_str("0x0000000000000000000000000000000000000000000000000000000000000002").unwrap(),
            ),
            block_number: Some(100),
            block_timestamp: Some(1643234567),
            transaction_hash: Some(
                B256::from_str("0x0000000000000000000000000000000000000000000000000000000000000003").unwrap(),
            ),
            transaction_index: Some(0),
            log_index: Some(0),
            removed: false,
        }
    }

    #[tokio::test]
    #[rstest]
    async fn test_successful_event_stream() {
        // Create a sequence of mock events
        let mock_events =
            vec![Ok((create_mock_event(), create_mock_log())), Ok((create_mock_event(), create_mock_log()))];

        // Create a mock stream from the events
        let mock_stream = iter(mock_events);

        // Create EthereumEventStream with mock stream
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let mut events = Vec::new();

        while let Some(Some(event)) = ethereum_stream.next().await {
            events.push(event);
        }

        assert_eq!(events.len(), 2);

        // Verify first event
        match &events[0] {
            Ok(event_data) => {
                assert_eq!(event_data.block_number, 100);
                assert_eq!(event_data.event_index, Some(0u64));
                // Add more assertions as needed
            }
            _ => panic!("Expected successful event"),
        }
    }

    #[tokio::test]
    #[rstest]
    async fn test_error_handling() {
        // Create a stream with an error
        let mock_events = vec![Err(alloy::sol_types::Error::InvalidLog { name: "", log: Box::default() })];

        let mock_stream = iter(mock_events);

        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let event = ethereum_stream.next().await.unwrap();

        match event {
            Some(Err(_)) => { /* Test passed */ }
            _ => panic!("Expected error event"),
        }
    }

    #[tokio::test]
    #[rstest]
    async fn test_empty_stream() {
        // Create an empty stream
        let mock_events: Vec<Result<(LogMessageToL2, Log), alloy::sol_types::Error>> = vec![];
        let mock_stream = iter(mock_events);

        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let event = ethereum_stream.next().await;

        assert!(event.unwrap().is_none(), "Expected None for empty stream");
    }

    #[tokio::test]
    #[rstest]
    async fn test_mixed_events() {
        // Create a stream with mixed success and error events
        let mock_events = vec![
            Ok((create_mock_event(), create_mock_log())),
            Err(alloy::sol_types::Error::InvalidLog { name: "", log: Box::default() }),
            Ok((create_mock_event(), create_mock_log())),
        ];

        let mock_stream = iter(mock_events);

        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let mut events = Vec::new();

        while let Some(Some(event)) = ethereum_stream.next().await {
            events.push(event);
        }

        assert_eq!(events.len(), 3);

        // Verify event sequence
        match &events[0] {
            Ok(_) => {}
            _ => panic!("First event should be successful"),
        }

        match &events[1] {
            Err(_) => {}
            _ => panic!("Second event should be an error"),
        }

        match &events[2] {
            Ok(_) => {}
            _ => panic!("Third event should be successful"),
        }
    }
}
