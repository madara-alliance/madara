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
use std::sync::{Arc, Mutex, RwLock};
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

const SLOW_CONFIRMED_HEAD_PHASE: Duration = Duration::from_secs(5);

/// Warns when one synchronous confirmed-head phase can visibly stall block production.
pub(crate) fn warn_if_confirmed_head_phase_slow(block_n: u64, phase: &'static str, elapsed: Duration) {
    if elapsed >= SLOW_CONFIRMED_HEAD_PHASE {
        tracing::warn!(
            block_number = block_n,
            phase,
            duration_ms = elapsed.as_secs_f64() * 1000.0,
            "confirmed_head_phase_slow"
        );
    }
}

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
    /// Initializes runtime counters from a replay boundary and any already executed durable prefix.
    /// Consistency flags are evaluated immediately so resumed boundaries cannot hide an existing mismatch.
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

    /// Returns whether execution has reached the configured replay count and terminal hash.
    /// Any recorded mismatch keeps the boundary open even when both counters appear complete.
    fn boundary_met(&self) -> bool {
        self.executed_tx_count == self.boundary.expected_tx_count
            && self.reached_last_tx_hash
            && self.mismatch.is_none()
    }

    /// Projects internal replay bookkeeping into the stable administrative RPC status shape.
    /// The boundary-met flag is derived at read time from the latest counters and mismatch state.
    fn to_status(&self) -> ReplayBlockBoundaryStatus {
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

    /// Recomputes replay-boundary mismatch state after any counter or hash update.
    /// The first mismatch is retained so later observations cannot hide its original cause.
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
            let last_executed_tx_hash =
                self.last_executed_tx_hash.map(|hash| format!("{hash:#x}")).unwrap_or_else(|| "<none>".to_string());
            let expected_last_tx_hash = format!("{:#x}", self.boundary.last_tx_hash);
            self.set_mismatch_if_empty(format!(
                "executed_tx_count reached expected count but last_executed_tx_hash={} does not match expected_last_tx_hash={}",
                last_executed_tx_hash, expected_last_tx_hash
            ));
        }
    }

    /// Stores the first replay-boundary mismatch and preserves it across later updates.
    /// This makes diagnostic status deterministic for callers polling the boundary.
    fn set_mismatch_if_empty(&mut self, message: String) {
        if self.mismatch.is_none() {
            self.mismatch = Some(message);
        }
    }
}

/// Converts an optional confirmed tip into its durable storage projection.
/// Absence maps to an explicitly empty chain rather than a synthetic genesis block.
fn storage_tip_from_confirmed_or_empty(confirmed_tip: Option<u64>) -> StorageHeadProjection {
    match confirmed_tip {
        Some(block_n) => StorageHeadProjection::Confirmed(block_n),
        None => StorageHeadProjection::Empty,
    }
}

/// Captures one runtime preconfirmed block as a persistable header-and-content projection.
/// Candidate transactions are excluded because only executed content is durable recovery input.
fn storage_tip_from_preconfirmed_block(block: &PreconfirmedBlock) -> StorageHeadProjection {
    StorageHeadProjection::Preconfirmed {
        header: block.header.clone(),
        content: block.content.borrow().executed_transactions().cloned().collect(),
    }
}

/// Selects the externally visible preconfirmed projection or falls back to the confirmed head.
/// A runtime block must match the projected external tip before it can be persisted.
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

type RuntimePreconfirmedBlocks = BTreeMap<u64, Arc<PreconfirmedBlock>>;

/// Returns the highest block number represented by the in-memory preconfirmed window.
/// An empty window has no runtime preconfirmed tip.
fn runtime_preconfirmed_tip_block_n(preconfirmed: &RuntimePreconfirmedBlocks) -> Option<u64> {
    preconfirmed.last_key_value().map(|(block_n, _)| *block_n)
}

/// Clones one block handle from the in-memory preconfirmed window.
/// Missing block numbers are reported as `None` without altering the window.
fn runtime_preconfirmed_block(
    preconfirmed: &RuntimePreconfirmedBlocks,
    block_n: u64,
) -> Option<Arc<PreconfirmedBlock>> {
    preconfirmed.get(&block_n).cloned()
}

