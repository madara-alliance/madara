//! We use [`rocksdb`] as the key-value store backend for the Madara node. Rocksdb Is highly
//! flexible, and you can find out more about the specific configuration we are using in
//! [`rocksdb_global_options`]. Rocksdb splits storage into columns, each having their own key-value
//! mappings for the data they store. We define this in the [`Column`] enum. Pay special attention
//! to the string mappings in [`Column::rocksdb_name`]: this is what is actually used under the hood
//! by Rocksdb and what you will see each column actually referred to as.
//!
//! # Storage API
//!
//! Storing new blocks is the responsibility of the consumers of this crate. In the madara node
//! architecture, this means: the sync service `mc-sync` (when we are syncing new blocks), or the
//! block production `mc-block-production` task when we are producing new blocks. For the sake of
//! documentation, we will call this service the _"block importer"_.
//!
//! We divide the backend storage API into two components: a high-level storage API, useful for full
//! block storage without shooting yourself in the foot, and a low-level API which allows for
//! granular and targeted updates to the database but requires you to pay special attention to what
//! you are doing.
//!
//! Note that the validity of the block being stored is not checked by neither of those APIs. It is
//! the responsibility of the block importer to check that blocks are valid before storing them in
//! db.
//!
//! ## High-level API
//!
//! The high-level API is quite simple: just call [`add_full_block_with_classes`] and it will handle
//! everything required to save blocks properly to the db, including any side effects to storage
//! such as incrementing the latest block number.
//!
//! ## Low-level API
//!
//! For the low-level API, there are a few responsibilities to follow. The database can store
//! partial blocks: blocks are divided into _headers_, _transactions & receipts_, _classes_,
//! _state diffs_ and _events_. These can be stored individually, so that for example if the node
//! can store a block's header faster than its other components, it can move on to the next block
//! and start storing _its_ header. Partial block storage allows the node to make progress in block
//! sync while minimizing the churn induced by certain heavy operations such as state root
//! computation.
//!
//! To store individual block components, refer to:
//!
//! - headers: [`store_block_header`]
//! - transactions & receipts: [`store_transactions`]
//! - classes: [`store_classes`]
//! - state diffs: [`store_state_diff`]
//! - events: [`store_events`]
//!
//! You will also need to call [`apply_to_global_trie`] once a block has been fully imported to
//! compute its state root.
//!
//! ### Parallelism
//!
//! Each of the low-level API functions can be called in parallel, however, [`apply_to_global_trie`]
//! needs to be called _sequentially_. This is because we cannot update the global trie across
//! multiple blocks at once. However, parallelism is still used inside of that function -
//! parallelism within a single block.
//!
//! ### Head Status
//!
//! Because each block component can be written to at different speeds, we need to keep track of the
//! advancement of each component stored this way. For example, we might have stored block headers
//! until block 6 but only have all block transactions and receipts until block 3.
//!
//! To address this issue, each block component has a [`BlockNStatus`] associated to it inside of
//! [`head_status`], which the block importer service can use however it wants. This includes block
//! numbers for [`headers`], [`state diffs`], [`classes`], [`transactions`], [`events`], and the
//! [`global trie`]. Unless you use the high-level API, _you will have to set these manually_ using
//! [`BlockNStatus::set_current`]!
//!
//! ### Sealing blocks
//!
//! [`head_status`] also contains an extra field, [`full block`], which acts differently from the
//! rest in that it is set by the backend crate. _You should not set this yourself!_
//!
//! The block importer service needs to call [`on_full_block_imported`] to mark a block as fully
//! imported. This function will increment [`full block`], marking a new block as available for
//! query in the database (sealed). It will also do some extra cleanup, such as recording metrics,
//! flushing the database if needed, as well as creating db backups if the backup flag has been set
//! when launching the node.
//!
//! ## Querying the db
//!
//! Any external crate reading the database should use [`DbBlockId`] when querying blocks from the
//! database. This ensures that any partial block data beyond the current [`full block`] will not be
//! visible to, eg. the rpc service.
//!
//! The block importer service can still bypass this restriction by using [`RawDbBlockId`] instead;
//! allowing it to see the partial data it has saved beyond the latest block marked as full. As a
//! general rule, you should avoid using this unless you really need to and you are sure of what you
//! are doing!
//!
//! [rocksdb_global_options]: rocksdb_options::rocksdb_global_options
//! [`add_full_block_with_classes`]: `MadaraBackend::add_full_block_with_classes`
//! [`store_block_header`]: MadaraBackend::store_block_header
//! [`store_transactions`]: MadaraBackend::store_transactions
//! [`store_classes`]: MadaraBackend::store_classes
//! [`store_state_diff`]: MadaraBackend::store_state_diff
//! [`store_events`]: MadaraBackend::store_events
//! [`apply_to_global_trie`]: MadaraBackend::apply_to_global_trie
//! [`BlockNStatus`]: chain_head::BlockNStatus
//! [`BlockNStatus::set_current`]: chain_head::BlockNStatus::set_current
//! [`head_status`]: MadaraBackend::head_status
//! [`headers`]: ChainHead::headers
//! [`state diffs`]: ChainHead::state_diffs
//! [`classes`]: ChainHead::classes
//! [`transactions`]: ChainHead::transactions
//! [`events`]: ChainHead::events
//! [`global trie`]: ChainHead::global_trie
//! [`full block`]: ChainHead::full_block
//! [`on_full_block_imported`]: MadaraBackend::on_full_block_imported
//! [`DbBlockId`]: db_block_id::DbBlockId
//! [`RawDbBlockId`]: db_block_id::RawDbBlockId

use crate::gas::L1GasQuoteCell;
use crate::preconfirmed::PreconfirmedBlock;
use crate::preconfirmed::PreconfirmedExecutedTransaction;
use crate::rocksdb::RocksDBConfig;
use crate::rocksdb::RocksDBStorage;
use crate::storage::StorageHeadProjection;
use crate::storage::StoredChainInfo;
use crate::sync_status::SyncStatusCell;
use chain_head::ChainHeadState;
use mc_class_exec::config::NativeConfig;
use mp_block::commitments::BlockCommitments;
use mp_block::commitments::CommitmentComputationContext;
use mp_block::header::CustomHeader;
use mp_block::BlockHeaderWithSignatures;
use mp_block::FullBlockWithoutCommitments;
use mp_block::TransactionWithReceipt;
use mp_chain_config::ChainConfig;
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_rpc::admin::{ReplayBlockBoundary, ReplayBlockBoundaryStatus};
use mp_state_update::StateDiff;
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::L1HandlerTransactionWithFee;
use prelude::*;
use starknet_api::core::ContractAddress;
use starknet_types_core::felt::Felt;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
pub mod metrics;
use metrics::metrics;
pub mod chain_head;
pub mod close_pipeline_contract;
pub mod migration;
mod prelude;
pub mod storage;

pub mod gas;
pub mod preconfirmed;
pub mod rocksdb;
pub mod subscription;
pub mod sync_status;
#[cfg(any(test, feature = "testing"))]
pub mod test_utils;
pub mod tests;
pub mod view;

use blockifier::bouncer::BouncerWeights;
pub use rocksdb::external_outbox::{ExternalOutboxEntry, ExternalOutboxId};
pub use rocksdb::global_trie::MerklizationTimings;
pub use storage::{
    DevnetPredeployedContractAccount, DevnetPredeployedKeys, EventFilter, MadaraStorage, MadaraStorageRead,
    MadaraStorageWrite, StorageTxIndex,
};
pub use view::{MadaraBlockView, MadaraConfirmedBlockView, MadaraPreconfirmedBlockView, MadaraStateView};

/// Timing information collected during the close_block DB operations.
/// All durations are captured for structured logging.
#[derive(Debug, Clone, Default)]
pub struct CloseBlockTimings {
    /// Time to fetch full block with classes
    pub get_full_block_with_classes: Duration,
    /// Time to compute block commitments
    pub block_commitments_compute: Duration,
    /// Total time for global trie merklization (apply_to_global_trie)
    pub merklization: Duration,
    /// Time to compute contract trie root (parallel with class trie)
    pub contract_trie_root: Duration,
    /// Time to compute class trie root (parallel with contract trie)
    pub class_trie_root: Duration,
    /// Time to commit contract storage trie
    pub contract_storage_trie_commit: Duration,
    /// Time to commit contract trie
    pub contract_trie_commit: Duration,
    /// Time to commit class trie
    pub class_trie_commit: Duration,
    /// Time to compute block hash
    pub block_hash_compute: Duration,
    /// Time to write block parts to database
    pub db_write_block_parts: Duration,
}

#[derive(Debug, Clone)]
struct ReplayBoundaryRuntime {
    boundary: ReplayBlockBoundary,
    dispatched_tx_count: u64,
    executed_tx_count: u64,
    last_executed_tx_hash: Option<Felt>,
    reached_last_tx_hash: bool,
    mismatch: Option<String>,
    closed: bool,
}

