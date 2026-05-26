use crate::error::SettlementClientError;
use crate::eth::error::EthereumClientError;
use crate::eth::StarknetCoreContract::LogMessageToL2;
use crate::messaging::MessageToL2WithMetadata;
use alloy::contract::EventPoller;
use alloy::rpc::types::Log;
use alloy::sol_types::SolEvent;
use futures::{Stream, StreamExt};
use mp_convert::{Felt, ToFelt};
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use std::future::Future;
use std::iter;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::{Instant, Sleep};

// Event conversion
impl TryFrom<(LogMessageToL2, Log)> for MessageToL2WithMetadata {
    type Error = SettlementClientError;

    fn try_from((event, log): (LogMessageToL2, Log)) -> Result<Self, Self::Error> {
        Ok(Self {
            l1_block_number: log.block_number.ok_or_else(|| -> SettlementClientError {
                EthereumClientError::MissingField("block_number in Ethereum log").into()
            })?,
            l1_block_hash: log
                .block_hash
                .ok_or_else(|| -> SettlementClientError {
                    EthereumClientError::MissingField("block_hash in Ethereum log").into()
                })?
                .0,
            l1_transaction_hash: log
                .transaction_hash
                .ok_or_else(|| -> SettlementClientError {
                    EthereumClientError::MissingField("transaction_hash in Ethereum log").into()
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
                    .collect::<Vec<_>>()
                    .into(),
            },
            event.fee.try_into().map_err(|_| -> SettlementClientError {
                EthereumClientError::Conversion("Fee value too large for u128 conversion".to_string()).into()
            })?,
        ))
    }
}

/// Workaround for an alloy bug (unfixed through v2.0.5) where providers like
/// Alchemy return HTTP 400 with a JSON-RPC "filter not found" body. Alloy's HTTP
/// transport wraps non-2xx responses as `RpcError::Transport(HttpError)` before
/// parsing the JSON-RPC body, so the poller's "filter not found" detection (which
/// only inspects `RpcError::ErrorResp`) never fires. The poller retries the dead
/// filter forever.
///
/// `LivenessStream` detects this by exploiting the fact that a healthy poller yields
/// a `Vec<Log>` batch every ~7s (even if empty), while a stuck poller yields nothing.
/// If `max_silence` elapses with no batch, the stream terminates so the caller can
/// reconnect with a fresh filter.
pub(crate) struct LivenessStream<S> {
    inner: S,
    max_silence: Duration,
    deadline: Pin<Box<Sleep>>,
}

impl<S> LivenessStream<S> {
    pub fn new(inner: S, max_silence: Duration) -> Self {
        Self { inner, max_silence, deadline: Box::pin(tokio::time::sleep(max_silence)) }
    }
}

impl<S: Stream + Unpin> Stream for LivenessStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.deadline.as_mut().poll(cx).is_ready() {
            tracing::warn!(
                "⟠ L1 event poller silent for {:?} — likely a dropped filter \
                 (alloy HTTP 400 workaround). Terminating stream to trigger reconnection.",
                self.max_silence
            );
            return Poll::Ready(None);
        }

        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(item)) => {
                let max_silence = self.max_silence;
                self.deadline.as_mut().reset(Instant::now() + max_silence);
                Poll::Ready(Some(item))
            }
            other => other,
        }
    }
}

/// How long the poller can be silent before we assume the filter is dead.
/// The alloy poller polls every ~7s, so 60s allows ~8 missed cycles.
const POLLER_LIVENESS_TIMEOUT: Duration = Duration::from_secs(60);

fn decode_log(log: &Log) -> alloy::sol_types::Result<LogMessageToL2> {
    let log_data: &alloy::primitives::LogData = log.as_ref();
    LogMessageToL2::decode_raw_log(log_data.topics().iter().copied(), &log_data.data)
}

type EthereumStreamItem = Result<(LogMessageToL2, Log), alloy::sol_types::Error>;
type EthereumStreamType = Pin<Box<dyn Stream<Item = EthereumStreamItem> + Send + 'static>>;

pub struct EthereumEventStream {
    pub stream: EthereumStreamType,
}

