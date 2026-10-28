use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction},
};
use mp_convert::Felt;
use mp_transactions::validated::ValidatedTransaction;
use rocksdb::{IteratorMode, ReadOptions};

const REJECTED_MEMPOOL_RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
const MAX_REJECTED_MEMPOOL_TRANSACTIONS: usize = 1_024;

#[derive(serde::Deserialize, serde::Serialize)]
struct LegacyRejectedMempoolTransaction {
    reason: String,
}

fn deserialize_rejection(bytes: &[u8]) -> Result<crate::storage::RejectedMempoolTransaction> {
    match super::deserialize(bytes) {
        Ok(rejection) => Ok(rejection),
        Err(_) => {
            let legacy: LegacyRejectedMempoolTransaction = super::deserialize(bytes)?;
            Ok(crate::storage::RejectedMempoolTransaction::new_at(legacy.reason, 0))
        }
    }
}

/// <txn_hash 32 bytes> => bincode(txn)
pub const MEMPOOL_TRANSACTIONS_COLUMN: Column = Column::new("mempool_transactions").set_point_lookup();
pub const MEMPOOL_REJECTED_TRANSACTIONS_COLUMN: Column =
    Column::new("mempool_rejected_transactions").set_point_lookup();

impl RocksDBStorageInner {
    pub(super) fn get_rejected_mempool_transaction(
        &self,
        tx_hash: &Felt,
    ) -> Result<Option<crate::storage::RejectedMempoolTransaction>> {
        let col = self.get_column(MEMPOOL_REJECTED_TRANSACTIONS_COLUMN);
        let key = super::serialize(tx_hash)?;
        let Some(value) = self.db.get_pinned_cf(&col, &key)? else { return Ok(None) };
        let rejection = deserialize_rejection(value.as_ref())?;
        let now = unix_timestamp_seconds();
        if rejection.rejected_at_unix_seconds() < now.saturating_sub(REJECTED_MEMPOOL_RETENTION_SECONDS) {
            self.db.delete_cf(&col, key)?;
            return Ok(None);
        }
        Ok(Some(rejection))
    }

    pub(super) fn write_rejected_mempool_transaction(
        &self,
        tx_hash: &Felt,
        rejection: &crate::storage::RejectedMempoolTransaction,
    ) -> Result<()> {
        let col = self.get_column(MEMPOOL_REJECTED_TRANSACTIONS_COLUMN);
        self.db.put_cf(&col, super::serialize(tx_hash)?, super::serialize(rejection)?)?;
        self.prune_rejected_mempool_transactions(
            unix_timestamp_seconds(),
            REJECTED_MEMPOOL_RETENTION_SECONDS,
            MAX_REJECTED_MEMPOOL_TRANSACTIONS,
        )
    }

    fn prune_rejected_mempool_transactions(&self, now: u64, max_age_seconds: u64, max_entries: usize) -> Result<()> {
        let col = self.get_column(MEMPOOL_REJECTED_TRANSACTIONS_COLUMN);
        let cutoff = now.saturating_sub(max_age_seconds);
        let mut retained = Vec::new();
        let mut batch = WriteBatchWithTransaction::default();

        for item in DBIterator::new_cf(&self.db, &col, ReadOptions::default(), IteratorMode::Start)
            .into_iter_items(|(key, value)| (key.to_vec(), deserialize_rejection(value)))
        {
            let (key, rejection) = item?;
            let rejection = rejection?;
            if rejection.rejected_at_unix_seconds() < cutoff {
                batch.delete_cf(&col, key);
            } else {
                retained.push((rejection.rejected_at_unix_seconds(), key));
            }
        }

        retained.sort_unstable();
        let excess = retained.len().saturating_sub(max_entries);
        for (_, key) in retained.into_iter().take(excess) {
            batch.delete_cf(&col, key);
        }
        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

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

fn unix_timestamp_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MadaraBackend, RejectedMempoolTransaction};
    use mp_chain_config::ChainConfig;

    #[test]
    fn rejected_transactions_are_pruned_by_age_and_max_count() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let now = unix_timestamp_seconds();
        for (hash, timestamp) in [(1, now - 30), (2, now - 20), (3, now - 10)] {
            backend
                .write_rejected_mempool_transaction(
                    &Felt::from(hash),
                    &RejectedMempoolTransaction::new_at("too large for proof", timestamp),
                )
                .unwrap();
        }

        backend.db.inner.prune_rejected_mempool_transactions(now, 25, 1).unwrap();

        assert_eq!(backend.get_rejected_mempool_transaction(&Felt::from(1)).unwrap(), None);
        assert_eq!(backend.get_rejected_mempool_transaction(&Felt::from(2)).unwrap(), None);
        assert!(backend.get_rejected_mempool_transaction(&Felt::from(3)).unwrap().is_some());
    }

    #[test]
    fn legacy_rejection_without_timestamp_expires_safely() {
        let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
        let hash = Felt::from(1);
        let col = backend.db.inner.get_column(MEMPOOL_REJECTED_TRANSACTIONS_COLUMN);
        backend
            .db
            .inner
            .db
            .put_cf(
                &col,
                super::super::serialize(&hash).unwrap(),
                super::super::serialize(&LegacyRejectedMempoolTransaction { reason: "legacy".into() }).unwrap(),
            )
            .unwrap();

        assert_eq!(backend.get_rejected_mempool_transaction(&hash).unwrap(), None);
    }
}
