use crate::{prelude::*, ChainTip};
use mp_block::{BlockId, BlockTag};

mod block;
mod block_confirmed;
mod block_preconfirmed;
mod state;

pub use block::MadaraBlockView;
pub use block_confirmed::MadaraConfirmedBlockView;
pub use block_preconfirmed::MadaraPreconfirmedBlockView;
pub use state::MadaraStateView;

// Returns (start_tx_index, to_take).
fn normalize_transactions_range(bounds: impl std::ops::RangeBounds<u64>) -> (usize, usize) {
    use std::ops::Bound;

    let (start, end) = (bounds.start_bound().cloned(), bounds.end_bound().cloned());

    let start_tx_index = match start {
        Bound::Excluded(start) => start.saturating_add(1),
        Bound::Included(start) => start,
        Bound::Unbounded => 0,
    };
    let start_tx_index = usize::try_from(start_tx_index).ok().unwrap_or(usize::MAX);
    let end_tx_index = match end {
        Bound::Excluded(end) => end,
        Bound::Included(end) => end.saturating_add(1),
        Bound::Unbounded => u64::MAX.into(),
    };
    let end_tx_index = usize::try_from(end_tx_index).unwrap_or(usize::MAX);

    let to_take = end_tx_index.saturating_sub(start_tx_index);

    (start_tx_index, to_take)
}

pub trait BlockViewResolvable: Sized {
    fn resolve_block_view<D: MadaraStorageRead>(
        &self,
        backend: &Arc<MadaraBackend<D>>,
    ) -> Result<Option<MadaraBlockView<D>>>;
}

impl<T: BlockViewResolvable> BlockViewResolvable for &T {
    fn resolve_block_view<D: MadaraStorageRead>(
        &self,
        backend: &Arc<MadaraBackend<D>>,
    ) -> Result<Option<MadaraBlockView<D>>> {
        (*self).resolve_block_view(backend)
    }
}

impl BlockViewResolvable for BlockId {
    fn resolve_block_view<D: MadaraStorageRead>(
        &self,
        backend: &Arc<MadaraBackend<D>>,
    ) -> Result<Option<MadaraBlockView<D>>> {
        match self {
            Self::Tag(BlockTag::Pending) => Ok(Some(backend.block_view_on_preconfirmed_or_fake().into())),
            Self::Tag(BlockTag::Latest) => Ok(backend.block_view_on_last_confirmed().map(|b| b.into())),
            Self::Hash(hash) => {
                if let Some(block_n) = backend.db.find_block_hash(&hash)? {
                    Ok(Some(
                        backend
                            .block_view_on_confirmed(block_n)
                            .with_context(|| {
                                format!("Block with hash {hash:#x} was found at {block_n} but no such block exists")
                            })?
                            .into(),
                    ))
                } else {
                    Ok(None)
                }
            }
            Self::Number(block_n) => Ok(backend.block_view_on_confirmed(*block_n).map(Into::into)),
        }
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Returns a view on a block. This view is used to query content from that block.
    /// Returns [`None`] if the block was not found.
    pub fn block_view(self: &Arc<Self>, block_id: impl BlockViewResolvable) -> Result<Option<MadaraBlockView<D>>> {
        block_id.resolve_block_view(self)
    }

    /// Returns a view on the last confirmed block. This view is used to query content from that block.
    /// Returns [`None`] if the database has no blocks.
    pub fn block_view_on_last_confirmed(self: &Arc<Self>) -> Option<MadaraConfirmedBlockView<D>> {
        self.latest_confirmed_block_n().map(|block_number| MadaraConfirmedBlockView::new(self.clone(), block_number))
    }

    /// Returns a view on a confirmed block. This view is used to query content from that block.
    /// Returns [`None`] if the block number is not yet confirmed.
    pub fn block_view_on_confirmed(self: &Arc<Self>, block_number: u64) -> Option<MadaraConfirmedBlockView<D>> {
        self.latest_confirmed_block_n()
            .filter(|n| n >= &block_number)
            .map(|_| MadaraConfirmedBlockView::new(self.clone(), block_number))
    }

    /// Returns a view on the preconfirmed block. This view is used to query content and listen for changes in that block.
    pub fn block_view_on_preconfirmed(self: &Arc<Self>) -> Option<MadaraPreconfirmedBlockView<D>> {
        self.preconfirmed_block().map(|block| MadaraPreconfirmedBlockView::new(self.clone(), block))
    }

    /// Returns a view on the latest block, which may be a preconfirmed block. This view is used to query content and listen for changes in that block.
    pub fn block_view_on_latest(self: &Arc<Self>) -> Option<MadaraBlockView<D>> {
        self.block_view_on_tip(self.chain_tip.borrow().clone())
    }

    /// Returns a view on the preconfirmed block. This view is used to query content and listen for changes in that block.
    /// This returns a fake preconfirmed block if there is not currently one in the backend.
    pub fn block_view_on_preconfirmed_or_fake(self: &Arc<Self>) -> MadaraPreconfirmedBlockView<D> {
        MadaraPreconfirmedBlockView::new(
            self.clone(),
            self.preconfirmed_block().unwrap_or_else(|| {
                // fake preconfirmed block.
                todo!()
            }),
        )
    }

    /// Returns a state view on the latest confirmed block state. This view can be used to query the state from this block and earlier.
    pub fn view_on_latest_confirmed(self: &Arc<Self>) -> MadaraStateView<D> {
        MadaraStateView::on_confirmed_or_empty(self.clone(), self.latest_confirmed_block_n())
    }

    /// Returns a state view on the latest block state, including pre-confirmed state. This view can be used to query the state from this block and earlier.
    pub fn view_on_latest(self: &Arc<Self>) -> MadaraStateView<D> {
        self.view_on_tip(self.chain_tip.borrow().clone())
    }

    pub fn block_view_on_tip(self: &Arc<MadaraBackend<D>>, tip: ChainTip) -> Option<MadaraBlockView<D>> {
        match tip {
            ChainTip::Empty => None,
            ChainTip::Confirmed(block_number) => Some(MadaraConfirmedBlockView::new(self.clone(), block_number).into()),
            ChainTip::Preconfirmed(block) => Some(MadaraPreconfirmedBlockView::new(self.clone(), block).into()),
        }
    }
    pub fn view_on_tip(self: &Arc<MadaraBackend<D>>, tip: ChainTip) -> MadaraStateView<D> {
        match tip {
            ChainTip::Empty => MadaraStateView::Empty(self.clone()),
            ChainTip::Confirmed(block_number) => MadaraConfirmedBlockView::new(self.clone(), block_number).into(),
            ChainTip::Preconfirmed(block) => MadaraPreconfirmedBlockView::new(self.clone(), block).into(),
        }
    }
}
