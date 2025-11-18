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
use crate::storage::StorageChainTip;
use crate::storage::StoredChainInfo;
use crate::sync_status::SyncStatusCell;
use mp_block::commitments::BlockCommitments;
use mp_block::commitments::CommitmentComputationContext;
use mp_block::header::CustomHeader;
use mp_block::BlockHeaderWithSignatures;
use mp_block::FullBlockWithoutCommitments;
use mp_block::TransactionWithReceipt;
use mp_chain_config::ChainConfig;
use mp_class::ConvertedClass;
use mp_receipt::EventWithTransactionHash;
use mp_state_update::StateDiff;
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::L1HandlerTransactionWithFee;
use prelude::*;
use std::path::Path;
use std::sync::{Arc, Mutex};
mod db_version;
mod prelude;
pub mod storage;

pub mod gas;
pub mod preconfirmed;
pub mod rocksdb;
pub mod subscription;
pub mod sync_status;
pub mod tests;
pub mod view;

use blockifier::bouncer::BouncerWeights;
pub use storage::{
    DevnetPredeployedContractAccount, DevnetPredeployedKeys, EventFilter, MadaraStorage, MadaraStorageRead,
    MadaraStorageWrite, StorageTxIndex,
};
pub use view::{MadaraBlockView, MadaraConfirmedBlockView, MadaraPreconfirmedBlockView, MadaraStateView};

/// Current chain tip.
#[derive(Default, Clone)]
pub enum ChainTip {
    /// Empty pre-genesis state. There are no blocks currently in the backend.
    #[default]
    Empty,
    /// Latest block is a confirmed block.
    Confirmed(/* block_number */ u64),
    /// Latest block is a preconfirmed block.
    Preconfirmed(Arc<PreconfirmedBlock>),
}

// Use [`Arc::ptr_eq`] for quick equality check: we don't want to compare the content of the transactions
// for the preconfirmed block case.
impl PartialEq for ChainTip {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Confirmed(l0), Self::Confirmed(r0)) => l0 == r0,
            (Self::Preconfirmed(l0), Self::Preconfirmed(r0)) => Arc::ptr_eq(l0, r0),
            _ => false,
        }
    }
}
impl Eq for ChainTip {}

impl fmt::Debug for ChainTip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "Empty"),
            Self::Confirmed(block_n) => write!(f, "Confirmed block_n={block_n}"),
            Self::Preconfirmed(preconfirmed_block) => {
                write!(f, "Preconfirmed block_n={}", preconfirmed_block.header.block_number)
            }
        }
    }
}

impl ChainTip {
    pub fn on_confirmed_block_n_or_empty(block_n: Option<u64>) -> Self {
        match block_n {
            Some(block_n) => Self::Confirmed(block_n),
            None => Self::Empty,
        }
    }

    /// Latest block_n, which may be the pre-confirmed block.
    pub fn block_n(&self) -> Option<u64> {
        match self {
            Self::Empty => None,
            Self::Confirmed(block_n) => Some(*block_n),
            Self::Preconfirmed(b) => Some(b.header.block_number),
        }
    }
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        match self {
            Self::Empty => None,
            Self::Preconfirmed(b) => b.header.block_number.checked_sub(1),
            Self::Confirmed(block_n) => Some(*block_n),
        }
    }
    pub fn is_preconfirmed(&self) -> bool {
        matches!(self, Self::Preconfirmed(_))
    }
    pub fn as_preconfirmed(&self) -> Option<&Arc<PreconfirmedBlock>> {
        match self {
            Self::Preconfirmed(b) => Some(b),
            _ => None,
        }
    }

    /// Convert to the chain tip type for use in the storage backend. It is distinct from our the internal
    /// ChainTip to hide implementation details from the storage implementation.
    fn to_storage(&self) -> StorageChainTip {
        match self {
            Self::Empty => StorageChainTip::Empty,
            Self::Confirmed(block_n) => StorageChainTip::Confirmed(*block_n),
            Self::Preconfirmed(preconfirmed_block) => StorageChainTip::Preconfirmed {
                header: preconfirmed_block.header.clone(),
                content: preconfirmed_block.content.borrow().executed_transactions().cloned().collect(),
            },
        }
    }
    pub fn from_storage(tip: StorageChainTip) -> Self {
        match tip {
            StorageChainTip::Empty => Self::Empty,
            StorageChainTip::Confirmed(block_n) => Self::Confirmed(block_n),
            StorageChainTip::Preconfirmed { header, content } => {
                Self::Preconfirmed(PreconfirmedBlock::new_with_content(header, content, /* candidates */ []).into())
            }
        }
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

    pub chain_tip: tokio::sync::watch::Sender<ChainTip>,

    /// Current finalized block_n on L1.
    latest_l1_confirmed: tokio::sync::watch::Sender<Option<u64>>,

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
}

