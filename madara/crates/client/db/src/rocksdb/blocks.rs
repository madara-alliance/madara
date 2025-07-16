use crate::{
    db::{EventFilter, GetEventContinuation, TxIndex},
    events_bloom_filter::{EventBloomReader, EventBloomSearcher},
    rocksdb::{iter_pinned::DBIterator, Column, RocksDBBackend},
    MadaraStorageError,
};
use itertools::Either;
use mp_block::{BlockHeaderWithSignatures, EventWithInfo, MadaraBlockInfo, TransactionWithReceipt};
use mp_convert::Felt;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use rocksdb::{Direction, IteratorMode, ReadOptions, WriteBatchWithTransaction};
use std::{iter};

/// <block_hash 32 bytes> => bincode(block_n)
pub(crate) const BLOCK_HASH_TO_BLOCK_N_COLUMN: &Column = &Column::new("block_hash_to_block_n").set_point_lookup();
/// <tx_hash 32 bytes> => bincode(block_n and tx_index)
pub(crate) const TX_HASH_TO_INDEX_COLUMN: &Column = &Column::new("tx_hash_to_index").set_point_lookup();
/// <block_n 4 bytes> => block_info
pub(crate) const BLOCK_INFO_COLUMN: &Column = &Column::new("block_info").set_point_lookup().use_blocks_mem_budget();
/// <block_n 4 bytes> => bincode(state diff)
pub(crate) const BLOCK_STATE_DIFF_COLUMN: &Column = &Column::new("block_state_diff").set_point_lookup();
pub(crate) const EVENT_BLOOM_COLUMN: &Column = &Column::new("event_bloom").set_point_lookup();

/// prefix [<block_n 4 bytes>] | <tx_index 4 bytes> => bincode(tx and receipt)
pub(crate) const BLOCK_TRANSACTIONS_COLUMN: &Column =
    &Column::new("block_inner").with_prefix_extractor_len(size_of::<u32>()).use_blocks_mem_budget();
const TRANSACTIONS_KEY_LEN: usize = 2 * size_of::<u32>();
fn make_transaction_column_key(block_n: u64, tx_index: u64) -> Result<[u8; TRANSACTIONS_KEY_LEN], MadaraStorageError> {
    let block_n = u32::try_from(block_n).map_err(|_| MadaraStorageError::InvalidBlockNumber)?;
    let tx_index = u32::try_from(tx_index).map_err(|_| MadaraStorageError::InvalidTxIndex)?;
    let mut key = [0u8; TRANSACTIONS_KEY_LEN];
    key[..4].copy_from_slice(&block_n.to_be_bytes());
    key[4..].copy_from_slice(&tx_index.to_be_bytes());
    Ok(key)
}

