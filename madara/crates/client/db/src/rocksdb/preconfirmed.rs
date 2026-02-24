use crate::{
    prelude::*,
    rocksdb::{Column, RocksDBStorageInner, WriteBatchWithTransaction},
};
use mp_block::header::PreconfirmedHeader;

pub const PRECONFIRMED_COLUMN: Column = Column::new("preconfirmed");

// PRECONFIRMED_COLUMN key layout (all keys are big-endian):
// - [0x10 | block_n | tx_index]: preconfirmed transaction/receipt entry.
// - [0x20 | block_n]: staged preconfirmed header (parallel merkle).

pub(crate) const PRECONFIRMED_TX_TAG: u8 = 0x10;
pub(crate) const PRECONFIRMED_STAGED_HEADER_TAG: u8 = 0x20;

pub(crate) fn preconfirmed_block_key(tag: u8, block_n: u64) -> [u8; 1 + size_of::<u64>()] {
    let mut key = [0u8; 1 + size_of::<u64>()];
    key[0] = tag;
    key[1..].copy_from_slice(&block_n.to_be_bytes());
    key
}

pub(crate) fn preconfirmed_tx_prefix(block_n: u64) -> [u8; 1 + size_of::<u64>()] {
    preconfirmed_block_key(PRECONFIRMED_TX_TAG, block_n)
}

pub(crate) fn preconfirmed_tx_key(block_n: u64, tx_index: u16) -> [u8; 1 + size_of::<u64>() + size_of::<u16>()] {
    let mut key = [0u8; 1 + size_of::<u64>() + size_of::<u16>()];
    key[..1 + size_of::<u64>()].copy_from_slice(&preconfirmed_tx_prefix(block_n));
    key[1 + size_of::<u64>()..].copy_from_slice(&tx_index.to_be_bytes());
    key
}

impl RocksDBStorageInner {
    pub(super) fn preconfirmed_delete_block_txs_in_batch(&self, block_n: u64, batch: &mut WriteBatchWithTransaction) {
        let preconfirmed_col = self.get_column(PRECONFIRMED_COLUMN);

        let start = preconfirmed_tx_prefix(block_n).to_vec();
        let end = if block_n == u64::MAX {
            vec![PRECONFIRMED_TX_TAG + 1]
        } else {
            preconfirmed_tx_prefix(block_n + 1).to_vec()
        };
        batch.delete_range_cf(&preconfirmed_col, start, end);
    }

    pub(super) fn parallel_merkle_set_staged_header(
        &self,
        block_n: u64,
        header: &PreconfirmedHeader,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let preconfirmed_col = self.get_column(PRECONFIRMED_COLUMN);
        batch.put_cf(
            &preconfirmed_col,
            preconfirmed_block_key(PRECONFIRMED_STAGED_HEADER_TAG, block_n),
            super::serialize(header)?,
        );
        Ok(())
    }

    pub(super) fn parallel_merkle_get_staged_block_header(&self, block_n: u64) -> Result<Option<PreconfirmedHeader>> {
        let Some(res) = self.db.get_pinned_cf(
            &self.get_column(PRECONFIRMED_COLUMN),
            preconfirmed_block_key(PRECONFIRMED_STAGED_HEADER_TAG, block_n),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    pub(super) fn parallel_merkle_clear_staged_header(&self, block_n: u64, batch: &mut WriteBatchWithTransaction) {
        let preconfirmed_col = self.get_column(PRECONFIRMED_COLUMN);
        batch.delete_cf(&preconfirmed_col, preconfirmed_block_key(PRECONFIRMED_STAGED_HEADER_TAG, block_n));
    }

    pub(super) fn parallel_merkle_clear_staged_headers_above(
        &self,
        target_block_n: u64,
        batch: &mut WriteBatchWithTransaction,
    ) {
        if target_block_n == u64::MAX {
            return;
        }

        let preconfirmed_col = self.get_column(PRECONFIRMED_COLUMN);
        let start = preconfirmed_block_key(PRECONFIRMED_STAGED_HEADER_TAG, target_block_n + 1).to_vec();
        let end = vec![PRECONFIRMED_STAGED_HEADER_TAG + 1];
        batch.delete_range_cf(&preconfirmed_col, start, end);
    }
}
