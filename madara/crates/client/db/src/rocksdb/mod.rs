use crate::{
    preconfirmed::PreconfirmedExecutedTransaction,
    prelude::*,
    rocksdb::{
        backup::BackupManager,
        column::{Column, ALL_COLUMNS},
        global_trie::{
            apply_to_global_trie, compute_global_trie_staged, get_state_root,
            in_memory::{
                compute_root_from_snapshot, compute_root_from_snapshot_sequential,
                compute_roots_in_parallel_from_snapshot, BonsaiOverlay, InMemoryRootComputation,
            },
            MerklizationTimings,
        },
        meta::StoredHeadProjectionWithoutContent,
        metrics::DbMetrics,
        options::rocksdb_global_options,
        snapshots::Snapshots,
    },
    storage::{
        ClassInfoWithBlockN, CompiledSierraWithBlockN, DevnetPredeployedKeys, EventFilter, MadaraStorageRead,
        MadaraStorageWrite, StorageHeadProjection, StorageTxIndex, StoredChainInfo,
    },
};

use bincode::Options;
use blockifier::bouncer::BouncerWeights;
use bonsai_trie::id::BasicId;

use mp_block::{EventWithInfo, MadaraBlockInfo, TransactionWithReceipt};
use mp_chain_config::StarknetVersion;
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::StateDiff;
use mp_transactions::{validated::ValidatedTransaction, L1HandlerTransactionWithFee};
use rocksdb::Options as RocksDBOptions;
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, FlushOptions, IteratorMode, MultiThreaded,
    WriteOptions,
};
use starknet_types_core::hash::StarkHash;
use std::{fmt, path::Path, sync::Arc, time::Instant};

mod backup;
mod blocks;
mod classes;
mod column;
mod events;
mod events_bloom_filter;
pub(crate) mod external_outbox;
mod iter_pinned;
mod l1_to_l2_messages;
mod mempool;
mod meta;
mod metrics;
mod options;
mod reorg;
mod rocksdb_snapshot;
mod snapshots;
mod state;

pub use snapshots::SnapshotRef;

// TODO: remove this pub. this is temporary until get_storage_proof is properly abstracted.
pub mod trie;
// TODO: remove this pub. this is temporary until get_storage_proof is properly abstracted.
pub mod global_trie;

type WriteBatchWithTransaction = rocksdb::WriteBatchWithTransaction<false>;
type DB = DBWithThreadMode<MultiThreaded>;

pub use options::{DbWriteMode, RocksDBConfig, StatsLevel};

const DB_UPDATES_BATCH_SIZE: usize = 1024;

fn bincode_opts() -> impl bincode::Options {
    bincode::DefaultOptions::new()
}

fn serialize_to_smallvec<A: smallvec::Array<Item = u8>>(
    value: &impl serde::Serialize,
) -> Result<smallvec::SmallVec<A>, bincode::Error> {
    let mut opt = bincode_opts();
    let mut v = smallvec::SmallVec::with_capacity((&mut opt).serialized_size(value)? as usize);
    // this *doesn't* call serialized_size under the hood - we have to do it ourselves to match this optimisation that `serialize` also benefits.
    opt.serialize_into(&mut v, value)?;
    Ok(v)
}

fn serialize(value: &impl serde::Serialize) -> Result<Vec<u8>, bincode::Error> {
    bincode_opts().serialize(value) // this calls serialized_size under the hood to get the vec capacity beforehand
}

fn deserialize<T: serde::de::DeserializeOwned>(bytes: impl AsRef<[u8]>) -> Result<T, bincode::Error> {
    bincode_opts().deserialize(bytes.as_ref())
}

pub(crate) struct RocksDBStorageInner {
    db: DB,
    global_opts: RocksDBOptions,
    writeopts: WriteOptions,
    config: RocksDBConfig,
}

impl Drop for RocksDBStorageInner {
    fn drop(&mut self) {
        tracing::debug!("⏳ Gracefully closing the database...");
        if let Err(error) = self.flush() {
            tracing::error!("Error when flushing the database during drop: {error:#}");
        }
        self.db.cancel_all_background_work(/* wait */ true);
    }
}

impl fmt::Debug for RocksDBStorageInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DBInner").field("config", &self.config).finish()
    }
}