impl ReplayBoundaryRuntime {
    fn from_boundary(boundary: ReplayBlockBoundary, seed_executed: u64, seed_last_hash: Option<Felt>) -> Self {
        let reached_last_tx_hash = seed_last_hash.map(|hash| hash == boundary.last_tx_hash).unwrap_or(false);
        let mut this = Self {
            boundary,
            dispatched_tx_count: seed_executed,
            executed_tx_count: seed_executed,
            last_executed_tx_hash: seed_last_hash,
            reached_last_tx_hash,
            mismatch: None,
            closed: false,
        };
        this.refresh_consistency_flags();
        this
    }

    fn boundary_met(&self) -> bool {
        self.executed_tx_count == self.boundary.expected_tx_count
            && self.reached_last_tx_hash
            && self.mismatch.is_none()
    }

    fn into_status(&self) -> ReplayBlockBoundaryStatus {
        ReplayBlockBoundaryStatus {
            block_n: self.boundary.block_n,
            expected_tx_count: self.boundary.expected_tx_count,
            dispatched_tx_count: self.dispatched_tx_count,
            executed_tx_count: self.executed_tx_count,
            last_executed_tx_hash: self.last_executed_tx_hash,
            reached_last_tx_hash: self.reached_last_tx_hash,
            boundary_met: self.boundary_met(),
            closed: self.closed,
            mismatch: self.mismatch.clone(),
        }
    }

    fn refresh_consistency_flags(&mut self) {
        if self.executed_tx_count > self.boundary.expected_tx_count {
            self.set_mismatch_if_empty(format!(
                "executed_tx_count={} exceeded expected_tx_count={}",
                self.executed_tx_count, self.boundary.expected_tx_count
            ));
        }

        if self.reached_last_tx_hash && self.executed_tx_count != self.boundary.expected_tx_count {
            self.set_mismatch_if_empty(format!(
                "last_tx_hash reached at tx_index={} but expected_tx_count={}",
                self.executed_tx_count, self.boundary.expected_tx_count
            ));
        }

        if self.executed_tx_count == self.boundary.expected_tx_count
            && self.last_executed_tx_hash != Some(self.boundary.last_tx_hash)
        {
            self.set_mismatch_if_empty(format!(
                "executed_tx_count reached expected count but last_executed_tx_hash={} does not match expected_last_tx_hash={}",
                self.last_executed_tx_hash
                    .map(|hash| format!("{hash:#x}"))
                    .unwrap_or_else(|| "<none>".to_string()),
                format!("{:#x}", self.boundary.last_tx_hash)
            ));
        }
    }

    fn set_mismatch_if_empty(&mut self, message: String) {
        if self.mismatch.is_none() {
            self.mismatch = Some(message);
        }
    }
}

fn storage_tip_from_confirmed_or_empty(confirmed_tip: Option<u64>) -> StorageHeadProjection {
    match confirmed_tip {
        Some(block_n) => StorageHeadProjection::Confirmed(block_n),
        None => StorageHeadProjection::Empty,
    }
}

fn storage_tip_from_preconfirmed_block(block: &PreconfirmedBlock) -> StorageHeadProjection {
    StorageHeadProjection::Preconfirmed {
        header: block.header.clone(),
        content: block.content.borrow().executed_transactions().cloned().collect(),
    }
}

fn storage_tip_from_head_projection(
    chain_head_state: ChainHeadState,
    preconfirmed: Option<Arc<PreconfirmedBlock>>,
) -> StorageHeadProjection {
    if let Some(preconfirmed_tip) = chain_head_state.external_preconfirmed_tip {
        if let Some(block) = preconfirmed.filter(|b| b.header.block_number == preconfirmed_tip) {
            return storage_tip_from_preconfirmed_block(&block);
        }
    }

    storage_tip_from_confirmed_or_empty(chain_head_state.confirmed_tip)
}

fn runtime_preconfirmed_block_n(preconfirmed: Option<&Arc<PreconfirmedBlock>>) -> Option<u64> {
    preconfirmed.map(|block| block.header.block_number)
}

fn classify_chain_head_transition(previous: ChainHeadState, next: ChainHeadState) -> &'static str {
    if previous == next {
        return "unchanged";
    }

    if next.confirmed_tip != previous.confirmed_tip {
        if next.external_preconfirmed_tip.is_some() {
            return "confirmed_advanced_with_external_preconfirmed";
        }
        return "confirmed_advanced_without_preconfirmed";
    }

    if next.external_preconfirmed_tip != previous.external_preconfirmed_tip {
        return "external_preconfirmed_updated";
    }

    if next.internal_preconfirmed_tip != previous.internal_preconfirmed_tip {
        return "internal_preconfirmed_updated";
    }

    "head_projection_updated"
}

/// Madara client database backend singleton.
#[derive(Debug)]
pub struct MadaraBackend<DB = RocksDBStorage> {
    // TODO: remove this pub. this is temporary until get_storage_proof is properly abstracted.
    pub db: DB,
    chain_config: Arc<ChainConfig>,
    // db_metrics: DbMetrics,
    watch_gas_quote: L1GasQuoteCell,
    config: MadaraBackendConfig,
    sync_status: SyncStatusCell,
    starting_block: Option<u64>,

    pub chain_head_state: tokio::sync::watch::Sender<ChainHeadState>,
    pub preconfirmed_block_runtime: tokio::sync::watch::Sender<Option<Arc<PreconfirmedBlock>>>,

    /// Current finalized block_n on L1.
    latest_l1_confirmed: tokio::sync::watch::Sender<Option<u64>>,

    /// Cairo Native execution configuration.
    ///
    /// This config is passed through to BlockifierStateAdapter for execution.
    /// The `enable_native_execution` flag in the config controls whether native execution is used.
    pub cairo_native_config: Arc<NativeConfig>,

    /// Keep the TempDir instance around so that the directory is not deleted until the MadaraBackend struct is dropped.
    #[cfg(any(test, feature = "testing"))]
    _temp_dir: Option<tempfile::TempDir>,

    /// Custom header used during block replay to ensure deterministic execution.
    ///
    /// When replaying a block, we must match the exact timestamp and gas configuration
    /// from the original block to reproduce the expected block hash. This field stores
    /// header overrides that are applied during transaction validation and execution,
    /// along with the expected block hash to validate against after block creation.
    /// # Important Notes
    /// - Custom header is different for each block and must be set per block
    /// - **Must verify** that the block number matches before use
    /// - **Must clear** after use to prevent reuse across different blocks
    /// - Access is thread-safe via Mutex to allow concurrent operations
    pub custom_header: Mutex<Option<CustomHeader>>,

    /// Replay boundary metadata and runtime progress.
    ///
    /// This is in-memory only and keyed by block number. It is used when replay mode is enabled
    /// by block production to prevent batch/executor from crossing source block boundaries.
    replay_boundaries: Mutex<BTreeMap<u64, ReplayBoundaryRuntime>>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecutionReadCacheConfig {
    /// Enable the execution read cache. Default: false.
    pub enabled: bool,
    /// Contracts to cache.
    ///
    /// - `None`: cache all contracts.
    /// - `Some(vec)`: cache only those contracts (allowlist mode). `Some([])` is valid and means
    ///   "cache none".
    pub contracts: Option<Vec<ContractAddress>>,
    /// Maximum cache size in bytes.
    pub max_memory_bytes: usize,
}

#[derive(Debug, Default)]
pub struct MadaraBackendConfig {
    pub flush_every_n_blocks: Option<u64>,
    /// When false, the preconfirmed block is never saved to database.
    pub save_preconfirmed: bool,
    pub unsafe_starting_block: Option<u64>,
    /// Skip creating backup before migration.
    /// WARNING: Without backup, there's no recovery if migration fails.
    /// Only use if you have external snapshots/backups.
    pub skip_migration_backup: bool,
    /// Execution-time read cache for hot contract state.
    pub execution_read_cache: ExecutionReadCacheConfig,
}

impl<D: MadaraStorage> MadaraBackend<D> {
    fn new_and_init(
        db: D,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
        cairo_native_config: Arc<NativeConfig>,
    ) -> Result<Self> {
        let mut backend = Self {
            db,
            // db_metrics: DbMetrics::register().context("Registering db metrics")?,
            chain_config,
            starting_block: config.unsafe_starting_block,
            config,
            sync_status: SyncStatusCell::default(),
            watch_gas_quote: L1GasQuoteCell::default(),
            cairo_native_config,
            #[cfg(any(test, feature = "testing"))]
            _temp_dir: None,
            chain_head_state: tokio::sync::watch::Sender::new(Default::default()),
            preconfirmed_block_runtime: tokio::sync::watch::Sender::new(None),
            latest_l1_confirmed: tokio::sync::watch::Sender::new(Default::default()),
            custom_header: Mutex::new(None),
            replay_boundaries: Mutex::new(BTreeMap::new()),
        };
        backend.init().context("Initializing madara backend")?;
        Ok(backend)
    }

