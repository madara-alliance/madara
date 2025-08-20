use crate::{prelude::*, ChainTip};
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
        let current_value = subscription.borrow().clone();
        Self { _backend: backend.clone(), current_value, subscription }
    }
    pub fn current(&self) -> &Option<u64> {
        &self.current_value
    }
    pub fn refresh(&mut self) {
        self.current_value = self.subscription.borrow_and_update().clone();
    }
    pub async fn recv(&mut self) -> &Option<u64> {
        self.subscription.changed().await.expect("Channel closed");
        self.current_value = self.subscription.borrow_and_update().clone();
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
        let current_value = subscription.current_value.clone();
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
        self.current_value.clone().and_then(|val| self.backend.block_view_on_confirmed(val))
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
            let next_block_to_return = self.current_value.block_n().map(|v| v + 1).unwrap_or(0);
            // Exclusive bound.
            let highest_block_plus_one =
                self.subscription.current().latest_confirmed_block_n().map(|v| v + 1).unwrap_or(0);

            if next_block_to_return < highest_block_plus_one {
                self.current_value = ChainTip::on_confirmed_block_n_or_empty(Some(next_block_to_return));
                return &self.current_value;
            }

            if self.subscription.current().is_preconfirmed() && self.tag == SubscribeNewBlocksTag::Preconfirmed {
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
}
