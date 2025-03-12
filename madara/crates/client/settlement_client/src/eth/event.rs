use crate::error::SettlementClientError;
use crate::eth::error::EthereumClientError;
use crate::eth::StarknetCoreContract::LogMessageToL2;
use crate::messaging::L1toL2MessagingEventData;
use crate::utils::u256_to_felt;
use alloy::contract::EventPoller;
use alloy::rpc::types::Log;
use alloy::transports::http::{Client, Http};
use futures::Stream;
use starknet_types_core::felt::Felt;
use std::pin::Pin;
use std::task::{Context, Poll};
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
    type Item = Result<L1toL2MessagingEventData, SettlementClientError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(result)) => match result {
                Ok((event, log)) => {
                    let event_data = (|| -> Result<L1toL2MessagingEventData, SettlementClientError> {
                        Ok(L1toL2MessagingEventData {
                            from: Felt::from_bytes_be_slice(event.fromAddress.as_slice()),
                            to: u256_to_felt(event.toAddress).map_err(|e| -> SettlementClientError {
                                EthereumClientError::Conversion(format!("Failed to convert toAddress to Felt: {}", e))
                                    .into()
                            })?,
                            selector: u256_to_felt(event.selector).map_err(|e| -> SettlementClientError {
                                EthereumClientError::Conversion(format!("Failed to convert selector to Felt: {}", e))
                                    .into()
                            })?,
                            nonce: u256_to_felt(event.nonce).map_err(|e| -> SettlementClientError {
                                EthereumClientError::Conversion(format!("Failed to convert nonce to Felt: {}", e))
                                    .into()
                            })?,
                            payload: event.payload.iter().try_fold(
                                Vec::with_capacity(event.payload.len()),
                                |mut acc, ele| -> Result<Vec<Felt>, SettlementClientError> {
                                    acc.push(u256_to_felt(*ele).map_err(|e| -> SettlementClientError {
                                        EthereumClientError::Conversion(format!(
                                            "Failed to convert payload element to Felt: {}",
                                            e
                                        ))
                                        .into()
                                    })?);
                                    Ok(acc)
                                },
                            )?,
                            fee: Some(event.fee.try_into().map_err(|_| -> SettlementClientError {
                                EthereumClientError::Conversion("Fee value too large for u128 conversion".to_string())
                                    .into()
                            })?),
                            transaction_hash: Felt::from_bytes_be_slice(
                                log.transaction_hash
                                    .ok_or_else(|| -> SettlementClientError {
                                        EthereumClientError::MissingField("transaction_hash in Ethereum log").into()
                                    })?
                                    .to_vec()
                                    .as_slice(),
                            ),
                            message_hash: None,
                            block_number: log.block_number.ok_or_else(|| -> SettlementClientError {
                                EthereumClientError::MissingField("block_number in Ethereum log").into()
                            })?,
                            event_index: Some(log.log_index.ok_or_else(|| -> SettlementClientError {
                                EthereumClientError::MissingField("log_index in Ethereum log").into()
                            })?),
                        })
                    })();

                    Poll::Ready(Some(event_data))
                }
                Err(e) => Poll::Ready(Some(Err(SettlementClientError::Ethereum(EthereumClientError::EventStream {
                    message: format!("Error processing Ethereum event stream: {}", e),
                })))),
            },
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
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
    ) -> Vec<Result<L1toL2MessagingEventData, SettlementClientError>> {
        stream
            .fold(Vec::new(), |mut acc, event| async move {
                acc.push(event);
                acc
            })
            .await
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
            assert_eq!(event_data.block_number, 100 + first_index);
            assert_eq!(event_data.event_index, Some(first_index));
            assert_eq!(event_data.nonce, Felt::from_bytes_be_slice(U256::from(first_index).to_be_bytes_vec().as_slice()));
        });

        // Test second event
        assert_matches!(events[1].as_ref(), Ok(event_data) => {
            assert_eq!(event_data.block_number, 100 + second_index);
            assert_eq!(event_data.event_index, Some(second_index));
            assert_eq!(event_data.nonce, Felt::from_bytes_be_slice(U256::from(second_index).to_be_bytes_vec().as_slice()));
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