/// Removes runtime preconfirmed entries outside the confirmed-floor/internal-frontier window.
/// An absent internal tip clears the runtime map completely.
fn prune_runtime_preconfirmed_blocks(preconfirmed: &mut RuntimePreconfirmedBlocks, chain_head_state: ChainHeadState) {
    if let Some(confirmed_tip) = chain_head_state.confirmed_tip {
        preconfirmed.retain(|block_n, _| *block_n > confirmed_tip);
    }

    if let Some(internal_tip) = chain_head_state.internal_preconfirmed_tip {
        preconfirmed.retain(|block_n, _| *block_n <= internal_tip);
    } else {
        preconfirmed.clear();
    }
}

/// Classifies a head transition into a stable diagnostic label.
/// Confirmed and external visibility changes take precedence over internal-only runahead.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReorgHead {
    /// Full backend tip state visible to subscribers at this point in time.
    pub tip: ChainHeadState,
    /// Latest confirmed block number associated with the tip.
    pub latest_confirmed_block_n: u64,
    /// Hash of that latest confirmed block.
    pub latest_confirmed_block_hash: Felt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReorgNotification {
    /// Chain head before the revert was applied.
    pub previous_head: ReorgHead,
    /// Chain head after the revert was applied.
    pub new_head: ReorgHead,
    /// First confirmed block number removed by the reorg.
    pub first_reverted_block_n: u64,
    /// Hash of the first confirmed block removed by the reorg.
    pub first_reverted_block_hash: Felt,
}

impl ReorgNotification {
    pub fn last_reverted_block_n(&self) -> u64 {
        self.previous_head.latest_confirmed_block_n
    }

    pub fn reverted_block_count(&self) -> u64 {
        self.last_reverted_block_n() - self.first_reverted_block_n + 1
    }
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
    pub preconfirmed_block_runtime: RwLock<RuntimePreconfirmedBlocks>,

    /// Serializes read-modify-write transitions of the canonical head and runtime preconfirmed map.
    ///
    /// Merkle computation and block-part writes deliberately remain outside this lock. Only the
    /// short projection transition is serialized so a confirmation cannot publish a stale copy
    /// over a concurrently-created preconfirmed block.
    head_projection_write_lock: Mutex<()>,

    /// Current finalized block_n on L1.
    latest_l1_confirmed: tokio::sync::watch::Sender<Option<u64>>,

    /// First-class reorg notifications for consumers that need more than the lossy chain-head watch.
    reorg_notifications: tokio::sync::broadcast::Sender<ReorgNotification>,

    /// Cairo Native execution configuration.
    ///
    /// This config is passed through to BlockifierStateAdapter for execution.
    /// The `enable_native_execution` flag in the config controls whether native execution is used.
    pub cairo_native_config: Arc<NativeConfig>,

    /// Keep the TempDir instance around so that the directory is not deleted until the MadaraBackend struct is dropped.
    #[cfg(any(test, feature = "testing"))]
    _temp_dir: Option<tempfile::TempDir>,

    /// Custom headers used during block replay to ensure deterministic execution.
    ///
    /// When replaying a block, we must match the exact timestamp and gas configuration
    /// from the original block to reproduce the expected block hash. These per-block
    /// overrides are applied during transaction validation and execution, along with the
    /// expected block hash to validate against after block creation.
    /// # Important Notes
    /// - Custom headers are keyed by block number because replay can prepare future blocks ahead of time
    /// - **Must verify** that the block number matches before use
    /// - **Must clear** the matching block entry after use to prevent reuse across different blocks
    /// - Access is thread-safe via Mutex to allow concurrent operations
    pub custom_headers: Mutex<std::collections::HashMap<u64, CustomHeader>>,

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

mod backend;

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

mod head_projection;

mod writer;
pub use writer::MadaraBackendWriter;

mod service_storage;
