use crate::{chain_head::ChainHeadState, preconfirmed::PreconfirmedBlock, prelude::*, ReorgNotification};
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
        if self.subscription.changed().await.is_err() {
            tracing::warn!("L1 confirmed watch channel closed; returning last observed value");
            return &self.current_value;
        }
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

/// Watch chain head state changes. This subscription returns the latest value whenever updated.
///
/// # Lag behavior
///
/// Notifications are discarded, only the latest one is returned.
#[derive(Debug)]
pub struct WatchChainHeadState<D: MadaraStorageRead> {
    _backend: Arc<MadaraBackend<D>>,
    current_value: ChainHeadState,
    subscription: tokio::sync::watch::Receiver<ChainHeadState>,
}
impl<D: MadaraStorageRead> WatchChainHeadState<D> {
    fn new(backend: &Arc<MadaraBackend<D>>) -> Self {
        let subscription = backend.chain_head_state.subscribe();
        let current_value = *subscription.borrow();
        Self { _backend: backend.clone(), current_value, subscription }
    }
    pub fn current(&self) -> &ChainHeadState {
        &self.current_value
    }
    pub fn refresh(&mut self) {
        self.current_value = *self.subscription.borrow_and_update();
    }
    pub async fn recv(&mut self) -> &ChainHeadState {
        if self.subscription.changed().await.is_err() {
            tracing::warn!("Chain head watch channel closed; returning last observed value");
            return &self.current_value;
        }
        self.current_value = *self.subscription.borrow_and_update();
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

/// Subscribe to new block heads for internal services.
/// This subscription is driven by [`WatchChainHeadState`], not by any separate tip cache.
///
/// # Lag behavior
///
/// Notifications of confirmed blocks are never missed. Notifications about preconfirmed blocks may be missed.
pub struct SubscribeInternalHeads<D: MadaraStorageRead> {
    backend: Arc<MadaraBackend<D>>,
    subscription: WatchChainHeadState<D>,
    tag: SubscribeNewBlocksTag,
    current_confirmed_tip: Option<u64>,
    current_preconfirmed: Option<Arc<PreconfirmedBlock>>,
}
impl<D: MadaraStorageRead> SubscribeInternalHeads<D> {
    fn new(backend: &Arc<MadaraBackend<D>>, tag: SubscribeNewBlocksTag) -> Self {
        let subscription = WatchChainHeadState::new(backend);
        let current_confirmed_tip = subscription.current().confirmed_tip;
        let current_preconfirmed =
            (tag == SubscribeNewBlocksTag::Preconfirmed).then(|| backend.internal_preconfirmed_block()).flatten();
        Self { backend: backend.clone(), subscription, tag, current_confirmed_tip, current_preconfirmed }
    }

    pub fn set_start_from(&mut self, block_n: u64) {
        self.current_confirmed_tip = block_n.checked_sub(1);
        self.current_preconfirmed = None;
    }

    pub fn current_confirmed_block_n(&self) -> Option<u64> {
        self.current_confirmed_tip
    }

    pub fn current_block_view(&self) -> Option<MadaraBlockView<D>> {
        self.current_preconfirmed
            .as_ref()
            .map(|block| MadaraPreconfirmedBlockView::new(self.backend.clone(), block.clone()).into())
            .or_else(|| {
                self.current_confirmed_tip.map(|n| MadaraConfirmedBlockView::new(self.backend.clone(), n).into())
            })
    }

    async fn advance_if_needed(&mut self) {
        loop {
            // Inclusive bound.
            let next_block_to_return = self.current_confirmed_tip.map(|v| v + 1).unwrap_or(0);
            // Exclusive bound.
            let highest_block_plus_one = self.subscription.current().confirmed_tip.map(|v| v + 1).unwrap_or(0);

            if next_block_to_return < highest_block_plus_one {
                self.current_confirmed_tip = Some(next_block_to_return);
                self.current_preconfirmed = None;
                return;
            }

            if self.tag == SubscribeNewBlocksTag::Preconfirmed {
                let expected_preconfirmed_tip = self.subscription.current().internal_preconfirmed_tip;
                if let Some(next_preconfirmed) = self.backend.internal_preconfirmed_block() {
                    let next_preconfirmed_tip = Some(next_preconfirmed.header.block_number);
                    let changed = self.current_preconfirmed.as_ref().is_none_or(|current| {
                        current.header.block_number != next_preconfirmed.header.block_number
                            || current.content.borrow().n_executed() != next_preconfirmed.content.borrow().n_executed()
                    });

                    if expected_preconfirmed_tip == next_preconfirmed_tip && changed {
                        self.current_confirmed_tip = next_preconfirmed.header.block_number.checked_sub(1);
                        self.current_preconfirmed = Some(next_preconfirmed);
                        return;
                    }
                }
            }

            self.subscription.recv().await;
        }
    }

    pub async fn next_block_view(&mut self) -> MadaraBlockView<D> {
        self.advance_if_needed().await;
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

    /// Watch chain head state. See [`WatchChainHeadState`] for details.
    pub fn watch_chain_head_state(self: &Arc<Self>) -> WatchChainHeadState<D> {
        WatchChainHeadState::new(self)
    }

    /// Subscribe to new blocks for internal services.
    /// This stream is driven by chain head state.
    pub fn subscribe_internal_heads(self: &Arc<Self>, tag: SubscribeNewBlocksTag) -> SubscribeInternalHeads<D> {
        SubscribeInternalHeads::new(self, tag)
    }

    /// Chain-head-driven block stream used by RPC subscriptions.
    ///
    /// This is the same stream as [`Self::subscribe_internal_heads`]; the alias preserves the
    /// established RPC-facing call site name while chain-head state remains the sole source of truth.
    pub fn subscribe_new_heads(self: &Arc<Self>, tag: SubscribeNewBlocksTag) -> SubscribeInternalHeads<D> {
        self.subscribe_internal_heads(tag)
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
