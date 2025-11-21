use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner, WriteBatchWithTransaction},
};
use mp_convert::Felt;
use mp_receipt::L1HandlerTransactionReceipt;
use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee};
use rocksdb::ReadOptions;

/// <core_contract_nonce 8 bytes> => bincode(pending message)
pub const L1_TO_L2_PENDING_MESSAGE_BY_NONCE: Column =
    Column::new("l1_to_l2_pending_message_by_nonce").set_point_lookup();
/// <core_contract_nonce 8 bytes> => txn hash
pub const L1_TO_L2_TXN_HASH_BY_NONCE: Column = Column::new("l1_to_l2_txn_hash_by_nonce").set_point_lookup();

impl RocksDBStorageInner {
    /// Also removed the given txns from the pending column.
    pub(super) fn messages_to_l2_write_trasactions<'a>(
        &self,
        txs: impl IntoIterator<Item = (&'a L1HandlerTransaction, &'a L1HandlerTransactionReceipt)>,
    ) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);

        for (txn, receipt) in txs {
            let key = txn.nonce.to_be_bytes();
            batch.delete_cf(&pending_cf, key);
            batch.put_cf(&on_l2_cf, key, receipt.transaction_hash.to_bytes_be());
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
    }

    /// If the message is already pending, this will overwrite it.
    pub(super) fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        self.db.put_cf_opt(&pending_cf, msg.tx.nonce.to_be_bytes(), super::serialize(&msg)?, &self.writeopts)?;
        Ok(())
    }

    /// If the message does not exist, this does nothing.
    pub(super) fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        self.db.delete_cf_opt(&pending_cf, core_contract_nonce.to_be_bytes(), &self.writeopts)?;
        Ok(())
    }

    pub(super) fn get_pending_message_to_l2(
        &self,
        core_contract_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        self.db.get_pinned_cf(&pending_cf, core_contract_nonce.to_be_bytes())?;
        let Some(res) = self.db.get_pinned_cf(&pending_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(super::deserialize(&res)?))
    }

    pub(super) fn get_next_pending_message_to_l2(
        &self,
        start_nonce: u64,
    ) -> Result<Option<L1HandlerTransactionWithFee>> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let binding = start_nonce.to_be_bytes();
        let mode = rocksdb::IteratorMode::From(&binding, rocksdb::Direction::Forward);
        let mut iter = DBIterator::new_cf(&self.db, &pending_cf, ReadOptions::default(), mode)
            .into_iter_values(|v| super::deserialize(v));

        iter.next().transpose()?.transpose().map_err(Into::into)
    }

    pub(super) fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        let Some(res) = self.db.get_pinned_cf(&on_l2_cf, core_contract_nonce.to_be_bytes())? else { return Ok(None) };
        Ok(Some(Felt::from_bytes_be(res[..].try_into().context("Deserializing felt")?)))
    }

    pub(super) fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        self.db.put_cf_opt(&on_l2_cf, core_contract_nonce.to_be_bytes(), txn_hash.to_bytes_be(), &self.writeopts)?;
        Ok(())
    }

    pub(super) fn message_to_l2_remove_txns(
        &self,
        core_contract_nonces: impl IntoIterator<Item = u64>,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        for core_contract_nonce in core_contract_nonces {
            batch.delete_cf(&on_l2_cf, core_contract_nonce.to_be_bytes());
        }
        Ok(())
    }
}
