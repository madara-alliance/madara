use crate::error::SettlementClientError;
use crate::eth::error::EthereumClientError;
use crate::eth::StarknetCoreContract::LogMessageToL2;
use crate::messaging::MessageToL2WithMetadata;
use alloy::contract::EventPoller;
use alloy::rpc::types::Log;
use alloy::transports::http::{Client, Http};
use futures::Stream;
use mp_convert::{Felt, ToFelt};
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use std::iter;
use std::pin::Pin;
use std::task::{Context, Poll};

// Event conversion
impl TryFrom<(LogMessageToL2, Log)> for MessageToL2WithMetadata {
    type Error = SettlementClientError;

    fn try_from((event, log): (LogMessageToL2, Log)) -> Result<Self, Self::Error> {
        Ok(Self {
            l1_block_number: log.block_number.ok_or_else(|| -> SettlementClientError {
                EthereumClientError::Conversion("No block number for event".to_string()).into()
            })?,
            l1_transaction_hash: log
                .transaction_hash
                .ok_or_else(|| -> SettlementClientError {
                    EthereumClientError::Conversion("No transaction hash for event".to_string()).into()
                })?
                .into(),
            message: event.try_into()?,
        })
    }
}
impl TryFrom<LogMessageToL2> for L1HandlerTransactionWithFee {
    type Error = SettlementClientError;

    fn try_from(event: LogMessageToL2) -> Result<Self, Self::Error> {
        Ok(Self::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: event.nonce.try_into().map_err(|_| -> SettlementClientError {
                    EthereumClientError::Conversion("Nonce value too large for u64 conversions".to_string()).into()
                })?,
                contract_address: event.toAddress.to_felt(),
                entry_point_selector: event.selector.to_felt(),
                calldata: iter::once(Felt::from_bytes_be_slice(event.fromAddress.as_slice()))
                    .chain(event.payload.into_iter().map(ToFelt::to_felt))
                    .collect(),
            },
            event.fee.try_into().map_err(|_| -> SettlementClientError {
                EthereumClientError::Conversion("Fee value too large for u128 conversion".to_string()).into()
            })?,
        ))
    }
}

type EthereumStreamItem = Result<(LogMessageToL2, Log), alloy::sol_types::Error>;
type EthereumStreamType = Pin<Box<dyn Stream<Item = EthereumStreamItem> + Send + 'static>>;

pub struct EthereumEventStream {
    pub stream: EthereumStreamType,
}

impl EthereumEventStream {
    pub fn new(watcher: EventPoller<Http<Client>, LogMessageToL2>) -> Self {
        let stream = watcher.into_stream();
        Self { stream: Box::pin(stream) }
    }
}

impl Stream for EthereumEventStream {
    type Item = Result<MessageToL2WithMetadata, SettlementClientError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Some(res) = futures::ready!(self.stream.as_mut().poll_next(cx)) else { return Poll::Ready(None) };

        Poll::Ready(Some(
            res.map_err(|e| {
                SettlementClientError::Ethereum(EthereumClientError::EventStream {
                    message: format!("Error processing Ethereum event stream: {}", e),
                })
            })
            .and_then(MessageToL2WithMetadata::try_from),
        ))
    }
}

#[cfg(test)]
pub mod eth_event_stream_tests {
    use super::*;
    use alloy::primitives::{Address, LogData, B256, U256};
    use assert_matches::assert_matches;
    use futures::stream::iter;
    use futures::StreamExt;
    use rstest::*;
    use std::str::FromStr;

    #[fixture]
    fn mock_event(#[default(1)] index: u64) -> LogMessageToL2 {
        LogMessageToL2 {
            fromAddress: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            toAddress: U256::from(index),
            selector: U256::from(2u64),
            fee: U256::from(1000u64 * index),
            nonce: U256::from(index),
            payload: vec![U256::from(index), U256::from(2u64)],
        }
    }

    #[fixture]
    fn mock_log(#[default(1)] index: u64) -> Log {
        Log {
            inner: alloy::primitives::Log {
                address: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
                data: LogData::default(),
            },
            block_hash: Some(
                B256::from_str("0x0000000000000000000000000000000000000000000000000000000000000002").unwrap(),
            ),
            block_number: Some(100 + index),
            block_timestamp: Some(1643234567 + index),
            transaction_hash: Some(
                B256::from_str("0x0000000000000000000000000000000000000000000000000000000000000003").unwrap(),
            ),
            transaction_index: Some(index),
            log_index: Some(index),
            removed: false,
        }
    }

