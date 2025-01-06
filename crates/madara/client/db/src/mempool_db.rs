use crate::DatabaseExt;
use crate::{Column, MadaraBackend, MadaraStorageError};
use mp_class::ConvertedClass;
use rocksdb::IteratorMode;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;

type Result<T, E = MadaraStorageError> = std::result::Result<T, E>;

/// A struct representing the readiness of a transaction.
///
/// A transaction is deemed ready when its nonce directly follows the previous
/// nonce store in db for that contract address. This guarantees that dependent
/// transactions are not executed out of order by the mempool.
///
/// [nonce] and [nonce_next] are precomputed and stored inside [NonceReadiness]
/// to avoid having to operate on a [Felt] inside the hot loop of the mempool.
///
/// [nonce]: Self::nonce
/// [nonce_next]: Self::nonce_next
#[must_use]
#[derive(Debug, Serialize, Deserialize)]
// TODO: separate the nonces from the readiness state to avoid unnecessary
// checks when retrieving the nonce
pub enum NonceReadiness {
    Ready { nonce: Felt, nonce_next: Felt },
    Pending { nonce: Felt, nonce_next: Felt },
}

impl NonceReadiness {
    #[inline(always)]
    pub fn ready(nonce: Felt, nonce_next: Felt) -> Self {
        debug_assert!(nonce + Felt::ONE == nonce_next);
        Self::Ready { nonce, nonce_next }
    }

    #[inline(always)]
    pub fn pending(nonce: Felt, nonce_next: Felt) -> Self {
        debug_assert!(nonce + Felt::ONE == nonce_next);
        Self::Pending { nonce, nonce_next }
    }

    #[inline(always)]
    pub fn nonce(&self) -> Felt {
        match self {
            Self::Ready { nonce, nonce_next: _ } => *nonce,
            Self::Pending { nonce, nonce_next: _ } => *nonce,
        }
    }

    #[inline(always)]
    pub fn nonce_next(&self) -> Felt {
        match self {
            Self::Ready { nonce: _, nonce_next } => *nonce_next,
            Self::Pending { nonce: _, nonce_next } => *nonce_next,
        }
    }
}

impl Default for NonceReadiness {
    fn default() -> Self {
        Self::Ready { nonce: Felt::ZERO, nonce_next: Felt::ONE }
    }
}

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
    nonce_readiness: &'a NonceReadiness,
}

#[derive(Deserialize)]
/// This struct is used as a templace to deserialize Mempool transactions from
/// the database.
pub struct DbMempoolTxInfoDecoder {
    pub saved_tx: SavedTransaction,
    pub converted_class: Option<ConvertedClass>,
    pub nonce_readiness: NonceReadiness,
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
        nonce_readiness: &NonceReadiness,
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
