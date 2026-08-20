use crate::{prelude::*, ChainTip, ReorgNotification};
use futures::{stream, Stream};
use std::sync::Arc;

/// Watch new last l1 confirmed block changes. This subscription will return a new notification everytime the value changes.
///
/// # Lag behavior
///
/// Notifications are discarded, only the latest one is returned.
#[derive(Debug)]
pub struct WatchL1Confirmed<D: MadaraStorageRead> {
    /// Keep backend around to keep sender alive.
    _backend: Arc<MadaraBackend<D>>,
    current_value: Option<u64>,
    subscription: tokio::sync::watch::Receiver<Option<u64>>,
}
impl<D: MadaraStorageRead> WatchL1Confirmed<D> {
    fn new(backend: &Arc<MadaraBackend<D>>) -> Self {
        let subscription = backend.latest_l1_confirmed.subscribe();
        let current_value = *subscription.borrow();
        Self { _backend: backend.clone(), current_value, subscription }
    }
    pub fn current(&self) -> &Option<u64> {
        &self.current_value
    }
    pub fn refresh(&mut self) {
        self.current_value = *self.subscription.borrow_and_update();
    }
    pub async fn recv(&mut self) -> &Option<u64> {
        self.subscription.changed().await.expect("Channel closed");
        self.current_value = *self.subscription.borrow_and_update();
        &self.current_value
    }
}

/// Subscribe to new blocks confirmed on l1. This will return a new notification everytime a new block
/// is confirmed on l1.
///
/// # Lag behavior
///
/// Notifications are never missed.
pub struct SubscribeNewL1Heads<D: MadaraStorageRead> {
    backend: Arc<MadaraBackend<D>>,
    subscription: WatchL1Confirmed<D>,
    current_value: Option<u64>,
}
impl<D: MadaraStorageRead> SubscribeNewL1Heads<D> {
    fn new(backend: &Arc<MadaraBackend<D>>) -> Self {
        let subscription = WatchL1Confirmed::new(backend);
        let current_value = subscription.current_value;
        Self { backend: backend.clone(), current_value, subscription }
    }
    pub fn set_start_from(&mut self, block_n: u64) {
        // We need to substract one
        self.current_value = block_n.checked_sub(1)
    }
    pub fn current(&self) -> &Option<u64> {
        &self.current_value
    }
    pub async fn next_head(&mut self) -> &Option<u64> {
        loop {
            // Inclusive bound.
            let next_block_to_return = self.current_value.map(|v| v + 1).unwrap_or(0);
            // Exclusive bound.
            let highest_block_plus_one = self.subscription.current().map(|v| v + 1).unwrap_or(0);

            if next_block_to_return < highest_block_plus_one {
                // A historical range is immediately ready; consume cooperative budget so a large catch-up
                // cannot starve peer tasks.
                tokio::task::coop::consume_budget().await;
                // Only advance after the yield point so cancelling this future cannot skip an unreturned head.
                self.current_value = Some(next_block_to_return);
                return &self.current_value;
            }

            self.subscription.recv().await;
        }
    }

    /// Returns [`None`] for pre-genesis.
    pub fn current_block_view(&self) -> Option<MadaraConfirmedBlockView<D>> {
        self.current_value.and_then(|val| self.backend.block_view_on_confirmed(val))
    }
    pub async fn next_block_view(&mut self) -> MadaraConfirmedBlockView<D> {
        self.next_head().await;
        self.current_block_view().expect("Cannot update chain to a pre-genesis state")
    }
    pub fn into_block_view_stream(self) -> impl Stream<Item = MadaraConfirmedBlockView<D>> {
        stream::unfold(self, |mut this| async move { Some((this.next_block_view().await, this)) })
    }
}

