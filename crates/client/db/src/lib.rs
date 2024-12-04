//! Madara database

use anyhow::Context;
use block_db::get_latest_block_n;
use bonsai_db::{BonsaiDb, DatabaseKeyMapping};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use db_metrics::DbMetrics;
use mp_chain_config::ChainConfig;
use mp_utils::service::{MadaraService, Service};
use rocksdb::backup::{BackupEngine, BackupEngineOptions};
use rocksdb::{BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, Env, FlushOptions, MultiThreaded};
use rocksdb_options::rocksdb_global_options;
use snapshots::Snapshots;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, fs};
use tokio::sync::{mpsc, oneshot};

mod error;
mod rocksdb_snapshot;
mod snapshots;

pub mod block_db;
pub mod bonsai_db;
pub mod class_db;
pub mod contract_db;
pub mod db_block_id;
pub mod db_metrics;
pub mod devnet_db;
pub mod l1_db;
mod rocksdb_options;
pub mod storage_updates;
pub mod tests;

pub use bonsai_db::GlobalTrie;
pub use bonsai_trie::{id::BasicId, MultiProof, ProofNode};
pub use error::{BonsaiStorageError, MadaraStorageError, TrieType};
pub type DB = DBWithThreadMode<MultiThreaded>;
pub use rocksdb;
pub type WriteBatchWithTransaction = rocksdb::WriteBatchWithTransaction<false>;

const DB_UPDATES_BATCH_SIZE: usize = 1024;

pub fn open_rocksdb(path: &Path) -> anyhow::Result<Arc<DB>> {
    let opts = rocksdb_global_options()?;
    tracing::debug!("opening db at {:?}", path.display());
    let db = DB::open_cf_descriptors(
        &opts,
        path,
        Column::ALL.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name(), col.rocksdb_options())),
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
        fs::create_dir_all(db_path).with_context(|| format!("creating directories {:?}", db_path))?;

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

/// Madara client database backend singleton.
#[derive(Debug)]
pub struct MadaraBackend {
    backup_handle: Option<mpsc::Sender<BackupRequest>>,
    db: Arc<DB>,
    chain_config: Arc<ChainConfig>,
    db_metrics: DbMetrics,
    snapshots: Arc<Snapshots>,
    trie_log_config: TrieLogConfig,
    sender_block_info: tokio::sync::broadcast::Sender<mp_block::MadaraBlockInfo>,
    #[cfg(feature = "testing")]
    _temp_dir: Option<tempfile::TempDir>,
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
    pub async fn new(
        base_path: &Path,
        backup_dir: Option<PathBuf>,
        restore_from_latest_backup: bool,
        chain_config: Arc<ChainConfig>,
        trie_log_config: TrieLogConfig,
    ) -> anyhow::Result<Self> {
        tracing::info!("💾 Opening database at: {}", base_path.display());

        let handle = MadaraBackend::open(
            base_path.to_owned(),
            backup_dir.clone(),
            restore_from_latest_backup,
            chain_config,
            trie_log_config,
        )
        .await?;

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

impl Service for DatabaseService {
    fn id(&self) -> MadaraService {
        MadaraService::Database
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

impl MadaraBackend {
    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }

    #[cfg(feature = "testing")]
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<MadaraBackend> {
        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let db = open_rocksdb(temp_dir.as_ref()).unwrap();
        let snapshots = Arc::new(Snapshots::new(Arc::clone(&db), None, Some(0), 5));
        Arc::new(Self {
            backup_handle: None,
            db,
            chain_config,
            db_metrics: DbMetrics::register().unwrap(),
            snapshots,
            trie_log_config: Default::default(),
            sender_block_info: tokio::sync::broadcast::channel(100).0,
            _temp_dir: Some(temp_dir),
        })
    }

    /// Open the db.
    pub async fn open(
        db_config_dir: PathBuf,
        backup_dir: Option<PathBuf>,
        restore_from_latest_backup: bool,
        chain_config: Arc<ChainConfig>,
        trie_log_config: TrieLogConfig,
    ) -> anyhow::Result<Arc<MadaraBackend>> {
        let db_path = db_config_dir.join("db");

        // when backups are enabled, a thread is spawned that owns the rocksdb BackupEngine (it is not thread safe) and it receives backup requests using a mpsc channel
        // There is also another oneshot channel involved: when restoring the db at startup, we want to wait for the backupengine to finish restoration before returning from open()
        let backup_handle = if let Some(backup_dir) = backup_dir {
            let (restored_cb_sender, restored_cb_recv) = oneshot::channel();

            let (sender, receiver) = mpsc::channel(1);
            let db_path = db_path.clone();
            std::thread::spawn(move || {
                spawn_backup_db_task(&backup_dir, restore_from_latest_backup, &db_path, restored_cb_sender, receiver)
                    .expect("Database backup thread")
            });

            tracing::debug!("blocking on db restoration");
            restored_cb_recv.await.context("Restoring database")?;
            tracing::debug!("done blocking on db restoration");

            Some(sender)
        } else {
            None
        };

        let db = open_rocksdb(&db_path)?;
        let current_block_n = get_latest_block_n(&db).context("Getting latest block_n from database")?;
        let snapshots = Arc::new(Snapshots::new(
            Arc::clone(&db),
            current_block_n,
            Some(trie_log_config.max_kept_snapshots),
            trie_log_config.snapshot_interval,
        ));

        let backend = Arc::new(Self {
            db_metrics: DbMetrics::register().context("Registering db metrics")?,
            backup_handle,
            db,
            chain_config: Arc::clone(&chain_config),
            snapshots,
            trie_log_config,
            sender_block_info: tokio::sync::broadcast::channel(100).0,
            #[cfg(feature = "testing")]
            _temp_dir: None,
        });
        backend.check_configuration()?;
        backend.update_metrics();
        Ok(backend)
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
            max_saved_trie_logs: Some(self.trie_log_config.max_saved_trie_logs),
            max_saved_snapshots: Some(self.trie_log_config.max_kept_snapshots),
            snapshot_interval: self.trie_log_config.snapshot_interval,
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
