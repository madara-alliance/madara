use crate::error::SettlementClientError;
use crate::messaging::MessageToL2WithMetadata;
use futures::Stream;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

/// A stream wrapper that filters events based on dynamic block confirmation depth
///
/// This uses a background task to periodically update the latest block number,
/// making the stream implementation much simpler. It's generic and works with any
/// settlement layer provider through a callback function.
///
/// Events that don't meet the confirmation depth requirement are buffered and
/// re-checked when the latest block number updates.
pub struct ConfirmationDepthFilteredStream<S> {
    inner: S,
    l1_msg_min_confirmations: u64,
    latest_block: Arc<AtomicU64>,
    waker: Arc<Mutex<Option<Waker>>>,
    _update_task: tokio::task::JoinHandle<()>,
    buffered_events: VecDeque<MessageToL2WithMetadata>,
}

impl<S> ConfirmationDepthFilteredStream<S>
where
    S: Stream<Item = Result<MessageToL2WithMetadata, SettlementClientError>> + Unpin,
{
    /// Create a new `ConfirmationDepthFilteredStream`
    ///
    /// # Arguments
    /// * `inner` - The inner stream to filter
    /// * `get_block_number` - A callback function that fetches the latest block number (should capture the provider and handle errors internally)
    /// * `polling_interval` - How often to poll for the latest block number
    /// * `min_confirmations` - Minimum number of confirmations required
    pub fn new<Fut>(
        inner: S,
        get_block_number: impl Fn() -> Fut + Send + Sync + 'static,
        polling_interval: Duration,
        l1_msg_min_confirmations: u64,
    ) -> Self
    where
        Fut: std::future::Future<Output = Option<u64>> + Send,
    {
        let latest_block = Arc::new(AtomicU64::new(0));
        let waker = Arc::new(Mutex::new(None::<Waker>));

        // Spawn background task to periodically update the latest block number
        let latest_block_clone = Arc::clone(&latest_block);
        let waker_clone = Arc::clone(&waker);
        let update_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(polling_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if let Some(block) = get_block_number().await {
                    if block != latest_block_clone.load(Ordering::Relaxed) {
                        latest_block_clone.store(block, Ordering::Relaxed);
                        tracing::trace!("Updated latest block number to {:?}", latest_block_clone);
                        // Wake the stream waker if registered
                        if let Ok(mut waker_guard) = waker_clone.lock() {
                            if let Some(w) = waker_guard.take() {
                                w.wake();
                            }
                        }
                    }
                }
            }
        });

        Self {
            inner,
            l1_msg_min_confirmations,
            latest_block,
            waker,
            _update_task: update_task,
            buffered_events: VecDeque::new(),
        }
    }
}

impl<S> Drop for ConfirmationDepthFilteredStream<S> {
    fn drop(&mut self) {
        // Abort the background task when the stream is dropped
        self._update_task.abort();
    }
}

