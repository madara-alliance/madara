use crate::{prelude::*, rocksdb::RocksDBStorage};
use mp_block::{MadaraMaybePreconfirmedBlockInfo, TransactionWithReceipt};
use mp_state_update::StateDiff;

#[derive(Debug, PartialEq, Eq)]
pub enum MadaraBlockView<D: MadaraStorageRead = RocksDBStorage> {
    Confirmed(MadaraConfirmedBlockView<D>),
    Preconfirmed(MadaraPreconfirmedBlockView<D>),
}

impl<D: MadaraStorageRead> fmt::Display for MadaraBlockView<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Confirmed(b) => write!(f, "{b}"),
            Self::Preconfirmed(b) => write!(f, "{b}"),
        }
    }
}

// derive(Clone) will put a D: Clone bounds which we don't want, so we have to implement clone by hand :(
impl<D: MadaraStorageRead> Clone for MadaraBlockView<D> {
    fn clone(&self) -> Self {
        match self {
            Self::Confirmed(view) => Self::Confirmed(view.clone()),
            Self::Preconfirmed(view) => Self::Preconfirmed(view.clone()),
        }
    }
}

impl<D: MadaraStorageRead> From<MadaraConfirmedBlockView<D>> for MadaraBlockView<D> {
    fn from(value: MadaraConfirmedBlockView<D>) -> Self {
        Self::Confirmed(value)
    }
}

impl<D: MadaraStorageRead> From<MadaraPreconfirmedBlockView<D>> for MadaraBlockView<D> {
    fn from(value: MadaraPreconfirmedBlockView<D>) -> Self {
        Self::Preconfirmed(value)
    }
}

impl<D: MadaraStorageRead> MadaraBlockView<D> {
    pub fn backend(&self) -> &Arc<MadaraBackend<D>> {
        match self {
            Self::Confirmed(view) => view.backend(),
            Self::Preconfirmed(view) => view.backend(),
        }
    }

    pub fn parent_block(&self) -> Option<MadaraConfirmedBlockView<D>> {
        self.block_number()
            .checked_sub(1)
            .map(|block_number| MadaraConfirmedBlockView::new(self.backend().clone(), block_number))
    }

    pub fn state_view_on_parent(&self) -> MadaraStateView<D> {
        MadaraStateView::on_confirmed_or_empty(self.backend().clone(), self.block_number().checked_sub(1))
    }

    pub fn state_view(&self) -> MadaraStateView<D> {
        self.clone().into()
    }

    pub fn block_number(&self) -> u64 {
        match self {
            Self::Confirmed(view) => view.block_number(),
            Self::Preconfirmed(view) => view.block_number(),
        }
    }

    pub fn is_on_l1(&self) -> bool {
        match self {
            Self::Confirmed(view) => view.is_on_l1(),
            Self::Preconfirmed(_) => false,
        }
    }
    pub fn is_confirmed(&self) -> bool {
        match self {
            Self::Confirmed(_) => true,
            Self::Preconfirmed(_) => false,
        }
    }
    pub fn is_preconfirmed(&self) -> bool {
        !self.is_confirmed()
    }

    pub fn get_block_info(&self) -> Result<MadaraMaybePreconfirmedBlockInfo> {
        match self {
            Self::Confirmed(view) => Ok(MadaraMaybePreconfirmedBlockInfo::Confirmed(view.get_block_info()?)),
            Self::Preconfirmed(view) => Ok(MadaraMaybePreconfirmedBlockInfo::Preconfirmed(view.get_block_info())),
        }
    }

    /// Note: when this function is called on a pre-confirmed block view, this will aggregate and normalize a state diff, which could be a
    /// somewhat an expensive operation.
    pub fn get_state_diff(&self) -> Result<StateDiff> {
        match self {
            Self::Confirmed(view) => view.get_state_diff(),
            Self::Preconfirmed(view) => view.get_normalized_state_diff(),
        }
    }

    pub fn get_executed_transaction(&self, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        match self {
            Self::Confirmed(view) => view.get_executed_transaction(tx_index),
            Self::Preconfirmed(view) => Ok(view.get_executed_transaction(tx_index)),
        }
    }

    pub fn get_executed_transactions(
        &self,
        bounds: impl std::ops::RangeBounds<u64>,
    ) -> Result<Vec<TransactionWithReceipt>> {
        match self {
            Self::Confirmed(view) => view.get_executed_transactions(bounds),
            Self::Preconfirmed(view) => Ok(view.get_executed_transactions(bounds)),
        }
    }

    pub fn into_preconfirmed(self) -> Option<MadaraPreconfirmedBlockView<D>> {
        match self {
            Self::Preconfirmed(view) => Some(view),
            _ => None,
        }
    }
    pub fn as_preconfirmed(&self) -> Option<&MadaraPreconfirmedBlockView<D>> {
        match self {
            Self::Preconfirmed(view) => Some(view),
            _ => None,
        }
    }
    pub fn as_preconfirmed_mut(&mut self) -> Option<&mut MadaraPreconfirmedBlockView<D>> {
        match self {
            Self::Preconfirmed(view) => Some(view),
            _ => None,
        }
    }
    pub fn into_confirmed(self) -> Option<MadaraConfirmedBlockView<D>> {
        match self {
            Self::Confirmed(view) => Some(view),
            _ => None,
        }
    }
    pub fn as_confirmed(&self) -> Option<&MadaraConfirmedBlockView<D>> {
        match self {
            Self::Confirmed(view) => Some(view),
            _ => None,
        }
    }
}
