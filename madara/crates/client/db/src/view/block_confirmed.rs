use crate::{prelude::*, rocksdb::RocksDBStorage};
use mp_block::{MadaraBlockInfo, TransactionWithReceipt};
use mp_state_update::StateDiff;

#[derive(Debug)]
pub struct MadaraConfirmedBlockView<D: MadaraStorageRead = RocksDBStorage> {
    backend: Arc<MadaraBackend<D>>,
    block_number: u64,
}

// derive(Clone) will put a D: Clone bounds which we don't want, so we have to implement clone by hand :(
impl<D: MadaraStorageRead> Clone for MadaraConfirmedBlockView<D> {
    fn clone(&self) -> Self {
        Self { backend: self.backend.clone(), block_number: self.block_number }
    }
}

impl<D: MadaraStorageRead> fmt::Display for MadaraConfirmedBlockView<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[Confirmed block at height #{}]", self.block_number)
    }
}

impl<D: MadaraStorageRead> MadaraConfirmedBlockView<D> {
    /// New view on the current block state. The block height must be confirmed.
    pub(crate) fn new(backend: Arc<MadaraBackend<D>>, block_number: u64) -> Self {
        Self { backend, block_number }
    }

    pub fn backend(&self) -> &Arc<MadaraBackend<D>> {
        &self.backend
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
        self.block_number
    }

    pub fn is_on_l1(&self) -> bool {
        self.backend.latest_l1_confirmed_block_n().is_some_and(|last_on_l1| self.block_number <= last_on_l1)
    }

    pub fn get_block_info(&self) -> Result<MadaraBlockInfo> {
        self.backend
            .db
            .get_block_info(self.block_number)?
            .with_context(|| format!("Block info at height {} should be found", self.block_number))
    }

    pub fn get_state_diff(&self) -> Result<StateDiff> {
        self.backend.db.get_block_state_diff(self.block_number)?.context("Block state diff should be found")
    }

    pub fn get_transaction(&self, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        let Some(tx_index) = usize::try_from(tx_index).ok() else { return Ok(None) };
        Ok(self.backend.db.get_transaction(self.block_number, tx_index as u64)?)
    }

    pub fn get_block_transactions(
        &self,
        bounds: impl std::ops::RangeBounds<u64>,
    ) -> Result<Vec<TransactionWithReceipt>> {
        let (from_tx_index, to_take) = super::normalize_transactions_range(bounds);

        self.backend
            .db
            .get_block_transactions(self.block_number, from_tx_index as u64)
            .take(to_take)
            .collect::<Result<_, _>>()
    }
}
