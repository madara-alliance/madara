//! A database backend storing data about deoxys chain
//!
//! # Usefulness
//! Starknet RPC methods use Starknet block hash as arguments to access on-chain values.
//! Because the Starknet blocks are wrapped inside the Substrate ones, we have no simple way to
//! index the chain storage using this hash.
//! Rather than iterating over all the Substrate blocks in order to find the one wrapping the
//! requested Starknet one, we maintain a StarknetBlockHash to SubstrateBlock hash mapping.
//!
//! # Databases supported
//! `paritydb` and `rocksdb` are both supported, behind the `kvdb-rocksd` and `parity-db` feature
//! flags. Support for custom databases is possible but not supported yet.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::{fmt, fs};

use anyhow::{Context, Result};
use bonsai_db::{BonsaiDb, DatabaseKeyMapping};
use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use mapping_db::MappingDb;
use meta_db::MetaDb;
use rocksdb::backup::{BackupEngine, BackupEngineOptions};

mod error;
mod mapping_db;
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBCompressionType, Env, MultiThreaded, OptimisticTransactionDB, Options,
};
use starknet_types_core::hash::{Pedersen, Poseidon};
pub mod bonsai_db;
mod meta_db;
pub mod storage_handler;
pub mod storage_updates;

pub use error::{BonsaiDbError, DbError};
pub use mapping_db::MappingCommitment;
use storage_handler::bonsai_identifier;
use tokio::sync::{mpsc, oneshot};

const DB_HASH_LEN: usize = 32;
/// Hash type that this backend uses for the database.
pub type DbHash = [u8; DB_HASH_LEN];

pub type DB = OptimisticTransactionDB<MultiThreaded>;

pub(crate) fn open_rocksdb(
    path: &Path,
    create: bool,
    backup_dir: Option<PathBuf>,
    restore_from_latest_backup: bool,
) -> Result<OptimisticTransactionDB<MultiThreaded>> {
    let mut opts = Options::default();
    opts.set_report_bg_io_stats(true);
    opts.set_use_fsync(false);
    opts.create_if_missing(create);
    opts.create_missing_column_families(true);
    opts.set_bytes_per_sync(1024 * 1024);
    opts.set_keep_log_file_num(1);
    opts.set_compression_type(DBCompressionType::Zstd);
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    opts.increase_parallelism(cores);

    if let Some(backup_dir) = backup_dir {
        let (restored_cb_sender, restored_cb_recv) = std::sync::mpsc::channel();
        // we use a channel from std because we're in a tokio context and the function is async
        // TODO make the function async or somethign..

        let (sender, receiver) = mpsc::channel(1);
        let db_path = path.to_owned();
        std::thread::spawn(move || {
            spawn_backup_db_task(&backup_dir, restore_from_latest_backup, &db_path, restored_cb_sender, receiver)
                .expect("database backup thread")
        });
        DB_BACKUP_SINGLETON.set(sender).ok().context("backend already initialized")?;

        log::debug!("blocking on db restoration");
        restored_cb_recv.recv().context("restoring database")?;
        log::debug!("done blocking on db restoration");
    }

    log::debug!("opening db at {:?}", path.display());
    let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
        &opts,
        path,
        Column::ALL.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name(), col.rocksdb_options())),
    )?;

    Ok(db)
}

