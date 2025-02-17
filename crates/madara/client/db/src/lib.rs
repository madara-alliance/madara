//! Madara database

use anyhow::Context;
use block_db::get_latest_block_n;
use bonsai_db::{BonsaiDb, DatabaseKeyMapping};
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use db_metrics::DbMetrics;
use mp_chain_config::ChainConfig;
use mp_rpc::EmittedEvent;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId};
use rocksdb::backup::{BackupEngine, BackupEngineOptions};
use rocksdb::{
    BoundColumnFamily, ColumnFamilyDescriptor, DBWithThreadMode, Env, FlushOptions, MultiThreaded, WriteOptions,
};
use rocksdb_options::rocksdb_global_options;
use snapshots::Snapshots;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, fs};
use tokio::sync::{mpsc, oneshot};

mod db_version;
mod error;
mod rocksdb_options;
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
pub mod mempool_db;
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

/// EventChannels manages a highly efficient and scalable pub/sub system for events with 16 specific channels
/// plus one "all" channel. This architecture provides several key benefits:
///
/// Benefits:
/// - Selective Subscription: Subscribers can choose between receiving all events or filtering for specific
///   senders, optimizing network and processing resources
/// - Memory Efficiency: The fixed number of channels (16) provides a good balance between granularity
///   and memory overhead
/// - Predictable Routing: The XOR-based hash function ensures consistent and fast mapping of sender
///   addresses to channels
///
/// Events are distributed based on the sender's address in the event, where each sender address
/// is mapped to one of the 16 specific channels using a simple XOR-based hash function.
/// Subscribers can choose to either receive all events or only events from specific senders
/// by subscribing to the corresponding channel.
pub struct EventChannels {
    /// Broadcast channel that receives all events regardless of their sender's address
    all_channels: tokio::sync::broadcast::Sender<EmittedEvent>,
    /// Array of 16 broadcast channels, each handling events from a subset of sender addresses
    /// The target channel for an event is determined by the sender's address mapping
    specific_channels: [tokio::sync::broadcast::Sender<EmittedEvent>; 16],
}

impl EventChannels {
    /// Creates a new EventChannels instance with the specified buffer capacity for each channel.
    /// Each channel (both all_channels and specific channels) will be able to buffer up to
    /// `capacity` events before older events are dropped.
    ///
    /// # Arguments
    /// * `capacity` - The maximum number of events that can be buffered in each channel
    ///
    /// # Returns
    /// A new EventChannels instance with initialized broadcast channels
    pub fn new(capacity: usize) -> Self {
        let (all_channels, _) = tokio::sync::broadcast::channel(capacity);

        let mut specific_channels = Vec::with_capacity(16);
        for _ in 0..16 {
            let (sender, _) = tokio::sync::broadcast::channel(capacity);
            specific_channels.push(sender);
        }

        Self { all_channels, specific_channels: specific_channels.try_into().unwrap() }
    }

    /// Subscribes to events based on an optional sender address filter
    ///
    /// # Arguments
    /// * `from_address` - Optional sender address to filter events:
    ///   * If `Some(address)`, subscribes only to events from senders whose addresses map
    ///     to the same channel as the provided address (address % 16)
    ///   * If `None`, subscribes to all events regardless of sender address
    ///
    /// # Returns
    /// A broadcast::Receiver that will receive either:
    /// * All events (if from_address is None)
    /// * Only events from senders whose addresses map to the same channel as the provided address
    ///
    /// # Warning
    /// This method only provides a coarse filtering mechanism based on address mapping.
    /// You will still need to implement additional filtering in your receiver logic because:
    /// * Multiple sender addresses map to the same channel
    /// * You may want to match the exact sender address rather than just its channel mapping
    ///
    /// # Implementation Details
    /// When a specific address is provided, the method:
    /// 1. Calculates the channel index using the sender's address
    /// 2. Subscribes to the corresponding specific channel
    ///
    /// This means you'll receive events from all senders whose addresses map to the same channel
    pub fn subscribe(&self, from_address: Option<Felt>) -> tokio::sync::broadcast::Receiver<EmittedEvent> {
        match from_address {
            Some(address) => {
                let channel_index = self.calculate_channel_index(&address);
                self.specific_channels[channel_index].subscribe()
            }
            None => self.all_channels.subscribe(),
        }
    }