#[derive(Debug, Default)]
pub struct MadaraBackendConfig {
    pub flush_every_n_blocks: Option<u64>,
    /// When false, the preconfirmed block is never saved to database.
    pub save_preconfirmed: bool,
    pub unsafe_starting_block: Option<u64>,
}

impl<D: MadaraStorage> MadaraBackend<D> {
    fn new_and_init(db: D, chain_config: Arc<ChainConfig>, config: MadaraBackendConfig) -> Result<Self> {
        let mut backend = Self {
            db,
            // db_metrics: DbMetrics::register().context("Registering db metrics")?,
            chain_config,
            starting_block: config.unsafe_starting_block,
            config,
            sync_status: SyncStatusCell::default(),
            watch_gas_quote: L1GasQuoteCell::default(),
            #[cfg(any(test, feature = "testing"))]
            _temp_dir: None,
            chain_tip: tokio::sync::watch::Sender::new(Default::default()),
            latest_l1_confirmed: tokio::sync::watch::Sender::new(Default::default()),
            custom_header: Mutex::new(None),
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

        // Init chain_tip and set starting block
        let chain_tip = ChainTip::from_storage(if let Some(starting_block) = self.starting_block {
            StorageChainTip::Confirmed(starting_block)
        } else {
            self.db.get_chain_tip()?
        });
        self.starting_block = chain_tip.latest_confirmed_block_n();
        // On startup, remove all blocks past the chain tip, in case we have partial blocks in db.
        self.db.remove_all_blocks_starting_from(
            chain_tip.latest_confirmed_block_n().map(|n| n + 1).unwrap_or(/* genesis */ 0),
        )?;
        self.chain_tip.send_replace(chain_tip);

        // Init L1 head
        self.latest_l1_confirmed.send_replace(self.db.get_confirmed_on_l1_tip()?);

        Ok(())
    }

    /// Get a write handle for the backend. This is the function you need to call to save new blocks, modify the preconfirmed block,
    /// and do any other such thing. The backend chain_tip can only be modified through this.
    ///
    /// As a caller, you are responsible for ensuring the backend is not being concurrently
    /// modified in an unexpected way. In practice, this means:
    /// - You are allowed to use the `write_*` low-level functions to write block parts concurrently.
    /// - You are not allowed to use the other functions to advance the chain tip
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

    /// Flush all pending writes to disk. Critical for databases with WAL disabled.
    /// Must be called before shutdown to ensure data persistence.
    pub fn flush(&self) -> Result<()> {
        self.db.flush()
    }
}

impl MadaraBackend<RocksDBStorage> {
    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<Self> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let db = RocksDBStorage::open(temp_dir.as_ref(), Default::default()).unwrap();
        let mut backend = Self::new_and_init(db, chain_config, Default::default()).unwrap();
        backend._temp_dir = Some(temp_dir);
        Arc::new(backend)
    }

    /// Open the db.
    pub fn open_rocksdb(
        base_path: &Path,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
        rocksdb_config: RocksDBConfig,
    ) -> Result<Arc<Self>> {
        // check if the db version is compatible with the current binary
        tracing::debug!("checking db version");
        if let Some(db_version) = db_version::check_db_version(base_path).context("Checking database version")? {
            tracing::debug!("version of existing db is {db_version}");
        }
        let db_path = base_path.join("db");
        let db = RocksDBStorage::open(&db_path, rocksdb_config).context("Opening rocksdb storage")?;
        Ok(Arc::new(Self::new_and_init(db, chain_config, config)?))
    }
}

