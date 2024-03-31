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

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
pub use error::{BonsaiDbError, DbError};
use kvdb::KeyValueDB;
pub use mapping_db::MappingCommitment;
pub use messaging_db::LastSyncedEventBlock;
use sierra_classes_db::SierraClassesDb;
use starknet_api::hash::StarkHash;
use starknet_types_core::hash::{Pedersen, Poseidon};

pub mod bonsai_db;
mod da_db;
mod db_opening_utils;
mod error;
mod l1_handler_tx_fee;
mod mapping_db;
mod messaging_db;
mod meta_db;
mod sierra_classes_db;
pub mod storage;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use bonsai_db::{BonsaiConfigs, BonsaiDb, TrieColumn};
use da_db::DaDb;
use l1_handler_tx_fee::L1HandlerTxFeeDb;
use mapping_db::MappingDb;
use messaging_db::MessagingDb;
use meta_db::MetaDb;
use sc_client_db::DatabaseSource;
use sp_database::Database;

const DB_HASH_LEN: usize = 32;
/// Hash type that this backend uses for the database.
pub type DbHash = [u8; DB_HASH_LEN];

struct DatabaseSettings {
    /// Where to find the database.
    pub source: DatabaseSource,
}

pub(crate) mod columns {
    /// Total number of columns.
    // ===== /!\ ===================================================================================
    // MUST BE INCREMENTED WHEN A NEW COLUMN IN ADDED
    // ===== /!\ ===================================================================================
    pub const NUM_COLUMNS: u32 = 19;

    pub const META: u32 = 0;
    pub const BLOCK_MAPPING: u32 = 1;
    pub const TRANSACTION_MAPPING: u32 = 2;
    pub const SYNCED_MAPPING: u32 = 3;
    pub const DA: u32 = 4;

    /// This column is used to map starknet block hashes to a list of transaction hashes that are
    /// contained in the block.
    ///
    /// This column should only be accessed if the `--cache` flag is enabled.
    pub const STARKNET_TRANSACTION_HASHES_CACHE: u32 = 5;

    /// This column is used to map starknet block numbers to their block hashes.
    ///
    /// This column should only be accessed if the `--cache` flag is enabled.
    pub const STARKNET_BLOCK_HASHES_CACHE: u32 = 6;

    /// This column contains last synchronized L1 block.
    pub const MESSAGING: u32 = 7;

    /// This column contains the Sierra contract classes
    pub const SIERRA_CONTRACT_CLASSES: u32 = 8;

    /// This column stores the fee paid on l1 for L1Handler transactions
    pub const L1_HANDLER_PAID_FEE: u32 = 9;

    /// The bonsai columns are triplicated since we need to set a column for
    ///
    /// const TRIE_LOG_CF: &str = "trie_log";
    /// const TRIE_CF: &str = "trie";
    /// const FLAT_CF: &str = "flat";
    /// as defined in https://github.com/keep-starknet-strange/bonsai-trie/blob/oss/src/databases/rocks_db.rs
    ///
    /// For each tries CONTRACTS, CLASSES and STORAGE
    pub const TRIE_BONSAI_CONTRACTS: u32 = 10;
    pub const FLAT_BONSAI_CONTRACTS: u32 = 11;
    pub const LOG_BONSAI_CONTRACTS: u32 = 12;

    pub const TRIE_BONSAI_CONTRACTS_STORAGE: u32 = 13;
    pub const FLAT_BONSAI_CONTRACTS_STORAGE: u32 = 14;
    pub const LOG_BONSAI_CONTRACTS_STORAGE: u32 = 15;

    pub const TRIE_BONSAI_CLASSES: u32 = 16;
    pub const FLAT_BONSAI_CLASSES: u32 = 17;
    pub const LOG_BONSAI_CLASSES: u32 = 18;
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
    bonsai_contract: Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>,
    bonsai_storage: Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>>,
    bonsai_class: Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Poseidon>>>,
}

// Singleton backing instance for `DeoxysBackend`
static BACKEND_SINGLETON: OnceLock<Arc<DeoxysBackend>> = OnceLock::new();

impl DeoxysBackend {
    /// Initializes a local database, returning a singleton backend instance.
    ///
    /// This backend should only be used to pass to substrate functions. Use the static functions
    /// defined below to access static fields instead.
    pub fn open(
        database: &DatabaseSource,
        db_config_dir: &Path,
        cache_more_things: bool,
    ) -> Result<&'static Arc<DeoxysBackend>, String> {
        BACKEND_SINGLETON
            .set(Arc::new(Self::init(database, db_config_dir, cache_more_things).unwrap()))
            .map_err(|_| "Backend already initialized")?;

        Ok(BACKEND_SINGLETON.get().unwrap())
    }

    fn init(database: &DatabaseSource, db_config_dir: &Path, cache_more_things: bool) -> Result<Self, String> {
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
                    _ => return Err("Supported db sources: `rocksdb` | `paritydb` | `auto`".to_string()),
                },
            },
            cache_more_things,
        )
    }

    fn new(config: &DatabaseSettings, cache_more_things: bool) -> Result<Self, String> {
        let db = db_opening_utils::open_database(config)?;
        let kvdb: Arc<dyn KeyValueDB> = db.0;
        let spdb: Arc<dyn Database<DbHash>> = db.1;

        let contract = BonsaiDb { db: kvdb.clone(), current_column: TrieColumn::Contract };
        let contract_storage = BonsaiDb { db: kvdb.clone(), current_column: TrieColumn::ContractStorage };
        let class = BonsaiDb { db: kvdb.clone(), current_column: TrieColumn::Class };
        let config = BonsaiConfigs::new(contract, contract_storage, class);

        Ok(Self {
            mapping: Arc::new(MappingDb::new(spdb.clone(), cache_more_things)),
            meta: Arc::new(MetaDb { db: spdb.clone() }),
            da: Arc::new(DaDb { db: spdb.clone() }),
            messaging: Arc::new(MessagingDb { db: spdb.clone() }),
            sierra_classes: Arc::new(SierraClassesDb { db: spdb.clone() }),
            l1_handler_paid_fee: Arc::new(L1HandlerTxFeeDb { db: spdb.clone() }),
            bonsai_contract: Arc::new(RwLock::new(config.contract)),
            bonsai_storage: Arc::new(RwLock::new(config.contract_storage)),
            bonsai_class: Arc::new(RwLock::new(config.class)),
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

    pub(crate) fn bonsai_contract() -> &'static Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_contract).expect("Backend not initialized")
    }

    pub(crate) fn bonsai_storage() -> &'static Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Pedersen>>> {
        BACKEND_SINGLETON.get().map(|backend| &backend.bonsai_storage).expect("Backend not initialized")
    }

    pub(crate) fn bonsai_class() -> &'static Arc<RwLock<BonsaiStorage<BasicId, BonsaiDb, Poseidon>>> {
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
