use crate::{
    preconfirmed::PreconfirmedExecutedTransaction,
    prelude::*,
    rocksdb::{
        backup::BackupManager,
        column::{Column, ALL_COLUMNS},
        global_trie::{
            apply_to_global_trie, get_state_root,
            in_memory::{self, BonsaiOverlay, InMemoryRootComputation, TrieLogMode},
            MerklizationTimings,
        },
        meta::{ParallelMerkleStagedState, StoredChainTipWithoutContent},
        metrics::DbMetrics,
        options::rocksdb_global_options,
        snapshots::Snapshots,
    },
    storage::{
        ClassInfoWithBlockN, CompiledSierraWithBlockN, DevnetPredeployedKeys, EventFilter, MadaraStorageRead,
        MadaraStorageWrite, StorageChainTip, StorageTxIndex, StoredChainInfo,
    },
};

use bincode::Options;
use blockifier::bouncer::BouncerWeights;
use bonsai_trie::id::BasicId;

use mp_block::{
    BlockHeaderWithSignatures, EventWithInfo, FullBlockWithoutCommitments, MadaraBlockInfo, TransactionWithReceipt,
};
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::StateDiff;
use mp_transactions::{validated::ValidatedTransaction, L1HandlerTransactionWithFee};
use rocksdb::{BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, FlushOptions, MultiThreaded, WriteOptions};
use std::{fmt, path::Path, sync::Arc};

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
mod preconfirmed;
mod rocksdb_snapshot;
mod snapshots;
mod state;

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

struct RocksDBStorageInner {
    db: DB,
    writeopts: WriteOptions,
    config: RocksDBConfig,
}

impl Drop for RocksDBStorageInner {
    fn drop(&mut self) {
        tracing::debug!("⏳ Gracefully closing the database...");
        self.flush().expect("Error when flushing the database");
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

    /// This method also works for partially saved blocks. (that's important for mc-sync, which may create partial blocks past the chain tip.
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

        // Reverse order
        for block_n in (starting_from_block_n..last_block_n_exclusive).rev() {
            let block_info = self.get_block_info(block_n)?.context("Block should be found")?;
            tracing::debug!("Remove block block_n={block_n}");

            let mut batch = WriteBatchWithTransaction::default();
            {
                if let Some(state_diff) = self.get_block_state_diff(block_n)? {
                    // State diff is in db.
                    self.classes_remove(state_diff.all_declared_classes(), &mut batch)?;
                    self.state_remove(block_n, &state_diff, &mut batch)?;
                }

                // This vec is empty if transactions for this block are not yet imported.
                let transactions: Vec<_> = self
                    .get_block_transactions(block_n, /* from_tx_index */ 0)
                    .take(block_info.tx_hashes.len())
                    .collect::<Result<_>>()?;

                self.events_remove_block(block_n, &mut batch)?;
                self.message_to_l2_remove_txns(
                    transactions.iter().filter_map(|v| v.transaction.as_l1_handler()).map(|tx| tx.nonce),
                    &mut batch,
                )?;

                self.blocks_remove_block(&block_info, &mut batch)?;
            }

            self.db
                .write(batch)
                .with_context(|| format!("Committing changes removing block_n={block_n} from database"))?;
        }

        Ok(())
    }

    /// Persist staged data for a block in resumable multi-stage commits.
    ///
    /// Flow:
    /// ```text
    /// Stage 1 (atomic batch):
    ///   remove pending L1->L2 nonces
    ///   + write staged header
    ///   + write txs + tx indices
    ///   + STAGED_STATE = Txns
    ///
    /// Stage 2 (existing writers):
    ///   materialize events into receipts
    ///   + compute/store commitments + staged block info
    ///   + write state diff + state/class updates + bouncer weights
    ///   + STAGED_STATE = Diff
    ///
    /// Stage 3:
    ///   store events bloom
    ///   + STAGED_STATE = Final
    /// ```
    ///
    /// Resume rule: stage state in META is the only progress marker and is updated last in each stage.
    fn write_parallel_merkle_staged_block_data(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        bouncer_weights: Option<&BouncerWeights>,
        consumed_core_contract_nonces: impl IntoIterator<Item = u64>,
        chain_id: Felt,
    ) -> Result<()> {
        let block_n = block.header.block_number;
        let latest_confirmed = match self.get_chain_tip_without_content()? {
            Some(StoredChainTipWithoutContent::Confirmed(n)) => Some(n),
            Some(StoredChainTipWithoutContent::Preconfirmed(header)) => header.block_number.checked_sub(1),
            None => None,
        };
        if latest_confirmed.is_some_and(|n| block_n <= n) {
            anyhow::bail!("cannot stage block_n={block_n}: block header already confirmed");
        }

        if self.parallel_merkle_has_staged_block(block_n)? {
            anyhow::bail!("parallel-merkle staged data already exists for block_n={block_n}");
        }

        // Staging is done in multiple commits to reduce batch size.
        // A single per-block staged-state key in META_COLUMN allows idempotent resume.
        let staged_state = self.parallel_merkle_get_staged_state(block_n)?;

        // Stage 1: Pending L1->L2 nonce removals + transactions + indices + staged header + txns marker (single atomic batch).
        if staged_state.is_none() {
            let mut batch = WriteBatchWithTransaction::default();
            self.remove_pending_messages_to_l2_in_batch(consumed_core_contract_nonces, &mut batch);
            self.parallel_merkle_set_staged_header(block_n, &block.header, &mut batch)?;
            self.parallel_merkle_set_staged_state(block_n, ParallelMerkleStagedState::Txns, &mut batch)?;
            self.blocks_store_transactions_with_indices_to_batch(block_n, &block.transactions, &mut batch)?;
            self.db.write_opt(batch, &self.writeopts)?;
        }

        // Stage 2: Materialize receipt events + commitments/staged block info + state diff/DB updates/classes.
        // These are written via existing code paths which may commit internally; staged state is updated last.
        if matches!(staged_state, None | Some(ParallelMerkleStagedState::Txns)) {
            self.blocks_store_events_to_receipts(block_n, &block.events)?;

            let commitments = self.blocks_compute_staged_commitments(block, chain_id);
            let tx_hashes = block.transactions.iter().map(|tx| *tx.receipt.transaction_hash()).collect::<Vec<_>>();
            let total_l2_gas_used = block.transactions.iter().map(|tx| tx.receipt.l2_gas_used()).sum::<u128>();
            self.blocks_store_staged_block_info(&block.header, &tx_hashes, total_l2_gas_used, &commitments)?;

            self.blocks_store_state_diff(block_n, &block.state_diff)?;
            self.state_apply_state_diff(block_n, &block.state_diff)?;

            if let Some(weights) = bouncer_weights {
                self.blocks_store_bouncer_weights(block_n, weights)?;
            }

            if !block.state_diff.migrated_compiled_classes.is_empty() {
                let migrations = block
                    .state_diff
                    .migrated_compiled_classes
                    .iter()
                    .map(|m| (m.class_hash, m.compiled_class_hash))
                    .collect::<Vec<_>>();
                self.update_class_v2_hashes(migrations)?;
            }
            self.store_classes(block_n, classes)?;

            let mut batch = WriteBatchWithTransaction::default();
            self.parallel_merkle_set_staged_state(block_n, ParallelMerkleStagedState::Diff, &mut batch)?;
            self.db.write_opt(batch, &self.writeopts)?;
        }

        // Stage 3: Events bloom + final staged marker.
        if !matches!(staged_state, Some(ParallelMerkleStagedState::Final)) {
            self.store_events_bloom(block_n, &block.events)?;
            let mut batch = WriteBatchWithTransaction::default();
            self.parallel_merkle_set_staged_state(block_n, ParallelMerkleStagedState::Final, &mut batch)?;
            self.db.write_opt(batch, &self.writeopts)?;
        }
        Ok(())
    }