/// Watch chain tip changes. This subscription will return a new notification everytime the chain tip changes.
/// This either means:
/// - The current pre-confirmed block is added/removed/replaced.
/// - A new confirmed block is imported.
///
/// # Lag behavior
///
/// Notifications are discarded, only the latest one is returned.
#[derive(Debug)]
pub struct WatchChainTip<D: MadaraStorageRead> {
    _backend: Arc<MadaraBackend<D>>,
    current_value: ChainTip,
    subscription: tokio::sync::watch::Receiver<ChainTip>,
}
impl<D: MadaraStorageRead> WatchChainTip<D> {
    fn new(backend: &Arc<MadaraBackend<D>>) -> Self {
        let subscription = backend.chain_tip.subscribe();
        let current_value = subscription.borrow().clone();
        Self { _backend: backend.clone(), current_value, subscription }
    }
    pub fn current(&self) -> &ChainTip {
        &self.current_value
    }
    pub fn refresh(&mut self) {
        self.current_value = self.subscription.borrow_and_update().clone();
    }
    pub async fn recv(&mut self) -> &ChainTip {
        self.subscription.changed().await.expect("Channel closed");
        self.current_value = self.subscription.borrow_and_update().clone();
        &self.current_value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubscribeNewBlocksTag {
    /// Returns notifications for Confirmed and Preconfirmed blocks.
    Preconfirmed,
    /// Returns notifications for Confirmed blocks only.
    Confirmed,
}

/// Subscribe to new blocks. When used with [`WatchBlockTag::Confirmed`], this will return a new notification
/// everytime a new block is confirmed. When used with [`WatchBlockTag::Preconfirmed`], this will return a new
/// notification everytime a new block is confirmed, and everytime a new preconfirmed block is added or replaced.
/// If a preconfirmed block is replaced (consensus failure, etc.) a new notification will be sent.
///
/// # Lag behavior
///
/// Notifications of confirmed blocks are never missed. Notifications about preconfirmed blocks may be missed.
pub struct SubscribeNewHeads<D: MadaraStorageRead> {
    backend: Arc<MadaraBackend<D>>,
    subscription: WatchChainTip<D>,
    tag: SubscribeNewBlocksTag,
    current_value: ChainTip,
}
impl<D: MadaraStorageRead> SubscribeNewHeads<D> {
    fn new(backend: &Arc<MadaraBackend<D>>, tag: SubscribeNewBlocksTag) -> Self {
        let subscription = WatchChainTip::new(backend);
        let current_value = subscription.current_value.clone();
        Self { backend: backend.clone(), current_value, subscription, tag }
    }
    pub fn set_start_from(&mut self, block_n: u64) {
        // We need to substract one
        self.current_value = ChainTip::on_confirmed_block_n_or_empty(block_n.checked_sub(1))
    }
    pub fn current(&self) -> &ChainTip {
        &self.current_value
    }
    pub async fn next_head(&mut self) -> &ChainTip {
        loop {
            // Inclusive bound.
            let next_block_to_return = self.current_value.latest_confirmed_block_n().map(|v| v + 1).unwrap_or(0);
            // Exclusive bound.
            let highest_block_plus_one =
                self.subscription.current().latest_confirmed_block_n().map(|v| v + 1).unwrap_or(0);

            if next_block_to_return < highest_block_plus_one {
                self.current_value = ChainTip::on_confirmed_block_n_or_empty(Some(next_block_to_return));
                return &self.current_value;
            }

            if self.tag == SubscribeNewBlocksTag::Preconfirmed
                && self.subscription.current().is_preconfirmed()
                && self.subscription.current() != &self.current_value
            {
                self.current_value = self.subscription.current().clone();
                return &self.current_value;
            }

            self.subscription.recv().await;
        }
    }

    /// Returns [`None`] for pre-genesis.
    pub fn current_block_view(&self) -> Option<MadaraBlockView<D>> {
        self.backend.block_view_on_tip(self.current_value.clone())
    }
    pub async fn next_block_view(&mut self) -> MadaraBlockView<D> {
        self.next_head().await;
        self.current_block_view().expect("Cannot update chain to a pre-genesis state")
    }
    pub fn into_block_view_stream(self) -> impl Stream<Item = MadaraBlockView<D>> {
        stream::unfold(self, |mut this| async move { Some((this.next_block_view().await, this)) })
    }
}

/// Subscribe to first-class reorg notifications emitted by the backend.
///
/// # Lag behavior
///
/// Notifications are buffered in a bounded broadcast channel and may be dropped for lagging receivers.
#[derive(Debug)]
pub struct SubscribeReorgs<D: MadaraStorageRead> {
    _backend: Arc<MadaraBackend<D>>,
    subscription: tokio::sync::broadcast::Receiver<ReorgNotification>,
}
impl<D: MadaraStorageRead> SubscribeReorgs<D> {
    fn new(backend: &Arc<MadaraBackend<D>>) -> Self {
        let subscription = backend.reorg_notifications.subscribe();
        Self { _backend: backend.clone(), subscription }
    }

    pub async fn recv(&mut self) -> Result<ReorgNotification, tokio::sync::broadcast::error::RecvError> {
        self.subscription.recv().await
    }

    pub fn try_recv(&mut self) -> Result<ReorgNotification, tokio::sync::broadcast::error::TryRecvError> {
        self.subscription.try_recv()
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Subscribe to new blocks. See [`WatchL1Confirmed`] for more details
    pub fn watch_l1_confirmed(self: &Arc<Self>) -> WatchL1Confirmed<D> {
        WatchL1Confirmed::new(self)
    }

    /// Subscribe to new blocks confirmed on l1. See [`SubscribeNewL1Heads`] for more details
    pub fn subscribe_new_l1_confirmed_heads(self: &Arc<Self>) -> SubscribeNewL1Heads<D> {
        SubscribeNewL1Heads::new(self)
    }

    /// Watch the chain tip. See [`WatchChainTip`] for more details
    pub fn watch_chain_tip(self: &Arc<Self>) -> WatchChainTip<D> {
        WatchChainTip::new(self)
    }

    /// Subscribe to new blocks. See [`SubscribeNewHeads`] for more details
    pub fn subscribe_new_heads(self: &Arc<Self>, tag: SubscribeNewBlocksTag) -> SubscribeNewHeads<D> {
        SubscribeNewHeads::new(self, tag)
    }

    /// Subscribe to dedicated reorg notifications emitted after successful chain reverts.
    pub fn subscribe_reorgs(self: &Arc<Self>) -> SubscribeReorgs<D> {
        SubscribeReorgs::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_chain_config::ChainConfig;
    use std::{
        future::{poll_fn, Future},
        pin::pin,
        sync::atomic::{AtomicBool, Ordering},
        task::Poll,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn historical_l1_backlog_yields_to_peer_tasks() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mut subscription = backend.subscribe_new_l1_confirmed_heads();
        backend.set_latest_l1_confirmed(Some(1_000)).expect("L1 tip should be set");

        let peer_ran = AtomicBool::new(false);
        let drain_backlog = async {
            for expected in 0..=1_000 {
                assert_eq!(*subscription.next_head().await, Some(expected));
            }
            assert!(peer_ran.load(Ordering::SeqCst), "historical catch-up monopolized the runtime");
        };
        let peer_task = async {
            peer_ran.store(true, Ordering::SeqCst);
        };

        tokio::join!(biased; drain_backlog, peer_task);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_historical_l1_head_is_returned_on_retry() {
        tokio::spawn(async {
            let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
            let mut subscription = backend.subscribe_new_l1_confirmed_heads();
            backend.set_latest_l1_confirmed(Some(1)).expect("L1 tip should be set");

            // Exhaust this task's cooperative budget without awaiting the Pending call, which would let Tokio
            // reschedule the task with a fresh budget.
            loop {
                let consumed = poll_fn(|cx| {
                    let future = tokio::task::coop::consume_budget();
                    let mut future = pin!(future);
                    Poll::Ready(future.as_mut().poll(cx).is_ready())
                })
                .await;
                if !consumed {
                    break;
                }
            }

            // Poll the subscription to its cooperative yield point, then cancel it before it returns a head.
            {
                let mut next_head = Box::pin(subscription.next_head());
                let yielded = poll_fn(|cx| Poll::Ready(next_head.as_mut().poll(cx).is_pending())).await;
                assert!(yielded, "subscription should yield when its cooperative budget is exhausted");
            }

            assert_eq!(*subscription.current(), None, "a cancelled call must not advance the subscription cursor");
            assert_eq!(*subscription.next_head().await, Some(0), "the cancelled head should be returned on retry");
            assert_eq!(*subscription.next_head().await, Some(1), "later heads should retain their order");
        })
        .await
        .expect("cancellation regression task should complete");
    }
}