    fn init(&mut self) -> Result<()> {
        // Check chain configuration
        if let Some(res) = self.db.get_stored_chain_info()? {
            if res.chain_id != self.chain_config.chain_id {
                bail!(
                    "The database has been created on the network \"{}\" (chain id `{}`), \
                            but the node is configured for network \"{}\" (chain id `{}`).",
                    res.chain_name,
                    res.chain_id,
                    self.chain_config.chain_name,
                    self.chain_config.chain_id
                )
            }
        } else {
            self.db.write_chain_info(&StoredChainInfo {
                chain_id: self.chain_config.chain_id.clone(),
                chain_name: self.chain_config.chain_name.clone(),
            })?;
        }

        // Initialize canonical chain head state.
        let stored_tip = if let Some(starting_block) = self.starting_block {
            StorageHeadProjection::Confirmed(starting_block)
        } else {
            self.db.get_head_projection()?
        };
        let (chain_head_state, preconfirmed) = self.build_runtime_head_projection(stored_tip)?;
        self.starting_block = chain_head_state.confirmed_tip;
        // On startup, remove all blocks past the head projection, in case we have partial blocks in db.
        self.db.remove_all_blocks_starting_from(
            chain_head_state.confirmed_tip.map(|n| n + 1).unwrap_or(/* genesis */ 0),
        )?;
        self.publish_head_projection(chain_head_state, preconfirmed)?;

        // Init L1 head
        self.latest_l1_confirmed.send_replace(self.db.get_confirmed_on_l1_tip()?);

        Ok(())
    }

    /// Get a write handle for the backend. This is the function you need to call to save new blocks, modify the preconfirmed block,
    /// and do any other such thing. The canonical chain head projection can only be modified through this.
    ///
    /// As a caller, you are responsible for ensuring the backend is not being concurrently
    /// modified in an unexpected way. In practice, this means:
    /// - You are allowed to use the `write_*` low-level functions to write block parts concurrently.
    /// - You are not allowed to use the other functions to advance the head projection
    ///
    /// Failure to do so could result in errors and/or invalid state, which includes invalid state being saved to the database.
    /// The functions are still safe to use, since it's a logic error and not a memory safety issue.
    ///
    /// In addition, all of the associated functions need to be called in a rayon thread pool context. **Do not call
    /// them from the tokio pool!**
    // TODO: ensure exclusive access? all of these requirements could be checked relatively cheaply. There are also
    // ways to make the aforementioned logic errors unrepreasentable by designing the API a little better.
    pub fn write_access(self: &Arc<Self>) -> MadaraBackendWriter<D> {
        MadaraBackendWriter { inner: self.clone() }
    }

    /// Set the current latest block confirmed on L1. This will also wake watchers to L1 head changes.
    ///
    /// Warning: It is invalid to set this new `latest_l1_confirmed` to a lower value than the current one, or
    /// to a higher value than the current block on l2.
    // FIXME: In these cases, the update should not succeed and an error should be returned.
    pub fn set_latest_l1_confirmed(&self, latest_l1_confirmed: Option<u64>) -> Result<()> {
        self.db.write_confirmed_on_l1_tip(latest_l1_confirmed)?;
        self.latest_l1_confirmed.send_replace(latest_l1_confirmed);
        Ok(())
    }

    pub fn get_custom_header(&self) -> Option<CustomHeader> {
        self.get_custom_header_with_clear(false)
    }

    pub fn get_custom_header_with_clear(&self, clear: bool) -> Option<CustomHeader> {
        let mut guard = self.custom_header.lock().expect("Poisoned lock");
        let result = guard.clone();

        if clear {
            *guard = None;
        }

        result
    }

    pub fn set_custom_header(&self, custom_header: CustomHeader) {
        let mut guard = self.custom_header.lock().expect("Poisoned lock");
        *guard = Some(custom_header);
    }

    fn replay_boundary_seed_from_preconfirmed(&self, block_n: u64) -> (u64, Option<Felt>) {
        if let Some(runtime_block) = self
            .preconfirmed_block_runtime
            .borrow()
            .as_ref()
            .filter(|block| block.header.block_number == block_n)
            .cloned()
        {
            let guard = runtime_block.content.borrow();
            let executed_tx_count = guard.n_executed() as u64;
            let last_executed_tx_hash =
                guard.executed_transactions().last().map(|tx| *tx.transaction.receipt.transaction_hash());
            return (executed_tx_count, last_executed_tx_hash);
        }

        match self.db.get_preconfirmed_block_data(block_n) {
            Ok(Some((_header, content))) => {
                let executed_tx_count = content.len() as u64;
                let last_executed_tx_hash = content.last().map(|tx| *tx.transaction.receipt.transaction_hash());
                (executed_tx_count, last_executed_tx_hash)
            }
            Ok(None) => (0, None),
            Err(err) => {
                tracing::warn!(
                    "Failed to read preconfirmed data while seeding replay boundary for block #{block_n}: {err:#}"
                );
                (0, None)
            }
        }
    }

    pub fn set_replay_boundary(&self, boundary: ReplayBlockBoundary) -> ReplayBlockBoundaryStatus {
        let (seed_executed, seed_last_hash) = self.replay_boundary_seed_from_preconfirmed(boundary.block_n);
        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");

        if let Some(existing) = guard.get_mut(&boundary.block_n) {
            if existing.boundary == boundary {
                if seed_executed > existing.executed_tx_count {
                    existing.executed_tx_count = seed_executed;
                    existing.last_executed_tx_hash = seed_last_hash.or(existing.last_executed_tx_hash);
                }
                existing.dispatched_tx_count = existing.dispatched_tx_count.max(seed_executed);
                if existing.last_executed_tx_hash.is_none() {
                    existing.last_executed_tx_hash = seed_last_hash;
                }
                existing.reached_last_tx_hash =
                    existing.last_executed_tx_hash.map(|hash| hash == existing.boundary.last_tx_hash).unwrap_or(false);
                existing.closed = false;
                existing.refresh_consistency_flags();
                let status = existing.into_status();
                tracing::info!(
                    "replay_boundary_set block_number={} expected_tx_count={} seeded_executed={} boundary_met={} closed={} mismatch={:?}",
                    status.block_n,
                    status.expected_tx_count,
                    status.executed_tx_count,
                    status.boundary_met,
                    status.closed,
                    status.mismatch
                );
                return status;
            }

            tracing::warn!(
                "Replacing replay boundary for block #{} (old expected_tx_count={}, old_last_tx_hash={:#x}, new expected_tx_count={}, new_last_tx_hash={:#x})",
                boundary.block_n,
                existing.boundary.expected_tx_count,
                existing.boundary.last_tx_hash,
                boundary.expected_tx_count,
                boundary.last_tx_hash
            );
        }

        let runtime = ReplayBoundaryRuntime::from_boundary(boundary.clone(), seed_executed, seed_last_hash);
        let status = runtime.into_status();
        guard.insert(boundary.block_n, runtime);
        tracing::info!(
            "replay_boundary_set block_number={} expected_tx_count={} seeded_executed={} boundary_met={} closed={} mismatch={:?}",
            status.block_n,
            status.expected_tx_count,
            status.executed_tx_count,
            status.boundary_met,
            status.closed,
            status.mismatch
        );
        status
    }

    pub fn get_replay_boundary_status(&self, block_n: u64) -> Option<ReplayBlockBoundaryStatus> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(ReplayBoundaryRuntime::into_status)
    }

    pub fn replay_boundary_exists(&self, block_n: u64) -> bool {
        self.replay_boundaries.lock().expect("Poisoned lock").contains_key(&block_n)
    }

    pub fn replay_boundary_remaining_execution_capacity(&self, block_n: u64) -> Option<u64> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(|entry| {
            if entry.closed || entry.mismatch.is_some() {
                0
            } else {
                entry.boundary.expected_tx_count.saturating_sub(entry.executed_tx_count)
            }
        })
    }

    pub fn replay_boundary_remaining_dispatch_capacity(&self, block_n: u64) -> Option<u64> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(|entry| {
            if entry.closed || entry.mismatch.is_some() {
                0
            } else {
                entry.boundary.expected_tx_count.saturating_sub(entry.dispatched_tx_count)
            }
        })
    }

    pub fn replay_boundary_is_met(&self, block_n: u64) -> Option<bool> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(ReplayBoundaryRuntime::boundary_met)
    }

    pub fn replay_boundary_record_dispatched(
        &self,
        block_n: u64,
        dispatched_tx_count: u64,
    ) -> Option<ReplayBlockBoundaryStatus> {
        if dispatched_tx_count == 0 {
            return self.get_replay_boundary_status(block_n);
        }

        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");
        let entry = guard.get_mut(&block_n)?;
        if entry.closed {
            return Some(entry.into_status());
        }

        entry.dispatched_tx_count = entry.dispatched_tx_count.saturating_add(dispatched_tx_count);
        if entry.dispatched_tx_count > entry.boundary.expected_tx_count {
            entry.set_mismatch_if_empty(format!(
                "dispatched_tx_count={} exceeded expected_tx_count={}",
                entry.dispatched_tx_count, entry.boundary.expected_tx_count
            ));
        }
        entry.refresh_consistency_flags();
        Some(entry.into_status())
    }

    pub fn replay_boundary_record_executed_hashes(
        &self,
        block_n: u64,
        tx_hashes: &[Felt],
    ) -> Option<ReplayBlockBoundaryStatus> {
        if tx_hashes.is_empty() {
            return self.get_replay_boundary_status(block_n);
        }

        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");
        let entry = guard.get_mut(&block_n)?;
        for tx_hash in tx_hashes {
            entry.executed_tx_count = entry.executed_tx_count.saturating_add(1);
            if entry.dispatched_tx_count < entry.executed_tx_count {
                entry.dispatched_tx_count = entry.executed_tx_count;
            }
            entry.last_executed_tx_hash = Some(*tx_hash);
            if *tx_hash == entry.boundary.last_tx_hash {
                entry.reached_last_tx_hash = true;
            }
            entry.refresh_consistency_flags();
        }
        Some(entry.into_status())
    }

    pub fn replay_boundary_mark_closed(&self, block_n: u64) -> Option<ReplayBlockBoundaryStatus> {
        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");
        let entry = guard.get_mut(&block_n)?;
        entry.closed = true;
        Some(entry.into_status())
    }

    /// Flush all pending writes to disk. Critical for databases with WAL disabled.
    /// Must be called before shutdown to ensure data persistence.
    pub fn flush(&self) -> Result<()> {
        self.db.flush()
    }
}