#[derive(Clone, Debug)]
pub struct AddFullBlockResult {
    pub new_state_root: Felt,
    pub commitments: BlockCommitments,
    pub block_hash: Felt,
    pub parent_block_hash: Felt,
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        self.chain_tip.borrow().latest_confirmed_block_n()
    }
    /// Latest block_n, which may be the pre-confirmed block.
    pub fn latest_block_n(&self) -> Option<u64> {
        self.chain_tip.borrow().block_n()
    }
    pub fn has_preconfirmed_block(&self) -> bool {
        self.chain_tip.borrow().is_preconfirmed()
    }
    pub fn latest_l1_confirmed_block_n(&self) -> Option<u64> {
        *self.latest_l1_confirmed.borrow()
    }

    pub fn preconfirmed_block(&self) -> Option<Arc<PreconfirmedBlock>> {
        self.chain_tip.borrow().as_preconfirmed().cloned()
    }

    /// Get the latest block_n that was in the db when this backend instance was initialized.
    pub fn get_starting_block(&self) -> Option<u64> {
        self.starting_block
    }

    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }
}

/// Structure holding exclusive access to write the blocks and the tip of the chain.
///
/// Note: All of the associated functions need to be called in a rayon thread pool context.
pub struct MadaraBackendWriter<D: MadaraStorage> {
    inner: Arc<MadaraBackend<D>>,
}

impl<D: MadaraStorage> MadaraBackendWriter<D> {
    fn replace_chain_tip(&self, new_tip: ChainTip) -> Result<()> {
        // Note: while you could think it is possible for the `chain_tip` to change between this next line when we
        // originally get it, and when we save the replace it to a new one, leading to possible corruption in this
        // race condition, we have explicitely forbidden `MadaraBackendWriter` as a whole to be used concurrently.
        let current_tip = self.inner.chain_tip.borrow().clone();

        // Detect if state transition is valid.
        match (&current_tip, &new_tip) {
            // Adding the genesis block, which can be preconfirmed or confirmed. Replacing empty with empty won't work (block_n returns None).
            (ChainTip::Empty, block) => ensure!(block.block_n() == Some(0), "Can only replace the empty chain tip with a genesis block. [current_tip={current_tip:?}, new_tip={new_tip:?}]"),
            // Never valid.
            (_, ChainTip::Empty) => bail!("Cannot replace the chain tip to empty. [current_tip={current_tip:?}, new_tip={new_tip:?}]"),
            // Block is closed, preconfirmed replaces confirmed at same height.
            // OR: preconfirmed block is being cleared.
            (ChainTip::Preconfirmed(preconfirmed), ChainTip::Confirmed(new_block_n)) => ensure!(
                preconfirmed.header.block_number == *new_block_n || preconfirmed.header.block_number == *new_block_n + 1,
                "Replacing chain tip from preconfirmed to confirmed requires the new block_n to match the previous one, or be one less than it. [current_tip={current_tip:?}, new_tip={new_tip:?}]"
            ),
            // New preconfirmed at same height, replacing the previous proposal.
            (ChainTip::Preconfirmed(preconfirmed), ChainTip::Preconfirmed(new_preconfirmed)) => ensure!(
                preconfirmed.header.block_number == new_preconfirmed.header.block_number,
                "Replacing chain tip from preconfirmed to preconfirmed requires the new block_n to match the previous one. [current_tip={current_tip:?}, new_tip={new_tip:?}]"
            ),
            // New preconfirmed block on top of a confirmed block.
            (ChainTip::Confirmed(block_n), ChainTip::Preconfirmed(preconfirmed)) => ensure!(
                block_n + 1 == preconfirmed.header.block_number,
                "Replacing chain tip from confirmed to preconfirmed requires the new block_n to be one plus the previous one. [current_tip={current_tip:?}, new_tip={new_tip:?}]"
            ),
            // New confirmed block is added on top of a confirmed block.
            (ChainTip::Confirmed(block_n), ChainTip::Confirmed(new_block_n)) => ensure!(
                block_n + 1 == *new_block_n,
                "Replacing chain tip from confirmed to confirmed requires the new block_n to be one plus the previous one. [current_tip={current_tip:?}, new_tip={new_tip:?}]"
            ),
        }

        let current_tip_in_db = if self.inner.config.save_preconfirmed {
            &current_tip
        } else {
            // Remove the pre-confirmed case: we save the parent confirmed in that case.
            &ChainTip::on_confirmed_block_n_or_empty(current_tip.latest_confirmed_block_n())
        };

        let new_tip_in_db = if self.inner.config.save_preconfirmed {
            &new_tip
        } else {
            // Remove the pre-confirmed case: we save the parent confirmed in that case.
            // Log when we're dropping a preconfirmed block
            if let ChainTip::Preconfirmed(preconfirmed_block) = &new_tip {
                tracing::info!(
                    "⚠️  Preconfirmed block #{} NOT saved to database. Block will be lost on restart.",
                    preconfirmed_block.header.block_number
                );
            }
            &ChainTip::on_confirmed_block_n_or_empty(new_tip.latest_confirmed_block_n())
        };
        // Write to db if needed.
        if current_tip_in_db != new_tip_in_db {
            self.inner.db.replace_chain_tip(&new_tip_in_db.to_storage())?;
        }

        // Write to the backend. This also sends the notification to subscribers :)
        self.inner.chain_tip.send_replace(new_tip);

        Ok(())
    }

