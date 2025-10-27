use crate::{preconfirmed::PreconfirmedBlock, prelude::*, ChainTip};

mod block;
mod block_confirmed;
mod block_preconfirmed;
mod state;

pub use block::MadaraBlockView;
pub use block_confirmed::MadaraConfirmedBlockView;
pub use block_preconfirmed::MadaraPreconfirmedBlockView;
use mp_block::{
    header::{BlockTimestamp, PreconfirmedHeader},
    TransactionWithReceipt,
};
pub use state::MadaraStateView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedTransactionWithBlockView<D: MadaraStorageRead> {
    pub transaction_index: u64,
    pub block: MadaraBlockView<D>,
}

impl<D: MadaraStorageRead> ExecutedTransactionWithBlockView<D> {
    pub fn get_transaction(&self) -> Result<TransactionWithReceipt> {
        self.block.get_executed_transaction(self.transaction_index)?.context("Transaction should be found")
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
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
    /// The preconfirmed block view will not include candidate transactions.
    pub fn block_view_on_preconfirmed(self: &Arc<Self>) -> Option<MadaraPreconfirmedBlockView<D>> {
        self.preconfirmed_block().map(|block| MadaraPreconfirmedBlockView::new(self.clone(), block))
    }

    /// Returns a view on the latest block, which may be a preconfirmed block. This view is used to query content and listen for changes in that block.
    /// The preconfirmed block view will not include candidate transactions.
    pub fn block_view_on_latest(self: &Arc<Self>) -> Option<MadaraBlockView<D>> {
        self.block_view_on_tip(self.chain_tip.borrow().clone())
    }

    /// Returns a view on the preconfirmed block. This view is used to query content and listen for changes in that block.
    /// This returns a fake preconfirmed block if there is not currently one in the backend.
    /// The preconfirmed block view will not include candidate transactions.
    pub fn block_view_on_preconfirmed_or_fake(self: &Arc<Self>) -> Result<MadaraPreconfirmedBlockView<D>> {
        let chain_tip = self.chain_tip.borrow();
        // TODO: cache the preconfirmed fake blocks.
        let block = match &*chain_tip {
            // Real preconfirmed block.
            ChainTip::Preconfirmed(block) => block.clone(),
            // Fake preconfirmed block, based on the previous block header. Most recent gas prices.
            ChainTip::Confirmed(parent_block_number) => {
                let parent_block_info = self
                    .block_view_on_confirmed(*parent_block_number)
                    .context("Parent block should be found")?
                    .get_block_info()?;
                PreconfirmedBlock::new(PreconfirmedHeader {
                    block_number: *parent_block_number + 1,
                    sequencer_address: parent_block_info.header.sequencer_address,
                    block_timestamp: BlockTimestamp::now(),
                    protocol_version: parent_block_info.header.protocol_version,
                    gas_prices: if let Some(quote) = self.get_last_l1_gas_quote() {
                        self.calculate_gas_prices(
                            &quote,
                            parent_block_info.header.gas_prices.strk_l2_gas_price,
                            parent_block_info.total_l2_gas_used,
                        )?
                    } else {
                        parent_block_info.header.gas_prices
                    },
                    l1_da_mode: parent_block_info.header.l1_da_mode,
                })
                .into()
            }
            // Fake preconfirmed block, based on chain config. Most recent gas prices.
            ChainTip::Empty => PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: 0,
                sequencer_address: self.chain_config().sequencer_address.to_felt(),
                block_timestamp: BlockTimestamp::now(),
                protocol_version: self.chain_config().latest_protocol_version,
                gas_prices: if let Some(quote) = self.get_last_l1_gas_quote() {
                    self.calculate_gas_prices(&quote, 0, 0)?
                } else {
                    Default::default()
                },
                l1_da_mode: self.chain_config().l1_da_mode,
            })
            .into(),
        };
        Ok(MadaraPreconfirmedBlockView::new(self.clone(), block))
    }

    /// Returns a state view on the latest confirmed block state. This view can be used to query the state from this block and earlier.
    /// The preconfirmed block view will not include candidate transactions.
    pub fn view_on_latest_confirmed(self: &Arc<Self>) -> MadaraStateView<D> {
        MadaraStateView::on_confirmed_or_empty(self.clone(), self.latest_confirmed_block_n())
    }

    /// Returns a state view on a confirmed block. This view can be used to query the state from this block and earlier.
    /// Returns [`None`] if the block is not confirmed.
    pub fn view_on_confirmed(self: &Arc<Self>, block_number: u64) -> Option<MadaraStateView<D>> {
        self.block_view_on_confirmed(block_number).map(Into::into)
    }

    /// Returns a state view on the latest block state, including pre-confirmed state. This view can be used to query the state from this block and earlier.
    /// The preconfirmed block view will not include candidate transactions.
    pub fn view_on_latest(self: &Arc<Self>) -> MadaraStateView<D> {
        self.view_on_tip(self.chain_tip.borrow().clone())
    }

    /// The preconfirmed block view will not include candidate transactions.
    /// This does not check that the block actually exists when given a confirmed block. Please be aware of that.
    pub fn block_view_on_tip(self: &Arc<MadaraBackend<D>>, tip: ChainTip) -> Option<MadaraBlockView<D>> {
        match tip {
            ChainTip::Empty => None,
            ChainTip::Confirmed(block_number) => Some(MadaraConfirmedBlockView::new(self.clone(), block_number).into()),
            ChainTip::Preconfirmed(block) => Some(MadaraPreconfirmedBlockView::new(self.clone(), block).into()),
        }
    }
    /// The preconfirmed block view will not include candidate transactions.
    /// This does not check that the block actually exists when given a confirmed block. Please be aware of that.
    pub fn view_on_tip(self: &Arc<MadaraBackend<D>>, tip: ChainTip) -> MadaraStateView<D> {
        match tip {
            ChainTip::Empty => MadaraStateView::Empty(self.clone()),
            ChainTip::Confirmed(block_number) => MadaraConfirmedBlockView::new(self.clone(), block_number).into(),
            ChainTip::Preconfirmed(block) => MadaraPreconfirmedBlockView::new(self.clone(), block).into(),
        }
    }
}