    /// Publishes an event to both the all_channels and the specific channel determined by the sender's address.
    /// The event will only be sent to channels that have active subscribers.
    ///
    /// # Arguments
    /// * `event` - The event to publish, containing the sender's address that determines the target specific channel
    ///
    /// # Returns
    /// * `Ok(usize)` - The sum of the number of subscribers that received the event across both channels
    /// * `Ok(0)` - If no subscribers exist in any channel
    /// * `Err` - If the event couldn't be sent
    pub fn publish(
        &self,
        event: EmittedEvent,
    ) -> Result<usize, Box<tokio::sync::broadcast::error::SendError<EmittedEvent>>> {
        let channel_index = self.calculate_channel_index(&event.event.from_address);
        let specific_channel = &self.specific_channels[channel_index];

        let mut total = 0;
        if self.all_channels.receiver_count() > 0 {
            total += self.all_channels.send(event.clone())?;
        }
        if specific_channel.receiver_count() > 0 {
            total += specific_channel.send(event)?;
        }
        Ok(total)
    }

    pub fn receiver_count(&self) -> usize {
        self.all_channels.receiver_count() + self.specific_channels.iter().map(|c| c.receiver_count()).sum::<usize>()
    }

    /// Calculates the target channel index for a given sender's address
    ///
    /// # Arguments
    /// * `address` - The Felt address of the event sender to calculate the channel index for
    ///
    /// # Returns
    /// A channel index between 0 and 15, calculated by XORing the two highest limbs of the address
    /// and taking the lowest 4 bits of the result.
    ///
    /// # Implementation Details
    /// Rather than using the last byte of the address, this function:
    /// 1. Gets the raw limbs representation of the address
    /// 2. XORs limbs[0] and limbs[1] (the two lowest limbs)
    /// 3. Uses the lowest 4 bits of the XOR result to determine the channel
    ///
    /// This provides a balanced distribution of addresses across channels by
    /// incorporating entropy from the address
    fn calculate_channel_index(&self, address: &Felt) -> usize {
        let limbs = address.to_raw();
        let hash = limbs[0] ^ limbs[1];
        (hash & 0x0f) as usize
    }
}

/// Madara client database backend singleton.
pub struct MadaraBackend {
    backup_handle: Option<mpsc::Sender<BackupRequest>>,
    db: Arc<DB>,
    chain_config: Arc<ChainConfig>,
    db_metrics: DbMetrics,
    snapshots: Arc<Snapshots>,
    trie_log_config: TrieLogConfig,
    sender_block_info: tokio::sync::broadcast::Sender<mp_block::MadaraBlockInfo>,
    sender_event: EventChannels,
    write_opt_no_wal: WriteOptions,
    #[cfg(any(test, feature = "testing"))]
    _temp_dir: Option<tempfile::TempDir>,
}

impl fmt::Debug for MadaraBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MadaraBackend")
            .field("backup_handle", &self.backup_handle)
            .field("db", &self.db)
            .field("chain_config", &self.chain_config)
            .field("db_metrics", &self.db_metrics)
            .field("sender_block_info", &self.sender_block_info)
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

impl MadaraBackend {
    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }

    #[cfg(any(test, feature = "testing"))]
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
            sender_event: EventChannels::new(100),
            write_opt_no_wal: make_write_opt_no_wal(),
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
        // check if the db version is compatible with the current binary
        tracing::debug!("checking db version");
        if let Some(db_version) = db_version::check_db_version(&db_config_dir).context("Checking database version")? {
            tracing::debug!("version of existing db is {db_version}");
        }

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
            sender_event: EventChannels::new(100),
            write_opt_no_wal: make_write_opt_no_wal(),
            #[cfg(any(test, feature = "testing"))]
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
