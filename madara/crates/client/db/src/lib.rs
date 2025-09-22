//! Madara database
//!
//! # Block storage
//!
//! Storing new blocks is the responsibility of the consumers of this crate. In the madara
//! node architecture, this means: the sync service mc-sync (when we are syncing new blocks), or
//! the block production mc-block-production task when we are producing blocks.
//! For the sake of the mc-db documentation, we will call this service the "block importer" downstream service.
//!
//! The block importer service has two ways of adding blocks to the database, namely:
//! - the easy [`MadaraBackend::add_full_block_with_classes`] function, which takes a full block, and does everything
//!   required to save it properly and increment the latest block number of the database.
//! - or, the more complicated lower level API that allows you to store partial blocks.
//!
//! Note that the validity of the block being stored is not checked for neither of those APIs.
//!
//! For the low-level API, there are a few responsibilities to follow:
//!
//! - The database can store partial blocks. Adding headers can be done using [`MadaraBackend::store_block_header`],
//!   transactions and receipts using [`MadaraBackend::store_transactions`], classes using [`MadaraBackend::class_db_store_block`],
//!   state diffs using [`MadaraBackend::store_state_diff`], events using [`MadaraBackend::store_events`]. Furthermore,
//!   [`MadaraBackend::apply_to_global_trie`] also needs to be called.
//! - Each of those functions can be called in parallel, however, [`MadaraBackend::apply_to_global_trie`] needs to be called
//!   sequentially. This is because we cannot support updating the global trie in an inter-block parallelism fashion. However,
//!   parallelism is still used inside of that function - intra-block parallelism.
//! - Each of these block parts has a [`chain_head::BlockNStatus`] associated inside of [`MadaraBackend::head_status`],
//!   which the block importer service can use however it wants. However, [`ChainHead::full_block`] is special,
//!   as it is updated by this crate.
//! - The block importer service needs to call [`MadaraBackend::on_block`] to mark a block as fully imported. This function
//!   will increment the [`ChainHead::full_block`] field, marking a new block. It will also record some metrics, flush the
//!   database if needed, and make may create db backups if the backend is configured to do so.
//!
//! In addition, readers of the database should use [`db_block_id::DbBlockId`] when querying blocks from the database.
//! This ensures that any partial block data beyond the current [`ChainHead::full_block`] will not be visible to, eg. the rpc
//! service. The block importer service can however bypass this restriction by using [`db_block_id::RawDbBlockId`] instead;
//! allowing it to see the partial data it has saved beyond the latest block marked as full.

use anyhow::Context;
use bonsai_db::{BonsaiDb, DatabaseKeyMapping};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use chain_head::ChainHead;
use db_metrics::DbMetrics;
use events::EventChannels;
use mp_block::EventWithInfo;
use mp_block::L1GasQuote;
use mp_block::MadaraBlockInfo;
use mp_chain_config::ChainConfig;
use mp_convert::Felt;
use mp_receipt::EventWithTransactionHash;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId};
use rocksdb::backup::{BackupEngine, BackupEngineOptions};
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, Env, FlushOptions, MultiThreaded, WriteOptions,
};
use rocksdb_options::rocksdb_global_options;
use snapshots::Snapshots;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, fs};
use tokio::sync::{mpsc, oneshot, RwLock};
use watch::BlockWatch;

mod chain_head;
mod db_version;
mod error;
mod events;
mod events_bloom_filter;
mod rocksdb_options;
mod rocksdb_snapshot;
mod snapshots;
mod watch;

pub mod block_db;
pub mod bonsai_db;
pub mod class_db;
pub mod contract_db;
pub mod db_block_id;
pub mod db_metrics;
pub mod devnet_db;
pub mod l1_db;
pub mod mempool_db;
pub mod storage_updates;
pub mod stream;
#[cfg(any(test, feature = "testing"))]
pub mod tests;
mod update_global_trie;

pub use bonsai_db::GlobalTrie;
pub use bonsai_trie::{id::BasicId, MultiProof, ProofNode};
pub use error::{BonsaiStorageError, MadaraStorageError, TrieType};
pub use rocksdb_options::{RocksDBConfig, StatsLevel};
pub use watch::{ClosedBlocksReceiver, LastBlockOnL1Receiver, PendingBlockReceiver, PendingTxsReceiver};
pub type DB = DBWithThreadMode<MultiThreaded>;
pub use rocksdb;
pub type WriteBatchWithTransaction = rocksdb::WriteBatchWithTransaction<false>;