impl MadaraBackend<RocksDBStorage> {
    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<Self> {
        Self::open_for_testing_with_config(chain_config, Default::default())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing_with_config(chain_config: Arc<ChainConfig>, config: MadaraBackendConfig) -> Arc<Self> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let db = RocksDBStorage::open(temp_dir.as_ref(), Default::default()).unwrap();
        // For tests, use default (disabled) Cairo Native config (no native execution)
        // Initialize compilation semaphore for tests (required even if native execution is disabled)
        let builder = mc_class_exec::config::NativeConfig::builder();
        let max_concurrent = builder.max_concurrent_compilations();
        mc_class_exec::init_compilation_semaphore(max_concurrent);
        let test_config = builder.build();
        let cairo_native_config = Arc::new(test_config);
        let mut backend = Self::new_and_init(db, chain_config, config, cairo_native_config).unwrap();
        backend._temp_dir = Some(temp_dir);
        Arc::new(backend)
    }

    /// Open the db.
    ///
    /// This function will:
    /// 1. Check the database version against the binary's expected version
    /// 2. Run any necessary migrations if the database is outdated
    /// 3. Create a fresh database if none exists
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The database version is newer than the binary (requires binary upgrade)
    /// - The database version is too old to migrate (requires resync)
    /// - A migration fails
    /// - The database cannot be opened
    pub fn open_rocksdb(
        base_path: &Path,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
        rocksdb_config: RocksDBConfig,
        cairo_native_config: Arc<NativeConfig>,
    ) -> Result<Arc<Self>> {
        use crate::migration::{MigrationRunner, MigrationStatus};

        /// Database version from build-time, injected by build.rs
        const REQUIRED_DB_VERSION_STR: &str = env!("DB_VERSION");
        /// Minimum database version that can be migrated from.
        const BASE_DB_VERSION_STR: &str = env!("DB_BASE_VERSION");

        let required_version: u32 =
            REQUIRED_DB_VERSION_STR.parse().expect("DB_VERSION must be a valid u32 (checked at build time)");
        let base_version: u32 =
            BASE_DB_VERSION_STR.parse().expect("DB_BASE_VERSION must be a valid u32 (checked at build time)");

        // Create base directory if it doesn't exist
        if !base_path.exists() {
            std::fs::create_dir_all(base_path).context("Creating database directory")?;
        }

        // Check and run migrations if needed
        let migration_runner = MigrationRunner::new(base_path, required_version, base_version)
            .with_skip_backup(config.skip_migration_backup);
        let status = migration_runner.check_status().context("Checking migration status")?;

        // Handle migration status and open the database
        let db_path = base_path.join("db");
        let db = match &status {
            MigrationStatus::FreshDatabase => {
                tracing::info!("📦 Creating new database at version {}", required_version);
                // Write the version file for fresh database
                migration_runner.initialize_fresh_database().context("Initializing fresh database")?;
                RocksDBStorage::open(&db_path, rocksdb_config).context("Opening RocksDB storage")?
            }
            MigrationStatus::NoMigrationNeeded => {
                tracing::debug!("✅ Database version {} matches binary, no migration needed", required_version);
                RocksDBStorage::open(&db_path, rocksdb_config).context("Opening RocksDB storage")?
            }
            MigrationStatus::MigrationRequired { current_version, target_version, migration_count } => {
                tracing::info!(
                    "🔄 Database migration required: v{} -> v{} ({} migration(s))",
                    current_version,
                    target_version,
                    migration_count
                );
                tracing::info!("⚠️  This is a one-time operation that may take several minutes...");

                // Open the database for migration and reuse it after
                let db =
                    RocksDBStorage::open(&db_path, rocksdb_config).context("Opening RocksDB storage for migration")?;

                // Run migrations
                migration_runner.run_migrations_with_storage(&db).context("Running database migrations")?;

                // Reuse the same DB instance instead of reopening
                db
            }
            MigrationStatus::DatabaseTooOld { current_version, base_version } => {
                bail!(
                    "Database version {} is too old (minimum supported: {}). \
                    Please delete the database directory and resync from scratch.",
                    current_version,
                    base_version
                );
            }
            MigrationStatus::DatabaseNewer { db_version, binary_version } => {
                bail!(
                    "Database version {} is newer than this binary supports ({}). \
                    Please upgrade to a newer version of the binary.",
                    db_version,
                    binary_version
                );
            }
        };

        Ok(Arc::new(Self::new_and_init(db, chain_config, config, cairo_native_config)?))
    }

    pub fn write_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<()> {
        self.db.write_parallel_merkle_checkpoint(block_n)
    }

    pub fn has_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<bool> {
        self.db.has_parallel_merkle_checkpoint(block_n)
    }

    pub fn get_parallel_merkle_latest_checkpoint(&self) -> Result<Option<u64>> {
        self.db.get_parallel_merkle_latest_checkpoint()
    }
}

#[cfg(any(test, feature = "testing"))]
pub use crate::rocksdb::external_outbox::set_external_outbox_write_failpoint;

#[derive(Clone, Debug)]
pub struct AddFullBlockResult {
    pub new_state_root: Felt,
    pub commitments: BlockCommitments,
    pub block_hash: Felt,
    pub parent_block_hash: Felt,
    /// Timing information from the close_block DB operations.
    pub timings: CloseBlockTimings,
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    fn register_projection_violation(message: String) {
        metrics().head_projection_violation_count.add(1, &[]);
        #[cfg(test)]
        metrics().head_projection_violation_count_test.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::error!(target: "db::chain_head_projection", "{message}");
    }

    fn load_preconfirmed_block_for_tip(
        &self,
        block_n: u64,
        stored_tip: &StorageHeadProjection,
    ) -> Result<Arc<PreconfirmedBlock>> {
        if let StorageHeadProjection::Preconfirmed { header, content } = stored_tip {
            if header.block_number == block_n {
                return Ok(Arc::new(PreconfirmedBlock::new_with_content(
                    header.clone(),
                    content.clone(),
                    /* candidates */ [],
                )));
            }
        }

        let (header, content) = self
            .db
            .get_preconfirmed_block_data(block_n)?
            .with_context(|| format!("Expected persisted preconfirmed block data for block #{block_n}"))?;
        Ok(Arc::new(PreconfirmedBlock::new_with_content(header, content, /* candidates */ [])))
    }

    fn build_runtime_head_projection(
        &self,
        stored_tip: StorageHeadProjection,
    ) -> Result<(ChainHeadState, Option<Arc<PreconfirmedBlock>>)> {
        let mut chain_head_state = ChainHeadState::from_head_projection(&stored_tip);
        let latest_preconfirmed_header_block_n = self.db.get_latest_preconfirmed_header_block_n()?;

        chain_head_state.internal_preconfirmed_tip = match (
            chain_head_state.external_preconfirmed_tip,
            latest_preconfirmed_header_block_n,
        ) {
            (None, None) => None,
            (Some(external_tip), None) => Some(external_tip),
            (Some(external_tip), Some(latest_header_tip)) => {
                ensure!(
                    latest_header_tip >= external_tip,
                    "Latest persisted preconfirmed header tip ({latest_header_tip}) is behind external preconfirmed tip ({external_tip}). [stored_tip={stored_tip:?}]"
                );
                Some(latest_header_tip)
            }
            (None, Some(latest_header_tip)) => {
                bail!(
                    "Found persisted preconfirmed header tip ({latest_header_tip}) while head projection has no external preconfirmed tip. [stored_tip={stored_tip:?}]"
                );
            }
        };

        let runtime_preconfirmed = if let Some(internal_tip) = chain_head_state.internal_preconfirmed_tip {
            Some(self.load_preconfirmed_block_for_tip(internal_tip, &stored_tip)?)
        } else {
            None
        };

        Ok((chain_head_state, runtime_preconfirmed))
    }