    // Helper function to process stream into a vector
    async fn collect_stream_events(
        stream: &mut EthereumEventStream,
    ) -> Vec<Result<MessageToL2WithMetadata, SettlementClientError>> {
        stream.collect::<Vec<_>>().await
    }

    #[rstest]
    #[tokio::test]
    async fn test_successful_event_stream(#[values(1, 2)] first_index: u64, #[values(3, 4)] second_index: u64) {
        let mock_events = vec![
            Ok((mock_event(first_index), mock_log(first_index))),
            Ok((mock_event(second_index), mock_log(second_index))),
        ];
        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let events = collect_stream_events(&mut ethereum_stream).await;

        assert_eq!(events.len(), 2);

        // Test first event
        assert_matches!(events[0].as_ref(), Ok(event_data) => {
            assert_eq!(event_data.l1_block_number, 100 + first_index);
            assert_eq!(event_data.message.tx.nonce, first_index);
        });

        // Test second event
        assert_matches!(events[1].as_ref(), Ok(event_data) => {
            assert_eq!(event_data.l1_block_number, 100 + second_index);
            assert_eq!(event_data.message.tx.nonce, second_index);
        });
    }

    #[tokio::test]
    async fn test_empty_stream() {
        let mock_events: Vec<Result<(LogMessageToL2, Log), alloy::sol_types::Error>> = vec![];
        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let event = ethereum_stream.next().await;
        assert_matches!(event, None, "Expected end of stream");
    }

    #[tokio::test]
    async fn test_error_handling() {
        let mock_events = vec![Err(alloy::sol_types::Error::InvalidLog { name: "", log: Box::default() })];
        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let event = ethereum_stream.next().await;
        assert_matches!(event, Some(Err(_)), "Expected error event");
    }

    #[rstest]
    #[tokio::test]
    async fn test_mixed_events(mock_event: LogMessageToL2, mock_log: Log) {
        let mock_events = vec![
            Ok((mock_event.clone(), mock_log.clone())),
            Err(alloy::sol_types::Error::InvalidLog { name: "", log: Box::default() }),
            Ok((mock_event, mock_log)),
        ];

        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };

        let events = collect_stream_events(&mut ethereum_stream).await;

        assert_eq!(events.len(), 3);
        assert!(events[0].as_ref().is_ok(), "First event should be successful");
        assert_matches!(
            events[1].as_ref(),
            Err(SettlementClientError::Ethereum(EthereumClientError::EventStream { .. })),
            "Second event should be an error"
        );
        assert!(events[2].as_ref().is_ok(), "Third event should be successful");
    }

    #[rstest]
    #[tokio::test]
    async fn test_missing_block_number(mock_event: LogMessageToL2, mock_log: Log) {
        let mock_events = vec![Ok((
            mock_event,
            Log {
                block_number: None, // Only block number is missing
                ..mock_log
            },
        ))];

        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };
        let events = collect_stream_events(&mut ethereum_stream).await;

        assert_eq!(events.len(), 1);
        assert_matches!(events[0].as_ref(), Err(SettlementClientError::Ethereum(EthereumClientError::MissingField(field))) => {
            assert_eq!(*field, "block_number in Ethereum log", "Error should mention missing block number");
        });
    }

    #[rstest]
    #[tokio::test]
    async fn test_missing_log_index(mock_event: LogMessageToL2, mock_log: Log) {
        let mock_events = vec![Ok((
            mock_event,
            Log {
                log_index: None, // Only log index is missing
                ..mock_log
            },
        ))];

        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };
        let events = collect_stream_events(&mut ethereum_stream).await;

        assert_eq!(events.len(), 1);
        assert_matches!(events[0].as_ref(), Err(SettlementClientError::Ethereum(EthereumClientError::MissingField(field))) => {
            assert_eq!(*field, "log_index in Ethereum log", "Error should mention missing log index");
        });
    }

    #[rstest]
    #[tokio::test]
    async fn test_missing_transaction_hash(mock_event: LogMessageToL2, mock_log: Log) {
        let mock_events = vec![Ok((
            mock_event,
            Log {
                transaction_hash: None, // Only transaction hash is missing
                ..mock_log
            },
        ))];

        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };
        let events = collect_stream_events(&mut ethereum_stream).await;

        assert_eq!(events.len(), 1);
        assert_matches!(events[0].as_ref(), Err(SettlementClientError::Ethereum(EthereumClientError::MissingField(field))) => {
            assert_eq!(*field, "transaction_hash in Ethereum log", "Error should mention missing transaction hash");
        });
    }
}
