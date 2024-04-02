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
use std::sync::{Arc, OnceLock};

use anyhow::{bail, Context, Result};
use bonsai_db::{BonsaiStorageAccess, DatabaseKeyMapping};
use da_db::DaDb;
use l1_handler_tx_fee::L1HandlerTxFeeDb;
use mapping_db::MappingDb;
use messaging_db::MessagingDb;
use meta_db::MetaDb;
use sc_client_db::DatabaseSource;

mod error;
mod mapping_db;
use rocksdb::{BoundColumnFamily, ColumnFamilyDescriptor, MultiThreaded, OptimisticTransactionDB, Options};
use sierra_classes_db::SierraClassesDb;
mod da_db;
mod messaging_db;
mod sierra_classes_db;
use starknet_api::hash::StarkHash;
use starknet_types_core::hash::{Pedersen, Poseidon};
pub mod bonsai_db;
mod l1_handler_tx_fee;
mod meta_db;

pub use error::{BonsaiDbError, DbError};
pub use mapping_db::MappingCommitment;
pub use messaging_db::LastSyncedEventBlock;

const DB_HASH_LEN: usize = 32;
/// Hash type that this backend uses for the database.
pub type DbHash = [u8; DB_HASH_LEN];

struct DatabaseSettings {
    /// Where to find the database.
    pub source: DatabaseSource,
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

    /// This column is used to map starknet block hashes to a list of transaction hashes that are
    /// contained in the block.
    ///
    /// This column should only be accessed if the `--cache` flag is enabled.
    StarknetTransactionHashesCache,

    /// This column is used to map starknet block numbers to their block hashes.
    ///
    /// This column should only be accessed if the `--cache` flag is enabled.
    StarknetBlockHashesCache,

    /// This column contains last synchronized L1 block.
    Messaging,

    /// This column contains the Sierra contract classes
    SierraContractClasses,

    /// This column stores the fee paid on l1 for L1Handler transactions
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
            Messaging,
            SierraContractClasses,
            L1HandlerPaidFee,
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
            Column::Messaging => "messaging",
            Column::SierraContractClasses => "sierra_contract_classes",
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
        self.cf_handle(col.rocksdb_name()).expect("column not inititalized")
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
    messaging: Arc<MessagingDb>,
    sierra_classes: Arc<SierraClassesDb>,
    l1_handler_paid_fee: Arc<L1HandlerTxFeeDb>,
    bonsai_contract: BonsaiStorageAccess<Pedersen>,
    bonsai_storage: BonsaiStorageAccess<Pedersen>,
    bonsai_class: BonsaiStorageAccess<Poseidon>,
}

// Singleton backing instance for `DeoxysBackend`
static BACKEND_SINGLETON: OnceLock<Arc<DeoxysBackend>> = OnceLock::new();

// TODO: add neogen to comment this :)
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
            },
            cache_more_things,
        )
    }

    fn new(config: &DatabaseSettings, cache_more_things: bool) -> Result<Self> {
        let db = Arc::new(open_database(config)?);

        Ok(Self {
            mapping: Arc::new(MappingDb::new(Arc::clone(&db), cache_more_things)),
            meta: Arc::new(MetaDb::new(Arc::clone(&db))),
            da: Arc::new(DaDb::new(Arc::clone(&db))),
            messaging: Arc::new(MessagingDb::new(Arc::clone(&db))),
            sierra_classes: Arc::new(SierraClassesDb::new(Arc::clone(&db))),
            l1_handler_paid_fee: Arc::new(L1HandlerTxFeeDb::new(Arc::clone(&db))),
            bonsai_contract: BonsaiStorageAccess::new(
                Arc::clone(&db),
                DatabaseKeyMapping {
                    flat: Column::BonsaiContractsFlat,
                    trie: Column::BonsaiContractsTrie,
                    trie_log: Column::BonsaiContractsLog,
                },
            ),
            bonsai_storage: BonsaiStorageAccess::new(
                Arc::clone(&db),
                DatabaseKeyMapping {
                    flat: Column::BonsaiContractsStorageFlat,
                    trie: Column::BonsaiContractsStorageTrie,
                    trie_log: Column::BonsaiContractsStorageLog,
                },
            ),
            bonsai_class: BonsaiStorageAccess::new(
                Arc::clone(&db),
                DatabaseKeyMapping {
                    flat: Column::BonsaiClassesFlat,
                    trie: Column::BonsaiClassesTrie,
                    trie_log: Column::BonsaiClassesLog,
                },
            ),
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

    /// Return the da database manager
    pub fn messaging() -> &'static Arc<MessagingDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.messaging).expect("Backend not initialized")
    }

    /// Return the sierra classes database manager
    pub fn sierra_classes() -> &'static Arc<SierraClassesDb> {
        BACKEND_SINGLETON.get().map(|backend| &backend.sierra_classes).expect("Backend not initialized")
    }

    pub fn bonsai_contract() -> &'static BonsaiStorageAccess<Pedersen> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_contract).expect("Backend not initialized")
    }

    pub fn bonsai_storage() -> &'static BonsaiStorageAccess<Pedersen> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_storage).expect("Backend not initialized")
    }

    pub fn bonsai_class() -> &'static BonsaiStorageAccess<Poseidon> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_class).expect("Backend not initialized")
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