    fn ensure_runtime_preconfirmed_alignment(
        chain_head_state: ChainHeadState,
        preconfirmed: Option<&Arc<PreconfirmedBlock>>,
    ) -> Result<()> {
        match (chain_head_state.internal_preconfirmed_tip, preconfirmed) {
            (None, None) => Ok(()),
            (Some(expected), Some(block)) if block.header.block_number == expected => Ok(()),
            (Some(expected), Some(block)) => {
                let message = format!(
                    "Runtime preconfirmed block number {} does not match head internal_preconfirmed_tip {}. [head={chain_head_state:?}]",
                    block.header.block_number, expected
                );
                Self::register_projection_violation(message.clone());
                bail!("{message}");
            }
            (Some(expected), None) => {
                let message = format!(
                    "Runtime preconfirmed block is missing while head expects internal preconfirmed tip {}. [head={chain_head_state:?}]",
                    expected
                );
                Self::register_projection_violation(message.clone());
                bail!("{message}");
            }
            (None, Some(block)) => {
                let message = format!(
                    "Runtime preconfirmed block {} exists while head has no internal preconfirmed tip. [head={chain_head_state:?}]",
                    block.header.block_number
                );
                Self::register_projection_violation(message.clone());
                bail!("{message}");
            }
        }
    }

    /// Ensure projected storage tip never gets ahead of canonical chain head state.
    ///
    /// Stale/lower confirmed values are allowed (lagging projection), but future/incompatible values are rejected.
    fn ensure_tip_not_ahead_of_head_state(
        chain_head_state: ChainHeadState,
        projected_storage_tip: &StorageHeadProjection,
    ) -> Result<()> {
        match projected_storage_tip {
            StorageHeadProjection::Empty => Ok(()),
            StorageHeadProjection::Confirmed(block_n) => {
                let is_allowed = chain_head_state.confirmed_tip.is_some_and(|confirmed| *block_n <= confirmed);
                if !is_allowed {
                    let message = format!(
                        "Projected storage tip confirmed block_n={} is ahead of canonical head confirmed_tip={:?}. [head={chain_head_state:?}, tip={projected_storage_tip:?}]",
                        block_n,
                        chain_head_state.confirmed_tip
                    );
                    Self::register_projection_violation(message.clone());
                    bail!("{message}");
                }
                Ok(())
            }
            StorageHeadProjection::Preconfirmed { header, .. } => {
                let expected = chain_head_state.external_preconfirmed_tip;
                if expected != Some(header.block_number) {
                    let message = format!(
                        "Projected storage tip preconfirmed block_n={} is incompatible with canonical head external_preconfirmed_tip={:?}. [head={chain_head_state:?}, tip={projected_storage_tip:?}]",
                        header.block_number,
                        expected
                    );
                    Self::register_projection_violation(message.clone());
                    bail!("{message}");
                }
                Ok(())
            }
        }
    }

    fn publish_head_projection(
        &self,
        chain_head_state: ChainHeadState,
        preconfirmed: Option<Arc<PreconfirmedBlock>>,
    ) -> Result<()> {
        chain_head_state.validate_cross_field_invariants().map_err(|err| {
            let message = err.to_string();
            Self::register_projection_violation(message.clone());
            anyhow::anyhow!(message)
        })?;
        Self::ensure_runtime_preconfirmed_alignment(chain_head_state, preconfirmed.as_ref())?;
        let projected_tip = storage_tip_from_head_projection(chain_head_state, preconfirmed.clone());
        Self::ensure_tip_not_ahead_of_head_state(chain_head_state, &projected_tip)?;

        let previous_head_state = *self.chain_head_state.borrow();
        let previous_runtime_preconfirmed_block_n =
            runtime_preconfirmed_block_n(self.preconfirmed_block_runtime.borrow().as_ref());
        let next_runtime_preconfirmed_block_n = runtime_preconfirmed_block_n(preconfirmed.as_ref());
        let transition = classify_chain_head_transition(previous_head_state, chain_head_state);

        tracing::info!(
            target: "db::chain_head_projection",
            transition,
            previous_confirmed_tip = ?previous_head_state.confirmed_tip,
            previous_external_preconfirmed_tip = ?previous_head_state.external_preconfirmed_tip,
            previous_internal_preconfirmed_tip = ?previous_head_state.internal_preconfirmed_tip,
            next_confirmed_tip = ?chain_head_state.confirmed_tip,
            next_external_preconfirmed_tip = ?chain_head_state.external_preconfirmed_tip,
            next_internal_preconfirmed_tip = ?chain_head_state.internal_preconfirmed_tip,
            previous_runtime_preconfirmed_block_n = ?previous_runtime_preconfirmed_block_n,
            next_runtime_preconfirmed_block_n = ?next_runtime_preconfirmed_block_n,
            "chain_head_state_updated"
        );
        tracing::info!(
            target: "db::chain_head_projection",
            "chain_head_state_updated transition={} prev_confirmed={:?} prev_external_preconfirmed={:?} prev_internal_preconfirmed={:?} next_confirmed={:?} next_external_preconfirmed={:?} next_internal_preconfirmed={:?} prev_runtime_preconfirmed={:?} next_runtime_preconfirmed={:?}",
            transition,
            previous_head_state.confirmed_tip,
            previous_head_state.external_preconfirmed_tip,
            previous_head_state.internal_preconfirmed_tip,
            chain_head_state.confirmed_tip,
            chain_head_state.external_preconfirmed_tip,
            chain_head_state.internal_preconfirmed_tip,
            previous_runtime_preconfirmed_block_n,
            next_runtime_preconfirmed_block_n
        );

        // Publish runtime preconfirmed first, then canonical head state.
        // This narrows the window where readers can observe a new head with stale runtime preconfirmed.
        self.preconfirmed_block_runtime.send_replace(preconfirmed);
        self.chain_head_state.send_replace(chain_head_state);

        Ok(())
    }

    /// Refresh in-memory head projection from persisted DB head projection.
    ///
    /// This is the canonical refresh path used by rpc/sync compatibility callsites after destructive DB operations.
    pub fn refresh_head_projection_from_db(&self) -> Result<()> {
        let (chain_head_state, preconfirmed) = self.build_runtime_head_projection(self.db.get_head_projection()?)?;
        self.publish_head_projection(chain_head_state, preconfirmed)
    }

    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        self.chain_head_state.borrow().confirmed_tip
    }
    /// Latest block_n, which may be the pre-confirmed block.
    pub fn latest_block_n(&self) -> Option<u64> {
        let head = self.chain_head_state.borrow();
        head.external_preconfirmed_tip.or(head.confirmed_tip)
    }
    pub fn has_preconfirmed_block(&self) -> bool {
        self.chain_head_state.borrow().external_preconfirmed_tip.is_some()
    }
    pub fn latest_l1_confirmed_block_n(&self) -> Option<u64> {
        *self.latest_l1_confirmed.borrow()
    }

    pub(crate) fn internal_preconfirmed_block(&self) -> Option<Arc<PreconfirmedBlock>> {
        let expected_preconfirmed = self.chain_head_state.borrow().internal_preconfirmed_tip?;
        self.preconfirmed_block_runtime
            .borrow()
            .as_ref()
            .filter(|block| block.header.block_number == expected_preconfirmed)
            .cloned()
    }

    pub fn preconfirmed_block(&self) -> Option<Arc<PreconfirmedBlock>> {
        let expected_preconfirmed = self.chain_head_state.borrow().external_preconfirmed_tip?;
        if let Some(runtime) = self
            .preconfirmed_block_runtime
            .borrow()
            .as_ref()
            .filter(|block| block.header.block_number == expected_preconfirmed)
            .cloned()
        {
            return Some(runtime);
        }

        match self.db.get_preconfirmed_block_data(expected_preconfirmed) {
            Ok(Some((header, content))) => {
                Some(Arc::new(PreconfirmedBlock::new_with_content(header, content, /* candidates */ [])))
            }
            Ok(None) => {
                tracing::warn!(
                    "Missing external preconfirmed block #{expected_preconfirmed} while head projection expects it"
                );
                None
            }
            Err(err) => {
                tracing::warn!("Failed to load external preconfirmed block #{expected_preconfirmed} from db: {err:#}");
                None
            }
        }
    }

    pub fn chain_head_state(&self) -> ChainHeadState {
        *self.chain_head_state.borrow()
    }

    /// Get the latest block_n that was in the db when this backend instance was initialized.
    pub fn get_starting_block(&self) -> Option<u64> {
        self.starting_block
    }

    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }

    pub fn execution_read_cache_config(&self) -> &ExecutionReadCacheConfig {
        &self.config.execution_read_cache
    }

    /// Get the runtime execution configuration from the database.
    pub fn get_runtime_exec_config(&self) -> Result<Option<mp_chain_config::RuntimeExecutionConfig>> {
        self.db.get_runtime_exec_config(&self.chain_config)
    }
}

/// Structure holding exclusive access to write the blocks and the tip of the chain.
///
/// Note: All of the associated functions need to be called in a rayon thread pool context.
pub struct MadaraBackendWriter<D: MadaraStorage> {
    inner: Arc<MadaraBackend<D>>,
}

