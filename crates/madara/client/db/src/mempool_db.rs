use crate::DatabaseExt;
use crate::{Column, MadaraBackend, MadaraStorageError};
use mp_class::ConvertedClass;
use rocksdb::IteratorMode;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

#[derive(Serialize, Deserialize)]
pub struct SavedTransaction {
    pub tx: mp_transactions::Transaction,
    pub paid_fee_on_l1: Option<u128>,
    pub contract_address: Option<Felt>,
    pub only_query: bool,
    pub arrived_at: u128,
}

#[derive(Serialize)]
/// This struct is used as a template to serialize Mempool transactions from the
/// database without any further allocation.
struct DbMempoolTxInfoEncoder<'a> {
    saved_tx: &'a SavedTransaction,
    converted_class: &'a Option<ConvertedClass>,
    nonce_readiness: bool,
}

#[derive(Deserialize)]
/// This struct is used as a templace to deserialize Mempool transactions from
/// the database.
pub struct DbMempoolTxInfoDecoder {
    pub saved_tx: SavedTransaction,
    pub converted_class: Option<ConvertedClass>,
    pub nonce_readiness: bool,
}

impl MadaraBackend {
    #[tracing::instrument(skip(self), fields(module = "MempoolDB"))]
    pub fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<(Felt, DbMempoolTxInfoDecoder)>> + '_ {
        let col = self.db.get_column(Column::MempoolTransactions);
        self.db.iterator_cf(&col, IteratorMode::Start).map(|kv| {
            let (k, v) = kv?;
            let hash: Felt = bincode::deserialize(&k)?;
            let tx_info: DbMempoolTxInfoDecoder = bincode::deserialize(&v)?;

            Result::<_>::Ok((hash, tx_info))
        })
    }

    #[tracing::instrument(skip(self), fields(module = "MempoolDB"))]
    pub fn remove_mempool_transaction(&self, tx_hash: &Felt) -> Result<()> {
        // Note: We do not use WAL here, as it will be flushed by saving the block. This is to
        // ensure saving the block and removing the tx from the saved mempool are both done at once
        // atomically.

        let col = self.db.get_column(Column::MempoolTransactions);
        self.db.delete_cf_opt(&col, bincode::serialize(tx_hash)?, &self.write_opt_no_wal)?;
        tracing::debug!("remove_mempool_tx {:?}", tx_hash);
        Ok(())
    }

    #[tracing::instrument(skip(self, saved_tx), fields(module = "MempoolDB"))]
    pub fn save_mempool_transaction(
        &self,
        saved_tx: &SavedTransaction,
        tx_hash: Felt,
        converted_class: &Option<ConvertedClass>,
        nonce_readiness: bool,
    ) -> Result<()> {
        // Note: WAL is used here
        // This is because we want it to be saved even if the node crashes before the next flush

        let col = self.db.get_column(Column::MempoolTransactions);
        let tx_with_class = DbMempoolTxInfoEncoder { saved_tx, converted_class, nonce_readiness };
        self.db.put_cf(&col, bincode::serialize(&tx_hash)?, bincode::serialize(&tx_with_class)?)?;
        tracing::debug!("save_mempool_tx {:?}", tx_hash);
        Ok(())
    }
}
