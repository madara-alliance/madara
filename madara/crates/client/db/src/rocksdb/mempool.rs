use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction},
};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use rocksdb::{IteratorMode, ReadOptions};

/// <txn_hash 32 bytes> => bincode(txn)
pub const MEMPOOL_TRANSACTIONS_COLUMN: Column = Column::new("mempool_transactions").set_point_lookup();

fn next_lexicographic_key(key: &[u8]) -> Option<Vec<u8>> {
    let mut next = key.to_vec();
    for i in (0..next.len()).rev() {
        if next[i] != u8::MAX {
            next[i] += 1;
            next.truncate(i + 1);
            return Some(next);
        }
    }
    None
}

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self), fields(module = "MempoolDB"))]
    pub(super) fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        DBIterator::new_cf(
            &self.db,
            &self.get_column(MEMPOOL_TRANSACTIONS_COLUMN),
            ReadOptions::default(),
            IteratorMode::Start,
        )
        .into_iter_values(|v| super::deserialize::<ValidatedTransaction>(&v))
        .map(|r| Ok(r??))
    }

    #[tracing::instrument(skip(self), fields(module = "MempoolDB"))]
    pub(super) fn clear_saved_mempool_transactions(&self) -> Result<()> {
        let col = self.get_column(MEMPOOL_TRANSACTIONS_COLUMN);
        let Some((first_key, _)) = self.db.iterator_cf(&col, IteratorMode::Start).next().transpose()? else {
            return Ok(());
        };
        let Some((last_key, _)) = self.db.iterator_cf(&col, IteratorMode::End).next().transpose()? else {
            return Ok(());
        };

        let first_key = first_key.to_vec();
        let last_key = last_key.to_vec();
        let mut batch = WriteBatchWithTransaction::default();

        if let Some(end_exclusive) = next_lexicographic_key(&last_key) {
            batch.delete_range_cf(&col, &first_key, &end_exclusive);
        } else {
            batch.delete_range_cf(&col, &first_key, &last_key);
            batch.delete_cf(&col, &last_key);
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, tx_hashes), fields(module = "MempoolDB"))]
    pub(super) fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        let col = self.get_column(MEMPOOL_TRANSACTIONS_COLUMN);

        let mut batch = WriteBatchWithTransaction::default();
        for tx_hash in tx_hashes {
            tracing::debug!("remove_mempool_tx {:#x}", tx_hash);
            batch.delete_cf(&col, super::serialize(&tx_hash)?);
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, tx), fields(module = "MempoolDB"))]
    pub(super) fn write_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()> {
        let col = self.get_column(MEMPOOL_TRANSACTIONS_COLUMN);
        self.db.put_cf(&col, super::serialize(&tx.hash)?, super::serialize(&tx)?)?;
        Ok(())
    }
}
