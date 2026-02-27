use crate::{chain_head::ChainHeadState, preconfirmed::PreconfirmedBlock, prelude::*, ChainTip};
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
        self.subscription.changed().await.expect("Channel closed");
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
    subscription: WatchChainHeadState<D>,
    tag: SubscribeNewBlocksTag,
    current_value: ChainTip,
}
impl<D: MadaraStorageRead> SubscribeNewHeads<D> {
    fn new(backend: &Arc<MadaraBackend<D>>, tag: SubscribeNewBlocksTag) -> Self {
        let subscription = WatchChainHeadState::new(backend);
        let current_value = ChainTip::from_chain_head_state(*subscription.current(), backend.preconfirmed_block());
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
            let highest_block_plus_one = self.subscription.current().confirmed_tip.map(|v| v + 1).unwrap_or(0);

            if next_block_to_return < highest_block_plus_one {
                self.current_value = ChainTip::on_confirmed_block_n_or_empty(Some(next_block_to_return));
                return &self.current_value;
            }

            if self.tag == SubscribeNewBlocksTag::Preconfirmed {
                let projected_tip =
                    ChainTip::from_chain_head_state(*self.subscription.current(), self.backend.preconfirmed_block());
                if projected_tip.is_preconfirmed() && projected_tip != self.current_value {
                    self.current_value = projected_tip;
                    return &self.current_value;
                }
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

/// Subscribe to new block heads for internal services.
/// This subscription is driven by [`WatchChainHeadState`], not by chain tip.
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
            (tag == SubscribeNewBlocksTag::Preconfirmed).then(|| backend.preconfirmed_block()).flatten();
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
                let expected_preconfirmed_tip = self.subscription.current().external_preconfirmed_tip;
                if let Some(next_preconfirmed) = self.backend.preconfirmed_block() {
                    let next_preconfirmed_tip = Some(next_preconfirmed.header.block_number);
                    let changed = self
                        .current_preconfirmed
                        .as_ref()
                        .is_none_or(|current| !Arc::ptr_eq(current, &next_preconfirmed));

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

    /// Watch chain head state. See [`WatchChainHeadState`] for details.
    pub fn watch_chain_head_state(self: &Arc<Self>) -> WatchChainHeadState<D> {
        WatchChainHeadState::new(self)
    }

    /// Subscribe to new blocks. See [`SubscribeNewHeads`] for more details
    pub fn subscribe_new_heads(self: &Arc<Self>, tag: SubscribeNewBlocksTag) -> SubscribeNewHeads<D> {
        SubscribeNewHeads::new(self, tag)
    }

    /// Subscribe to new blocks for internal services.
    /// This stream is driven by chain head state.
    pub fn subscribe_internal_heads(self: &Arc<Self>, tag: SubscribeNewBlocksTag) -> SubscribeInternalHeads<D> {
        SubscribeInternalHeads::new(self, tag)
    }
}