    /// Confirm a block only after staged state is `Final`.
    ///
    /// This atomically writes confirmed header info, advances chain tip, and clears staged metadata.
    fn confirm_parallel_merkle_staged_block(&self, header: &BlockHeaderWithSignatures) -> Result<()> {
        let block_n = header.header.block_number;
        if !self.parallel_merkle_has_staged_block(block_n)? {
            anyhow::bail!("cannot confirm block_n={block_n}: staged marker not found");
        }
        let staged_info = self
            .get_block_info(block_n)?
            .with_context(|| format!("cannot confirm block_n={block_n}: staged block info not found"))?;

        // If block production already advanced the chain tip to a newer preconfirmed block,
        // we must not clear the preconfirmed column nor overwrite the chain tip meta entry.
        // Doing so would drop the active preconfirmed block and break executor/finalizer overlap.
        let keep_preconfirmed = matches!(
            self.get_chain_tip_without_content()?,
            Some(StoredChainTipWithoutContent::Preconfirmed(header)) if header.block_number > block_n
        );

        let mut batch = WriteBatchWithTransaction::default();
        self.blocks_store_header_with_info_to_batch(
            header,
            &staged_info.tx_hashes,
            staged_info.total_l2_gas_used,
            &mut batch,
        )?;
        if !keep_preconfirmed {
            self.replace_chain_tip_with_confirmed_in_batch(block_n, &mut batch)?;
        }
        self.parallel_merkle_clear_staged_block(block_n, &mut batch);

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
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

impl RocksDBStorage {
    pub fn open(path: &Path, config: RocksDBConfig) -> Result<Self> {
        let opts = rocksdb_global_options(&config)?;
        tracing::debug!("Opening db at {:?}", path.display());
        let db = DB::open_cf_descriptors(
            &opts,
            path,
            ALL_COLUMNS.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(&config))),
        )?;

        let writeopts = config.write_mode.to_write_options();
        tracing::info!("📝 Database write mode: {}", config.write_mode);
        let inner = Arc::new(RocksDBStorageInner { writeopts, db, config: config.clone() });

        let head_block_n = inner.get_chain_tip_without_content()?.and_then(|c| match c {
            StoredChainTipWithoutContent::Confirmed(block_n) => Some(block_n),
            StoredChainTipWithoutContent::Preconfirmed(header) => header.block_number.checked_sub(1),
        });

        let snapshot = Snapshots::new(inner.clone(), head_block_n, config.max_kept_snapshots, config.snapshot_interval);

        Ok(Self {
            inner,
            snapshots: snapshot.into(),
            metrics: DbMetrics::register().context("Registering database metrics")?,
            backup: BackupManager::start_if_enabled(path, &config).context("Startup backup manager")?,
        })
    }

    /// Flush all pending writes to disk. This is important when WAL is disabled.
    /// Should be called before shutdown to ensure data persistence.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    /// Get a reference to the underlying RocksDB instance.
    ///
    /// This is primarily used for database migrations that need direct access
    /// to the raw DB for low-level operations.
    ///
    /// # Warning
    ///
    /// Direct manipulation of the DB can lead to data corruption if not done
    /// carefully. This should only be used by the migration system.
    pub fn inner_db(&self) -> &DB {
        &self.inner.db
    }

    pub fn write_parallel_merkle_staged_block_data_with_consumed_nonces(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        bouncer_weights: Option<&BouncerWeights>,
        consumed_core_contract_nonces: impl IntoIterator<Item = u64>,
        chain_id: Felt,
    ) -> Result<()> {
        self.inner.write_parallel_merkle_staged_block_data(
            block,
            classes,
            bouncer_weights,
            consumed_core_contract_nonces,
            chain_id,
        )
    }

