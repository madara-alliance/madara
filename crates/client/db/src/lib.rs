//! A database backend storing data about madara chain
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

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use anyhow::{bail, Context, Result};
use bonsai_db::{BonsaiDb, DatabaseKeyMapping};
use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use da_db::DaDb;
use l1_handler_tx_fee::L1HandlerTxFeeDb;
use mapping_db::MappingDb;
use meta_db::MetaDb;
use sc_client_db::DatabaseSource;

mod error;
mod mapping_db;
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBCompressionType, MultiThreaded, OptimisticTransactionDB, Options,
};
mod da_db;
use starknet_api::hash::StarkHash;
use starknet_types_core::hash::{Pedersen, Poseidon};
pub mod bonsai_db;
mod l1_handler_tx_fee;
mod meta_db;
pub mod storage_handler;
pub mod storage_updates;

pub use error::{BonsaiDbError, DbError};
pub use mapping_db::MappingCommitment;
use storage_handler::bonsai_identifier;

const DB_HASH_LEN: usize = 32;
/// Hash type that this backend uses for the database.
pub type DbHash = [u8; DB_HASH_LEN];

struct DatabaseSettings {
    /// Where to find the database.
    pub source: DatabaseSource,
    pub max_saved_trie_logs: Option<usize>,
    pub max_saved_snapshots: Option<usize>,
    pub snapshot_interval: u64,
}

impl From<&DatabaseSettings> for BonsaiStorageConfig {
    fn from(val: &DatabaseSettings) -> Self {
        BonsaiStorageConfig {
            max_saved_trie_logs: val.max_saved_trie_logs,
            max_saved_snapshots: val.max_saved_snapshots,
            snapshot_interval: val.snapshot_interval,
        }
    }
}

pub type DB = OptimisticTransactionDB<MultiThreaded>;

pub(crate) fn open_database(config: &DatabaseSettings) -> Result<DB> {
    Ok(match &config.source {
        DatabaseSource::RocksDb { path, .. } => open_rocksdb(path, true)?,
        DatabaseSource::Auto { paritydb_path: _, rocksdb_path, .. } => open_rocksdb(rocksdb_path, false)?,
        _ => bail!("only the rocksdb database source is supported at the moment"),
    })
}

pub(crate) fn open_rocksdb(path: &Path, create: bool) -> Result<OptimisticTransactionDB<MultiThreaded>> {
    let mut opts = Options::default();
    opts.set_report_bg_io_stats(true);
    opts.set_use_fsync(false);
    opts.create_if_missing(create);
    opts.create_missing_column_families(true);
    opts.set_bytes_per_sync(1024 * 1024);
    opts.set_keep_log_file_num(1);
    opts.set_compression_type(DBCompressionType::Zstd);
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    opts.increase_parallelism(i32::max(cores / 2, 1));

    let db = OptimisticTransactionDB::<MultiThreaded>::open_cf_descriptors(
        &opts,
        path,
        Column::ALL.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name(), col.rocksdb_options())),
    )?;

    Ok(db)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Meta,
    BlockMapping,
    TransactionMapping,
    SyncedMapping,
    Da,
    BlockHashToNumber,
    BlockNumberToHash,
    BlockStateDiff,
    ContractClassData,
    ContractData,
    ContractClassHashes,
    ContractStorage,

    /// This column is used to map starknet block hashes to a list of transaction hashes that are
    /// contained in the block.
    ///
    /// This column should only be accessed if the `--cache` flag is enabled.
    StarknetTransactionHashesCache,

    /// This column is used to map starknet block numbers to their block hashes.
    ///
    /// This column should only be accessed if the `--cache` flag is enabled.
    StarknetBlockHashesCache,

    // TODO: remove this
    L1HandlerPaidFee,

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
            TransactionMapping,
            SyncedMapping,
            Da,
            StarknetTransactionHashesCache,
            StarknetBlockHashesCache,
            L1HandlerPaidFee,
            BlockHashToNumber,
            BlockNumberToHash,
            BlockStateDiff,
            ContractClassData,
            ContractData,
            ContractStorage,
            ContractClassHashes,
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
            Column::Da => "da",
            Column::StarknetTransactionHashesCache => "starknet_transaction_hashes_cache",
            Column::StarknetBlockHashesCache => "starnet_block_hashes_cache",
            Column::L1HandlerPaidFee => "l1_handler_paid_fee",
            Column::BonsaiContractsTrie => "bonsai_contracts_trie",
            Column::BonsaiContractsFlat => "bonsai_contracts_flat",
            Column::BonsaiContractsLog => "bonsai_contracts_log",
            Column::BonsaiContractsStorageTrie => "bonsai_contracts_storage_trie",
            Column::BonsaiContractsStorageFlat => "bonsai_contracts_storage_flat",
            Column::BonsaiContractsStorageLog => "bonsai_contracts_storage_log",
            Column::BonsaiClassesTrie => "bonsai_classes_trie",
            Column::BonsaiClassesFlat => "bonsai_classes_flat",
            Column::BonsaiClassesLog => "bonsai_classes_log",
            Column::BlockHashToNumber => "block_hash_to_number_trie",
            Column::BlockNumberToHash => "block_to_hash_trie",
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