impl RocksDBBackend {
    #[tracing::instrument(skip(self))]
    pub(super) fn find_block_hash_impl(&self, block_hash: &Felt) -> Result<Option<u64>, MadaraStorageError> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN), &block_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn find_transaction_hash_impl(&self, tx_hash: &Felt) -> Result<Option<TxIndex>, MadaraStorageError> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(TX_HASH_TO_INDEX_COLUMN), &tx_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_info_impl(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>, MadaraStorageError> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_INFO_COLUMN), &block_n.to_be_bytes())? else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_state_diff_impl(&self, block_n: u64) -> Result<Option<StateDiff>, MadaraStorageError> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(BLOCK_STATE_DIFF_COLUMN), &block_n.to_be_bytes())?
        else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_transaction_impl(
        &self,
        block_n: u64,
        tx_index: u64,
    ) -> Result<Option<TransactionWithReceipt>, MadaraStorageError> {
        let Some(res) = self.db.get_pinned_cf(
            &self.get_column(BLOCK_TRANSACTIONS_COLUMN),
            &make_transaction_column_key(block_n, tx_index)?,
        )?
        else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_block_transactions_impl(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt, MadaraStorageError>> + '_ {
        let from = match make_transaction_column_key(block_n, from_tx_index) {
            Ok(from) => from,
            Err(err) => return Either::Left(iter::once(Err(err))),
        };

        let mut options = ReadOptions::default();
        options.set_prefix_same_as_start(true);
        let iter = DBIterator::new_cf(
            &self.db,
            &self.get_column(BLOCK_TRANSACTIONS_COLUMN),
            options,
            IteratorMode::From(&from, rocksdb::Direction::Forward),
        )
        .into_iter_values(|bytes| bincode::deserialize::<TransactionWithReceipt>(bytes))
        .map(|res| Ok(res??));

        Either::Right(iter)
    }

    /// Retrieves an iterator over event bloom filters starting from the specified block.
    ///
    /// This method returns an iterator that yields (block_number, bloom_filter) pairs,
    /// allowing for efficient filtering of potential blocks containing matching events.
    /// Only blocks containing events will have bloom filters, which is why we return
    /// the block number with each filter - this allows us to identify gaps in the sequence
    /// where blocks had no events.
    ///
    /// Note: The caller should consume this iterator quickly to avoid pinning RocksDB
    /// resources for an extended period.
    fn get_event_filter_stream(
        &self,
        block_n: u64,
    ) -> impl Iterator<Item = Result<(u64, EventBloomReader), MadaraStorageError>> + '_ {
        let col = self.get_column(EVENT_BLOOM_COLUMN);
        let block_n = block_n.to_be_bytes();
        let iter_mode = IteratorMode::From(&block_n, Direction::Forward);
        let iter = self.db.iterator_cf(&col, iter_mode);

        iter.map(|kvs| {
            kvs.map_err(MadaraStorageError::from).and_then(|(key, value)| {
                let stored_block_n: u64 = bincode::deserialize(&key).map_err(MadaraStorageError::from)?;
                let bloom = bincode::deserialize(&value).map_err(MadaraStorageError::from)?;
                Ok((stored_block_n, bloom))
            })
        })
    }

    /// Retrieves events that match the specified filter criteria within a block range.
    ///
    /// This implementation uses a two-phase filtering approach:
    /// 1. First use bloom filters to quickly identify blocks that *might* contain matching events
    /// 2. Then retrieve and process only those candidate blocks
    ///
    /// The method processes blocks incrementally to avoid keeping RocksDB iterators open for too long.
    ///
    /// ### Returns
    /// - A vector of events that match the filter criteria, up to `max_events` in size.
    /// - The returned events are collected across multiple blocks within the specified range.
    #[tracing::instrument(skip(self), fields(module = "BlockDB"))]
    pub fn get_filtered_events_impl(&self, filter: EventFilter) -> Result<(Vec<EventWithInfo>, Option<GetEventContinuation>), MadaraStorageError> {
        let key_filter = EventBloomSearcher::new(filter.from_address, filter.keys_pattern);
        let mut events_infos = Vec::new();
        let mut current_block = filter.start_block;

        'event_block_research: while current_block <= end_block && events_infos.len() < filter.max_events {
            'bloom_research: {
                // Scope the filter stream iterator to ensure it's dropped promptly
                let filter_event_stream = self.get_event_filter_stream(current_block)?;

                for filter_block in filter_event_stream {
                    let (block_n, bloom_filter) = filter_block?;

                    // Stop if we've gone beyond the requested range
                    if block_n > end_block {
                        break 'event_block_research;
                    }

                    // Use the bloom filter to quickly check if the block might contain relevant events.
                    // - This avoids unnecessary block retrieval if no matching events exist.
                    if key_filter.search(&bloom_filter) {
                        current_block = block_n;
                        break 'bloom_research;
                    }
                }
                // If no bloom filter was found, there's no more blocks whith events to process in DB.
                break 'event_block_research;
            } // RocksDB iterator is dropped here

            // Retrieve the full block data since we now suspect it contains relevant events.
            let block = self.get_block_info_impl(current_block)?.ok_or_else(|| {
                MadaraStorageError::InconsistentStorage(
                    format!("Bloom filter found but block not found for block {current_block}").into(),
                )
            })?;

            // Determine starting event index based on whether we're continuing from a previous query
            let skip_events = if current_block == start_block { start_event_index } else { 0 };

            // Extract matching events from the block
            let mut iter = block.
                .enumerate()
                .skip(skip_events)
                .filter(|(_, event)| filter.matches(&event.event));

            // Take exactly enough events to fill the requested chunk size.
            events_infos.extend(iter.by_ref().take(max_events - events_infos.len()).map(|(_, event)| event));

            current_block = current_block
                .checked_add(1)
                .ok_or(MadaraStorageError::InconsistentStorage("Block number overflow".into()))?;
        }

        Ok(events_infos)
    }

    pub fn store_block_header(&self, header: BlockHeaderWithSignatures) -> Result<MadaraBlockInfo, MadaraStorageError> {
        let mut tx = WriteBatchWithTransaction::default();
        let block_n = header.header.block_number;

        let block_hash_to_block_n_col = self.get_column(BLOCK_HASH_TO_BLOCK_N_COLUMN);
        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);

        let info = MadaraBlockInfo { header: header.header, block_hash: header.block_hash, tx_hashes: vec![] };

        tx.put_cf(&block_info_col, block_n.to_be_bytes(), bincode::serialize(&info)?);
        tx.put_cf(&block_hash_to_block_n_col, &header.block_hash.to_bytes_be(), &super::serialize_to_smallvec::<[u8; 16]>(&block_n)?);

        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(info)
    }

    pub fn store_transactions(
        &self,
        block_n: u64,
        value: &[TransactionWithReceipt],
    ) -> Result<(), MadaraStorageError> {
        // Save l1 core contract nonce to tx mapping.
        self.l1_db_save_transactions(
            value.iter().filter_map(|v| v.transaction.as_l1_handler().zip(v.receipt.as_l1_handler())),
        )?;

        let mut tx = WriteBatchWithTransaction::default();

        let block_info_col = self.get_column(BLOCK_INFO_COLUMN);
        let block_txs_col = self.get_column(BLOCK_TRANSACTIONS_COLUMN);
        let tx_hash_to_index_col = self.get_column(TX_HASH_TO_INDEX_COLUMN);

        for (tx_index, transaction) in value.iter().enumerate() {
            tx.put_cf(
                &tx_hash_to_index_col,
                transaction.receipt.transaction_hash().to_bytes_be(),
                super::serialize_to_smallvec::<[u8; 16]>(&TxIndex {
                    block_n,
                    tx_index: tx_index as _,
                })?,
            );
            tx.put_cf(&block_txs_col, make_transaction_column_key(block_n, tx_index as _)?, bincode::serialize(transaction)?);
        }

        // update block info tx hashes (we should get rid of this field at some point IMO)
        let mut block_info: MadaraBlockInfo =
            bincode::deserialize(&self.db.get_cf(&block_info_col, block_n.to_be_bytes())?.unwrap_or_default())?;
        block_info.tx_hashes = value.iter().map(|tx_with_receipt| *tx_with_receipt.receipt.transaction_hash()).collect();
        tx.put_cf(&block_info_col, block_n.to_be_bytes(), bincode::serialize(&block_info)?);

        self.db.write_opt(tx, &self.writeopts_no_wal)?;
        Ok(())
    }

    pub fn store_state_diff(&self, block_n: u64, value: StateDiff) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();

        let block_n_to_state_diff = self.get_column(Column::BlockNToStateDiff);
        let block_n_encoded = bincode::serialize(&block_n)?;
        batch.put_cf(&block_n_to_state_diff, &block_n_encoded, &bincode::serialize(&value)?);
        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        self.contract_db_store_block(block_n, ContractDbBlockUpdate::from_state_diff(value))?;

        Ok(())
    }

    pub fn store_events(&self, block_n: u64, value: Vec<EventWithTransactionHash>) -> Result<(), MadaraStorageError> {
        let mut batch = WriteBatchWithTransaction::default();

        let block_n_to_block_inner = self.get_column(Column::BlockNToBlockInner);
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
}