const DB_UPDATES_BATCH_SIZE: usize = 1024;

fn open_rocksdb(path: &Path, config: &RocksDBConfig) -> anyhow::Result<Arc<DB>> {
    let opts = rocksdb_global_options(config)?;
    tracing::debug!("opening db at {:?}", path.display());
    let db = DB::open_cf_descriptors(
        &opts,
        path,
        Column::ALL.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name(), col.rocksdb_options(config))),
    )?;

    Ok(Arc::new(db))
}

/// This runs in another thread as the backup engine is not thread safe
fn spawn_backup_db_task(
    backup_dir: &Path,
    restore_from_latest_backup: bool,
    db_path: &Path,
    db_restored_cb: oneshot::Sender<()>,
    mut recv: mpsc::Receiver<BackupRequest>,
) -> anyhow::Result<()> {
    let mut backup_opts = BackupEngineOptions::new(backup_dir).context("Creating backup options")?;
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    backup_opts.set_max_background_operations(cores);

    let mut engine = BackupEngine::open(&backup_opts, &Env::new().context("Creating rocksdb env")?)
        .context("Opening backup engine")?;

    if restore_from_latest_backup {
        tracing::info!("⏳ Restoring latest backup...");
        tracing::debug!("restore path is {db_path:?}");
        fs::create_dir_all(db_path).with_context(|| format!("Creating parent directories {:?}", db_path))?;

        let opts = rocksdb::backup::RestoreOptions::default();
        engine.restore_from_latest_backup(db_path, db_path, &opts).context("Restoring database")?;
        tracing::debug!("restoring latest backup done");
    }

    db_restored_cb.send(()).ok().context("Receiver dropped")?;

    while let Some(BackupRequest { callback, db }) = recv.blocking_recv() {
        engine.create_new_backup_flush(&db, true).context("Creating rocksdb backup")?;
        let _ = callback.send(());
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Column {
    // Blocks storage
    // block_n => Block info
    BlockNToBlockInfo,
    // block_n => Block inner
    BlockNToBlockInner,
    /// Many To One
    TxHashToBlockN,
    /// One To One
    BlockHashToBlockN,
    /// One To One
    BlockNToStateDiff,
    /// block_n => bloom filter for events
    EventBloom,
    /// Meta column for block storage (sync tip, pending block)
    BlockStorageMeta,

    /// Contract class hash to class data
    ClassInfo,
    ClassCompiled,
    PendingClassInfo,
    PendingClassCompiled,

    // History of contract class hashes
    // contract_address history block_number => class_hash
    ContractToClassHashes,

    // History of contract nonces
    // contract_address history block_number => nonce
    ContractToNonces,

    // Pending columns for contract db
    PendingContractToClassHashes,
    PendingContractToNonces,
    PendingContractStorage,

    // History of contract key => values
    // (contract_address, storage_key) history block_number => felt
    ContractStorage,

    // Each bonsai storage has 3 columns
    BonsaiContractsTrie,
    BonsaiContractsFlat,
    BonsaiContractsLog,

    BonsaiContractsStorageTrie,
    BonsaiContractsStorageFlat,
    BonsaiContractsStorageLog,

    BonsaiClassesTrie,
    BonsaiClassesFlat,
    BonsaiClassesLog,

    CoreContractNonceToTxnHash,
    // List of pending l1 to l2 messages to handle.
    CoreContractNonceToPendingMsg,

    /// Devnet: stores the private keys for the devnet predeployed contracts
    Devnet,

    MempoolTransactions,
}

impl fmt::Debug for Column {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rocksdb_name())
    }
}

impl fmt::Display for Column {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.rocksdb_name())
    }
}