    pub fn compute_parallel_merkle_root_from_snapshot_base(
        &self,
        snapshot_base_block_n: u64,
        block_n: u64,
        state_diff: &StateDiff,
        include_overlay: bool,
        trie_log_mode: TrieLogMode,
    ) -> Result<InMemoryRootComputation> {
        let (snapshot_block_n, snapshot) =
            self.snapshots.get_at_or_before(snapshot_base_block_n).with_context(|| {
                format!("No compatible snapshot at-or-before requested snapshot_base_block_n={snapshot_base_block_n}")
            })?;
        in_memory::compute_root_from_snapshot(self, snapshot, block_n, state_diff, include_overlay, trie_log_mode)
            .with_context(|| {
                format!(
                    "Computing parallel merkle root from snapshot_base_block_n={snapshot_base_block_n} using snapshot_at={snapshot_block_n:?} for block_n={block_n}"
                )
            })
    }

    pub fn flush_parallel_merkle_overlay_and_checkpoint(
        &self,
        block_n: u64,
        overlay: &BonsaiOverlay,
        trie_log_mode: TrieLogMode,
    ) -> Result<()> {
        in_memory::flush_overlay_and_checkpoint(self, block_n, overlay, trie_log_mode)
            .with_context(|| format!("Flushing parallel merkle overlay and checkpointing block_n={block_n}"))
    }

    /// Resolve reorg base to the nearest persisted parallel-merkle checkpoint at-or-before `requested_block_n`.
    ///
    /// This base checkpoint is used to restore trie state first, then we can replay state-diffs
    /// up to the exact requested block.
    fn resolve_revert_target_block_n(&self, requested_block_n: u64) -> Result<u64> {
        Ok(self.inner.get_parallel_merkle_checkpoint_at_or_before(requested_block_n)?.unwrap_or(requested_block_n))
    }

    fn collect_state_diffs_inclusive(&self, from_block_n: u64, to_block_n: u64) -> Result<Vec<StateDiff>> {
        if from_block_n > to_block_n {
            return Ok(Vec::new());
        }

        let mut state_diffs = Vec::with_capacity((to_block_n - from_block_n + 1) as usize);
        for block_n in from_block_n..=to_block_n {
            let state_diff = self
                .inner
                .get_block_state_diff(block_n)
                .with_context(|| format!("Loading block state diff for replay block_n={block_n}"))?
                .ok_or_else(|| anyhow::anyhow!("Missing block state diff for replay block_n={block_n}"))?;
            state_diffs.push(state_diff);
        }

        Ok(state_diffs)
    }

    fn replay_tries_from_floor_to_target(
        &self,
        floor_block_n: u64,
        target_block_n: u64,
        expected_state_root: Felt,
    ) -> Result<()> {
        if target_block_n <= floor_block_n {
            return Ok(());
        }

        let replay_start_block_n =
            floor_block_n.checked_add(1).ok_or_else(|| anyhow::anyhow!("Replay start block overflow"))?;
        let replay_diffs = self
            .collect_state_diffs_inclusive(replay_start_block_n, target_block_n)
            .with_context(|| {
                format!(
                    "Collecting replay state diffs in range [{replay_start_block_n}..={target_block_n}] after floor={floor_block_n}"
                )
            })?;

        tracing::info!(
            floor_block_n,
            target_block_n,
            replay_start_block_n,
            replay_diffs_count = replay_diffs.len(),
            "🌳 REORG: Replaying state diffs from floor checkpoint to exact requested target",
        );

        let (replayed_root, _timings) = apply_to_global_trie(self, replay_start_block_n, replay_diffs.iter())
            .with_context(|| {
                format!("Replaying global trie state diffs in range [{replay_start_block_n}..={target_block_n}]")
            })?;

        if replayed_root != expected_state_root {
            anyhow::bail!(
                "Replayed trie root mismatch at target block_n={target_block_n}: expected={expected_state_root:#x}, replayed={replayed_root:#x}"
            );
        }

        let persisted_root = get_state_root(self)
            .with_context(|| format!("Reading persisted state root after replay at block_n={target_block_n}"))?;
        if persisted_root != replayed_root {
            anyhow::bail!(
                "Persisted trie root mismatch after replay at block_n={target_block_n}: persisted={persisted_root:#x}, replayed={replayed_root:#x}"
            );
        }

        Ok(())
    }
}

impl MadaraStorageRead for RocksDBStorage {
    // Blocks

    fn find_block_hash(&self, block_hash: &Felt) -> Result<Option<u64>> {
        self.inner
            .find_block_hash(block_hash)
            .with_context(|| format!("Finding block number for block_hash={block_hash:#x}"))
    }
    fn find_transaction_hash(&self, tx_hash: &Felt) -> Result<Option<StorageTxIndex>> {
        self.inner
            .find_transaction_hash(tx_hash)
            .with_context(|| format!("Finding transaction index for tx_hash={tx_hash:#x}"))
    }
    fn get_block_info(&self, block_n: u64) -> Result<Option<MadaraBlockInfo>> {
        self.inner.get_block_info(block_n).with_context(|| format!("Getting block info for block_n={block_n}"))
    }
    fn get_block_state_diff(&self, block_n: u64) -> Result<Option<StateDiff>> {
        self.inner
            .get_block_state_diff(block_n)
            .with_context(|| format!("Getting block state diff for block_n={block_n}"))
    }

