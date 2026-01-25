use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner},
};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use rocksdb::{IteratorMode, ReadOptions};

#[cfg(any(test, feature = "testing"))]
use std::sync::atomic::{AtomicBool, Ordering};

/// <txn_hash 32 bytes> => bincode(txn)
pub const MEMPOOL_EXTERNAL_OUTBOX_COLUMN: Column = Column::new("mempool_external_outbox").set_point_lookup();

#[cfg(any(test, feature = "testing"))]
static FORCE_EXTERNAL_OUTBOX_WRITE_ERROR: AtomicBool = AtomicBool::new(false);

#[cfg(any(test, feature = "testing"))]
pub(super) fn set_external_outbox_write_failpoint(enabled: bool) {
    FORCE_EXTERNAL_OUTBOX_WRITE_ERROR.store(enabled, Ordering::Relaxed);
}

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self), fields(module = "ExternalOutboxDB"))]
    pub(super) fn iter_external_outbox(
        &self,
        limit: usize,
    ) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        DBIterator::new_cf(
            &self.db,
            &self.get_column(MEMPOOL_EXTERNAL_OUTBOX_COLUMN),
            ReadOptions::default(),
            IteratorMode::Start,
        )
        .into_iter_values(|v| super::deserialize::<ValidatedTransaction>(&v))
        .map(|r| Ok(r??))
        .take(limit)
    }

    #[tracing::instrument(skip(self, tx), fields(module = "ExternalOutboxDB"))]
    pub(super) fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<()> {
        #[cfg(any(test, feature = "testing"))]
        if FORCE_EXTERNAL_OUTBOX_WRITE_ERROR.load(Ordering::Relaxed) {
            bail!("Forced external outbox write failure");
        }
        let col = self.get_column(MEMPOOL_EXTERNAL_OUTBOX_COLUMN);
        self.db.put_cf(&col, super::serialize(&tx.hash)?, super::serialize(tx)?)?;
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "ExternalOutboxDB"))]
    pub(super) fn delete_external_outbox(&self, tx_hash: Felt) -> Result<()> {
        let col = self.get_column(MEMPOOL_EXTERNAL_OUTBOX_COLUMN);
        self.db.delete_cf_opt(&col, super::serialize(&tx_hash)?, &self.writeopts)?;
        Ok(())
    }
}