    /// Append transactions to the current preconfirmed block. Returns an error if there is no preconfirmed block.
    /// Replaces all candidate transactions with the content of `replace_candidates`.
    pub fn append_to_preconfirmed(
        &self,
        executed: &[PreconfirmedExecutedTransaction],
        replace_candidates: impl IntoIterator<Item = Arc<ValidatedTransaction>>,
    ) -> Result<()> {
        let block = self.inner.preconfirmed_block().context("There is no current preconfirmed block")?;

        if self.inner.config.save_preconfirmed {
            let start_tx_index = block.content.borrow().n_executed();
            // We don't save candidate transactions.
            self.inner.db.append_preconfirmed_content(start_tx_index as u64, executed)?;
        }

        block.append(executed.iter().cloned(), replace_candidates);

        Ok(())
    }

    /// Returns an error if there is no preconfirmed block. Returns the block hash for the closed block.
    pub fn close_preconfirmed(
        &self,
        pre_v0_13_2_hash_override: bool,
        state_diff: Option<StateDiff>,
    ) -> Result<AddFullBlockResult> {
        let (mut block, classes) = self
            .inner
            .block_view_on_preconfirmed()
            .context("There is no current preconfirmed block")?
            .get_full_block_with_classes()?;

        if let Some(mut state_diff) = state_diff {
            state_diff.old_declared_contracts =
                std::mem::replace(&mut block.state_diff.old_declared_contracts, state_diff.old_declared_contracts);
            block.state_diff = state_diff;
        }

        // Write the block & apply to global trie

        let result = self.write_new_confirmed_inner(&block, &classes, pre_v0_13_2_hash_override)?;

        self.new_confirmed_block(block.header.block_number)?;

        Ok(result)
    }

    /// Clears the current preconfirmed block. Does nothing when the backend has no preconfirmed block.
    pub fn clear_preconfirmed(&self) -> Result<()> {
        self.replace_chain_tip(ChainTip::on_confirmed_block_n_or_empty(self.inner.latest_confirmed_block_n()))
    }

    /// Start a new preconfirmed block on top of the latest confirmed block. Deletes and replaces the current preconfirmed block if present.
    /// Warning: Caller is responsible for ensuring the block_number is the one following the current confirmed block.
    pub fn new_preconfirmed(&self, block: PreconfirmedBlock) -> Result<()> {
        self.replace_chain_tip(ChainTip::Preconfirmed(Arc::new(block)))
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
        let result = self.write_new_confirmed_inner(block, classes, pre_v0_13_2_hash_override)?;

        self.new_confirmed_block(block_n)?;
        Ok(result)
    }