    fn get_block_bouncer_weights(&self, block_n: u64) -> Result<Option<BouncerWeights>> {
        self.inner
            .get_block_bouncer_weight(block_n)
            .with_context(|| format!("Getting block bouncer weights for block_n={block_n}"))
    }
    fn get_transaction(&self, block_n: u64, tx_index: u64) -> Result<Option<TransactionWithReceipt>> {
        self.inner
            .get_transaction(block_n, tx_index)
            .with_context(|| format!("Getting block transaction for block_n={block_n} tx_index={tx_index}"))
    }
    fn get_block_transactions(
        &self,
        block_n: u64,
        from_tx_index: u64,
    ) -> impl Iterator<Item = Result<TransactionWithReceipt>> + '_ {
        self.inner.get_block_transactions(block_n, from_tx_index).map(move |e| {
            e.with_context(|| format!("Getting block transactions for block_n={block_n} from_tx_index={from_tx_index}"))
        })
    }

    // State

    fn get_storage_at(&self, block_n: u64, contract_address: &Felt, key: &Felt) -> Result<Option<Felt>> {
        self.inner.get_storage_at(block_n, contract_address, key).with_context(|| {
            format!("Getting storage value for block_n={block_n} contract_address={contract_address:#x} key={key:#x}")
        })
    }
    fn get_contract_nonce_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner
            .get_contract_nonce_at(block_n, contract_address)
            .with_context(|| format!("Getting nonce for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn get_contract_class_hash_at(&self, block_n: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner
            .get_contract_class_hash_at(block_n, contract_address)
            .with_context(|| format!("Getting class_hash for block_n={block_n} contract_address={contract_address:#x}"))
    }
    fn is_contract_deployed_at(&self, block_n: u64, contract_address: &Felt) -> Result<bool> {
        self.inner.is_contract_deployed_at(block_n, contract_address).with_context(|| {
            format!("Checking if contract is deployed for block_n={block_n} contract_address={contract_address:#x}")
        })
    }

    // Classes

    fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>> {
        self.inner.get_class(class_hash).with_context(|| format!("Getting class info for class_hash={class_hash:#x}"))
    }
    fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<CompiledSierraWithBlockN>> {
        self.inner
            .get_class_compiled(compiled_class_hash)
            .with_context(|| format!("Getting class compiled for compiled_class_hash={compiled_class_hash:#x}"))
    }

    // Events

    fn get_events(&self, filter: EventFilter) -> Result<Vec<EventWithInfo>> {
        self.inner.get_filtered_events(filter.clone()).with_context(|| format!("Getting events for filter={filter:?}"))
    }

    // Meta

    fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        self.inner.get_devnet_predeployed_keys().context("Getting devnet predeployed contracts keys")
    }
    fn get_chain_tip(&self) -> Result<StorageChainTip> {
        self.inner.get_chain_tip().context("Getting chain tip from db")
    }
    fn get_latest_confirmed_block_n(&self) -> Result<Option<u64>> {
        self.inner.blocks_latest_confirmed_block_n().context("Getting latest confirmed block number from db")
    }
    fn get_confirmed_on_l1_tip(&self) -> Result<Option<u64>> {
        self.inner.get_confirmed_on_l1_tip().context("Getting confirmed block on l1 tip")
    }
    fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        self.inner.get_l1_messaging_sync_tip().context("Getting l1 messaging sync tip")
    }
    fn get_stored_chain_info(&self) -> Result<Option<StoredChainInfo>> {
        self.inner.get_stored_chain_info().context("Getting stored chain info from db")
    }
    fn get_latest_applied_trie_update(&self) -> Result<Option<u64>> {
        self.inner.get_latest_applied_trie_update().context("Getting latest applied trie update info from db")
    }
    fn get_runtime_exec_config(
        &self,
        backend_chain_config: &mp_chain_config::ChainConfig,
    ) -> Result<Option<mp_chain_config::RuntimeExecutionConfig>> {
        self.inner.get_runtime_exec_config(backend_chain_config).context("Getting runtime execution config from db")
    }
    fn get_snap_sync_latest_block(&self) -> Result<Option<u64>> {
        self.inner.get_snap_sync_latest_block().context("Getting snap sync latest block from db")
    }
    fn has_parallel_merkle_staged_block(&self, block_n: u64) -> Result<bool> {
        self.inner
            .parallel_merkle_has_staged_block(block_n)
            .with_context(|| format!("Checking parallel merkle staged marker for block_n={block_n}"))
    }
    fn get_parallel_merkle_staged_block_header(
        &self,
        block_n: u64,
    ) -> Result<Option<mp_block::header::PreconfirmedHeader>> {
        self.inner
            .parallel_merkle_get_staged_block_header(block_n)
            .with_context(|| format!("Getting parallel merkle staged header for block_n={block_n}"))
    }
    fn get_parallel_merkle_staged_blocks(&self) -> Result<Vec<u64>> {
        self.inner.parallel_merkle_get_staged_blocks().context("Getting parallel merkle staged blocks")
    }
    fn has_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<bool> {
        self.inner
            .has_parallel_merkle_checkpoint(block_n)
            .with_context(|| format!("Checking parallel merkle checkpoint for block_n={block_n}"))
    }
    fn get_parallel_merkle_latest_checkpoint(&self) -> Result<Option<u64>> {
        self.inner.get_parallel_merkle_latest_checkpoint().context("Getting latest parallel merkle checkpoint")
    }

    // L1 to L2 messages

    fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.inner
            .get_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Getting pending message to l2 with nonce={core_contract_nonce}"))
    }
    fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.inner
            .get_next_pending_message_to_l2(start_nonce)
            .with_context(|| format!("Getting next pending message to l2 with start_nonce={start_nonce}"))
    }
    fn get_l1_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<mp_convert::L1TransactionHash>> {
        self.inner
            .get_l1_txn_hash_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting l1 txn hash by nonce={core_contract_nonce}"))
    }
    fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.inner
            .get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
            .with_context(|| format!("Getting next pending message to l2 with nonce={core_contract_nonce}"))
    }
    fn get_messages_to_l2_by_l1_tx_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<Option<crate::storage::L1ToL2MessagesByL1TxHash>> {
        self.inner
            .get_messages_to_l2_by_l1_tx_hash(l1_tx_hash)
            .with_context(|| format!("Getting messages to l2 by l1_tx_hash_bytes={:?}", l1_tx_hash.0))
    }
    fn get_message_to_l2_index_entry(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<Option<crate::storage::L1ToL2MessageIndexEntry>> {
        self.inner.get_message_to_l2_index_entry(l1_tx_hash, core_contract_nonce).with_context(|| {
            format!(
                "Getting l1->l2 message index entry l1_tx_hash_bytes={:?} nonce={core_contract_nonce}",
                l1_tx_hash.0
            )
        })
    }

    // Mempool

    fn get_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        self.inner.get_mempool_transactions().map(|res| res.context("Getting mempool transactions"))
    }
    fn get_external_outbox_transactions(
        &self,
        limit: usize,
    ) -> impl Iterator<Item = Result<external_outbox::ExternalOutboxEntry>> + '_ {
        self.inner.iter_external_outbox(limit).map(|res| res.context("Getting external outbox transactions"))
    }

    fn get_external_outbox_size_estimate(&self) -> Result<u64> {
        self.inner.external_outbox_size_estimate().context("Getting external outbox size estimate")
    }
}