impl Column {
    pub const ALL: &'static [Self] = {
        use Column::*;
        &[
            BlockNToBlockInfo,
            BlockNToBlockInner,
            TxHashToBlockN,
            BlockHashToBlockN,
            BlockStorageMeta,
            BlockNToStateDiff,
            EventBloom,
            ClassInfo,
            ClassCompiled,
            PendingClassInfo,
            PendingClassCompiled,
            ContractToClassHashes,
            ContractToNonces,
            ContractStorage,
            BonsaiContractsTrie,
            BonsaiContractsFlat,
            BonsaiContractsLog,
            BonsaiContractsStorageTrie,
            BonsaiContractsStorageFlat,
            BonsaiContractsStorageLog,
            BonsaiClassesTrie,
            BonsaiClassesFlat,
            BonsaiClassesLog,
            CoreContractNonceToTxnHash,
            CoreContractNonceToPendingMsg,
            PendingContractToClassHashes,
            PendingContractToNonces,
            PendingContractStorage,
            Devnet,
            MempoolTransactions,
        ]
    };
    pub const NUM_COLUMNS: usize = Self::ALL.len();

    pub(crate) fn rocksdb_name(&self) -> &'static str {
        use Column::*;
        match self {
            BlockNToBlockInfo => "block_n_to_block_info",
            BlockNToBlockInner => "block_n_to_block_inner",
            TxHashToBlockN => "tx_hash_to_block_n",
            BlockHashToBlockN => "block_hash_to_block_n",
            BlockStorageMeta => "block_storage_meta",
            BlockNToStateDiff => "block_n_to_state_diff",
            EventBloom => "event_bloom",
            BonsaiContractsTrie => "bonsai_contracts_trie",
            BonsaiContractsFlat => "bonsai_contracts_flat",
            BonsaiContractsLog => "bonsai_contracts_log",
            BonsaiContractsStorageTrie => "bonsai_contracts_storage_trie",
            BonsaiContractsStorageFlat => "bonsai_contracts_storage_flat",
            BonsaiContractsStorageLog => "bonsai_contracts_storage_log",
            BonsaiClassesTrie => "bonsai_classes_trie",
            BonsaiClassesFlat => "bonsai_classes_flat",
            BonsaiClassesLog => "bonsai_classes_log",
            ClassInfo => "class_info",
            ClassCompiled => "class_compiled",
            PendingClassInfo => "pending_class_info",
            PendingClassCompiled => "pending_class_compiled",
            ContractToClassHashes => "contract_to_class_hashes",
            ContractToNonces => "contract_to_nonces",
            ContractStorage => "contract_storage",
            CoreContractNonceToTxnHash => "core_contract_nonce_to_txn_hash",
            CoreContractNonceToPendingMsg => "core_contract_nonce_to_pending_msg",
            PendingContractToClassHashes => "pending_contract_to_class_hashes",
            PendingContractToNonces => "pending_contract_to_nonces",
            PendingContractStorage => "pending_contract_storage",
            Devnet => "devnet",
            MempoolTransactions => "mempool_transactions",
        }
    }
}

#[cfg(test)]
#[test]
fn test_column_all() {
    assert_eq!(Column::ALL.len(), Column::NUM_COLUMNS);
}

pub trait DatabaseExt {
    fn get_column(&self, col: Column) -> Arc<BoundColumnFamily<'_>>;
}

impl DatabaseExt for DB {
    fn get_column(&self, col: Column) -> Arc<BoundColumnFamily<'_>> {
        let name = col.rocksdb_name();
        match self.cf_handle(name) {
            Some(column) => column,
            None => panic!("column {name} not initialized"),
        }
    }
}

fn make_write_opt_no_wal() -> WriteOptions {
    let mut opts = WriteOptions::new();
    opts.disable_wal(true);
    opts
}

#[derive(Debug)]
pub struct TrieLogConfig {
    pub max_saved_trie_logs: usize,
    pub max_kept_snapshots: usize,
    pub snapshot_interval: u64,
}

impl Default for TrieLogConfig {
    fn default() -> Self {
        Self { max_saved_trie_logs: 0, max_kept_snapshots: 0, snapshot_interval: 5 }
    }
}

#[derive(Default, Clone)]
pub enum SyncStatus {
    #[default]
    NotRunning,
    Running {
        highest_block_n: u64,
        highest_block_hash: Felt,
    },
}

#[derive(Default)]
pub(crate) struct SyncStatusCell(RwLock<SyncStatus>);
impl SyncStatusCell {
    pub(crate) async fn set(&self, sync_status: SyncStatus) {
        let mut status = self.0.write().await;
        *status = sync_status;
    }
    pub(crate) async fn get(&self) -> SyncStatus {
        self.0.read().await.clone()
    }
}

/// Madara client database backend singleton.
pub struct MadaraBackend {
    backup_handle: Option<mpsc::Sender<BackupRequest>>,
    db: Arc<DB>,
    chain_config: Arc<ChainConfig>,
    db_metrics: DbMetrics,
    snapshots: Arc<Snapshots>,
    head_status: ChainHead,
    watch_events: EventChannels,
    watch_blocks: BlockWatch,
    watch_gas_quote: tokio::sync::watch::Sender<Option<L1GasQuote>>,
    /// WriteOptions with wal disabled
    writeopts_no_wal: WriteOptions,
    config: MadaraBackendConfig,
    // keep the TempDir instance around so that the directory is not deleted until the MadaraBackend struct is dropped.
    #[cfg(any(test, feature = "testing"))]
    _temp_dir: Option<tempfile::TempDir>,
    sync_status: SyncStatusCell,
    starting_block: Option<u64>,
}