impl<D: MadaraStorage> MadaraBackendWriter<D> {
    fn transition_to_confirmed_or_empty(&self, new_confirmed_tip: Option<u64>) -> Result<()> {
        // Note: concurrent use of `MadaraBackendWriter` is forbidden by contract.
        let current_head_state = *self.inner.chain_head_state.borrow();

        let current_preconfirmed_runtime = self.inner.preconfirmed_block_runtime.borrow().clone();
        MadaraBackend::<D>::ensure_runtime_preconfirmed_alignment(
            current_head_state,
            current_preconfirmed_runtime.as_ref(),
        )?;

        let next_chain_head_state = match new_confirmed_tip {
            Some(block_n) => current_head_state.next_for_confirmed(block_n)?,
            None => bail!("Cannot replace chain head to empty"),
        };

        let next_runtime_preconfirmed = match next_chain_head_state.internal_preconfirmed_tip {
            Some(expected_block_n) => Some(
                current_preconfirmed_runtime
                    .clone()
                    .filter(|block| block.header.block_number == expected_block_n)
                    .with_context(|| {
                        format!(
                            "Runtime preconfirmed block should remain available at internal tip #{expected_block_n}"
                        )
                    })?,
            ),
            None => None,
        };

        if self.inner.config.save_preconfirmed {
            let new_tip_in_db = if let Some(external_tip) = next_chain_head_state.external_preconfirmed_tip {
                if let Some(block) =
                    next_runtime_preconfirmed.as_ref().filter(|block| block.header.block_number == external_tip)
                {
                    storage_tip_from_preconfirmed_block(block)
                } else {
                    let (header, content) =
                        self.inner.db.get_preconfirmed_block_data(external_tip)?.with_context(|| {
                            format!("Expected persisted preconfirmed block data for block #{external_tip}")
                        })?;
                    StorageHeadProjection::Preconfirmed { header, content }
                }
            } else {
                storage_tip_from_confirmed_or_empty(next_chain_head_state.confirmed_tip)
            };

            if self.inner.db.get_head_projection()? != new_tip_in_db {
                self.inner.db.replace_head_projection(&new_tip_in_db)?;
            }
        } else {
            let new_tip_in_db = storage_tip_from_confirmed_or_empty(next_chain_head_state.confirmed_tip);
            if self.inner.db.get_head_projection()? != new_tip_in_db {
                self.inner.db.replace_head_projection(&new_tip_in_db)?;
            }
        }

        self.inner.publish_head_projection(next_chain_head_state, next_runtime_preconfirmed)
    }

    fn transition_to_preconfirmed(&self, preconfirmed: Arc<PreconfirmedBlock>) -> Result<()> {
        // Note: concurrent use of `MadaraBackendWriter` is forbidden by contract.
        let current_head_state = *self.inner.chain_head_state.borrow();

        let current_preconfirmed_runtime = self.inner.preconfirmed_block_runtime.borrow().clone();
        MadaraBackend::<D>::ensure_runtime_preconfirmed_alignment(
            current_head_state,
            current_preconfirmed_runtime.as_ref(),
        )?;

        let next_chain_head_state = current_head_state.next_for_preconfirmed(preconfirmed.header.block_number)?;

        if self.inner.config.save_preconfirmed {
            let internal_only_advance = current_head_state.external_preconfirmed_tip.is_some()
                && next_chain_head_state.external_preconfirmed_tip == current_head_state.external_preconfirmed_tip
                && next_chain_head_state.internal_preconfirmed_tip != current_head_state.internal_preconfirmed_tip;

            if internal_only_advance {
                self.inner.db.write_preconfirmed_header(&preconfirmed.header)?;
                let executed_transactions: Vec<PreconfirmedExecutedTransaction> =
                    preconfirmed.content.borrow().executed_transactions().cloned().collect();
                if !executed_transactions.is_empty() {
                    self.inner.db.append_preconfirmed_content(
                        preconfirmed.header.block_number,
                        0,
                        &executed_transactions,
                    )?;
                }
            } else {
                self.inner.db.replace_head_projection(&storage_tip_from_preconfirmed_block(preconfirmed.as_ref()))?;
            }
        } else {
            let new_tip_in_db = storage_tip_from_confirmed_or_empty(next_chain_head_state.confirmed_tip);
            if self.inner.db.get_head_projection()? != new_tip_in_db {
                self.inner.db.replace_head_projection(&new_tip_in_db)?;
            }
        }

        self.inner.publish_head_projection(next_chain_head_state, Some(preconfirmed))
    }

    /// Append transactions to the current preconfirmed block. Returns an error if there is no preconfirmed block.
    /// Replaces all candidate transactions with the content of `replace_candidates`.
    pub fn append_to_preconfirmed(
        &self,
        executed: &[PreconfirmedExecutedTransaction],
        replace_candidates: impl IntoIterator<Item = Arc<ValidatedTransaction>>,
    ) -> Result<()> {
        let block = self.inner.internal_preconfirmed_block().context("There is no current preconfirmed block")?;

        if self.inner.config.save_preconfirmed {
            let start_tx_index = block.content.borrow().n_executed();
            // We don't save candidate transactions.
            self.inner.db.append_preconfirmed_content(block.header.block_number, start_tx_index as u64, executed)?;
        }

        block.append(executed.iter().cloned(), replace_candidates);

        Ok(())
    }

    /// Returns an error if there is no preconfirmed block. Returns the block hash for the closed block.
    ///
    /// When `state_diff` is provided, this function uses an optimized path that skips the expensive
    /// `get_normalized_state_diff()` computation (which queries the DB for every storage entry).
    /// The provided `state_diff` should already contain all necessary fields including
    /// `old_declared_contracts`, `deployed_contracts`, and `replaced_classes`.
    pub fn close_preconfirmed(
        &self,
        pre_v0_13_2_hash_override: bool,
        block_n: u64,
        state_diff: StateDiff,
    ) -> Result<AddFullBlockResult> {
        let fetch_start = Instant::now();
        let preconfirmed_view = self
            .inner
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("There is no preconfirmed block #{block_n}"))?;
        let (mut block, classes) = preconfirmed_view.get_full_block_without_state_diff()?;
        let fetch_duration = fetch_start.elapsed();
        let fetch_secs = fetch_duration.as_secs_f64();
        metrics().get_full_block_without_state_diff_duration.record(fetch_secs, &[]);
        metrics().get_full_block_without_state_diff_last.record(fetch_secs, &[]);

        block.state_diff = state_diff;

        // Write the block & apply to global trie

        let result = self.write_new_confirmed_inner(&block, &classes, pre_v0_13_2_hash_override, fetch_duration)?;

        self.new_confirmed_block(block.header.block_number)?;

