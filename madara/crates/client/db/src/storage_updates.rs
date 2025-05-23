use crate::contract_db::ContractDbBlockUpdate;
use crate::db_block_id::DbBlockId;
use crate::events_bloom_filter::EventBloomWriter;
use crate::Column;
use crate::DatabaseExt;
use crate::MadaraBackend;
use crate::MadaraStorageError;
use crate::WriteBatchWithTransaction;
use mp_block::commitments::CommitmentComputationContext;
use mp_block::FullBlock;
use mp_block::MadaraBlockInfo;
use mp_block::MadaraBlockInner;
use mp_block::MadaraPendingBlockInfo;
use mp_block::PendingFullBlock;
use mp_block::TransactionWithReceipt;
use mp_block::{BlockHeaderWithSignatures, MadaraPendingBlock};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::EventWithTransactionHash;
use mp_receipt::TransactionReceipt;
use mp_state_update::StateDiff;
use starknet_types_core::felt::Felt;

fn store_events_to_receipts(
    receipts: &mut [TransactionReceipt],
    events: Vec<EventWithTransactionHash>,
) -> Result<(), MadaraStorageError> {
    for receipt in receipts.iter_mut() {
        let events_mut = receipt.events_mut();
        // just in case we stored them with receipt earlier, overwrite them
        events_mut.clear()
    }

    let mut inner_m = receipts.iter_mut().peekable();
    for ev in events {
        let receipt_mut = loop {
            let Some(receipt) = inner_m.peek_mut() else {
                return Err(MadaraStorageError::InconsistentStorage(
                    format!("No transaction for hash {:#x}", ev.transaction_hash).into(),
                ));
            };

            if receipt.transaction_hash() == ev.transaction_hash {
                break receipt;
            }
            let _item = inner_m.next();
        };

        receipt_mut.events_mut().push(ev.event);
    }
    Ok(())
}

impl MadaraBackend {
    /// Add a new block to the db, calling the `on_full_block_imported` handler that handles flushes and backups when they are enabled,
    /// and update all the statuses.
    /// When using the MadaraBackend API to store new blocks, you either have the choice of using this simple function to store
    /// an entire new block, or, you are responsible for calling the `store_xxx`, applying global state, and calling the on_full_block_imported
    /// function when the full block is stored in the backend.
    ///
    /// This function takes a PendingFullBlock because the commitments can only be computed once we have applied the state to the global
    /// tries. This function will close the block and compute the commitments.
    ///
    /// The function returns the new computed block_hash.
    pub async fn add_full_block_with_classes(
        &self,
        block: PendingFullBlock,
        block_n: u64,
        converted_classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
    ) -> anyhow::Result<Felt> {
        let state_diff = block.state_diff.clone();

        let new_global_state_root = self.apply_to_global_trie(block_n, [&state_diff])?;

        let block = block.close_block(
            &CommitmentComputationContext {
                protocol_version: self.chain_config.latest_protocol_version,
                chain_id: self.chain_config.chain_id.to_felt(),
            },
            block_n,
            new_global_state_root,
            pre_v0_13_2_hash_override,
        );
        let block_hash = block.block_hash;

        let events = block.events.clone();

        let block_info = self.store_full_block(block)?;
        self.head_status.headers.set_current(Some(block_n));
        self.head_status.transactions.set_current(Some(block_n));
        self.head_status.state_diffs.set_current(Some(block_n));
        self.head_status.events.set_current(Some(block_n));

        self.store_block_classes(block_n, converted_classes)?;
        self.head_status.classes.set_current(Some(block_n));

        self.head_status.global_trie.set_current(Some(block_n));

        self.on_full_block_imported(block_info.into(), events).await?;
        self.flush()?;

        Ok(block_hash)
    }

    fn store_full_block(&self, block: FullBlock) -> Result<MadaraBlockInfo, MadaraStorageError> {
        let block_n = block.header.block_number;
        let block_info = self.store_block_header(BlockHeaderWithSignatures {
            header: block.header,
            block_hash: block.block_hash,
            consensus_signatures: vec![],
        })?;
        self.store_transactions(block_n, block.transactions)?;
        self.store_state_diff(block_n, block.state_diff)?;
        self.store_events(block_n, block.events)?;
        Ok(block_info)
    }

    pub fn store_pending_block_with_classes(
        &self,
        block: PendingFullBlock,
        converted_classes: &[ConvertedClass],
    ) -> Result<(), MadaraStorageError> {
        self.class_db_store_pending(converted_classes)?;
        self.store_pending_block(block)?;
        Ok(())
    }

    pub fn store_pending_block(&self, block: PendingFullBlock) -> Result<(), MadaraStorageError> {
        let info = MadaraPendingBlockInfo {
            header: block.header,
            tx_hashes: block.transactions.iter().map(|tx| tx.receipt.transaction_hash()).collect(),
        };
        let (transactions, receipts) = block.transactions.into_iter().map(|tx| (tx.transaction, tx.receipt)).unzip();
        let mut inner = MadaraBlockInner { transactions, receipts };
        store_events_to_receipts(&mut inner.receipts, block.events)?;

        self.block_db_store_pending(&MadaraPendingBlock { info: info.clone(), inner }, &block.state_diff)?;
        self.contract_db_store_pending(ContractDbBlockUpdate::from_state_diff(block.state_diff))?;

        self.watch_blocks.update_pending(info.into());
        Ok(())
    }

    pub fn store_block_header(&self, header: BlockHeaderWithSignatures) -> Result<MadaraBlockInfo, MadaraStorageError> {
        // Clear pending block when storing a new block header. This is the best place to do it IMO since
        // it would make no sense to be able to store a block header if there is also a pending block, and
        // we want to be sure to clear the pending block if we restart the sync pipeline.
        self.clear_pending_block()?;

        let mut tx = WriteBatchWithTransaction::default();
        let block_n = header.header.block_number;

        let block_hash_to_block_n = self.db.get_column(Column::BlockHashToBlockN);
        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);