impl fmt::Debug for MadaraBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("MadaraBackend");
        s.field("backup_handle", &self.backup_handle)
            .field("db", &self.db)
            .field("chain_config", &self.chain_config)
            .field("db_metrics", &self.db_metrics)
            .field("config", &self.config)
            .finish()
    }
}

pub struct DatabaseService {
    handle: Arc<MadaraBackend>,
}

impl DatabaseService {
    /// Create a new database service.
    ///
    /// # Arguments
    ///
    /// * `base_path` - The path to the database directory.
    /// * `backup_dir` - Optional path to the backup directory.
    /// * `restore_from_latest_backup` - Whether to restore the database from the latest backup.
    /// * `chain_config` - The chain configuration.
    ///
    /// # Returns
    ///
    /// A new database service.
    ///
    pub async fn new(chain_config: Arc<ChainConfig>, config: MadaraBackendConfig) -> anyhow::Result<Self> {
        tracing::info!("💾 Opening database at: {}", config.base_path.display());

        let handle = MadaraBackend::open(chain_config, config).await?;

        if let Some(block_n) = handle.head_status().latest_full_block_n() {
            tracing::info!("📦 Database latest block: #{block_n}");
        }

        Ok(Self { handle })
    }

    pub fn backend(&self) -> &Arc<MadaraBackend> {
        &self.handle
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Self {
        Self { handle: MadaraBackend::open_for_testing(chain_config) }
    }
}

impl Service for DatabaseService {}

impl ServiceId for DatabaseService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::Database.svc_id()
    }
}

struct BackupRequest {
    callback: oneshot::Sender<()>,
    db: Arc<DB>,
}

impl Drop for MadaraBackend {
    fn drop(&mut self) {
        tracing::info!("⏳ Gracefully closing the database...");
        self.flush().expect("Error when flushing the database"); // flush :)
        self.db.cancel_all_background_work(true);
    }
}

#[derive(Debug)]
pub struct MadaraBackendConfig {
    pub base_path: PathBuf,
    pub backup_dir: Option<PathBuf>,
    pub restore_from_latest_backup: bool,
    pub trie_log: TrieLogConfig,
    pub backup_every_n_blocks: Option<u64>,
    pub flush_every_n_blocks: Option<u64>,
    pub rocksdb: RocksDBConfig,
}

impl MadaraBackendConfig {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            base_path: base_path.as_ref().to_path_buf(),
            backup_dir: None,
            restore_from_latest_backup: false,
            trie_log: Default::default(),
            backup_every_n_blocks: None,
            flush_every_n_blocks: None,
            rocksdb: Default::default(),
        }
    }
    pub fn backup_dir(self, backup_dir: Option<PathBuf>) -> Self {
        Self { backup_dir, ..self }
    }
    pub fn restore_from_latest_backup(self, restore_from_latest_backup: bool) -> Self {
        Self { restore_from_latest_backup, ..self }
    }
    pub fn backup_every_n_blocks(self, backup_every_n_blocks: Option<u64>) -> Self {
        Self { backup_every_n_blocks, ..self }
    }
    pub fn flush_every_n_blocks(self, flush_every_n_blocks: Option<u64>) -> Self {
        Self { flush_every_n_blocks, ..self }
    }
    pub fn trie_log(self, trie_log: TrieLogConfig) -> Self {
        Self { trie_log, ..self }
    }
}