        Ok(result)
    }

    /// Clears the current preconfirmed block. Does nothing when the backend has no preconfirmed block.
    pub fn clear_preconfirmed(&self) -> Result<()> {
        self.transition_to_confirmed_or_empty(self.inner.latest_confirmed_block_n())
    }

    /// Write the runtime execution configuration to the database.
    pub fn write_runtime_exec_config(&self, config: &mp_chain_config::RuntimeExecutionConfig) -> Result<()> {
        self.inner.db.write_runtime_exec_config(config)
    }

    /// Start a new preconfirmed block on top of the latest confirmed block. Deletes and replaces the current preconfirmed block if present.
    /// Warning: Caller is responsible for ensuring the block_number is the one following the current confirmed block.
    pub fn new_preconfirmed(&self, block: PreconfirmedBlock) -> Result<()> {
        self.transition_to_preconfirmed(Arc::new(block))
    }

    /// Add a block. Returns the block hash.
    /// Warning: Caller is responsible for ensuring the block_number is the one following the current confirmed block.
    pub fn add_full_block_with_classes(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
    ) -> Result<AddFullBlockResult> {
        let block_n = block.header.block_number;
        // For add_full_block_with_classes, no get_full_block_with_classes is needed as block is already provided
        let result = self.write_new_confirmed_inner(block, classes, pre_v0_13_2_hash_override, Duration::ZERO)?;

        self.new_confirmed_block(block_n)?;
        Ok(result)
    }

    /// Does not change the head projection. Performs merkelization (global tries update) and block hash computation, and saves
    /// all the block parts. Returns the block hash and timing information.
    /// Note: The `get_full_block_with_classes` timing must be provided by the caller.
    fn write_new_confirmed_inner(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
        get_full_block_with_classes_duration: Duration,
    ) -> Result<AddFullBlockResult> {
        let mut timings = CloseBlockTimings {
            get_full_block_with_classes: get_full_block_with_classes_duration,
            ..Default::default()
        };

        let parent_block_hash = if let Some(last_block) = self.inner.block_view_on_last_confirmed() {
            last_block.get_block_info()?.block_hash
        } else {
            Felt::ZERO // genesis
        };

        let commitments_start = Instant::now();
        let commitments = BlockCommitments::compute(
            &CommitmentComputationContext {
                protocol_version: self.inner.chain_config.latest_protocol_version,
                chain_id: self.inner.chain_config.chain_id.to_felt(),
            },
            &block.transactions,
            &block.state_diff,
            &block.events,
        );
        timings.block_commitments_compute = commitments_start.elapsed();
        let commitments_secs = timings.block_commitments_compute.as_secs_f64();
        metrics().block_commitments_compute_duration.record(commitments_secs, &[]);
        metrics().block_commitments_compute_last.record(commitments_secs, &[]);

        let (global_state_root, merklization_timings) =
            self.apply_to_global_trie(block.header.block_number, [&block.state_diff])?;

        // Copy merklization timings
        timings.merklization = merklization_timings.total;
        timings.contract_trie_root = merklization_timings.contract_trie_root;
        timings.class_trie_root = merklization_timings.class_trie_root;
        timings.contract_storage_trie_commit = merklization_timings.contract_trie.storage_commit;
        timings.contract_trie_commit = merklization_timings.contract_trie.trie_commit;
        timings.class_trie_commit = merklization_timings.class_trie.trie_commit;

        let header =
            block.header.clone().into_confirmed_header(parent_block_hash, commitments.clone(), global_state_root);

        let hash_start = Instant::now();
        let block_hash = header.compute_hash(self.inner.chain_config.chain_id.to_felt(), pre_v0_13_2_hash_override);
        timings.block_hash_compute = hash_start.elapsed();
        let hash_secs = timings.block_hash_compute.as_secs_f64();
        metrics().block_hash_compute_duration.record(hash_secs, &[]);
        metrics().block_hash_compute_last.record(hash_secs, &[]);

        tracing::info!("Block hash {block_hash:#x} computed for #{}", block.header.block_number);

        if let Some(header) = self.inner.get_custom_header_with_clear(true) {
            let is_valid = header.is_block_hash_as_expected(&block_hash);
            if !is_valid {
                tracing::warn!("Block hash not as expected for {}", block.header.block_number);
            }
        }

        // Save the block.

        let write_start = Instant::now();
        self.write_header(BlockHeaderWithSignatures { header, block_hash, consensus_signatures: vec![] })?;
        self.write_transactions(block.header.block_number, &block.transactions)?;
        self.write_state_diff(block.header.block_number, &block.state_diff)?;
        self.write_events(block.header.block_number, &block.events)?;
        self.write_classes(block.header.block_number, classes)?;
        timings.db_write_block_parts = write_start.elapsed();
        let write_secs = timings.db_write_block_parts.as_secs_f64();
        metrics().db_write_block_parts_duration.record(write_secs, &[]);
        metrics().db_write_block_parts_last.record(write_secs, &[]);

        Ok(AddFullBlockResult {
            new_state_root: global_state_root,
            commitments,
            block_hash,
            parent_block_hash,
            timings,
        })
    }

    /// Like `write_new_confirmed_inner` but uses a precomputed state root and merklization timings
    /// instead of calling `apply_to_global_trie` inline. Used by the parallel merkle path where
    /// trie root computation runs in a dedicated worker.
    fn write_new_confirmed_with_precomputed_root(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
        precomputed_root: Felt,
        merklization_timings: rocksdb::global_trie::MerklizationTimings,
        get_full_block_with_classes_duration: Duration,
    ) -> Result<AddFullBlockResult> {
        let mut timings = CloseBlockTimings {
            get_full_block_with_classes: get_full_block_with_classes_duration,
            ..Default::default()
        };

        let parent_block_hash = if let Some(last_block) = self.inner.block_view_on_last_confirmed() {
            last_block.get_block_info()?.block_hash
        } else {
            Felt::ZERO // genesis
        };

        let commitments_start = Instant::now();
        let commitments = BlockCommitments::compute(
            &CommitmentComputationContext {
                protocol_version: self.inner.chain_config.latest_protocol_version,
                chain_id: self.inner.chain_config.chain_id.to_felt(),
            },
            &block.transactions,
            &block.state_diff,
            &block.events,
        );
        timings.block_commitments_compute = commitments_start.elapsed();
        let commitments_secs = timings.block_commitments_compute.as_secs_f64();
        metrics().block_commitments_compute_duration.record(commitments_secs, &[]);
        metrics().block_commitments_compute_last.record(commitments_secs, &[]);

        // Use precomputed root instead of calling apply_to_global_trie
        let global_state_root = precomputed_root;

        // Copy merklization timings from the worker
        timings.merklization = merklization_timings.total;
        timings.contract_trie_root = merklization_timings.contract_trie_root;
        timings.class_trie_root = merklization_timings.class_trie_root;
        timings.contract_storage_trie_commit = merklization_timings.contract_trie.storage_commit;
        timings.contract_trie_commit = merklization_timings.contract_trie.trie_commit;
        timings.class_trie_commit = merklization_timings.class_trie.trie_commit;

        let header =
            block.header.clone().into_confirmed_header(parent_block_hash, commitments.clone(), global_state_root);

        let hash_start = Instant::now();
        let block_hash = header.compute_hash(self.inner.chain_config.chain_id.to_felt(), pre_v0_13_2_hash_override);
        timings.block_hash_compute = hash_start.elapsed();
        let hash_secs = timings.block_hash_compute.as_secs_f64();
        metrics().block_hash_compute_duration.record(hash_secs, &[]);
        metrics().block_hash_compute_last.record(hash_secs, &[]);

        tracing::info!("Block hash {block_hash:#x} computed for #{} (parallel merkle)", block.header.block_number);

        if let Some(header) = self.inner.get_custom_header_with_clear(true) {
            let is_valid = header.is_block_hash_as_expected(&block_hash);
            if !is_valid {
                tracing::warn!("Block hash not as expected for {}", block.header.block_number);
            }
        }

        // Save the block.
        let write_start = Instant::now();
        self.write_header(BlockHeaderWithSignatures { header, block_hash, consensus_signatures: vec![] })?;
        self.write_transactions(block.header.block_number, &block.transactions)?;
        self.write_state_diff(block.header.block_number, &block.state_diff)?;
        self.write_events(block.header.block_number, &block.events)?;
        self.write_classes(block.header.block_number, classes)?;
        timings.db_write_block_parts = write_start.elapsed();
        let write_secs = timings.db_write_block_parts.as_secs_f64();
        metrics().db_write_block_parts_duration.record(write_secs, &[]);
        metrics().db_write_block_parts_last.record(write_secs, &[]);

        Ok(AddFullBlockResult {
            new_state_root: global_state_root,
            commitments,
            block_hash,
            parent_block_hash,
            timings,
        })
    }

    /// Write preconfirmed block parts with a precomputed state root from a parallel worker.
    ///
    /// This does NOT advance the confirmed head. Callers that need to confirm must invoke
    /// `new_confirmed_block` only after any required durability steps (e.g. boundary flush)
    /// complete successfully.
    pub fn write_preconfirmed_with_precomputed_root(
        &self,
        pre_v0_13_2_hash_override: bool,
        block_n: u64,
        state_diff: StateDiff,
        precomputed_root: Felt,
        merklization_timings: rocksdb::global_trie::MerklizationTimings,
    ) -> Result<AddFullBlockResult> {
        let fetch_start = Instant::now();
        let preconfirmed_view = self
            .inner
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("There is no preconfirmed block #{block_n}"))?;
        let (mut block, classes) = preconfirmed_view.get_full_block_without_state_diff()?;
        let fetch_duration = fetch_start.elapsed();
        let fetch_secs = fetch_duration.as_secs_f64();
        metrics().get_full_block_without_state_diff_duration.record(fetch_secs, &[]);
        metrics().get_full_block_without_state_diff_last.record(fetch_secs, &[]);

        block.state_diff = state_diff;

        self.write_new_confirmed_with_precomputed_root(
            &block,
            &classes,
            pre_v0_13_2_hash_override,
            precomputed_root,
            merklization_timings,
            fetch_duration,
        )
    }

    /// Close a preconfirmed block with a precomputed state root from a parallel worker.
    /// This is the parallel merkle counterpart of `close_preconfirmed`.
    #[deprecated(
        note = "Use write_preconfirmed_with_precomputed_root + new_confirmed_block in phased order to preserve boundary durability semantics."
    )]
    pub fn close_preconfirmed_with_precomputed_root(
        &self,
        pre_v0_13_2_hash_override: bool,
        block_n: u64,
        state_diff: StateDiff,
        precomputed_root: Felt,
        merklization_timings: rocksdb::global_trie::MerklizationTimings,
    ) -> Result<AddFullBlockResult> {
        let result = self.write_preconfirmed_with_precomputed_root(
            pre_v0_13_2_hash_override,
            block_n,
            state_diff,
            precomputed_root,
            merklization_timings,
        )?;
        self.new_confirmed_block(block_n)?;
        Ok(result)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_header(&self, header: BlockHeaderWithSignatures) -> Result<()> {
        self.inner.db.write_header(header)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_transactions(&self, block_n: u64, txs: &[TransactionWithReceipt]) -> Result<()> {
        self.inner.db.write_transactions(block_n, txs)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_state_diff(&self, block_n: u64, value: &StateDiff) -> Result<()> {
        self.inner.db.write_state_diff(block_n, value)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_bouncer_weights(&self, block_n: u64, value: &BouncerWeights) -> Result<()> {
        self.inner.db.write_bouncer_weights(block_n, value)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_events(&self, block_n: u64, txs: &[EventWithTransactionHash]) -> Result<()> {
        self.inner.db.write_events(block_n, txs)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// **Warning**: The caller must ensure no block parts is saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn write_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        self.inner.db.write_classes(block_n, converted_classes)
    }

    /// Update the compiled_class_hash_v2 (BLAKE hash) for existing classes (SNIP-34 migration).
    /// This updates the ClassInfo stored in the database with the new v2 hash.
    pub fn update_class_v2_hashes(&self, migrations: Vec<(Felt, Felt)>) -> Result<()> {
        self.inner.db.update_class_v2_hashes(migrations)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    ///
    /// Write a state diff to the global tries.
    /// Returns the new state root.
    ///
    /// **Warning**: The caller must ensure no block parts are saved on top of an existing confirmed block.
    /// You are only allowed to write block parts past the latest confirmed block.
    pub fn apply_to_global_trie<'a>(
        &self,
        start_block_n: u64,
        state_diffs: impl IntoIterator<Item = &'a StateDiff>,
    ) -> Result<(Felt, rocksdb::global_trie::MerklizationTimings)> {
        self.inner.db.apply_to_global_trie(start_block_n, state_diffs)
    }

    /// Lower level access to writing primitives. This is only used by the sync process, which
    /// saves block parts separately for performance reasons.
    /// This function in particular marks a fully imported block as confirmed. It also clears the current preconfirmed block, if any.
    ///
    /// **Warning**: The caller must ensure this new imported block is the one following the current confirmed block.
    /// You are not allowed to call this function with earlier or later blocks.
    /// In addition, you must have fully imported the block using the low level writing primitives for each of the block
    /// parts.
    pub fn new_confirmed_block(&self, block_number: u64) -> Result<()> {
        // Flush the most latest state to db to reduce data loss
        if self
            .inner
            .config
            .flush_every_n_blocks
            .is_some_and(|flush_every_n_blocks| block_number.checked_rem(flush_every_n_blocks) == Some(0))
        {
            tracing::debug!("Flushing.");
            self.inner.db.flush().context("Periodic database flush")?;
        }

        // Update snapshots for storage proofs. (TODO (heemank 10/11/2025): decouple this logic)
        self.inner.db.on_new_confirmed_head(block_number)?;

        // Advance chain & clear preconfirmed atomically
        self.transition_to_confirmed_or_empty(Some(block_number))?;
        // Confirmed-path immediate GC for block-keyed preconfirmed persistence.
        self.inner.db.delete_preconfirmed_rows_up_to(block_number)?;

        Ok(())
    }

    // /// Returns the total storage size
    // pub fn update_metrics(&self) -> u64 {
    //     self.db_metrics.update(&self.db)
    // }
}

// Delegate these db reads/writes. These are related to specific services, and are not specific to a block view / the head projection writer handle.
impl<D: MadaraStorageRead> MadaraBackend<D> {
    pub fn get_l1_messaging_sync_tip(&self) -> Result<Option<u64>> {
        self.db.get_l1_messaging_sync_tip()
    }
    pub fn get_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.db.get_pending_message_to_l2(core_contract_nonce)
    }
    pub fn get_next_pending_message_to_l2(&self, start_nonce: u64) -> Result<Option<L1HandlerTransactionWithFee>> {
        self.db.get_next_pending_message_to_l2(start_nonce)
    }
    /// Returns the L1 transaction hash which emitted the L1->L2 message for the given core contract nonce, if known.
    ///
    /// This is written by the settlement client during L1 messaging sync and is later used to answer
    /// `starknet_getMessagesStatus` without making L1 requests at RPC time.
    pub fn get_l1_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<mp_convert::L1TransactionHash>> {
        self.db.get_l1_txn_hash_by_nonce(core_contract_nonce)
    }
    pub fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.db.get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
    }
    pub fn get_l1_handler_l1_block_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<u64>> {
        self.db.get_l1_handler_l1_block_by_nonce(core_contract_nonce)
    }
    /// Returns all messages sent by a given L1 transaction, as `(nonce, consumed_l2_tx_hash_if_known)`.
    pub fn get_messages_to_l2_by_l1_tx_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<Option<crate::storage::L1ToL2MessagesByL1TxHash>> {
        self.db.get_messages_to_l2_by_l1_tx_hash(l1_tx_hash)
    }
    /// Returns the status entry for a specific `(l1_tx_hash, nonce)` message index key.
    pub fn get_message_to_l2_index_entry(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<Option<crate::storage::L1ToL2MessageIndexEntry>> {
        self.db.get_message_to_l2_index_entry(l1_tx_hash, core_contract_nonce)
    }
    pub fn get_saved_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        self.db.get_mempool_transactions()
    }
    pub fn get_external_outbox_transactions(
        &self,
        limit: usize,
    ) -> impl Iterator<Item = Result<ExternalOutboxEntry>> + '_ {
        self.db.get_external_outbox_transactions(limit)
    }
    pub fn get_external_outbox_size_estimate(&self) -> Result<u64> {
        self.db.get_external_outbox_size_estimate()
    }
    pub fn get_devnet_predeployed_keys(&self) -> Result<Option<DevnetPredeployedKeys>> {
        self.db.get_devnet_predeployed_keys()
    }
    pub fn get_latest_applied_trie_update(&self) -> Result<Option<u64>> {
        self.db.get_latest_applied_trie_update()
    }
    pub fn get_snap_sync_latest_block(&self) -> Result<Option<u64>> {
        self.db.get_snap_sync_latest_block()
    }
}
// Delegate these db reads/writes. These are related to specific services, and are not specific to a block view / the head projection writer handle.
impl<D: MadaraStorageWrite> MadaraBackend<D> {
    pub fn write_l1_messaging_sync_tip(&self, l1_block_n: Option<u64>) -> Result<()> {
        self.db.write_l1_messaging_sync_tip(l1_block_n)
    }
    pub fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        self.db.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash)
    }
    pub fn write_l1_handler_l1_block_by_nonce(&self, core_contract_nonce: u64, l1_block_n: u64) -> Result<()> {
        self.db.write_l1_handler_l1_block_by_nonce(core_contract_nonce, l1_block_n)
    }
    pub fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        self.db.write_pending_message_to_l2(msg)
    }
    pub fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        self.db.remove_pending_message_to_l2(core_contract_nonce)
    }
    /// Stores the L1 transaction hash which emitted the L1->L2 message identified by `core_contract_nonce`.
    pub fn write_l1_txn_hash_by_nonce(
        &self,
        core_contract_nonce: u64,
        l1_tx_hash: &mp_convert::L1TransactionHash,
    ) -> Result<()> {
        self.db.write_l1_txn_hash_by_nonce(core_contract_nonce, l1_tx_hash)
    }
    /// Inserts a "seen on L1" marker for the `(l1_tx_hash, nonce)` pair, if the key is missing.
    ///
    /// This is idempotent and will not overwrite an already-consumed entry.
    pub fn insert_message_to_l2_seen_marker(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
    ) -> Result<bool> {
        self.db.insert_message_to_l2_seen_marker(l1_tx_hash, core_contract_nonce)
    }
    /// Writes the consumed L2 transaction hash for the `(l1_tx_hash, nonce)` pair.
    pub fn write_message_to_l2_consumed_txn_hash(
        &self,
        l1_tx_hash: &mp_convert::L1TransactionHash,
        core_contract_nonce: u64,
        l2_tx_hash: &Felt,
    ) -> Result<()> {
        self.db.write_message_to_l2_consumed_txn_hash(l1_tx_hash, core_contract_nonce, l2_tx_hash)
    }
    pub fn write_devnet_predeployed_keys(&self, devnet_keys: &DevnetPredeployedKeys) -> Result<()> {
        self.db.write_devnet_predeployed_keys(devnet_keys)
    }
    pub fn remove_saved_mempool_transactions(&self, tx_hashes: impl IntoIterator<Item = Felt>) -> Result<()> {
        self.db.remove_mempool_transactions(tx_hashes)
    }
    pub fn write_saved_mempool_transaction(&self, tx: &ValidatedTransaction) -> Result<()> {
        self.db.write_mempool_transaction(tx)
    }
    pub fn write_external_outbox(&self, tx: &ValidatedTransaction) -> Result<ExternalOutboxId> {
        self.db.write_external_outbox(tx)
    }
    pub fn delete_external_outbox(&self, id: ExternalOutboxId) -> Result<()> {
        self.db.delete_external_outbox(id)
    }
    pub fn write_latest_applied_trie_update(&self, block_n: &Option<u64>) -> Result<()> {
        self.db.write_latest_applied_trie_update(block_n)
    }
    pub fn write_snap_sync_latest_block(&self, block_n: &Option<u64>) -> Result<()> {
        self.db.write_snap_sync_latest_block(block_n)
    }

    /// Revert the blockchain to a specific block hash.
    pub fn revert_to(&self, new_tip_block_hash: &Felt) -> Result<(u64, Felt)> {
        self.db.revert_to(new_tip_block_hash)
    }
}