        let info = MadaraBlockInfo { header: header.header, block_hash: header.block_hash, tx_hashes: vec![] };

        let block_n_encoded = bincode::serialize(&block_n)?;
        tx.put_cf(&block_n_to_block, block_n.to_be_bytes(), bincode::serialize(&info)?);
        tx.put_cf(&block_hash_to_block_n, &bincode::serialize(&header.block_hash)?, &block_n_encoded);

        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(info)
    }

    pub fn store_transactions(
        &self,
        block_n: u64,
        value: Vec<TransactionWithReceipt>,
    ) -> Result<(), MadaraStorageError> {
        let mut tx = WriteBatchWithTransaction::default();

        let block_n_to_block = self.db.get_column(Column::BlockNToBlockInfo);
        let block_n_to_block_inner = self.db.get_column(Column::BlockNToBlockInner);
        let tx_hash_to_block_n = self.db.get_column(Column::TxHashToBlockN);

        let block_n_encoded = bincode::serialize(&block_n)?;

        for transction in &value {
            tx.put_cf(
                &tx_hash_to_block_n,
                bincode::serialize(&transction.receipt.transaction_hash())?,
                &block_n_encoded,
            );
        }

        // update block info tx hashes (we should get rid of this field at some point IMO)
        let mut block_info: MadaraBlockInfo =
            bincode::deserialize(&self.db.get_cf(&block_n_to_block, block_n.to_be_bytes())?.unwrap_or_default())?;
        block_info.tx_hashes = value.iter().map(|tx_with_receipt| tx_with_receipt.receipt.transaction_hash()).collect();
        tx.put_cf(&block_n_to_block, block_n.to_be_bytes(), bincode::serialize(&block_info)?);

        let (transactions, receipts) = value.into_iter().map(|t| (t.transaction, t.receipt)).unzip();
        let block_inner = MadaraBlockInner { transactions, receipts };
        tx.put_cf(&block_n_to_block_inner, &block_n_encoded, &bincode::serialize(&block_inner)?);

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

        let events_bloom = {
            let mut events_iter = value.iter().map(|event_with_tx_hash| &event_with_tx_hash.event).peekable();
            if events_iter.peek().is_none() {
                None
            } else {
                // TODO: move this computation out of the storage layer
                Some(EventBloomWriter::from_events(events_iter))
            }
        };

        store_events_to_receipts(&mut inner.receipts, value)?;

        if let Some(events_bloom) = events_bloom {
            self.store_bloom(block_n, events_bloom)?;
        }

        batch.put_cf(&block_n_to_block_inner, &block_n_encoded, &bincode::serialize(&inner)?);
        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        Ok(())
    }

    fn store_bloom(&self, block_n: u64, bloom: EventBloomWriter) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();

        let block_n_to_bloom = self.db.get_column(Column::EventBloom);
        let block_n_encoded = bincode::serialize(&block_n)?;
        batch.put_cf(&block_n_to_bloom, &block_n_encoded, &bincode::serialize(&bloom)?);
        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        Ok(())
    }

    /// NB: This functions needs to run on the rayon thread pool
    /// todo: depreacate this function. It is only used in tests.
    // #[cfg(any(test, feature = "testing"))]
    pub fn store_block(
        &self,
        block: mp_block::MadaraMaybePendingBlock,
        state_diff: StateDiff,
        converted_classes: Vec<ConvertedClass>,
    ) -> Result<(), MadaraStorageError> {
        use mp_block::{MadaraBlock, MadaraMaybePendingBlockInfo};

        let block_n = block.info.block_n();
        let state_diff_cpy = state_diff.clone();

        let task_block_db = || match block.info {
            MadaraMaybePendingBlockInfo::Pending(info) => {
                self.block_db_store_pending(
                    &MadaraPendingBlock { info: info.clone(), inner: block.inner },
                    &state_diff_cpy,
                )?;
                self.watch_blocks.update_pending(info.into());
                Ok(())
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
            Some(block_n) => self.store_block_classes(block_n, &converted_classes),
        };

        let ((r1, r2), r3) = rayon::join(|| rayon::join(task_block_db, task_contract_db), task_class_db);

        r1.and(r2).and(r3)?;

        self.snapshots.set_new_head(crate::db_block_id::DbBlockId::from_block_n(block_n));

        if let Some(block_n) = block_n {
            self.head_status.full_block.set_current(Some(block_n));
            self.head_status.headers.set_current(Some(block_n));
            self.head_status.state_diffs.set_current(Some(block_n));
            self.head_status.transactions.set_current(Some(block_n));
            self.head_status.classes.set_current(Some(block_n));
            self.head_status.events.set_current(Some(block_n));
            self.head_status.global_trie.set_current(Some(block_n));
            self.save_head_status_to_db()?;
        }

        Ok(())
    }

    pub fn clear_pending_block(&self) -> Result<(), MadaraStorageError> {
        let parent_block = if let Some(block_n) = self.get_latest_block_n()? {
            Some(
                self.get_block_info(&DbBlockId::Number(block_n))?
                    .ok_or(MadaraStorageError::InconsistentStorage("Can't find block info".into()))?
                    .into_closed()
                    .ok_or(MadaraStorageError::InconsistentStorage(
                        "Got a pending block when trying to get a closed one".into(),
                    ))?,
            )
        } else {
            None
        };
        self.watch_blocks.clear_pending(parent_block.as_ref());
        self.block_db_clear_pending()?;
        self.contract_db_clear_pending()?;
        self.class_db_clear_pending()?;
        Ok(())
    }
}