impl EthereumEventStream {
    pub fn new(watcher: EventPoller<LogMessageToL2>) -> Self {
        // Don't use `watcher.into_stream()` — alloy's PollerStream swallows transport
        // errors internally and retries forever, hiding the "filter not found" case
        // from Alchemy (HTTP 400). Instead, wrap the raw poller with a liveness check
        // that terminates the stream when the poller stops yielding batches.
        let poller_stream = watcher.poller.into_stream();
        let stream = LivenessStream::new(poller_stream, POLLER_LIVENESS_TIMEOUT)
            .flat_map(futures::stream::iter)
            .map(|log: Log| decode_log(&log).map(|e| (e, log)));
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

    fn receiver_stream<T: Send + 'static>(rx: tokio::sync::mpsc::Receiver<T>) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        Box::pin(futures::stream::unfold(rx, |mut rx| async { rx.recv().await.map(|item| (item, rx)) }))
    }

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

    #[rstest]
    #[tokio::test]
    async fn test_missing_block_hash(mock_event: LogMessageToL2, mock_log: Log) {
        let mock_events = vec![Ok((
            mock_event,
            Log {
                block_hash: None, // Only block hash is missing
                ..mock_log
            },
        ))];

        let mock_stream = iter(mock_events);
        let mut ethereum_stream = EthereumEventStream { stream: Box::pin(mock_stream) };
        let events = collect_stream_events(&mut ethereum_stream).await;

        assert_eq!(events.len(), 1);
        assert_matches!(events[0].as_ref(), Err(SettlementClientError::Ethereum(EthereumClientError::MissingField(field))) => {
            assert_eq!(*field, "block_hash in Ethereum log", "Error should mention missing block hash");
        });
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_stream_terminates_on_silence() {
        let (tx, rx) = tokio::sync::mpsc::channel::<i32>(10);
        let mut stream = LivenessStream::new(receiver_stream(rx), Duration::from_secs(5));

        tx.send(42).await.unwrap();
        assert_eq!(stream.next().await, Some(42));

        // Advance past the liveness timeout with no further items
        tokio::time::advance(Duration::from_secs(6)).await;

        assert!(stream.next().await.is_none(), "stream should terminate after silence timeout");
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_stream_resets_on_activity() {
        let (tx, rx) = tokio::sync::mpsc::channel::<i32>(10);
        let mut stream = LivenessStream::new(receiver_stream(rx), Duration::from_secs(5));

        // Send items at 3s intervals — each resets the 5s deadline
        for i in 0..3 {
            tokio::time::advance(Duration::from_secs(3)).await;
            tx.send(i).await.unwrap();
            assert_eq!(stream.next().await, Some(i), "item {i} should pass through");
        }

        // Now go silent for longer than the timeout
        tokio::time::advance(Duration::from_secs(6)).await;
        assert!(stream.next().await.is_none(), "stream should terminate after silence");
    }

    #[tokio::test(start_paused = true)]
    async fn test_ethereum_event_stream_terminates_on_stuck_poller() {
        // Simulate a poller that yields one batch then goes silent (mimicking the
        // alloy poller stuck in an HTTP 400 error-retry loop).
        let (tx, rx) = tokio::sync::mpsc::channel::<Vec<Log>>(10);
        let inner = receiver_stream(rx);
        let liveness = LivenessStream::new(inner, Duration::from_secs(5));
        let decoded_stream =
            liveness.flat_map(futures::stream::iter).map(|log: Log| decode_log(&log).map(|e| (e, log)));
        let mut stream = EthereumEventStream { stream: Box::pin(decoded_stream) };

        // Advance 3s, then send an empty batch (normal poller heartbeat).
        // Advancing first ensures the deadline reset pushes it forward from
        // time=3+5=8s instead of the original time=0+5=5s.
        tokio::time::advance(Duration::from_secs(3)).await;
        tx.send(vec![]).await.unwrap();

        // Poll once so LivenessStream processes the empty batch and resets the
        // deadline to time=8s. flat_map produces no items from an empty vec,
        // so poll returns Pending.
        assert!(futures::poll!(stream.next()).is_pending());

        // Advance to time=7s — past the ORIGINAL 5s deadline but before the
        // reset deadline (8s). Stream should survive.
        tokio::time::advance(Duration::from_secs(4)).await;
        assert!(futures::poll!(stream.next()).is_pending(), "stream should survive past original deadline");

        // Now the poller "gets stuck" — no more batches arrive.
        // Advance to time=9s — past the reset deadline (8s).
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(stream.next().await.is_none(), "stream should terminate when poller is stuck");
    }
}