impl MadaraStorageWrite for RocksDBStorage {
    fn write_parallel_merkle_staged_block_data(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        bouncer_weights: Option<&BouncerWeights>,
        chain_id: Felt,
    ) -> Result<()> {
        tracing::debug!("Writing parallel merkle staged block data block_n={}", block.header.block_number);
        self.write_parallel_merkle_staged_block_data_with_consumed_nonces(
            block,
            classes,
            bouncer_weights,
            std::iter::empty(),
            chain_id,
        )
        .with_context(|| format!("Writing parallel merkle staged block data block_n={}", block.header.block_number))
    }

    fn confirm_parallel_merkle_staged_block(&self, header: BlockHeaderWithSignatures) -> Result<()> {
        let block_n = header.header.block_number;
        tracing::debug!("Confirming parallel merkle staged block block_n={block_n}");
        self.inner
            .confirm_parallel_merkle_staged_block(&header)
            .with_context(|| format!("Confirming parallel merkle staged block block_n={block_n}"))
    }

    fn write_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<()> {
        tracing::debug!("Writing parallel merkle checkpoint block_n={block_n}");
        self.inner
            .write_parallel_merkle_checkpoint(block_n)
            .with_context(|| format!("Writing parallel merkle checkpoint block_n={block_n}"))
    }

    fn write_header(&self, header: mp_block::BlockHeaderWithSignatures) -> Result<()> {
        tracing::debug!("Writing header {}", header.header.block_number);
        let block_n = header.header.block_number;
        self.inner
            .blocks_store_block_header(header)
            .with_context(|| format!("Storing block_header for block_n={block_n}"))
    }

    fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()> {
        tracing::debug!("Writing transactions {block_n}");
        // Save l1 core contract nonce to tx mapping.
        self.inner
            .messages_to_l2_write_trasactions(
                txs.iter().filter_map(|v| v.transaction.as_l1_handler().zip(v.receipt.as_l1_handler())),
            )
            .with_context(|| format!("Updating L1 state when storing transactions for block_n={block_n}"))?;

        self.inner
            .blocks_store_transactions(block_n, txs)
            .with_context(|| format!("Storing transactions for block_n={block_n}"))
    }

    fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        tracing::debug!("Writing state diff {block_n}");

        // Update compiled_class_hash_v2 for SNIP-34 migrated classes
        if !value.migrated_compiled_classes.is_empty() {
            let migrations: Vec<(Felt, Felt)> =
                value.migrated_compiled_classes.iter().map(|m| (m.class_hash, m.compiled_class_hash)).collect();
            tracing::debug!("Updating {} class v2 hashes (SNIP-34 migrations) for block {}", migrations.len(), block_n);
            self.inner.update_class_v2_hashes(migrations).context("Updating class v2 hashes")?;
        }

