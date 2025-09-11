use crate::{
    prelude::*,
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBStorageInner},
    storage::StorageTxIndex,
};
use itertools::{Either, Itertools};
use mp_block::{BlockHeaderWithSignatures, MadaraBlockInfo, TransactionWithReceipt};
use mp_convert::Felt;
use mp_state_update::StateDiff;
use rocksdb::{IteratorMode, ReadOptions, WriteBatchWithTransaction};
use std::iter;

/// <block_hash 32 bytes> => bincode(block_n)
pub const BLOCK_HASH_TO_BLOCK_N_COLUMN: Column = Column::new("block_hash_to_block_n").set_point_lookup();
/// <tx_hash 32 bytes> => bincode(block_n and tx_index)
pub const TX_HASH_TO_INDEX_COLUMN: Column = Column::new("tx_hash_to_index").set_point_lookup();
/// <block_n 4 bytes> => block_info
pub const BLOCK_INFO_COLUMN: Column = Column::new("block_info").set_point_lookup().use_blocks_mem_budget();
/// <block_n 4 bytes> => bincode(state diff)
pub const BLOCK_STATE_DIFF_COLUMN: Column = Column::new("block_state_diff").set_point_lookup();

/// prefix [<block_n 4 bytes>] | <tx_index 2 bytes> => bincode(tx and receipt)
pub const BLOCK_TRANSACTIONS_COLUMN: Column =
    Column::new("block_transactions").with_prefix_extractor_len(size_of::<u32>()).use_blocks_mem_budget();