impl RocksDBStorageInner {
    fn get_column(&self, col: Column) -> Arc<BoundColumnFamily<'_>> {
        let name = col.rocksdb_name;
        match self.db.cf_handle(name) {
            Some(column) => column,
            None => panic!("column {name} not initialized"),
        }
    }

    fn flush(&self) -> anyhow::Result<()> {
        tracing::debug!("doing a db flush");
        let mut opts = FlushOptions::default();
        opts.set_wait(true);
        // we have to collect twice here :/
        let columns = column::ALL_COLUMNS.iter().map(|e| self.get_column(e.clone())).collect::<Vec<_>>();
        let columns = columns.iter().collect::<Vec<_>>();

        self.db.flush_cfs_opt(&columns, &opts).context("Flushing database")?;

        Ok(())
    }

    /// This method also works for partially saved blocks. (that's important for mc-sync, which may create partial blocks past the head projection.
    /// We also want to remove them!)
    fn remove_all_blocks_starting_from(&self, starting_from_block_n: u64) -> Result<()> {
        // Find the last block. We want to revert blocks in reverse order to make sure we can recover if the node
        // crashes at any point during the call of this function.

        tracing::debug!("Remove blocks starting_from_block_n={starting_from_block_n}");

        let mut last_block_n_exclusive = starting_from_block_n;
        while self.get_block_info(last_block_n_exclusive)?.is_some() {
            last_block_n_exclusive += 1;
        }

        tracing::debug!("Removing blocks range {starting_from_block_n}..{last_block_n_exclusive} in reverse order");

        let mut earliest_reverted_l1_source_block = None;

        // Reverse order
        for block_n in (starting_from_block_n..last_block_n_exclusive).rev() {
            let block_info = self.get_block_info(block_n)?.context("Block should be found")?;
            tracing::debug!("Remove block block_n={block_n}");

            let mut batch = WriteBatchWithTransaction::default();
            {
                if let Some(state_diff) = self.get_block_state_diff(block_n)? {
                    // State diff is in db.
                    self.classes_revert_state_diff(&state_diff, &mut batch)?;
                    self.state_remove(block_n, &state_diff, &mut batch)?;
                }

                // This vec is empty if transactions for this block are not yet imported.
                let transactions: Vec<_> = self
                    .get_block_transactions(block_n, /* from_tx_index */ 0)
                    .take(block_info.tx_hashes.len())
                    .collect::<Result<_>>()?;

                self.events_remove_block(block_n, &mut batch)?;
                let l1_handler_nonces: Vec<u64> =
                    transactions.iter().filter_map(|v| v.transaction.as_l1_handler().map(|tx| tx.nonce)).collect();
                for nonce in l1_handler_nonces.iter().copied() {
                    if let Some(source_block) = self.get_l1_handler_l1_block_by_nonce(nonce)? {
                        earliest_reverted_l1_source_block = Some(
                            earliest_reverted_l1_source_block
                                .map_or(source_block, |current: u64| current.min(source_block)),
                        );
                    }
                }
                self.message_to_l2_revert_unconfirmed_consumption(&l1_handler_nonces, &mut batch)?;

                self.blocks_remove_block(&block_info, &mut batch)?;
            }

            self.db
                .write(batch)
                .with_context(|| format!("Committing changes removing block_n={block_n} from database"))?;
        }

        // Older versions marked an L1 message consumed as soon as transaction rows were written.
        // If startup removes one of those partial blocks, rewind far enough to reconstruct a
        // pending payload that may already have been deleted. New writes retain the pending row,
        // so this is primarily a backward-compatible recovery path.
        if let (Some(source_block), Some(current_sync_tip)) =
            (earliest_reverted_l1_source_block, self.get_l1_messaging_sync_tip()?)
        {
            let replay_tip = source_block.saturating_sub(1).min(current_sync_tip);
            if replay_tip < current_sync_tip {
                self.write_l1_messaging_sync_tip(Some(replay_tip))?;
                tracing::info!(
                    "Rewound L1 messaging sync tip from {current_sync_tip} to {replay_tip} after removing partial blocks"
                );
            }
        }

        Ok(())
    }

    /// Bonsai trie log keys are ordered by the committed revision id first.
    ///
    /// Madara uses `bonsai_trie::id::BasicId`, and bonsai serializes that id as a big-endian
    /// `u64` (`BasicId::to_bytes`). That means the lexicographically-last key in a trie-log
    /// column belongs to the latest committed revision for that trie.
    fn latest_bonsai_log_id(&self, column: Column) -> anyhow::Result<Option<u64>> {
        let handle = self.get_column(column);
        let mut iter = self.db.iterator_cf(&handle, IteratorMode::End);

        match iter.next() {
            None => Ok(None),
            Some(Ok((key, _))) => {
                let key = key.as_ref();
                anyhow::ensure!(
                    key.len() >= 8,
                    "Malformed bonsai trie log key: expected at least 8 bytes, got {}",
                    key.len()
                );

                let mut id_bytes = [0u8; 8];
                id_bytes.copy_from_slice(&key[..8]);
                Ok(Some(u64::from_be_bytes(id_bytes)))
            }
            Some(Err(err)) => Err(err).context("Reading latest bonsai trie log key"),
        }
    }
}

/// Implementation of [`MadaraStorageRead`] and [`MadaraStorageWrite`] interface using rocksdb.
#[derive(Debug, Clone)]
pub struct RocksDBStorage {
    inner: Arc<RocksDBStorageInner>,
    backup: BackupManager,
    snapshots: Arc<Snapshots>,
    metrics: DbMetrics,
}

mod backend;

mod storage_read;

mod storage_write;