impl<S> Stream for ConfirmationDepthFilteredStream<S>
where
    S: Stream<Item = Result<MessageToL2WithMetadata, SettlementClientError>> + Unpin,
{
    type Item = Result<MessageToL2WithMetadata, SettlementClientError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        let latest = this.latest_block.load(Ordering::Relaxed);
        let threshold = latest.saturating_sub(this.l1_msg_min_confirmations);

        // First, check if the first buffered event now meets the threshold
        // Events are buffered in order, so if the first one meets the threshold, we can return it.
        // If it doesn't, none of the remaining ones will either (since they're older/higher block numbers).
        if let Some(event) = this.buffered_events.front() {
            if event.l1_block_number <= threshold {
                let event = this.buffered_events.pop_front().unwrap();
                return Poll::Ready(Some(Ok(event)));
            }
        }

        // Poll the inner stream for new events
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                // Check if this event meets the confirmation depth requirement
                if event.l1_block_number <= threshold {
                    return Poll::Ready(Some(Ok(event)));
                }

                // Event doesn't have enough confirmations yet, buffer it for later
                tracing::debug!(
                    "Buffering event at block {} (needs {} confirmations, latest block: {})",
                    event.l1_block_number,
                    this.l1_msg_min_confirmations,
                    latest
                );
                this.buffered_events.push_back(event);
                // Register waker so we get notified when block number updates
                // Store the waker instead of spawning a task
                if let Ok(mut waker_guard) = this.waker.lock() {
                    *waker_guard = Some(cx.waker().clone());
                }
                // Return Pending to allow other tasks (like block number updates) to run
                // On the next poll, we'll check buffered events first
                Poll::Pending
            }
            Poll::Ready(Some(Err(e))) => {
                // Pass through errors
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                // Stream ended - check if we have any buffered events that meet the threshold
                // Only return events that satisfy the minimum confirmation requirement
                if let Some(event) = this.buffered_events.front() {
                    if event.l1_block_number <= threshold {
                        let event = this.buffered_events.pop_front().unwrap();
                        return Poll::Ready(Some(Ok(event)));
                    } else {
                        // There are buffered events but they don't meet the threshold yet
                        // Register waker so we get notified when block number updates
                        if let Ok(mut waker_guard) = this.waker.lock() {
                            *waker_guard = Some(cx.waker().clone());
                        }
                        // Return Pending to allow the background task to update the block number
                        // The stream will be polled again later
                        return Poll::Pending;
                    }
                }
                // No buffered events left, stream is truly ended
                Poll::Ready(None)
            }
            Poll::Pending => {
                // Inner stream is pending, but we might have buffered events that now meet the threshold
                // Register waker so we get notified when block number updates
                if !this.buffered_events.is_empty() {
                    if let Ok(mut waker_guard) = this.waker.lock() {
                        *waker_guard = Some(cx.waker().clone());
                    }
                }
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messaging::MessageToL2WithMetadata;
    use alloy::primitives::U256;
    use futures::stream;
    use futures::StreamExt;
    use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
    use starknet_types_core::felt::Felt;

    // Helper function to create a mock event
    fn create_mock_event(l1_block_number: u64, nonce: u64) -> MessageToL2WithMetadata {
        MessageToL2WithMetadata {
            l1_block_number,
            l1_transaction_hash: U256::from(nonce),
            message: L1HandlerTransactionWithFee::new(
                L1HandlerTransaction {
                    version: Felt::ZERO,
                    nonce,
                    contract_address: Felt::from(456),
                    entry_point_selector: Felt::from(789),
                    calldata: vec![Felt::from(123), Felt::from(1), Felt::from(2)].into(),
                },
                1000,
            ),
        }
    }

    #[tokio::test]
    async fn test_event_buffered_until_confirmation_threshold_met() {
        let min_confirmations = 10;
        let event1_block = 100;
        let event2_block = 101;
        let initial_latest_block = 109; // threshold = 99, events at 100 and 101 should be buffered
        let final_latest_block = 110; // threshold = 100, event at 100 should pass, 101 still buffered
        let final_latest_block2 = 111; // threshold = 101, event at 101 should pass

        let event1 = create_mock_event(event1_block, 1);
        let event2 = create_mock_event(event2_block, 2);
        let base_stream = stream::iter(vec![Ok(event1.clone()), Ok(event2.clone())]);

        let latest_block_state = Arc::new(AtomicU64::new(initial_latest_block));
        let latest_block_for_callback = latest_block_state.clone();

        let mut filtered_stream = ConfirmationDepthFilteredStream::new(
            base_stream,
            move || {
                let latest_block = latest_block_for_callback.clone();
                async move { Some(latest_block.load(Ordering::Relaxed)) }
            },
            Duration::from_millis(50),
            min_confirmations,
        );

        // Wait a bit for the background task to initialize
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Initially, events should be buffered (latest_block = 109, threshold = 99, events at 100 and 101 > 99)
        // Use timeout to verify they don't return immediately
        let result = tokio::time::timeout(Duration::from_millis(50), filtered_stream.next()).await;
        assert!(result.is_err(), "Events should be buffered and not returned immediately");

        // Update latest block to meet threshold for first event
        latest_block_state.store(final_latest_block, Ordering::Relaxed);

        // Wait for background task to update and poll again
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now the first event should be released (latest_block = 110, threshold = 100, event at 100 <= 100)
        let result = filtered_stream.next().await;
        assert!(result.is_some(), "First event should be released after threshold is met");
        let released_event = result.unwrap().unwrap();
        assert_eq!(released_event.l1_block_number, event1_block);

        // Second event should still be buffered (threshold = 100, event at 101 > 100)
        let result = tokio::time::timeout(Duration::from_millis(50), filtered_stream.next()).await;
        assert!(result.is_err(), "Second event should still be buffered");

        // Update latest block to meet threshold for second event
        latest_block_state.store(final_latest_block2, Ordering::Relaxed);

        // Wait for background task to update
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now the second event should be released (latest_block = 111, threshold = 101, event at 101 <= 101)
        let result = filtered_stream.next().await;
        assert!(result.is_some(), "Second event should be released after threshold is met");
        let released_event = result.unwrap().unwrap();
        assert_eq!(released_event.l1_block_number, event2_block);
    }

    #[tokio::test]
    async fn test_stream_ends_with_buffered_events() {
        let min_confirmations = 10;
        let event_block = 100;
        let initial_latest_block = 109; // threshold = 99, event at 100 should be buffered
        let final_latest_block = 110; // threshold = 100, event at 100 should pass

        let event = create_mock_event(event_block, 1);
        // Create a stream that ends immediately after emitting the event
        let base_stream = stream::iter(vec![Ok(event.clone())]);

        let latest_block_state = Arc::new(AtomicU64::new(initial_latest_block));
        let latest_block_for_callback = latest_block_state.clone();

        let mut filtered_stream = ConfirmationDepthFilteredStream::new(
            base_stream,
            move || {
                let latest_block = latest_block_for_callback.clone();
                async move { Some(latest_block.load(Ordering::Relaxed)) }
            },
            Duration::from_millis(50),
            min_confirmations,
        );

        // Wait a bit for the background task to initialize
        tokio::time::sleep(Duration::from_millis(10)).await;

        // First poll: event gets buffered, inner stream ends, but event doesn't meet threshold
        // Should timeout because it returns Pending (line 136)
        let result = tokio::time::timeout(Duration::from_millis(50), filtered_stream.next()).await;
        assert!(
            result.is_err(),
            "Should return Pending when stream ends with buffered event that doesn't meet threshold"
        );

        // Update latest block to meet threshold
        latest_block_state.store(final_latest_block, Ordering::Relaxed);

        // Wait for background task to update
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now should return the buffered event (line 130-131)
        let result = filtered_stream.next().await;
        assert!(result.is_some(), "Should return buffered event when it meets threshold");
        let released_event = result.unwrap().unwrap();
        assert_eq!(released_event.l1_block_number, event_block);

        // Next poll should return None (line 140) - stream is truly ended
        let result = filtered_stream.next().await;

        assert!(result.is_none(), "Should return None when stream ends and no buffered events remain");
    }
}