const TRANSACTIONS_KEY_LEN: usize = size_of::<u32>() + size_of::<u16>();
fn make_transaction_column_key(block_n: u32, tx_index: u16) -> [u8; TRANSACTIONS_KEY_LEN] {
    let mut key = [0u8; TRANSACTIONS_KEY_LEN];
    key[..4].copy_from_slice(&block_n.to_be_bytes());
    key[4..].copy_from_slice(&tx_index.to_be_bytes());
    key
}

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self))]
    pub(super) fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN), block_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize::<u32>(&res)?.into()))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<StorageTxIndex>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(TX_HASH_TO_INDEX_COLUMN), tx_hash.to_bytes_be())? else {
            return Ok(None);
        };
        let res = super::deserialize::<(u32, u16)>(&res)?;
        Ok(Some(StorageTxIndex { block_number: res.0.into(), transaction_index: res.1.into() }))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        let Some(block_n) = u32::try_from(block_n).ok() else { return Ok(None) }; // Every OOB block_n returns not found.
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_INFO_COLUMN), block_n.to_be_bytes())? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>> {
        let Some(block_n) = u32::try_from(block_n).ok() else { return Ok(None) }; // Every OOB block_n returns not found.
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_STATE_DIFF_COLUMN), block_n.to_be_bytes())? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        let Some((block_n, tx_index)) = Option::zip(u32::try_from(block_n).ok(), u16::try_from(tx_index).ok()) else {
            return Ok(None); // If it overflows u32, it means it is not found. (we can't store block_n > u32::MAX / tx_index > u16::MAX)
        };
        let Some(res) = self.db.get_pinned_cf(
            &self.get_column(BLOCK_TRANSACTIONS_COLUMN),
            make_transaction_column_key(block_n, tx_index),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_ {
        let Some((block_n, from_tx_index)) =
            Option::zip(u32::try_from(block_n).ok(), u16::try_from(from_tx_index).ok())
        else {
            return Either::Left(iter::empty()); // If it overflows u32, it means it is not found. (we can't store block_n > u32::MAX / tx_index > u16::MAX)
        };
        let from = make_transaction_column_key(block_n, from_tx_index);

        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let iter = DBIterator::new_cf(
            &self.db,
            &self.get_column(BLOCK_TRANSACTIONS_COLUMN),
            options,
            IteratorMode::From(&from, rocksdb::Direction::Forward),
        )
        .into_iter_values(|bytes| super::deserialize::<TransactionWithReceipt>(bytes))
        .map(|res| Ok(res??));

        Either::Right(iter)
    }

    #[tracing::instrument(skip(self, header))]
    pub(super) fn blocks_store_block_header(&self, header: BlockHeaderWithSignatures) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let block_n = u32::try_from(header.header.block_number).context("Converting block_n to u32")?;

        let block_hash_to_block_n_col = self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN);
        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);

        let info = MadaraBlockInfo {
            header: header.header,
            block_hash: header.block_hash,
            tx_hashes: vec![],
            total_l2_gas_used: 0,
        };

        batch.put_cf(&block_info_col, block_n.to_be_bytes(), super::serialize(&info)?);
        batch.put_cf(
            &block_hash_to_block_n_col,
            header.block_hash.to_bytes_be(),
            &super::serialize_to_smallvec::<[u8; 16]>(&block_n)?,
        );

        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_transactions(&self, block_number: u64, value: &[TransactionWithReceipt]) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();

        tracing::debug!(
            "Write {block_number} => {:?}",
            value.iter().map(|v| v.receipt.transaction_hash()).collect::<Vec<_>>()
        );

        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);
        let tx_hash_to_index_col = self.get_column(TX_HASH_TO_INDEX_COLUMN);

        let block_n_u32 = u32::try_from(block_number).context("Converting block_n to u32")?;

        for (tx_index, transaction) in value.iter().enumerate() {
            let tx_index_u16 = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(
                &tx_hash_to_index_col,
                transaction.receipt.transaction_hash().to_bytes_be(),
                super::serialize_to_smallvec::<[u8; 16]>(&(block_n_u32, tx_index_u16))?,
            );
            batch.put_cf(
                &block_txs_col,
                make_transaction_column_key(block_n_u32, tx_index_u16),
                super::serialize(transaction)?,
            );
        }

        // Update block info tx hashes
        // Also update total_l2_gas_used.
        let mut block_info: MadaraBlockInfo = super::deserialize(
            &self.db.get_pinned_cf(&block_info_col, block_n_u32.to_be_bytes())?.context("Block info not found")?,
        )?;
        block_info.total_l2_gas_used = value.iter().map(|tx| tx.receipt.l2_gas_used()).sum();
        block_info.tx_hashes =
            value.iter().map(|tx_with_receipt| *tx_with_receipt.receipt.transaction_hash()).collect();
        batch.put_cf(&block_info_col, block_n_u32.to_be_bytes(), super::serialize(&block_info)?);

        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_state_diff(&self, block_number: u64, value: &StateDiff) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        let block_n_u32 = u32::try_from(block_number).context("Converting block_n to u32")?;

        let block_n_to_state_diff = self.get_column(BLOCK_STATE_DIFF_COLUMN);
        batch.put_cf(&block_n_to_state_diff, block_n_u32.to_be_bytes(), &super::serialize(value)?);
        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        Ok(())
    }

    #[tracing::instrument(skip(self, value))]
    pub(super) fn blocks_store_events_to_receipts(
        &self,
        block_n: u64,
        value: &[mp_receipt::EventWithTransactionHash],
    ) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();

        let mut events = value.iter().peekable();
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);

        for (tx_index, transaction) in self.get_block_transactions(block_n, /* from_tx_index */ 0).enumerate() {
            let mut transaction = transaction?;
            let transaction_hash = *transaction.receipt.transaction_hash();

            transaction.receipt.events_mut().clear();
            transaction.receipt.events_mut().extend(
                events.peeking_take_while(|tx| tx.transaction_hash == transaction_hash).map(|tx| tx.event.clone()),
            );

            let block_n = u32::try_from(block_n).context("Converting block_n to u32")?;
            let tx_index = u16::try_from(tx_index).context("Converting tx_index to u16")?;
            batch.put_cf(
                &block_txs_col,
                make_transaction_column_key(block_n, tx_index),
                super::serialize(&transaction)?,
            );
        }

        self.db.write_opt(batch, &self.writeopts_no_wal)?;
        Ok(())
    }
}