// Returns (start_tx_index, to_take).
fn normalize_transactions_range(bounds: impl std::ops::RangeBounds<u64>) -> (usize, usize) {
    use std::ops::Bound;

    let (start, end) = (bounds.start_bound().cloned(), bounds.end_bound().cloned());

    let start_tx_index = match start {
        Bound::Excluded(start) => start.saturating_add(1),
        Bound::Included(start) => start,
        Bound::Unbounded => 0,
    };
    let start_tx_index = usize::try_from(start_tx_index).unwrap_or(usize::MAX);
    let end_tx_index = match end {
        Bound::Excluded(end) => end,
        Bound::Included(end) => end.saturating_add(1),
        Bound::Unbounded => u64::MAX,
    };
    let end_tx_index = usize::try_from(end_tx_index).unwrap_or(usize::MAX);

    let to_take = end_tx_index.saturating_sub(start_tx_index);

    (start_tx_index, to_take)
}

#[cfg(test)]
mod tests {
    use super::normalize_transactions_range;

    #[test]
    #[allow(clippy::reversed_empty_ranges)]
    fn test_normalize_transactions_range() {
        assert_eq!(normalize_transactions_range(5..10), (5, 5));
        assert_eq!(normalize_transactions_range(5..=10), (5, 6));
        assert_eq!(normalize_transactions_range(5..5), (5, 0));
        assert_eq!(normalize_transactions_range(5..=5), (5, 1));
        assert_eq!(normalize_transactions_range(..10), (0, 10));
        assert_eq!(normalize_transactions_range(..=10), (0, 11));
        assert_eq!(normalize_transactions_range(5..), (5, usize::MAX - 5));
        assert_eq!(normalize_transactions_range(..), (0, usize::MAX));
        assert_eq!(normalize_transactions_range(0..5), (0, 5));
        assert_eq!(normalize_transactions_range(10..5), (10, 0)); // backwards (empty)
    }
}