impl MadaraBackend {
    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }

    fn new(
        backup_handle: Option<mpsc::Sender<BackupRequest>>,
        db: Arc<DB>,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
    ) -> anyhow::Result<Self> {
        let snapshots = Arc::new(Snapshots::new(
            Arc::clone(&db),
            ChainHead::load_from_db(&db).context("Getting latest block_n from database")?.global_trie.current(),
            Some(config.trie_log.max_kept_snapshots),
            config.trie_log.snapshot_interval,
        ));
        let backend = Self {
            writeopts_no_wal: make_write_opt_no_wal(),
            db_metrics: DbMetrics::register().context("Registering db metrics")?,
            backup_handle,
            db,
            chain_config,
            watch_events: EventChannels::new(100),
            config,
            starting_block: None,
            sync_status: SyncStatusCell::default(),
            head_status: ChainHead::default(),
            snapshots,
            watch_blocks: BlockWatch::new(),
            watch_gas_quote: tokio::sync::watch::channel(None).0,
            #[cfg(any(test, feature = "testing"))]
            _temp_dir: None,
        };
        backend.watch_blocks.init_initial_values(&backend).context("Initializing watch channels initial values")?;
        Ok(backend)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<MadaraBackend> {
        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let config = MadaraBackendConfig::new(&temp_dir);
        let db = open_rocksdb(temp_dir.as_ref(), &config.rocksdb).unwrap();
        let mut backend = Self::new(None, db, chain_config, config).unwrap();
        backend._temp_dir = Some(temp_dir);
        Arc::new(backend)
    }

    /// Open the db.
    pub async fn open(
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
    ) -> anyhow::Result<Arc<MadaraBackend>> {
        // check if the db version is compatible with the current binary
        tracing::debug!("checking db version");
        if let Some(db_version) =
            db_version::check_db_version(&config.base_path).context("Checking database version")?
        {
            tracing::debug!("version of existing db is {db_version}");
        }

        let db_path = config.base_path.join("db");

        // when backups are enabled, a thread is spawned that owns the rocksdb BackupEngine (it is not thread safe) and it receives backup requests using a mpsc channel
        // There is also another oneshot channel involved: when restoring the db at startup, we want to wait for the backupengine to finish restoration before returning from open()
        let backup_handle = if let Some(backup_dir) = config.backup_dir.clone() {
            let (restored_cb_sender, restored_cb_recv) = oneshot::channel();

            let (sender, receiver) = mpsc::channel(1);
            let db_path = db_path.clone();
            std::thread::spawn(move || {
                spawn_backup_db_task(
                    &backup_dir,
                    config.restore_from_latest_backup,
                    &db_path,
                    restored_cb_sender,
                    receiver,
                )
                .expect("Database backup thread")
            });

            tracing::debug!("blocking on db restoration");
            restored_cb_recv.await.context("Restoring database")?;
            tracing::debug!("done blocking on db restoration");

            Some(sender)
        } else {
            None
        };

        let db = open_rocksdb(&db_path, &config.rocksdb)?;

        let mut backend = Self::new(backup_handle, db, chain_config, config)?;
        backend.check_configuration()?;
        backend.load_head_status_from_db()?;
        backend.update_metrics();
        backend.set_starting_block(backend.head_status.latest_full_block_n());
        Ok(Arc::new(backend))
    }

    /// This function needs to be called by the downstream block importer consumer service to mark a
    /// new block as fully imported. See the [module documentation](self) to get details on what this exactly means.
    pub async fn on_full_block_imported(
        &self,
        block_info: Arc<MadaraBlockInfo>,
        events: impl IntoIterator<Item = EventWithTransactionHash>,
    ) -> anyhow::Result<()> {
        let block_n = block_info.header.block_number;
        self.head_status.set_latest_full_block_n(Some(block_n));
        self.snapshots.set_new_head(db_block_id::DbBlockId::Number(block_n));

        for (index, event) in events.into_iter().enumerate() {
            if let Err(e) = self.watch_events.publish(EventWithInfo {
                event: event.event,
                block_hash: Some(block_info.block_hash),
                block_number: Some(block_info.header.block_number),
                transaction_hash: event.transaction_hash,
                event_index_in_block: index,
            }) {
                tracing::debug!("Failed to send event to subscribers: {e}");
            }
        }
        self.watch_blocks.on_new_block(block_info);

        self.save_head_status_to_db()?;

        // Always flush after saving head status to ensure it's persisted
        // This is critical for restart scenarios where we need to know the last synced block
        self.flush().context("Flushing database after head status update")?;

        // Also flush based on the configured interval if set
        if self
            .config
            .flush_every_n_blocks
            .is_some_and(|every_n_blocks| every_n_blocks != 0 && block_n % every_n_blocks == 0)
        {
            self.flush().context("Periodic database flush")?;
        }

        if self
            .config
            .backup_every_n_blocks
            .is_some_and(|every_n_blocks| every_n_blocks != 0 && block_n % every_n_blocks == 0)
        {
            self.backup().await.context("Making DB backup")?;
        }
        Ok(())
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        tracing::debug!("doing a db flush");
        let mut opts = FlushOptions::default();
        opts.set_wait(true);
        // we have to collect twice here :/
        let columns = Column::ALL.iter().map(|e| self.db.get_column(*e)).collect::<Vec<_>>();
        let columns = columns.iter().collect::<Vec<_>>();

        self.db.flush_cfs_opt(&columns, &opts).context("Flushing database")?;

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub async fn backup(&self) -> anyhow::Result<()> {
        let (callback_sender, callback_recv) = oneshot::channel();
        let _res = self
            .backup_handle
            .as_ref()
            .context("backups are not enabled")?
            .try_send(BackupRequest { callback: callback_sender, db: Arc::clone(&self.db) });
        callback_recv.await.context("Backups task died :(")?;
        Ok(())
    }

    // tries

    pub(crate) fn get_bonsai<H: StarkHash + Send + Sync>(
        &self,
        map: DatabaseKeyMapping,
    ) -> BonsaiStorage<BasicId, BonsaiDb, H> {
        let config = BonsaiStorageConfig {
            max_saved_trie_logs: Some(self.config.trie_log.max_saved_trie_logs),
            max_saved_snapshots: Some(self.config.trie_log.max_kept_snapshots),
            snapshot_interval: self.config.trie_log.snapshot_interval,
        };

        BonsaiStorage::new(
            BonsaiDb::new(Arc::clone(&self.db), Arc::clone(&self.snapshots), map),
            config,
            // Every global tree has keys of 251 bits.
            251,
        )
    }

    pub fn contract_trie(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: Column::BonsaiContractsFlat,
            trie: Column::BonsaiContractsTrie,
            log: Column::BonsaiContractsLog,
        })
    }

    pub fn contract_storage_trie(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: Column::BonsaiContractsStorageFlat,
            trie: Column::BonsaiContractsStorageTrie,
            log: Column::BonsaiContractsStorageLog,
        })
    }

    pub fn class_trie(&self) -> GlobalTrie<Poseidon> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: Column::BonsaiClassesFlat,
            trie: Column::BonsaiClassesTrie,
            log: Column::BonsaiClassesLog,
        })
    }

    /// Returns the total storage size
    pub fn update_metrics(&self) -> u64 {
        self.db_metrics.update(&self.db)
    }

    /// Remove all data associated with a specific block
    fn remove_block_data(&self, block_n: u64) -> anyhow::Result<()> {
        tracing::debug!("Removing all data for block #{}", block_n);

        // Get block hash before removing anything
        let block_hash = self.get_block_hash(&db_block_id::RawDbBlockId::Number(block_n))?;

        // Remove block info
        let col = self.db.get_column(Column::BlockNToBlockInfo);
        self.db.delete_cf_opt(&col, bincode::serialize(&block_n)?, &self.writeopts_no_wal)?;

        // Remove block inner
        let col = self.db.get_column(Column::BlockNToBlockInner);
        self.db.delete_cf_opt(&col, bincode::serialize(&block_n)?, &self.writeopts_no_wal)?;

        // Remove state diff
        let col = self.db.get_column(Column::BlockNToStateDiff);
        self.db.delete_cf_opt(&col, bincode::serialize(&block_n)?, &self.writeopts_no_wal)?;

        // Remove block hash mapping
        if let Some(hash) = block_hash {
            let col = self.db.get_column(Column::BlockHashToBlockN);
            self.db.delete_cf_opt(&col, bincode::serialize(&hash)?, &self.writeopts_no_wal)?;
        }

        // Remove event bloom filter
        let col = self.db.get_column(Column::EventBloom);
        self.db.delete_cf_opt(&col, bincode::serialize(&block_n)?, &self.writeopts_no_wal)?;

        // Remove transactions and their mappings
        self.remove_block_transactions(block_n)?;

        // Remove classes
        self.remove_block_classes(block_n)?;

        // Remove events
        self.remove_block_events(block_n)?;

        Ok(())
    }

    /// Remove all transactions and their mappings for a block
    fn remove_block_transactions(&self, block_n: u64) -> anyhow::Result<()> {
        // Get the block inner to find all transactions
        let col = self.db.get_column(Column::BlockNToBlockInner);
        if let Some(inner_bytes) = self.db.get_cf(&col, bincode::serialize(&block_n)?)? {
            let inner: mp_block::MadaraBlockInner = bincode::deserialize(&inner_bytes)?;

            // Remove transaction hash mappings
            // Use receipts to get transaction hashes since they have the hash field
            let tx_col = self.db.get_column(Column::TxHashToBlockN);
            for receipt in inner.receipts.iter() {
                let tx_hash = receipt.transaction_hash();
                self.db.delete_cf_opt(&tx_col, bincode::serialize(&tx_hash)?, &self.writeopts_no_wal)?;
                tracing::trace!("Removed tx hash mapping for {:#x}", tx_hash);
            }
        }

        Ok(())
    }

    /// Remove all classes associated with a block
    fn remove_block_classes(&self, block_n: u64) -> anyhow::Result<()> {
        // Get state diff to find all declared classes
        let col = self.db.get_column(Column::BlockNToStateDiff);
        if let Some(state_diff_bytes) = self.db.get_cf(&col, bincode::serialize(&block_n)?)? {
            let state_diff: mp_state_update::StateDiff = bincode::deserialize(&state_diff_bytes)?;

            // Remove class info and compiled class data
            let class_info_col = self.db.get_column(Column::ClassInfo);
            let class_compiled_col = self.db.get_column(Column::ClassCompiled);

            for declared_item in state_diff.declared_classes.iter() {
                self.db.delete_cf_opt(&class_info_col, bincode::serialize(&declared_item.class_hash)?, &self.writeopts_no_wal)?;
                self.db.delete_cf_opt(&class_compiled_col, bincode::serialize(&declared_item.compiled_class_hash)?, &self.writeopts_no_wal)?;
                tracing::trace!("Removed class {:#x}", declared_item.class_hash);
            }

            // Remove deprecated declared classes
            for class_hash in state_diff.deprecated_declared_classes.iter() {
                self.db.delete_cf_opt(&class_info_col, bincode::serialize(&class_hash)?, &self.writeopts_no_wal)?;
                self.db.delete_cf_opt(&class_compiled_col, bincode::serialize(&class_hash)?, &self.writeopts_no_wal)?;
                tracing::trace!("Removed deprecated class {:#x}", class_hash);
            }
        }

        Ok(())
    }

    /// Remove all events associated with a block
    fn remove_block_events(&self, _block_n: u64) -> anyhow::Result<()> {
        // Events are stored by block number, so just remove the entire block's events
        // This is handled by remove_block_data when removing the block inner
        // which contains the events
        Ok(())
    }

    /// Rollback the database to a specific block number
    /// This removes all blocks after the specified block number
    pub fn rollback_to_block(&self, target_block_n: u64) -> anyhow::Result<()> {
        tracing::warn!("⏮️ Rolling back database to block #{}", target_block_n);


        self.clear_pending_block()?;
        let state_diffs = self.block_db_revert(target_block_n)?;
        self.contract_db_revert(&state_diffs)?;
        self.class_db_revert(&state_diffs)?;

        // Get the current latest block from head_status
        let current_block = self.head_status.latest_full_block_n();

        tracing::info!(">>>>>> Rollback completed basanth {:?}", current_block);
        let contract_root = self.contract_trie().root_hash(bonsai_identifier::CONTRACT)
            .map_err(|e| anyhow::anyhow!("Failed to get contract root after rollback: {}", e))?;
        let class_root = self.class_trie().root_hash(bonsai_identifier::CLASS)
            .map_err(|e| anyhow::anyhow!("Failed to get class root after rollback: {}", e))?;


        tracing::info!(">>>>>> 📊 After rollback to block #{}: contract_root={:#x}, class_root={:#x}",
            target_block_n, contract_root, class_root);

        // If no blocks found in head_status, try to find the actual latest block from database
        let current_block = if let Some(block) = current_block {
            block
        } else {
            // Look for the highest block number in the database
            // We'll iterate from target_block_n upwards to find the actual latest
            let mut latest = target_block_n;
            let col = self.db.get_column(Column::BlockNToBlockInfo);

            // Check if we have any blocks at all by checking block 0 (genesis)
            if self.db.get_cf(&col, 0u64.to_be_bytes())?.is_none() {
                tracing::warn!("No blocks found in database, cannot rollback");
                return Ok(());
            }

            // Find the actual latest block by probing upwards
            loop {
                let next_block = latest + 100; // Check in increments of 100
                if self.db.get_cf(&col, next_block.to_be_bytes())?.is_some() {
                    latest = next_block;
                } else {
                    // Binary search between latest and next_block
                    let mut left = latest;
                    let mut right = next_block;
                    while left < right {
                        let mid = (left + right + 1) / 2;
                        if self.db.get_cf(&col, mid.to_be_bytes())?.is_some() {
                            left = mid;
                        } else {
                            right = mid - 1;
                        }
                    }
                    latest = left;
                    break;
                }
            }

            tracing::info!("Found actual latest block in database: {}", latest);
            latest
        };

        if target_block_n >= current_block {
            tracing::info!("Target block {} is >= current block {}, nothing to rollback", target_block_n, current_block);
            return Ok(());
        }

        // Clear all blocks after target_block_n using comprehensive removal
        for block_n in (target_block_n + 1)..=current_block {
            // Use the comprehensive removal method that cleans up EVERYTHING
            // self.remove_block_data(block_n)?;
        }

        // Revert bonsai tries to the target block (critical for state consistency)
        // This is the key addition from PR #296
        tracing::info!("🔄 Reverting bonsai tries to block #{}", target_block_n);

        let target_block_id = BasicId::new(target_block_n);

        // IMPORTANT: When reverting, we need to revert from the current block to the target block
        // If we're rolling back from block 52 to block 23, we need to revert from 52 to 23
        let revert_from_block = current_block;
        let revert_from_id = BasicId::new(revert_from_block);
        
        // Revert contract trie
        self.contract_trie()
            .revert_to(target_block_id, revert_from_id)
            .map_err(|e| anyhow::anyhow!("Failed to revert contract trie: {}", e))?;
        tracing::debug!("✓ Contract trie reverted from block #{} to block #{}", revert_from_block, target_block_n);
        
        // Revert contract storage trie  
        self.contract_storage_trie()
            .revert_to(target_block_id, revert_from_id)
            .map_err(|e| anyhow::anyhow!("Failed to revert contract storage trie: {}", e))?;
        tracing::debug!("✓ Contract storage trie reverted from block #{} to block #{}", revert_from_block, target_block_n);
        
        // Revert class trie
        self.class_trie()
            .revert_to(target_block_id, revert_from_id)
            .map_err(|e| anyhow::anyhow!("Failed to revert class trie: {}", e))?;
        tracing::debug!("✓ Class trie reverted from block #{} to block #{}", revert_from_block, target_block_n);
        
        // CRITICAL: Commit all tries after reverting to ensure consistency
        // This ensures the tries are in a clean state before applying new state diffs
        self.contract_trie().commit(target_block_id)
            .map_err(|e| anyhow::anyhow!("Failed to commit contract trie after revert: {}", e))?;
        self.contract_storage_trie().commit(target_block_id)
            .map_err(|e| anyhow::anyhow!("Failed to commit contract storage trie after revert: {}", e))?;
        self.class_trie().commit(target_block_id)
            .map_err(|e| anyhow::anyhow!("Failed to commit class trie after revert: {}", e))?;

        // tracing::debug!("✓ All tries committed at block #{}", target_block_n);

        // CRITICAL: Get the state roots at target block for verification
        let contract_root = self.contract_trie().root_hash(bonsai_identifier::CONTRACT)
            .map_err(|e| anyhow::anyhow!("Failed to get contract root after rollback: {}", e))?;
        let class_root = self.class_trie().root_hash(bonsai_identifier::CLASS)
            .map_err(|e| anyhow::anyhow!("Failed to get class root after rollback: {}", e))?;


        tracing::info!("📊 After rollback to block #{}: contract_root={:#x}, class_root={:#x}",
            target_block_n, contract_root, class_root);

        // Log that rollback is complete with tries committed
        tracing::info!("State tries rolled back and committed at block {}", target_block_n);

        // Update snapshots head to match the rollback target
        self.snapshots.set_new_head(db_block_id::DbBlockId::Number(target_block_n));
        tracing::debug!("✓ Snapshots head updated to block #{}", target_block_n);

        // Update head status - MUST update all pipeline heads including global_trie and classes
        self.head_status.set_latest_full_block_n(Some(target_block_n));
        self.head_status.headers.set_current(Some(target_block_n));
        self.head_status.state_diffs.set_current(Some(target_block_n));
        self.head_status.classes.set_current(Some(target_block_n));
        self.head_status.transactions.set_current(Some(target_block_n));
        self.head_status.events.set_current(Some(target_block_n));
        self.head_status.global_trie.set_current(Some(target_block_n));

        // Save updated head status
        self.save_head_status_to_db()?;

        // Clear pending block as it's now invalid
        self.clear_pending_block()?;

        // Flush to ensure consistency
        self.flush()?;

        tracing::info!("✅ Rollback complete. Database now at block #{}", target_block_n);
        Ok(())
    }
}

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = b"0xcontract";
    pub const CLASS: &[u8] = b"0xclass";
}
