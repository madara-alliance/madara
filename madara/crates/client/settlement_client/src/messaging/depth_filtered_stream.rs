use crate::error::SettlementClientError;
use crate::messaging::MessageToL2WithMetadata;
use futures::Stream;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
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

        // Spawn background task to periodically update the latest block number
        let latest_block_clone = Arc::clone(&latest_block);
        let update_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(polling_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if let Some(block) = get_block_number().await {
                    latest_block_clone.store(block, Ordering::Relaxed);
                    tracing::trace!("Updated latest block number to {}", block);
                }
            }
        });

        Self {
            inner,
            l1_msg_min_confirmations,
            latest_block,
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
                // Return Pending to allow other tasks (like block number updates) to run
                // On the next poll, we'll check buffered events first
                Poll::Pending
            }
            Poll::Ready(Some(Err(e))) => {
                // Pass through errors
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                // Stream ended - check if we have any buffered events left
                if let Some(event) = this.buffered_events.pop_front() {
                    return Poll::Ready(Some(Ok(event)));
                }
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