pub(crate) trait DatabaseExt {
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

pub mod static_keys {
    pub const CURRENT_SYNCING_TIPS: &[u8] = b"CURRENT_SYNCING_TIPS";
    pub const LAST_PROVED_BLOCK: &[u8] = b"LAST_PROVED_BLOCK";
    pub const LAST_SYNCED_L1_EVENT_BLOCK: &[u8] = b"LAST_SYNCED_L1_EVENT_BLOCK";
}

/// Returns the Starknet database directory.
pub fn starknet_database_dir(db_config_dir: &Path, db_path: &str) -> PathBuf {
    db_config_dir.join("starknet").join(db_path)
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
/// * `l1_handler_paid_fee`: @antyro what is this for?
/// * `bonsai_contract`: Bezu-bonsai trie used to compute the contract root.
/// * `bonsai_storage`: Bezu-bonsai trie used to compute the storage root for each contract.
/// * `bonsai_class`: Bezu-bonsai trie used to compute the class root.
pub struct DeoxysBackend {
    meta: Arc<MetaDb>,
    mapping: Arc<MappingDb>,
    da: Arc<DaDb>,
    l1_handler_paid_fee: Arc<L1HandlerTxFeeDb>,
    bonsai_contract: RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
    bonsai_storage: RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
    bonsai_class: RwLock<BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>>,
}

// Singleton backing instance for `DeoxysBackend`
static BACKEND_SINGLETON: OnceLock<Arc<DeoxysBackend>> = OnceLock::new();

static DB_SINGLETON: OnceLock<Arc<DB>> = OnceLock::new();

impl DeoxysBackend {
    /// Initializes a local database, returning a singleton backend instance.
    ///
    /// This backend should only be used to pass to substrate functions. Use the static functions
    /// defined below to access static fields instead.
    pub fn open(
        database: &DatabaseSource,
        db_config_dir: &Path,
        cache_more_things: bool,
    ) -> Result<&'static Arc<DeoxysBackend>> {
        BACKEND_SINGLETON
            .set(Arc::new(Self::init(database, db_config_dir, cache_more_things).unwrap()))
            .ok()
            .context("Backend already initialized")?;

        Ok(BACKEND_SINGLETON.get().unwrap())
    }

    fn init(database: &DatabaseSource, db_config_dir: &Path, cache_more_things: bool) -> Result<Self> {
        Self::new(
            &DatabaseSettings {
                source: match database {
                    DatabaseSource::RocksDb { .. } => {
                        DatabaseSource::RocksDb { path: starknet_database_dir(db_config_dir, "rockdb"), cache_size: 0 }
                    }
                    DatabaseSource::ParityDb { .. } => {
                        DatabaseSource::ParityDb { path: starknet_database_dir(db_config_dir, "paritydb") }
                    }
                    DatabaseSource::Auto { .. } => DatabaseSource::Auto {
                        rocksdb_path: starknet_database_dir(db_config_dir, "rockdb"),
                        paritydb_path: starknet_database_dir(db_config_dir, "paritydb"),
                        cache_size: 0,
                    },
                    _ => bail!("Supported db sources: `rocksdb` | `paritydb` | `auto`"),
                },
                max_saved_trie_logs: None,
                max_saved_snapshots: None,
                snapshot_interval: 100,
            },
            cache_more_things,
        )
    }

    fn new(config: &DatabaseSettings, cache_more_things: bool) -> Result<Self> {
        DB_SINGLETON.set(Arc::new(open_database(config)?)).unwrap();
        let db = DB_SINGLETON.get().unwrap();
        let bonsai_config = BonsaiStorageConfig::from(config);

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

        Ok(Self {
            mapping: Arc::new(MappingDb::new(Arc::clone(db), cache_more_things)),
            meta: Arc::new(MetaDb::new(Arc::clone(db))),
            da: Arc::new(DaDb::new(Arc::clone(db))),
            l1_handler_paid_fee: Arc::new(L1HandlerTxFeeDb::new(Arc::clone(db))),
            bonsai_contract: RwLock::new(bonsai_contract),
            bonsai_storage: RwLock::new(bonsai_contract_storage),
            bonsai_class: RwLock::new(bonsai_classes),
        })
    }

    /// Return the mapping database manager
    pub fn mapping() -> &'static Arc<MappingDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.mapping).expect("Backend not initialized")
    }

    /// Return the meta database manager
    pub fn meta() -> &'static Arc<MetaDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.meta).expect("Backend not initialized")
    }

    /// Return the da database manager
    pub fn da() -> &'static Arc<DaDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.da).expect("Backend not initialized")
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

    pub(crate) fn expose_db() -> &'static Arc<DB> {
        DB_SINGLETON.get().expect("Databsae not initialized")
    }

    /// Return l1 handler tx paid fee database manager
    pub fn l1_handler_paid_fee() -> &'static Arc<L1HandlerTxFeeDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.l1_handler_paid_fee).expect("Backend not initialized")
    }

    /// In the future, we will compute the block global state root asynchronously in the client,
    /// using the Starknet-Bonzai-trie.
    /// That what replaces it for now :)
    pub fn temporary_global_state_root_getter() -> StarkHash {
        Default::default()
    }
}