fn spawn_backup_db_task(
    backup_dir: &Path,
    restore_from_latest_backup: bool,
    db_path: &Path,
    db_restored_cb: std::sync::mpsc::Sender<()>,
    mut recv: mpsc::Receiver<BackupRequest>,
) -> Result<()> {
    // we use a thread to do that as backup engine is not thread safe

    let mut backup_opts = BackupEngineOptions::new(backup_dir).context("creating backup options")?;
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    backup_opts.set_max_background_operations(cores);

    let mut engine = BackupEngine::open(&backup_opts, &Env::new().context("creating rocksdb env")?)
        .context("opening backup engine")?;

    if restore_from_latest_backup {
        log::info!("⏳ Restoring latest backup...");
        log::debug!("restore path is {db_path:?}");
        fs::create_dir_all(db_path).with_context(|| format!("creating directories {:?}", db_path))?;

        let opts = rocksdb::backup::RestoreOptions::default();
        engine.restore_from_latest_backup(db_path, db_path, &opts).context("restoring database")?;
        log::debug!("restoring latest backup done");
    }

    db_restored_cb.send(()).ok().context("receiver dropped")?;
    drop(db_restored_cb);

    while let Some(BackupRequest(callback)) = recv.blocking_recv() {
        let db = DB_SINGLETON.get().context("getting rocksdb instance")?;
        engine.create_new_backup_flush(db, true).context("creating rocksdb backup")?;
        let _ = callback.send(());
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Meta,
    // Starknet block hash to Substrate block hash
    BlockMapping,
    // Substrate block hash to true if it contains a Starknet block
    SyncedMapping,
    // Transaction hash to Substrate block hash
    TransactionMapping,
    /// Starknet block hash to list of starknet transaction hashes
    StarknetTransactionHashesMapping,
    /// Block number to block Starknet block hash
    StarknetBlockHashesMapping,
    /// Starknet block hash to block number
    StarknetBlockNumberMapping,

    /// Contract class hash to class data
    ContractClassData,
    /// Contract address to class hash and nonce history
    ContractData,
    /// Class hash to compiled class hash
    ContractClassHashes,
    /// Contract address and storage key to storage value history
    ContractStorage,
    /// Block number to state diff
    BlockStateDiff,

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
}

impl Column {
    fn iter() -> impl Iterator<Item = Self> {
        Self::ALL.iter().copied()
    }
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
            Meta,
            BlockMapping,
            SyncedMapping,
            TransactionMapping,
            StarknetTransactionHashesMapping,
            StarknetBlockHashesMapping,
            StarknetBlockNumberMapping,
            ContractClassData,
            ContractData,
            ContractClassHashes,
            ContractStorage,
            BlockStateDiff,
            BonsaiContractsTrie,
            BonsaiContractsFlat,
            BonsaiContractsLog,
            BonsaiContractsStorageTrie,
            BonsaiContractsStorageFlat,
            BonsaiContractsStorageLog,
            BonsaiClassesTrie,
            BonsaiClassesFlat,
            BonsaiClassesLog,
        ]
    };
    pub const NUM_COLUMNS: usize = Self::ALL.len();

    pub(crate) fn rocksdb_name(&self) -> &'static str {
        match self {
            Column::Meta => "meta",
            Column::BlockMapping => "block_mapping",
            Column::TransactionMapping => "transaction_mapping",
            Column::SyncedMapping => "synced_mapping",
            Column::StarknetTransactionHashesMapping => "starknet_transaction_hashes_mapping",
            Column::StarknetBlockHashesMapping => "starnet_block_hashes_mapping",
            Column::StarknetBlockNumberMapping => "starknet_block_number_mapping",
            Column::BonsaiContractsTrie => "bonsai_contracts_trie",
            Column::BonsaiContractsFlat => "bonsai_contracts_flat",
            Column::BonsaiContractsLog => "bonsai_contracts_log",
            Column::BonsaiContractsStorageTrie => "bonsai_contracts_storage_trie",
            Column::BonsaiContractsStorageFlat => "bonsai_contracts_storage_flat",
            Column::BonsaiContractsStorageLog => "bonsai_contracts_storage_log",
            Column::BonsaiClassesTrie => "bonsai_classes_trie",
            Column::BonsaiClassesFlat => "bonsai_classes_flat",
            Column::BonsaiClassesLog => "bonsai_classes_log",
            Column::BlockStateDiff => "block_state_diff",
            Column::ContractClassData => "contract_class_data",
            Column::ContractData => "contract_data",
            Column::ContractClassHashes => "contract_class_hashes",
            Column::ContractStorage => "contrac_storage",
        }
    }

    /// Per column rocksdb options, like memory budget, compaction profiles, block sizes for hdd/sdd
    /// etc. TODO: add basic sensible defaults
    pub(crate) fn rocksdb_options(&self) -> Options {
        // match self {
        //     _ => Options::default(),
        // }
        Options::default()
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

/// Deoxys client database backend singleton.
///
/// New instance returned upon first creation only and should only be passed to Substrate
/// functions. Use the static functions defined below to access individual backend databases
/// instead.
///
/// * `meta`: stores data aboud the current state of the chain.
/// * `mapping`: maps Starknet blocks to Substrate blocks.
/// * `da`: store Data Availability info that needs to be written to the Ethereum L1.
/// * `messaging`: Stores Ethereum L1 messaging data.
/// * `sierra_classes`: @antyro what is this for?
/// * `bonsai_contract`: Bezu-bonsai trie used to compute the contract root.
/// * `bonsai_storage`: Bezu-bonsai trie used to compute the storage root for each contract.
/// * `bonsai_class`: Bezu-bonsai trie used to compute the class root.
pub struct DeoxysBackend {
    meta: Arc<MetaDb>,
    mapping: Arc<MappingDb>,
    bonsai_contract: RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
    bonsai_storage: RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
    bonsai_class: RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>>,
}

// Singleton backing instance for `DeoxysBackend`
static BACKEND_SINGLETON: OnceLock<Arc<DeoxysBackend>> = OnceLock::new();

static DB_SINGLETON: OnceLock<Arc<DB>> = OnceLock::new();

struct BackupRequest(oneshot::Sender<()>);
static DB_BACKUP_SINGLETON: OnceLock<mpsc::Sender<BackupRequest>> = OnceLock::new();

pub struct DBDropHook; // TODO(HACK): db really really shouldnt be in a static
impl Drop for DBDropHook {
    fn drop(&mut self) {
        let backend = BACKEND_SINGLETON.get().unwrap() as *const _ as *mut Arc<DeoxysBackend>;
        let db = DB_SINGLETON.get().unwrap() as *const _ as *mut Arc<DB>;
        // TODO(HACK): again, i can't emphasize enough how bad of a hack this is
        unsafe {
            std::ptr::drop_in_place(backend);
            std::ptr::drop_in_place(db);
        }
    }
}

impl DeoxysBackend {
    /// Initializes a local database, returning a singleton backend instance.
    ///
    /// This backend should only be used to pass to substrate functions. Use the static functions
    /// defined below to access static fields instead.
    pub fn open(
        db_config_dir: &Path,
        backup_dir: Option<PathBuf>,
        restore_from_latest_backup: bool,
    ) -> Result<&'static Arc<DeoxysBackend>> {
        let db_path = db_config_dir.join("starknet/rockdb"); //.deoxysdb/chains/starknet/starknet/rockdb

        let db =
            Arc::new(open_rocksdb(&db_path, true, backup_dir, restore_from_latest_backup).context("opening database")?);
        DB_SINGLETON.set(db).ok().context("db already loaded")?;

        let db = DB_SINGLETON.get().unwrap();

        let bonsai_config = BonsaiStorageConfig {
            max_saved_trie_logs: Some(0),
            max_saved_snapshots: Some(0),
            snapshot_interval: u64::MAX,
        };

        let mut bonsai_contract = BonsaiStorage::new(
            BonsaiDb::new(
                db,
                DatabaseKeyMapping {
                    flat: Column::BonsaiContractsFlat,
                    trie: Column::BonsaiContractsTrie,
                    log: Column::BonsaiContractsLog,
                },
            ),
            bonsai_config.clone(),
        )
        .unwrap();
        bonsai_contract.init_tree(bonsai_identifier::CONTRACT).unwrap();

        let bonsai_contract_storage = BonsaiStorage::new(
            BonsaiDb::new(
                db,
                DatabaseKeyMapping {
                    flat: Column::BonsaiContractsStorageFlat,
                    trie: Column::BonsaiContractsStorageTrie,
                    log: Column::BonsaiContractsStorageLog,
                },
            ),
            bonsai_config.clone(),
        )
        .unwrap();

        let mut bonsai_classes = BonsaiStorage::new(
            BonsaiDb::new(
                db,
                DatabaseKeyMapping {
                    flat: Column::BonsaiClassesFlat,
                    trie: Column::BonsaiClassesTrie,
                    log: Column::BonsaiClassesLog,
                },
            ),
            bonsai_config.clone(),
        )
        .unwrap();
        bonsai_classes.init_tree(bonsai_identifier::CLASS).unwrap();

        let backend = Arc::new(Self {
            mapping: Arc::new(MappingDb::new(Arc::clone(db), cache_more_things)),
            meta: Arc::new(MetaDb::new(Arc::clone(db))),
            bonsai_contract: RwLock::new(bonsai_contract),
            bonsai_storage: RwLock::new(bonsai_contract_storage),
            bonsai_class: RwLock::new(bonsai_classes),
        });

        BACKEND_SINGLETON.set(backend).ok().context("backend already initialized")?;

        Ok(BACKEND_SINGLETON.get().unwrap())
    }

    pub async fn backup() -> Result<()> {
        let chann = DB_BACKUP_SINGLETON.get().context("backups are not enabled")?;
        let (callback_sender, callback_recv) = oneshot::channel();
        chann.send(BackupRequest(callback_sender)).await.context("backups are not enabled")?;
        callback_recv.await.context("backups task died :(")?;
        Ok(())
    }

    /// Return the mapping database manager
    pub fn mapping() -> &'static Arc<MappingDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.mapping).expect("Backend not initialized")
    }

    /// Return the meta database manager
    pub fn meta() -> &'static Arc<MetaDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.meta).expect("Backend not initialized")
    }

    pub(crate) fn bonsai_contract() -> &'static RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_contract).expect("Backend not initialized")
    }

    pub(crate) fn bonsai_storage() -> &'static RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_storage).expect("Backend not initialized")
    }

    pub(crate) fn bonsai_class() -> &'static RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_class).expect("Backend not initialized")
    }

    pub fn expose_db() -> &'static Arc<DB> {
        DB_SINGLETON.get().expect("Databsae not initialized")
    }

    pub fn compact() {
        Self::expose_db().compact_range(None::<&[u8]>, None::<&[u8]>);
    }
}
