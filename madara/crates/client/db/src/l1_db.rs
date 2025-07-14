use crate::{Column, DatabaseExt, MadaraBackend, MadaraStorageError, WriteBatchWithTransaction};
use mp_convert::Felt;
use mp_receipt::L1HandlerTransactionReceipt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};

pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";

/// We add method in MadaraBackend to be able to handle L1->L2 messaging related data
impl MadaraBackend {
    /// Also removed the given txns from the pending column.
    pub fn l1_db_save_transactions<'a>(
        &self,
        txs: impl IntoIterator<Item = (&'a L1HandlerTransaction, &'a L1HandlerTransactionReceipt)>,
    ) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        let on_l2_cf = self.db.get_column(Column::CoreContractNonceToTxnHash);

        for (txn, receipt) in txs {
            let key = txn.nonce.to_be_bytes();
            batch.delete_cf(&pending_cf, key);
            batch.put_cf(&on_l2_cf, key, receipt.transaction_hash.to_bytes_be());
        }

        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }

    /// If the message is already pending, this will overwrite it.
    pub fn add_pending_message_to_l2(&self, msg: L1HandlerTransactionWithFee) -> Result<(), MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        self.db.put_cf_opt(
            &pending_cf,
            msg.tx.nonce.to_be_bytes(),
            bincode::serialize(&msg)?,
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    /// If the message does not exist, this does nothing.
    pub fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<(), MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        self.db.delete_cf_opt(&pending_cf, core_contract_nonce.to_be_bytes(), &self.writeopts_no_wal)?;
        Ok(())
    }

    pub fn get_pending_message_to_l2(
        &self,
        core_contract_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>, MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        self.db.get_cf(&pending_cf, core_contract_nonce.to_be_bytes())?;
        let Some(res) = self.db.get_pinned_cf(&pending_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(bincode::deserialize(&res)?))
    }

    pub fn get_next_pending_message_to_l2(
        &self,
        start_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>, MadaraStorageError> {
        let pending_cf = self.db.get_column(Column::CoreContractNonceToPendingMsg);
        let binding = start_nonce.to_be_bytes();
        let mode = rocksdb::IteratorMode::From(&binding, rocksdb::Direction::Forward);
        let mut iter = self.db.iterator_cf(&pending_cf, mode);

        match iter.next() {
            Some(res) => Ok(Some(bincode::deserialize(&res?.1)?)),
            None => Ok(None),
        }
    }

    pub fn get_l1_handler_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
    ) -> Result<Option<Felt>, MadaraStorageError> {
        let on_l2_cf = self.db.get_column(Column::CoreContractNonceToTxnHash);
        let Some(res) = self.db.get_pinned_cf(&on_l2_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(Felt::from_bytes_be(
            res[..].try_into().map_err(|_| MadaraStorageError::InconsistentStorage("Malformated felt".into()))?,
        )))
    }

    #[cfg(feature = "testing")]
    pub fn set_l1_handler_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        txn_hash: Felt,
    ) -> Result<(), MadaraStorageError> {
        let on_l2_cf = self.db.get_column(Column::CoreContractNonceToTxnHash);
        self.db.put_cf_opt(
            &on_l2_cf,
            core_contract_nonce.to_be_bytes(),
            txn_hash.to_bytes_be(),
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    /// Set the latest l1_block synced for the messaging worker.
    pub fn set_l1_messaging_sync_tip(&self, l1_block_n: u64) -> Result<(), MadaraStorageError> {
        let meta_cf = self.db.get_column(Column::BlockStorageMeta);
        self.db.put_cf_opt(&meta_cf, LAST_SYNCED_L1_EVENT_BLOCK, l1_block_n.to_be_bytes(), &self.writeopts_no_wal)?;
        Ok(())
    }

    /// Get the latest l1_block synced for the messaging worker.
    pub fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>, MadaraStorageError> {
        let meta_cf = self.db.get_column(Column::BlockStorageMeta);
        let Some(data) = self.db.get_pinned_cf(&meta_cf, LAST_SYNCED_L1_EVENT_BLOCK)? else { return Ok(None) };
        Ok(Some(u64::from_be_bytes(
            data[..]
                .try_into()
                .map_err(|_| MadaraStorageError::InconsistentStorage("Malformated saved l1_block_n".into()))?,
        )))
    }
}