    /// Does not change the chain tip. Performs merkelization (global tries update) and block hash computation, and saves
    /// all the block parts. Returns the block hash.
    fn write_new_confirmed_inner(
        &self,
        block: &FullBlockWithoutCommitments,
        classes: &[ConvertedClass],
        pre_v0_13_2_hash_override: bool,
    ) -> Result<AddFullBlockResult> {
        let parent_block_hash = if let Some(last_block) = self.inner.block_view_on_last_confirmed() {
            last_block.get_block_info()?.block_hash
        } else {
            Felt::ZERO // genesis
        };

        let commitments = BlockCommitments::compute(
            &CommitmentComputationContext {
                protocol_version: self.inner.chain_config.latest_protocol_version,
                chain_id: self.inner.chain_config.chain_id.to_felt(),
            },
            &block.transactions,
            &block.state_diff,
            &block.events,
        );

        // Global state root and block hash.
        let global_state_root = self.apply_to_global_trie(block.header.block_number, [&block.state_diff])?;

        let header =
            block.header.clone().into_confirmed_header(parent_block_hash, commitments.clone(), global_state_root);
        let block_hash = header.compute_hash(self.inner.chain_config.chain_id.to_felt(), pre_v0_13_2_hash_override);

        tracing::info!("🙇 Block hash {:?} computed for #{}", block_hash, block.header.block_number);

        if let Some(header) = self.inner.get_custom_header_with_clear(true) {
            let is_valid = header.is_block_hash_as_expected(&block_hash);
            if !is_valid {
                tracing::warn!("Block hash not as expected for {}", block.header.block_number);
            }
        }

        // Save the block.

        self.write_header(BlockHeaderWithSignatures { header, block_hash, consensus_signatures: vec![] })?;
        self.write_transactions(block.header.block_number, &block.transactions)?;
        self.write_state_diff(block.header.block_number, &block.state_diff)?;
        self.write_events(block.header.block_number, &block.events)?;
        self.write_classes(block.header.block_number, classes)?;

        Ok(AddFullBlockResult { new_state_root: global_state_root, commitments, block_hash, parent_block_hash })
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
    ) -> Result<Felt> {
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
        self.replace_chain_tip(ChainTip::Confirmed(block_number))?;

        Ok(())
    }

    // /// Returns the total storage size
    // pub fn update_metrics(&self) -> u64 {
    //     self.db_metrics.update(&self.db)
    // }
}

// Delegate these db reads/writes. These are related to specific services, and are not specific to a block view / the chain tip writer handle.
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
    pub fn get_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64) -> Result<Option<Felt>> {
        self.db.get_l1_handler_txn_hash_by_nonce(core_contract_nonce)
    }
    pub fn get_saved_mempool_transactions(&self) -> impl Iterator<Item = Result<ValidatedTransaction>> + '_ {
        self.db.get_mempool_transactions()
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
// Delegate these db reads/writes. These are related to specific services, and are not specific to a block view / the chain tip writer handle.
impl<D: MadaraStorageWrite> MadaraBackend<D> {
    pub fn write_l1_messaging_sync_tip(&self, l1_block_n: Option<u64>) -> Result<()> {
        self.db.write_l1_messaging_sync_tip(l1_block_n)
    }
    pub fn write_l1_handler_txn_hash_by_nonce(&self, core_contract_nonce: u64, txn_hash: &Felt) -> Result<()> {
        self.db.write_l1_handler_txn_hash_by_nonce(core_contract_nonce, txn_hash)
    }
    pub fn write_pending_message_to_l2(&self, msg: &L1HandlerTransactionWithFee) -> Result<()> {
        self.db.write_pending_message_to_l2(msg)
    }
    pub fn remove_pending_message_to_l2(&self, core_contract_nonce: u64) -> Result<()> {
        self.db.remove_pending_message_to_l2(core_contract_nonce)
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
