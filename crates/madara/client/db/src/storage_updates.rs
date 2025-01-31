use crate::contract_db::ContractDbBlockUpdate;
use crate::db_block_id::DbBlockId;
use crate::Column;
use crate::DatabaseExt;
use crate::MadaraBackend;
use crate::MadaraStorageError;
use crate::WriteBatchWithTransaction;
use mp_block::MadaraBlockInfo;
use mp_block::MadaraBlockInner;
use mp_block::TransactionWithReceipt;
use mp_block::{
    BlockHeaderWithSignatures, MadaraBlock, MadaraMaybePendingBlock, MadaraMaybePendingBlockInfo, MadaraPendingBlock,
};
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_receipt::TransactionReceipt;
use mp_state_update::StateDiff;

impl MadaraBackend {
    pub fn store_block_header(&self, header: BlockHeaderWithSignatures) -> Result<(), MadaraStorageError> {
        let mut tx = WriteBatchWithTransaction::default();
        let block_n = header.header.block_number;

        let block_hash_to_block_n = self.db.get_column(Column::BlockHashToBlockN);
        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);

        let info = MadaraBlockInfo { header: header.header, block_hash: header.block_hash, tx_hashes: vec![] };

        let block_n_encoded = bincode::serialize(&block_n)?;
        tx.put_cf(&block_n_to_block, block_n.to_be_bytes(), bincode::serialize(&info)?);
        tx.put_cf(&block_hash_to_block_n, &bincode::serialize(&header.block_hash)?, &block_n_encoded);

        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(())
    }

    pub fn store_transactions(
        &self,
        block_n: u64,
        value: Vec<TransactionWithReceipt>,
    ) -> Result<(), MadaraStorageError> {
        let mut tx = WriteBatchWithTransaction::default();

        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);
        let block_n_to_block_inner = self.db.get_column(Column::BlockNToBlockInner);

        let block_n_encoded = bincode::serialize(&block_n)?;

        // update block info tx hashes (we should get rid of this field at some point IMO)
        let mut block_info: MadaraBlockInfo =
            bincode::deserialize(&self.db.get_cf(&block_n_to_block, block_n.to_be_bytes())?.unwrap_or_default())?;
        block_info.tx_hashes = value.iter().map(|tx_with_receipt| tx_with_receipt.receipt.transaction_hash()).collect();
        tx.put_cf(&block_n_to_block, block_n.to_be_bytes(), bincode::serialize(&block_info)?);

        let (transactions, receipts) = value.into_iter().map(|t| (t.transaction, t.receipt)).unzip();
        let block_inner = MadaraBlockInner { transactions, receipts };
        tx.put_cf(&block_n_to_block_inner, &block_n_encoded, &bincode::serialize(&block_inner)?);

        // TODO: other columns

        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(())
    }

    pub fn store_state_diff(&self, block_n: u64, value: StateDiff) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();

        let block_n_to_state_diff = self.db.get_column(Column::BlockNToStateDiff);
        let block_n_encoded = bincode::serialize(&block_n)?;
        batch.put_cf(&block_n_to_state_diff, &block_n_encoded, &bincode::serialize(&value)?);
        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        self.contract_db_store_block(block_n, ContractDbBlockUpdate::from_state_diff(value))?;

        Ok(())
    }

    pub fn store_events(&self, block_n: u64, value: Vec<EventWithTransactionHash>) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();

        let block_n_to_block_inner = self.db.get_column(Column::BlockNToBlockInner);
        let block_n_encoded = bincode::serialize(&block_n)?;

        // update block transactions (TODO: we should separate receipts and events)
        let mut inner: MadaraBlockInner =
            bincode::deserialize(&self.db.get_cf(&block_n_to_block_inner, &block_n_encoded)?.unwrap_or_default())?;

        // just in case we stored them with receipt earlier, overwrite them
        for receipt in inner.receipts.iter_mut() {
            let events_mut = match receipt {
                TransactionReceipt::Invoke(receipt) => &mut receipt.events,
                TransactionReceipt::L1Handler(receipt) => &mut receipt.events,
                TransactionReceipt::Declare(receipt) => &mut receipt.events,
                TransactionReceipt::Deploy(receipt) => &mut receipt.events,
                TransactionReceipt::DeployAccount(receipt) => &mut receipt.events,
            };
            events_mut.clear()
        }

        let mut inner_m = inner.receipts.iter_mut().peekable();
        for ev in value {
            let receipt_mut = loop {
                let Some(receipt) = inner_m.peek_mut() else {
                    return Err(MadaraStorageError::InconsistentStorage(
                        format!("No transaction for hash {:#x} in block_n {block_n}", ev.transaction_hash).into(),
                    ));
                };

                if receipt.transaction_hash() == ev.transaction_hash {
                    break receipt;
                }
                let _item = inner_m.next();
            };

            let events_mut = match receipt_mut {
                TransactionReceipt::Invoke(receipt) => &mut receipt.events,
                TransactionReceipt::L1Handler(receipt) => &mut receipt.events,
                TransactionReceipt::Declare(receipt) => &mut receipt.events,
                TransactionReceipt::Deploy(receipt) => &mut receipt.events,
                TransactionReceipt::DeployAccount(receipt) => &mut receipt.events,
            };

            events_mut.push(ev.event);
        }

        batch.put_cf(&block_n_to_block_inner, &block_n_encoded, &bincode::serialize(&inner)?);
        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        Ok(())
    }
}

impl MadaraBackend {
    /// NB: This functions needs to run on the rayon thread pool
    pub fn store_block(
        &self,
        block: MadaraMaybePendingBlock,
        state_diff: StateDiff,
        converted_classes: Vec<ConvertedClass>,
    ) -> Result<(), MadaraStorageError> {
        let block_n = block.info.block_n();
        let state_diff_cpy = state_diff.clone();

        // Clear in every case, even when storing a pending block
        self.clear_pending_block()?;

        let task_block_db = || match block.info {
            MadaraMaybePendingBlockInfo::Pending(info) => {
                self.block_db_store_pending(&MadaraPendingBlock { info, inner: block.inner }, &state_diff_cpy)
            }
            MadaraMaybePendingBlockInfo::NotPending(info) => {
                self.block_db_store_block(&MadaraBlock { info, inner: block.inner }, &state_diff_cpy)
            }
        };

        let task_contract_db = || {
            let update = ContractDbBlockUpdate::from_state_diff(state_diff);

            match block_n {
                None => self.contract_db_store_pending(update),
                Some(block_n) => self.contract_db_store_block(block_n, update),
            }
        };

        let task_class_db = || match block_n {
            None => self.class_db_store_pending(&converted_classes),
            Some(block_n) => self.class_db_store_block(block_n, &converted_classes),
        };

        let ((r1, r2), r3) = rayon::join(|| rayon::join(task_block_db, task_contract_db), task_class_db);

        r1.and(r2).and(r3)?;

        self.snapshots.set_new_head(DbBlockId::from_block_n(block_n));
        Ok(())
    }

    pub fn clear_pending_block(&self) -> Result<(), MadaraStorageError> {
        self.block_db_clear_pending()?;
        self.contract_db_clear_pending()?;
        self.class_db_clear_pending()?;
        Ok(())
    }
}
