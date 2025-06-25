use crate::DatabaseExt;
use crate::{Column, MadaraBackend, MadaraStorageError};
use mp_transactions::validated::ValidatedMempoolTx;
use rocksdb::{IteratorMode, WriteBatch};
use starknet_types_core::felt::Felt;

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

impl MadaraBackend {
    #[tracing::instrument(skip(self), fields(module = "MempoolDB"))]
    pub fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedMempoolTx>> + '_ {
        let col = self.db.get_column(Column::MempoolTransactions);
        self.db.iterator_cf(&col, IteratorMode::Start).map(|kv| {
            let (_, v) = kv?;
            let tx: ValidatedMempoolTx = bincode::deserialize(&v)?;
            Ok(tx)
        })
    }

    pub fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        // Note: We do not use WAL here, as it will be flushed by saving the block. This is to
        // ensure saving the block and removing the tx from the saved mempool are both done at once
        // atomically.

        let col = self.db.get_column(Column::MempoolTransactions);

        let mut batch = WriteBatch::default();
        for tx_hash in tx_hashes {
            tracing::debug!("remove_mempool_tx {:#x}", tx_hash);
            batch.delete_cf(&col, bincode::serialize(&tx_hash)?);
        }

        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, tx), fields(module = "MempoolDB"))]
    pub fn save_mempool_transaction(&self, tx: &ValidatedMempoolTx) -> Result<()> {
        // Note: WAL is used here
        // This is because we want it to be saved even if the node crashes before the next flush

        let hash = tx.tx_hash;
        let col = self.db.get_column(Column::MempoolTransactions);
        let tx_with_class = tx;
        self.db.put_cf(&col, bincode::serialize(&hash)?, bincode::serialize(&tx_with_class)?)?;
        tracing::debug!("save_mempool_tx {:?}", hash);
        Ok(())
    }
}
