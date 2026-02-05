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
/// <core_contract_nonce 8 bytes> => paid_fee_on_l1 (u128 as 16 bytes big-endian)
pub const L1_TO_L2_PAID_FEE_BY_NONCE: Column = Column::new("l1_to_l2_paid_fee_by_nonce").set_point_lookup();

// Fallback fee used when re-inserting pending L1->L2 messages during a revert
// when the fee is not found in storage (for messages confirmed before this fix).
const REVERT_PENDING_FEE_ON_L1_FALLBACK: u128 = 1_000_000_000_000;

impl RocksDBStorageInner {
    /// Also removed the given txns from the pending column.
    /// Stores the paid_fee_on_l1 from pending messages so it can be recovered during revert.
    pub(super) fn messages_to_l2_write_transactions<'a>(
        &self,
        txs: impl IntoIterator<Item = (&'a L1HandlerTransaction, &'a L1HandlerTransactionReceipt)>,
    ) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        let fee_cf = self.get_column(L1_TO_L2_PAID_FEE_BY_NONCE);

        for (txn, receipt) in txs {
            let key = txn.nonce.to_be_bytes();

            // Read the pending message to get the paid_fee_on_l1 before deleting
            if let Some(pending_msg) = self.get_pending_message_to_l2(txn.nonce)? {
                // Store the fee so we can recover it during revert
                batch.put_cf(&fee_cf, key, pending_msg.paid_fee_on_l1.to_be_bytes());
            }

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

    /// Get the paid_fee_on_l1 for a confirmed L1 handler transaction by its nonce.
    pub(super) fn get_l1_handler_paid_fee_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<u128>> {
        let fee_cf = self.get_column(L1_TO_L2_PAID_FEE_BY_NONCE);
        let Some(res) = self.db.get_pinned_cf(&fee_cf, core_contract_nonce.to_be_bytes())? else {
            return Ok(None);
        };
        let bytes: [u8; 16] = res[..].try_into().context("Deserializing paid_fee_on_l1")?;
        Ok(Some(u128::from_be_bytes(bytes)))
    }

    /// Restore L1->L2 messages as pending during a revert.
    ///
    /// This removes the consumed mapping (nonce -> tx hash) and re-inserts the
    /// message into the pending column so it can be re-processed if still valid on L1.
    /// Uses the stored paid_fee_on_l1 if available, otherwise falls back to a default value.
    pub(super) fn messages_to_l2_restore_pending<'a>(
        &self,
        txs: impl IntoIterator<Item = &'a L1HandlerTransaction>,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let pending_cf = self.get_column(L1_TO_L2_PENDING_MESSAGE_BY_NONCE);
        let on_l2_cf = self.get_column(L1_TO_L2_TXN_HASH_BY_NONCE);
        let fee_cf = self.get_column(L1_TO_L2_PAID_FEE_BY_NONCE);

        for tx in txs {
            let nonce_bytes = tx.nonce.to_be_bytes();

            // Get the stored paid_fee_on_l1, falling back to default if not found
            // (for messages confirmed before this fix was implemented)
            let paid_fee_on_l1 =
                self.get_l1_handler_paid_fee_by_nonce(tx.nonce)?.unwrap_or(REVERT_PENDING_FEE_ON_L1_FALLBACK);

            // Remove consumed mapping (nonce -> tx hash)
            batch.delete_cf(&on_l2_cf, nonce_bytes);

            // Remove the stored fee (it will be re-stored when confirmed again)
            batch.delete_cf(&fee_cf, nonce_bytes);

            // Re-insert as pending with the actual paid_fee_on_l1
            let pending = L1HandlerTransactionWithFee::new(tx.clone(), paid_fee_on_l1);
            batch.put_cf(&pending_cf, nonce_bytes, super::serialize(&pending)?);
        }

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