        self.inner
            .blocks_store_state_diff(block_n, value)
            .with_context(|| format!("Storing state diff for block_n={block_n}"))?;
        self.inner
            .state_apply_state_diff(block_n, value)
            .with_context(|| format!("Applying state from state diff for block_n={block_n}"))
    }

    fn write_bouncer_weights(&self, block_n: u64, value: &BouncerWeights) -> Result<()> {
        tracing::debug!("Writing bouncer weights for block_n={block_n}");
        self.inner
            .blocks_store_bouncer_weights(block_n, value)
            .with_context(|| format!("Storing bouncer weights for block_n={block_n}"))
    }

    fn write_events(&self, block_n: u64, events: &[mp_receipt::EventWithTransactionHash]) -> Result<()> {
        tracing::debug!("Writing events {block_n}");
        self.inner
            .blocks_store_events_to_receipts(block_n, events)
            .with_context(|| format!("Storing events to receipts for block_n={block_n}"))?;
        self.inner
            .store_events_bloom(block_n, events)
            .with_context(|| format!("Storing events bloom filter for block_n={block_n}"))
    }

    fn write_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        tracing::debug!("Writing classes {block_n}");
        self.inner.store_classes(block_n, converted_classes)
    }

    fn update_class_v2_hashes(&self, migrations: Vec<(Felt, Felt)>) -> Result<()> {
        tracing::debug!("Updating {} class v2 hashes (SNIP-34 migrations)", migrations.len());
        self.inner.update_class_v2_hashes(migrations).context("Updating class v2 hashes")
    }

    fn replace_chain_tip(&self, chain_tip: &StorageChainTip) -> Result<()> {
        tracing::debug!("Replace chain tip {chain_tip:?}");
        self.inner.replace_chain_tip(chain_tip).context("Replacing chain tip in db")
    }

    fn append_preconfirmed_content(
        &self,
        block_n: u64,
        start_tx_index: u64,
        txs: &[PreconfirmedExecutedTransaction],
    ) -> Result<()> {
        tracing::debug!(
            "Append preconfirmed content block_n={block_n}, start_tx_index={start_tx_index}, new_txs={}",
            txs.len()
        );
        self.inner
            .append_preconfirmed_content(block_n, start_tx_index, txs)
            .context("Appending to preconfirmed content to db")
    }

    fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        tracing::debug!(
            "Write l1 handler tx hash by nonce core_contract_nonce={core_contract_nonce}, txn_hash={txn_hash:#x}"
        );
        self.inner.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash).with_context(|| {
            format!("Writing l1 handler txn hash by nonce nonce={core_contract_nonce} txn_hash={txn_hash:#x}")
        })
    }
    fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        tracing::debug!("Write pending message to l2 nonce={}", msg.tx.nonce);
        let nonce = msg.tx.nonce;
        self.inner
            .write_pending_message_to_l2(msg)
            .with_context(|| format!("Writing pending message to l2 nonce={nonce}"))
    }
    fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        tracing::debug!("Remove pending message to l2 nonce={core_contract_nonce}");
        self.inner
            .remove_pending_message_to_l2(core_contract_nonce)
            .with_context(|| format!("Removing pending message to l2 nonce={core_contract_nonce}"))
    }
    fn write_l1_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<()> {
        tracing::debug!(
            "Write l1 txn hash by nonce core_contract_nonce={core_contract_nonce} l1_tx_hash_bytes={:?}",
            l1_tx_hash.0
        );
        self.inner.write_l1_txn_hash_by_nonce(core_contract_nonce, l1_tx_hash).with_context(|| {
            format!(
                "Writing l1 txn hash by nonce core_contract_nonce={core_contract_nonce} l1_tx_hash_bytes={:?}",
                l1_tx_hash.0
            )
        })
    }
    fn insert_message_to_l2_seen_marker(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<bool> {
        tracing::debug!(
            "Insert l1->l2 message seen marker l1_tx_hash_bytes={:?} nonce={core_contract_nonce}",
            l1_tx_hash.0
        );
        self.inner.insert_message_to_l2_seen_marker(l1_tx_hash, core_contract_nonce).with_context(|| {
            format!(
                "Inserting l1->l2 message seen marker l1_tx_hash_bytes={:?} nonce={core_contract_nonce}",
                l1_tx_hash.0
            )
        })
    }
    fn write_message_to_l2_consumed_txn_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
        l2_tx_hash: &Felt,
    ) -> Result<()> {
        tracing::debug!(
            "Write consumed l1->l2 message l1_tx_hash_bytes={:?} nonce={core_contract_nonce} l2_tx_hash={l2_tx_hash:#x}",
            l1_tx_hash.0
        );
        self.inner.write_message_to_l2_consumed_txn_hash(l1_tx_hash, core_contract_nonce, l2_tx_hash).with_context(
            || {
                format!(
                    "Writing consumed l1->l2 message l1_tx_hash_bytes={:?} nonce={core_contract_nonce} l2_tx_hash={l2_tx_hash:#x}",
                    l1_tx_hash.0
                )
            },
        )
    }

    fn write_chain_info(&self, info: &StoredChainInfo) -> Result<()> {
        tracing::debug!("Write chain info");
        self.inner.write_chain_info(info)
    }
    fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        tracing::debug!("Write devnet keys");
        self.inner.write_devnet_predeployed_keys(devnet_keys).context("Writing devnet predeployed keys to db")
    }
    fn write_l1_messaging_sync_tip(&self, block_n: Option<u64>) -> Result<()> {
        tracing::debug!("Write l1 messaging tip block_n={block_n:?}");
        self.inner.write_l1_messaging_sync_tip(block_n).context("Writing l1 messaging sync tip")
    }
    fn write_confirmed_on_l1_tip(&self, block_n: Option<u64>) -> Result<()> {
        tracing::debug!("Write confirmed on l1 tip block_n={block_n:?}");
        self.inner.write_confirmed_on_l1_tip(block_n).context("Writing confirmed on l1 tip")
    }
    fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()> {
        tracing::debug!("Write latest applied trie update block_n={block_n:?}");
        self.inner.write_latest_applied_trie_update(block_n).context("Writing latest applied trie update block_n")
    }
    fn write_runtime_exec_config(&self, config: &mp_chain_config::RuntimeExecutionConfig) -> Result<()> {
        tracing::debug!("Writing runtime execution config");
        self.inner.write_runtime_exec_config(config).context("Writing runtime execution config")
    }
    fn write_snap_sync_latest_block(&self, block_n: &Option<u64>) -> Result<()> {
        tracing::debug!("Write snap sync latest block block_n={block_n:?}");
        self.inner.write_snap_sync_latest_block(block_n).context("Writing snap sync latest block")
    }

    fn remove_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        tracing::debug!("Remove mempool transactions");
        self.inner.remove_mempool_transactions(tx_hashes).context("Removing mempool transactions from db")
    }
    fn write_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()> {
        let tx_hash = tx.hash;
        tracing::debug!("Writing mempool transaction from db for tx_hash={tx_hash:#x}");
        self.inner
            .write_mempool_transaction(tx)
            .with_context(|| format!("Writing mempool transaction from db for tx_hash={tx_hash:#x}"))
    }
    fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<external_outbox::ExternalOutboxId> {
        let tx_hash = tx.hash;
        tracing::debug!("Writing external outbox transaction for tx_hash={tx_hash:#x}");
        self.inner
            .write_external_outbox(tx)
            .with_context(|| format!("Writing external outbox transaction for tx_hash={tx_hash:#x}"))
    }
    fn delete_external_outbox(&self, id: external_outbox::ExternalOutboxId) -> Result<()> {
        tracing::debug!("Removing external outbox transaction arrived_at_ms={} uuid={:x?}", id.arrived_at_ms, id.uuid);
        self.inner
            .delete_external_outbox(id)
            .with_context(|| format!("Deleting external outbox transaction arrived_at_ms={}", id.arrived_at_ms))
    }

    fn get_state_root_hash(&self) -> Result<Felt> {
        get_state_root(self)
    }

    fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
    ) -> Result<(Felt, MerklizationTimings)> {
        tracing::debug!("Applying state diff to global trie start_block_n={start_block_n}");
        apply_to_global_trie(self, start_block_n, state_diffs).context("Applying state diff to global trie")
    }

    fn flush(&self) -> Result<()> {
        tracing::debug!("Flushing");
        self.inner.flush().context("Flushing RocksDB database")?;
        self.backup.backup_if_enabled(&self.inner).context("Backing up RocksDB database")
    }

    fn on_new_confirmed_head(&self, block_n: u64) -> Result<()> {
        tracing::debug!("on_new_confirmed_head block_n={block_n}");
        self.snapshots.set_new_head(block_n);
        self.metrics.update(self);
        Ok(())
    }

    fn remove_all_blocks_starting_from(&self, starting_from_block_n: u64) -> Result<()> {
        tracing::debug!("remove_all_blocks_starting_from starting_from_block_n={starting_from_block_n}");
        self.inner
            .remove_all_blocks_starting_from(starting_from_block_n)
            .with_context(|| format!("Removing all blocks in range [{starting_from_block_n}..] from database"))
    }

    /// Reverts the blockchain state during a chain reorganization.
    ///
    /// The trie reorg always starts from the nearest persisted checkpoint floor (`checkpoint <= requested`).
    /// If the requested block is above that floor, state diffs are replayed from `floor + 1..=requested`
    /// to reconstruct and commit the exact requested trie state before final DB pruning.
    ///
    /// # Arguments
    ///
    /// * `new_tip_block_hash` - Requested block hash to revert towards. This must be an existing
    ///   ancestor of the current chain tip.
    ///
    /// # Returns
    ///
    /// Returns `Ok((block_number, block_hash))` where:
    /// * `block_number` - The block number of the new chain tip
    /// * `block_hash` - The block hash of the exact requested new chain tip
    ///
    /// # Implementation Details
    ///
    /// The revert process performs the following steps in order:
    ///
    /// 1. **Validation**: Finds and validates the requested target block exists
    /// 2. **Floor Resolution**: Resolves nearest persisted checkpoint at-or-before requested block
    /// 3. **Trie Floor Revert**: Reverts tries from current tip to floor checkpoint and commits floor
    /// 4. **Exact Target Replay**: If requested > floor, replays state diffs and validates requested state root
    /// 5. **Block Database Revert**: Removes blocks in `(requested..=current_tip]` and collects state diffs
    /// 6. **Contract & Class Revert**: Uses collected state diffs to revert contract and class databases
    /// 7. **Metadata Reorg**: Prunes staged/checkpoint metadata to requested block and sets checkpoint marker
    /// 8. **Chain Tip Update**: Updates chain tip to requested block
    /// 9. **Snapshot Update**: Updates head snapshot to requested block
    /// 10. **Applied Update Reset**: Resets latest_applied_trie_update marker
    /// 11. **Database Flush**: Ensures all changes are persisted to disk
    ///
    /// # Notes
    ///
    /// * After calling this function, the caller MUST refresh the backend's chain_tip cache
    ///   by reading from the database, as this function only updates the database state.
    /// * This is a destructive operation - all blocks after the requested block are permanently removed.
    /// * The function is atomic - if any step fails, the database may be in an inconsistent state.
    fn revert_to(&self, new_tip_block_hash: &Felt) -> Result<(u64, Felt)> {
        tracing::info!("Reverting blockchain to block_hash={new_tip_block_hash:#x}");

        let requested_block_n = self
            .inner
            .find_block_hash(new_tip_block_hash)
            .context("Finding target block for reorg")?
            .ok_or_else(|| anyhow::anyhow!("Target block hash {new_tip_block_hash:#x} not found"))?;

        let requested_block_info =
            self.inner
                .get_block_info(requested_block_n)
                .context("Getting target block info")?
                .ok_or_else(|| anyhow::anyhow!("Target block info not found for block_n={requested_block_n}"))?;

        let current_tip = match self.inner.get_chain_tip()? {
            StorageChainTip::Empty => anyhow::bail!("Cannot revert when chain is empty"),
            StorageChainTip::Confirmed(block_n) => block_n,
            StorageChainTip::Preconfirmed { header, .. } => {
                header.block_number.checked_sub(1).ok_or_else(|| anyhow::anyhow!("Preconfirmed block is at genesis"))?
            }
        };

        let current_tip_info = self
            .inner
            .get_block_info(current_tip)
            .context("Getting current tip block info")?
            .ok_or_else(|| anyhow::anyhow!("Current tip block info not found"))?;

        if requested_block_n > current_tip {
            anyhow::bail!("Cannot revert to block_n={requested_block_n} which is > current tip={current_tip}");
        }

        if requested_block_n == current_tip {
            tracing::info!("🔄 REORG: Requested block_n={requested_block_n} is already current tip, no revert needed");
            return Ok((requested_block_n, requested_block_info.block_hash));
        }

        let floor_block_n = self
            .resolve_revert_target_block_n(requested_block_n)
            .context("Resolving nearest persisted trie commit-id for reorg target")?;
        let floor_block_info = if floor_block_n == requested_block_n {
            requested_block_info.clone()
        } else {
            self.inner
                .get_block_info(floor_block_n)
                .context("Getting floored target block info")?
                .ok_or_else(|| anyhow::anyhow!("Floored target block info not found for block_n={floor_block_n}"))?
        };

        if floor_block_n != requested_block_n {
            tracing::warn!(
                requested_block_n,
                floored_target_block_n = floor_block_n,
                "🔄 REORG: requested target has no persisted trie commit-id marker; replay from floor checkpoint required"
            );
        }

        tracing::info!(
            "🔄 REORG: Starting blockchain reorganization from block_n={current_tip} to requested block_n={requested_block_n} via floor block_n={floor_block_n}",
        );
        tracing::info!(
            "🔄 REORG: Floor block hash={:#x}, current tip hash={:#x}, requested hash={:#x}",
            floor_block_info.block_hash,
            current_tip_info.block_hash,
            new_tip_block_hash,
        );

        let floor_id = BasicId::new(floor_block_n);
        let current_id = BasicId::new(current_tip);

        tracing::info!("🌳 REORG: Reverting bonsai tries from current={} to floor={}", current_tip, floor_block_n);

        tracing::debug!("🌳 REORG: Reverting contract trie...");
        self.contract_trie()
            .revert_to(floor_id, current_id)
            .map_err(|e| anyhow::anyhow!("Failed to revert contract trie: {e:?}"))?;
        tracing::info!("✅ REORG: Contract trie reverted successfully");

        tracing::debug!("🌳 REORG: Reverting contract storage trie...");
        self.contract_storage_trie()
            .revert_to(floor_id, current_id)
            .map_err(|e| anyhow::anyhow!("Failed to revert contract storage trie: {e:?}"))?;
        tracing::info!("✅ REORG: Contract storage trie reverted successfully");

        tracing::debug!("🌳 REORG: Reverting class trie...");
        self.class_trie()
            .revert_to(floor_id, current_id)
            .map_err(|e| anyhow::anyhow!("Failed to revert class trie: {e:?}"))?;
        tracing::info!("✅ REORG: Class trie reverted successfully");

        tracing::info!("💾 REORG: Committing tries at floor block_n={}...", floor_block_n);
        self.contract_trie()
            .commit(floor_id)
            .map_err(|e| anyhow::anyhow!("Failed to commit contract trie after revert: {e:?}"))?;
        self.contract_storage_trie()
            .commit(floor_id)
            .map_err(|e| anyhow::anyhow!("Failed to commit contract storage trie after revert: {e:?}"))?;
        self.class_trie()
            .commit(floor_id)
            .map_err(|e| anyhow::anyhow!("Failed to commit class trie after revert: {e:?}"))?;
        tracing::info!("✅ REORG: Floor tries committed successfully");

        let (target_block_n, target_block_info) = if requested_block_n == floor_block_n {
            (requested_block_n, floor_block_info)
        } else {
            self.replay_tries_from_floor_to_target(
                floor_block_n,
                requested_block_n,
                requested_block_info.header.global_state_root,
            )
            .with_context(|| {
                format!("Replaying tries from floor block_n={floor_block_n} to requested block_n={requested_block_n}")
            })?;
            tracing::info!("✅ REORG: Exact trie target reconstructed at block_n={requested_block_n}");
            (requested_block_n, requested_block_info)
        };

        // Revert database state using the three revert functions
        // First, revert blocks and collect state diffs
        tracing::info!("📦 REORG: Starting block database revert...");
        let state_diffs =
            self.inner.block_db_revert(target_block_n, current_tip).context("Reverting blocks database")?;
        tracing::info!("✅ REORG: Block database reverted, collected {} state diffs", state_diffs.len());

        // Then use those state diffs to revert contract and class state
        tracing::info!("📝 REORG: Starting contract database revert...");
        self.inner.contract_db_revert(&state_diffs).context("Reverting contract database")?;
        tracing::info!("✅ REORG: Contract database reverted successfully");

        tracing::info!("🎓 REORG: Starting class database revert...");
        self.inner.class_db_revert(&state_diffs).context("Reverting class database")?;
        tracing::info!("✅ REORG: Class database reverted successfully");

        tracing::info!("🧾 REORG: Pruning parallel merkle metadata to block_n={}", target_block_n);
        self.inner
            .parallel_merkle_reorg_metadata_to(target_block_n)
            .context("Pruning parallel merkle metadata after reorg")?;
        if !self.inner.has_parallel_merkle_checkpoint(target_block_n)? {
            tracing::info!("🧾 REORG: Writing checkpoint marker for exact target block_n={target_block_n}");
            self.inner
                .write_parallel_merkle_checkpoint(target_block_n)
                .with_context(|| format!("Writing checkpoint marker for target block_n={target_block_n}"))?;
        }
        tracing::info!("✅ REORG: Parallel merkle metadata pruned successfully");

        tracing::info!("🔗 REORG: Updating chain tip to block_n={}", target_block_n);
        let new_tip = StorageChainTip::Confirmed(target_block_n);
        self.replace_chain_tip(&new_tip).context("Updating chain tip after reorg")?;
        tracing::info!("✅ REORG: Chain tip updated successfully");

        tracing::info!("📸 REORG: Updating snapshots to new head block_n={}", target_block_n);
        self.snapshots.set_new_head(target_block_n);
        tracing::info!("✅ REORG: Snapshots updated successfully");

        tracing::info!("🔄 REORG: Resetting latest_applied_trie_update to block_n={}", target_block_n);
        self.write_latest_applied_trie_update(&Some(target_block_n))
            .context("Resetting latest_applied_trie_update after reorg")?;
        tracing::info!("✅ REORG: latest_applied_trie_update reset successfully");

        tracing::info!("💾 REORG: Flushing database to persist changes...");
        self.flush().context("Flushing database after reorg")?;
        tracing::info!("✅ REORG: Database flushed successfully");

        tracing::info!(
            "🎉 REORG: Blockchain reorganization completed successfully! Reverted to block_n={target_block_n} block_hash={:#x}",
            target_block_info.block_hash
        );

        Ok((target_block_n, target_block_info.block_hash))
    }
}
