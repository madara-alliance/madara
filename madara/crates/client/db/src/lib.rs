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
//! - Each of these block parts have a [`chain_head::BlockNStatus`] associated inside of [`MadaraBackend::head_status`],
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
use mp_block::MadaraBlockInfo;
use mp_chain_config::ChainConfig;
use mp_receipt::EventWithTransactionHash;
use mp_rpc::EmittedEvent;
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
use tokio::sync::{mpsc, oneshot};
use watch::BlockWatch;

mod chain_head;
mod db_version;
mod error;
mod events;
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
pub mod tests;
mod update_global_trie;

pub use bonsai_db::GlobalTrie;
pub use bonsai_trie::{id::BasicId, MultiProof, ProofNode};
pub use error::{BonsaiStorageError, MadaraStorageError, TrieType};
pub use rocksdb_options::RocksDBConfig;
pub use watch::{ClosedBlocksReceiver, PendingBlockReceiver};
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

    L1Messaging,
    L1MessagingNonce,

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
            L1Messaging,
            L1MessagingNonce,
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
            L1Messaging => "l1_messaging",
            L1MessagingNonce => "l1_messaging_nonce",
            PendingContractToClassHashes => "pending_contract_to_class_hashes",
            PendingContractToNonces => "pending_contract_to_nonces",
            PendingContractStorage => "pending_contract_storage",
            Devnet => "devnet",
            MempoolTransactions => "mempool_transactions",
        }
    }
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

pub struct MadaraBackend {
    backup_handle: Option<mpsc::Sender<BackupRequest>>,
    db: Arc<DB>,
    chain_config: Arc<ChainConfig>,
    db_metrics: DbMetrics,
    snapshots: Arc<Snapshots>,
    head_status: ChainHead,
    events_watch: EventChannels,
    watch: BlockWatch,
    /// WriteOptions with wal disabled
    writeopts_no_wal: WriteOptions,
    config: MadaraBackendConfig,
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
    #[cfg(any(test, feature = "testing"))]
    pub temp_dir: Option<tempfile::TempDir>,
    #[cfg(not(any(test, feature = "testing")))]
    pub temp_dir: Option<()>,
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
            temp_dir: None,
        }
    }
    #[cfg(any(test, feature = "testing"))]
    pub fn new_temp_dir(temp_dir: tempfile::TempDir) -> Self {
        let config = Self::new(temp_dir.as_ref());
        Self { temp_dir: Some(temp_dir), ..config }
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
            ChainHead::load_from_db(&db).context("Getting latest block_n from database")?.global_trie.get(),
            Some(config.trie_log.max_kept_snapshots),
            config.trie_log.snapshot_interval,
        ));
        let backend = Self {
            writeopts_no_wal: make_write_opt_no_wal(),
            db_metrics: DbMetrics::register().context("Registering db metrics")?,
            backup_handle,
            db,
            chain_config,
            events_watch: EventChannels::new(100),
            config,
            head_status: ChainHead::default(),
            snapshots,
            watch: BlockWatch::new(),
        };
        backend.watch.init_initial_values(&backend).context("Initializing watch channels initial values")?;
        Ok(backend)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<MadaraBackend> {
        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let config = MadaraBackendConfig::new_temp_dir(temp_dir);
        let db = open_rocksdb(config.temp_dir.as_ref().unwrap().path(), &config.rocksdb).unwrap();
        Arc::new(Self::new(None, db, chain_config, config).unwrap())
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
        Ok(Arc::new(backend))
    }

    /// This function needs to be called by the downstream block importer consumer service to mark a
    /// new block as fully imported. See the [module documentation](self) to get details on what this exactly means.
    pub async fn on_full_block(
        &self,
        block_info: Arc<MadaraBlockInfo>,
        events: impl IntoIterator<Item = EventWithTransactionHash>,
    ) -> anyhow::Result<()> {
        let block_n = block_info.header.block_number;
        self.head_status.set_to_height(Some(block_n));
        self.snapshots.set_new_head(db_block_id::DbBlockId::Number(block_n));

        for event in events {
            if let Err(e) = self.events_watch.publish(EmittedEvent {
                event: event.event.into(),
                block_hash: Some(block_info.block_hash),
                block_number: Some(block_info.header.block_number),
                transaction_hash: event.transaction_hash,
            }) {
                tracing::debug!("Failed to send event to subscribers: {e}");
            }
        }
        self.watch.on_new_block(block_info);

        self.save_head_status_to_db()?;

        if self
            .config
            .flush_every_n_blocks
            .is_some_and(|every_n_blocks| every_n_blocks != 0 && block_n % every_n_blocks == 0)
        {
            self.flush().context("Flushing database")?;
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
}

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = b"0xcontract";
    pub const CLASS: &[u8] = b"0xclass";
}
