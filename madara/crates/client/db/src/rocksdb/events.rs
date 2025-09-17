use crate::{
    prelude::*,
    rocksdb::{
        events_bloom_filter::{EventBloomReader, EventBloomSearcher, EventBloomWriter},
        iter_pinned::DBIterator,
        Column, RocksDBStorageInner, WriteBatchWithTransaction,
    },
    storage::EventFilter,
};
use itertools::{Either, Itertools};
use mp_block::EventWithInfo;
use mp_receipt::EventWithTransactionHash;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::iter;

// <block_n 4 bytes> => bloom
pub const EVENTS_BLOOM_COLUMN: Column = Column::new("events_bloom");

impl RocksDBStorageInner {
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
    fn get_event_filter_stream(&self, block_n: u64) -> impl Iterator<Item = Result<(u64, EventBloomReader)>> + '_ {
        let col = self.get_column(EVENTS_BLOOM_COLUMN);
        let Some(block_n) = u32::try_from(block_n).ok() else { return Either::Left(iter::empty()) }; // Every OOB block_n returns not found.
        let block_n_bytes = block_n.to_be_bytes();
        Either::Right(
            DBIterator::new_cf(
                &self.db,
                &col,
                ReadOptions::default(),
                IteratorMode::From(&block_n_bytes, Direction::Forward),
            )
            .into_iter_items(|(k, v)| {
                let block_n = u32::from_be_bytes(k.try_into().context("Expected key to be u32")?);
                anyhow::Ok((u64::from(block_n), super::deserialize(v).context("Deserializing event bloom filter")?))
            })
            .map(|res| res?),
        )
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
    #[tracing::instrument(skip(self))]
    pub(super) fn get_filtered_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        let key_filter = EventBloomSearcher::new(filter.from_address.as_ref(), filter.keys_pattern.as_deref());
        let mut events_infos = Vec::new();
        let mut current_block = filter.start_block;

        'event_block_research: while current_block <= filter.end_block && events_infos.len() < filter.max_events {
            'bloom_research: {
                // Scope the filter stream iterator to ensure it's dropped promptly
                let filter_event_stream = self.get_event_filter_stream(current_block);

                for filter_block in filter_event_stream {
                    let (block_n, bloom_filter) = filter_block?;

                    // Stop if we've gone beyond the requested range
                    if block_n > filter.end_block {
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

            // Determine starting event index based on whether we're continuing from a previous query
            let skip_events = if current_block == filter.start_block { filter.start_event_index } else { 0 };

            let block_info = self
                .get_block_info(current_block)?
                .with_context(|| format!("Block {current_block} in bloom column but not block info"))?;

            // Extract matching events from the block
            let block_txs = self.get_block_transactions(current_block, /* from_tx_index */ 0);
            block_txs.process_results(|iter| {
                events_infos.extend(
                    iter.enumerate()
                    .flat_map(|(tx_index, tx)| {
                        let transaction_hash = *tx.receipt.transaction_hash();
                        tx.receipt.into_events().into_iter().enumerate().map(move |(event_index, event)| {
                            EventWithInfo {
                                event,
                                block_number: current_block,
                                block_hash: Some(block_info.block_hash),
                                transaction_hash,
                                transaction_index: tx_index as _,
                                event_index_in_block: event_index as _,
                                in_preconfirmed: false
                            }
                        })
                    })
                    // Take exactly enough events to fill the requested chunk size.
                    .filter(|event| filter.matches(&event.event))
                    .skip(skip_events)
                    .take(filter.max_events - events_infos.len()),
                )
            })?;

            let Some(next_block) = current_block.checked_add(1) else { return Ok(events_infos) };
            current_block = next_block;
        }

        Ok(events_infos)
    }

    pub(super) fn store_events_bloom(&self, block_n: u64, value: &[EventWithTransactionHash]) -> Result<()> {
        if value.is_empty() {
            return Ok(());
        }

        let writer = EventBloomWriter::from_events(value.iter().map(|ev| &ev.event));
        let block_n = u32::try_from(block_n).context("Converting block_n to u32")?;

        self.db.put_cf_opt(
            &self.get_column(EVENTS_BLOOM_COLUMN),
            block_n.to_be_bytes(),
            &super::serialize(&writer)?,
            &self.writeopts_no_wal,
        )?;
        Ok(())
    }

    pub(super) fn events_remove_block(&self, block_n: u64, batch: &mut WriteBatchWithTransaction) -> Result<()> {
        let block_n_u32 = u32::try_from(block_n).context("Converting block_n to u32")?;
        batch.delete_cf(&self.get_column(EVENTS_BLOOM_COLUMN), block_n_u32.to_be_bytes());
        Ok(())
    }
}
