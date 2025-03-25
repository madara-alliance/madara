use crate::DatabaseExt;
use crate::{Column, MadaraBackend, MadaraStorageError};
use mp_transactions::validated::ValidatedMempoolTx;
use rocksdb::IteratorMode;
use serde::{Deserialize, Serialize};
use starknet_api::core::Nonce;
use starknet_types_core::felt::Felt;

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

/// A nonce is deemed ready when it directly follows the previous nonce in db
/// for a contract address. This guarantees that dependent transactions are not
/// executed out of order by the mempool.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum NonceStatus {
    #[default]
    Ready,
    Pending,
}

/// Information used to assess the [readiness] of a transaction.
///
/// A transaction is deemed ready when its nonce directly follows the previous
/// nonce store in db for that contract address.
///
/// [nonce] and [nonce_next] are precomputed to avoid operating on a [Felt]
/// inside the hot path in the mempool.
///
/// [readiness]: NonceStatus
/// [nonce]: Self::nonce
/// [nonce_next]: Self::nonce_next
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NonceInfo {
    pub readiness: NonceStatus,
    pub nonce: Nonce,
    pub nonce_next: Nonce,
}

impl NonceInfo {
    pub fn ready(nonce: Nonce, nonce_next: Nonce) -> Self {
        debug_assert_eq!(nonce_next, nonce.try_increment().unwrap());
        Self { readiness: NonceStatus::Ready, nonce, nonce_next }
    }

    pub fn pending(nonce: Nonce, nonce_next: Nonce) -> Self {
        debug_assert_eq!(nonce_next, nonce.try_increment().unwrap());
        Self { readiness: NonceStatus::Pending, nonce, nonce_next }
    }
}

impl Default for NonceInfo {
    fn default() -> Self {
        Self::ready(Nonce(Felt::ZERO), Nonce(Felt::ONE))
    }
}

#[derive(Serialize)]
/// This struct is used as a template to serialize Mempool transactions from the
/// database without any further allocation.
struct DbMempoolTxInfoEncoder<'a> {
    tx: &'a ValidatedMempoolTx,
    nonce_info: &'a NonceInfo,
}

#[derive(Deserialize)]
/// This struct is used as a template to deserialize Mempool transactions from
/// the database.
pub struct DbMempoolTxInfoDecoder {
    pub tx: ValidatedMempoolTx,
    pub nonce_readiness: NonceInfo,
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
        self.db.delete_cf_opt(&col, bincode::serialize(tx_hash)?, &self.writeopts_no_wal)?;
        tracing::debug!("remove_mempool_tx {:?}", tx_hash);
        Ok(())
    }

    #[tracing::instrument(skip(self, tx), fields(module = "MempoolDB"))]
    pub fn save_mempool_transaction(&self, tx: &ValidatedMempoolTx, nonce_info: &NonceInfo) -> Result<()> {
        // Note: WAL is used here
        // This is because we want it to be saved even if the node crashes before the next flush

        let hash = tx.tx_hash;
        let col = self.db.get_column(Column::MempoolTransactions);
        let tx_with_class = DbMempoolTxInfoEncoder { tx, nonce_info };
        self.db.put_cf(&col, bincode::serialize(&hash)?, bincode::serialize(&tx_with_class)?)?;
        tracing::debug!("save_mempool_tx {:?}", hash);
        Ok(())
    }
}
