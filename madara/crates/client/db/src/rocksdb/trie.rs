use crate::rocksdb::column::Column;
use crate::rocksdb::snapshots::{SnapshotRef, Snapshots};
use crate::rocksdb::{RocksDBStorage, RocksDBStorageInner, WriteBatchWithTransaction};
use bonsai_trie::id::Id;
use bonsai_trie::{
    BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage, BonsaiStorageConfig, ByteVec, DBError, DatabaseKey,
};
use rocksdb::{Direction, IteratorMode};
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::LazyLock;

pub const BONSAI_CONTRACT_FLAT_COLUMN: Column = Column::new("bonsai_contract_flat").set_point_lookup();
pub const BONSAI_CONTRACT_TRIE_COLUMN: Column = Column::new("bonsai_contract_trie").set_point_lookup();
pub const BONSAI_CONTRACT_LOG_COLUMN: Column = Column::new("bonsai_contract_log");
pub const BONSAI_CONTRACT_STORAGE_FLAT_COLUMN: Column = Column::new("bonsai_contract_storage_flat").set_point_lookup();
pub const BONSAI_CONTRACT_STORAGE_TRIE_COLUMN: Column = Column::new("bonsai_contract_storage_trie").set_point_lookup();
pub const BONSAI_CONTRACT_STORAGE_LOG_COLUMN: Column = Column::new("bonsai_contract_storage_log");
pub const BONSAI_CLASS_FLAT_COLUMN: Column = Column::new("bonsai_class_flat").set_point_lookup();
pub const BONSAI_CLASS_TRIE_COLUMN: Column = Column::new("bonsai_class_trie").set_point_lookup();
pub const BONSAI_CLASS_LOG_COLUMN: Column = Column::new("bonsai_class_log");

/// Fixed ordering used by `bonsai_db_ops_snapshot()` for per-CF counters.
pub const BONSAI_CF_NAMES: [&str; 9] = [
    "bonsai_contract_flat",
    "bonsai_contract_trie",
    "bonsai_contract_log",
    "bonsai_contract_storage_flat",
    "bonsai_contract_storage_trie",
    "bonsai_contract_storage_log",
    "bonsai_class_flat",
    "bonsai_class_trie",
    "bonsai_class_log",
];

fn bonsai_cf_index(name: &str) -> Option<usize> {
    match name {
        "bonsai_contract_flat" => Some(0),
        "bonsai_contract_trie" => Some(1),
        "bonsai_contract_log" => Some(2),
        "bonsai_contract_storage_flat" => Some(3),
        "bonsai_contract_storage_trie" => Some(4),
        "bonsai_contract_storage_log" => Some(5),
        "bonsai_class_flat" => Some(6),
        "bonsai_class_trie" => Some(7),
        "bonsai_class_log" => Some(8),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BonsaiDbOpsSnapshot {
    pub enabled: bool,
    pub get_calls: [u64; 9],
    pub get_value_bytes: [u64; 9],
    pub insert_calls: [u64; 9],
    pub insert_old_value_present: [u64; 9],
    pub insert_old_value_bytes: [u64; 9],
    pub remove_calls: [u64; 9],
    pub remove_old_value_present: [u64; 9],
    pub remove_old_value_bytes: [u64; 9],
    pub contains_calls: [u64; 9],
    pub contains_hits: [u64; 9],
    pub iter_calls: [u64; 9],
    pub iter_keys: [u64; 9],
    pub iter_key_bytes: [u64; 9],
    pub iter_value_bytes: [u64; 9],
    pub write_batch_calls: u64,
    pub get_key_bytes: [u64; 9],
    pub insert_key_bytes: [u64; 9],
    pub insert_value_bytes: [u64; 9],
    pub remove_key_bytes: [u64; 9],
    pub contains_key_bytes: [u64; 9],
}

struct BonsaiDbOpsCounters {
    enabled: AtomicBool,
    get_calls: [AtomicU64; 9],
    get_value_bytes: [AtomicU64; 9],
    insert_calls: [AtomicU64; 9],
    insert_old_value_present: [AtomicU64; 9],
    insert_old_value_bytes: [AtomicU64; 9],
    remove_calls: [AtomicU64; 9],
    remove_old_value_present: [AtomicU64; 9],
    remove_old_value_bytes: [AtomicU64; 9],
    contains_calls: [AtomicU64; 9],
    contains_hits: [AtomicU64; 9],
    iter_calls: [AtomicU64; 9],
    iter_keys: [AtomicU64; 9],
    iter_key_bytes: [AtomicU64; 9],
    iter_value_bytes: [AtomicU64; 9],
    write_batch_calls: AtomicU64,
    get_key_bytes: [AtomicU64; 9],
    insert_key_bytes: [AtomicU64; 9],
    insert_value_bytes: [AtomicU64; 9],
    remove_key_bytes: [AtomicU64; 9],
    contains_key_bytes: [AtomicU64; 9],
}

impl BonsaiDbOpsCounters {
    fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            get_calls: std::array::from_fn(|_| AtomicU64::new(0)),
            get_value_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            insert_calls: std::array::from_fn(|_| AtomicU64::new(0)),
            insert_old_value_present: std::array::from_fn(|_| AtomicU64::new(0)),
            insert_old_value_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            remove_calls: std::array::from_fn(|_| AtomicU64::new(0)),
            remove_old_value_present: std::array::from_fn(|_| AtomicU64::new(0)),
            remove_old_value_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            contains_calls: std::array::from_fn(|_| AtomicU64::new(0)),
            contains_hits: std::array::from_fn(|_| AtomicU64::new(0)),
            iter_calls: std::array::from_fn(|_| AtomicU64::new(0)),
            iter_keys: std::array::from_fn(|_| AtomicU64::new(0)),
            iter_key_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            iter_value_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            write_batch_calls: AtomicU64::new(0),
            get_key_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            insert_key_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            insert_value_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            remove_key_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
            contains_key_bytes: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

static BONSAI_DB_OPS: LazyLock<BonsaiDbOpsCounters> = LazyLock::new(BonsaiDbOpsCounters::new);

pub fn bonsai_db_ops_set_enabled(enabled: bool) {
    BONSAI_DB_OPS.enabled.store(enabled, Ordering::Relaxed);
}

pub fn bonsai_db_ops_snapshot() -> BonsaiDbOpsSnapshot {
    let enabled = BONSAI_DB_OPS.enabled.load(Ordering::Relaxed);
    BonsaiDbOpsSnapshot {
        enabled,
        get_calls: BONSAI_DB_OPS.get_calls.each_ref().map(|v| v.load(Ordering::Relaxed)),
        get_value_bytes: BONSAI_DB_OPS.get_value_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        insert_calls: BONSAI_DB_OPS.insert_calls.each_ref().map(|v| v.load(Ordering::Relaxed)),
        insert_old_value_present: BONSAI_DB_OPS.insert_old_value_present.each_ref().map(|v| v.load(Ordering::Relaxed)),
        insert_old_value_bytes: BONSAI_DB_OPS.insert_old_value_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        remove_calls: BONSAI_DB_OPS.remove_calls.each_ref().map(|v| v.load(Ordering::Relaxed)),
        remove_old_value_present: BONSAI_DB_OPS.remove_old_value_present.each_ref().map(|v| v.load(Ordering::Relaxed)),
        remove_old_value_bytes: BONSAI_DB_OPS.remove_old_value_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        contains_calls: BONSAI_DB_OPS.contains_calls.each_ref().map(|v| v.load(Ordering::Relaxed)),
        contains_hits: BONSAI_DB_OPS.contains_hits.each_ref().map(|v| v.load(Ordering::Relaxed)),
        iter_calls: BONSAI_DB_OPS.iter_calls.each_ref().map(|v| v.load(Ordering::Relaxed)),
        iter_keys: BONSAI_DB_OPS.iter_keys.each_ref().map(|v| v.load(Ordering::Relaxed)),
        iter_key_bytes: BONSAI_DB_OPS.iter_key_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        iter_value_bytes: BONSAI_DB_OPS.iter_value_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        write_batch_calls: BONSAI_DB_OPS.write_batch_calls.load(Ordering::Relaxed),
        get_key_bytes: BONSAI_DB_OPS.get_key_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        insert_key_bytes: BONSAI_DB_OPS.insert_key_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        insert_value_bytes: BONSAI_DB_OPS.insert_value_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        remove_key_bytes: BONSAI_DB_OPS.remove_key_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
        contains_key_bytes: BONSAI_DB_OPS.contains_key_bytes.each_ref().map(|v| v.load(Ordering::Relaxed)),
    }
}

pub type GlobalTrie<H> = BonsaiStorage<BasicId, BonsaiDB, H>;

pub use bonsai_trie::id::BasicId;
pub use bonsai_trie::ProofNode;

/// Wrapper because bonsai requires a special DBError trait implementation.
/// TODO: Remove that upstream in bonsai-trie, this is dumb.
#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct TrieError(#[from] rocksdb::Error);
impl DBError for TrieError {}

/// Wrapper because bonsai's error type does not implement [std::error::Error].
/// TODO: Fix that upstream in bonsai-trie, this is seriously dumb.
#[derive(thiserror::Error, Debug)]
#[error("Global trie error: {0:#}")]
pub struct WrappedBonsaiError(pub bonsai_trie::BonsaiStorageError<TrieError>);

impl RocksDBStorage {
    fn get_bonsai<H: StarkHash + Send + Sync>(
        &self,
        column_mapping: DatabaseKeyMapping,
    ) -> BonsaiStorage<BasicId, BonsaiDB, H> {
        BonsaiStorage::new(
            BonsaiDB { backend: self.inner.clone(), column_mapping, snapshots: self.snapshots.clone() },
            BonsaiStorageConfig {
                max_saved_trie_logs: self.inner.config.max_saved_trie_logs,
                max_saved_snapshots: self.inner.config.max_kept_snapshots,
                snapshot_interval: self.inner.config.snapshot_interval,
            },
            // Every global tree has keys of 251 bits.
            251,
        )
    }
    pub fn contract_trie(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_TRIE_COLUMN,
            log: BONSAI_CONTRACT_LOG_COLUMN,
        })
    }
    pub fn contract_storage_trie(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        })
    }
    pub fn class_trie(&self) -> GlobalTrie<Poseidon> {
        self.get_bonsai(DatabaseKeyMapping {
            flat: BONSAI_CLASS_FLAT_COLUMN,
            trie: BONSAI_CLASS_TRIE_COLUMN,
            log: BONSAI_CLASS_LOG_COLUMN,
        })
    }
}

#[derive(Clone, Debug)]
struct DatabaseKeyMapping {
    flat: Column,
    trie: Column,
    log: Column,
}

impl DatabaseKeyMapping {
    pub(crate) fn map(&self, key: &DatabaseKey) -> &Column {
        match key {
            DatabaseKey::Trie(_) => &self.trie,
            DatabaseKey::Flat(_) => &self.flat,
            DatabaseKey::TrieLog(_) => &self.log,
        }
    }
}

pub struct BonsaiDB {
    backend: Arc<RocksDBStorageInner>,
    snapshots: Arc<Snapshots>,
    /// Mapping from `DatabaseKey` => rocksdb column name
    column_mapping: DatabaseKeyMapping,
}

impl fmt::Debug for BonsaiDB {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BonsaiDB {{}}")
    }
}

impl BonsaiDatabase for BonsaiDB {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    #[tracing::instrument(skip(self, key))]
    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", key);
        let idx = if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            bonsai_cf_index(self.column_mapping.map(key).rocksdb_name)
        } else {
            None
        };
        if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            if let Some(idx) = idx {
                BONSAI_DB_OPS.get_calls[idx].fetch_add(1, Ordering::Relaxed);
                BONSAI_DB_OPS.get_key_bytes[idx].fetch_add(key.as_slice().len() as u64, Ordering::Relaxed);
            }
        }
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        let value = self.backend.db.get_cf(&handle, key.as_slice())?;
        if let (Some(idx), Some(value)) = (idx, value.as_ref()) {
            BONSAI_DB_OPS.get_value_bytes[idx].fetch_add(value.len() as u64, Ordering::Relaxed);
        }
        Ok(value.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix))]
    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        tracing::trace!("Getting by prefix from RocksDB: {:?}", prefix);
        let handle = self.backend.get_column(self.column_mapping.map(prefix).clone());
        let iter = self.backend.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        let mut iter_key_bytes: u64 = 0;
        let mut iter_value_bytes: u64 = 0;
        let out: Vec<_> = iter
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix.as_slice()) {
                        iter_key_bytes += key.len() as u64;
                        iter_value_bytes += value.len() as u64;
                        // nb: to_vec on a Box<[u8]> is a noop conversion
                        Some((key.to_vec().into(), value.to_vec().into()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            if let Some(idx) = bonsai_cf_index(self.column_mapping.map(prefix).rocksdb_name) {
                BONSAI_DB_OPS.iter_calls[idx].fetch_add(1, Ordering::Relaxed);
                BONSAI_DB_OPS.iter_keys[idx].fetch_add(out.len() as u64, Ordering::Relaxed);
                BONSAI_DB_OPS.iter_key_bytes[idx].fetch_add(iter_key_bytes, Ordering::Relaxed);
                BONSAI_DB_OPS.iter_value_bytes[idx].fetch_add(iter_value_bytes, Ordering::Relaxed);
            }
        }

        Ok(out)
    }

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        let idx = if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            bonsai_cf_index(self.column_mapping.map(key).rocksdb_name)
        } else {
            None
        };
        if let Some(idx) = idx {
            BONSAI_DB_OPS.contains_calls[idx].fetch_add(1, Ordering::Relaxed);
            BONSAI_DB_OPS.contains_key_bytes[idx].fetch_add(key.as_slice().len() as u64, Ordering::Relaxed);
        }
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        let exists = self.backend.db.get_cf(&handle, key.as_slice())?.is_some();
        if exists {
            if let Some(idx) = idx {
                BONSAI_DB_OPS.contains_hits[idx].fetch_add(1, Ordering::Relaxed);
            }
        }
        Ok(exists)
    }

    #[tracing::instrument(skip(self, key, value, batch))]
    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let idx = if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            bonsai_cf_index(self.column_mapping.map(key).rocksdb_name)
        } else {
            None
        };
        if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            if let Some(idx) = idx {
                BONSAI_DB_OPS.insert_calls[idx].fetch_add(1, Ordering::Relaxed);
                BONSAI_DB_OPS.insert_key_bytes[idx].fetch_add(key.as_slice().len() as u64, Ordering::Relaxed);
                BONSAI_DB_OPS.insert_value_bytes[idx]
                    .fetch_add(value.len().try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
            }
        }
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());

        let old_value = self.backend.db.get_cf(&handle, key.as_slice())?;
        if let (Some(idx), Some(old)) = (idx, old_value.as_ref()) {
            BONSAI_DB_OPS.insert_old_value_present[idx].fetch_add(1, Ordering::Relaxed);
            BONSAI_DB_OPS.insert_old_value_bytes[idx]
                .fetch_add(old.len().try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
        }
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.backend.db.put_cf_opt(&handle, key.as_slice(), value, &self.backend.writeopts)?;
        }
        Ok(old_value.map(Into::into))
    }

    #[tracing::instrument(skip(self, key, batch))]
    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Removing from RocksDB: {:?}", key);
        let idx = if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            bonsai_cf_index(self.column_mapping.map(key).rocksdb_name)
        } else {
            None
        };
        if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            if let Some(idx) = idx {
                BONSAI_DB_OPS.remove_calls[idx].fetch_add(1, Ordering::Relaxed);
                BONSAI_DB_OPS.remove_key_bytes[idx].fetch_add(key.as_slice().len() as u64, Ordering::Relaxed);
            }
        }
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        let old_value = self.backend.db.get_cf(&handle, key.as_slice())?;
        if let (Some(idx), Some(old)) = (idx, old_value.as_ref()) {
            BONSAI_DB_OPS.remove_old_value_present[idx].fetch_add(1, Ordering::Relaxed);
            BONSAI_DB_OPS.remove_old_value_bytes[idx]
                .fetch_add(old.len().try_into().unwrap_or(u64::MAX), Ordering::Relaxed);
        }
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.backend.db.delete_cf_opt(&handle, key.as_slice(), &self.backend.writeopts)?;
        }
        Ok(old_value.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix))]
    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", prefix);
        let handle = self.backend.get_column(self.column_mapping.map(prefix).clone());
        let iter = self.backend.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        let mut batch = self.create_batch();
        let mut removed: u64 = 0;
        for kv in iter {
            if let Ok((key, _)) = kv {
                if key.starts_with(prefix.as_slice()) {
                    batch.delete_cf(&handle, &key);
                    removed += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            if let Some(idx) = bonsai_cf_index(self.column_mapping.map(prefix).rocksdb_name) {
                BONSAI_DB_OPS.iter_calls[idx].fetch_add(1, Ordering::Relaxed);
                BONSAI_DB_OPS.iter_keys[idx].fetch_add(removed, Ordering::Relaxed);
                BONSAI_DB_OPS.remove_calls[idx].fetch_add(removed, Ordering::Relaxed);
            }
        }
        drop(handle);
        self.write_batch(batch)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, batch))]
    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        if BONSAI_DB_OPS.enabled.load(Ordering::Relaxed) {
            BONSAI_DB_OPS.write_batch_calls.fetch_add(1, Ordering::Relaxed);
        }
        Ok(self.backend.db.write_opt(batch, &self.backend.writeopts)?)
    }
}

fn to_changed_key(k: &DatabaseKey) -> (u8, ByteVec) {
    (
        match k {
            DatabaseKey::Trie(_) => 0,
            DatabaseKey::Flat(_) => 1,
            DatabaseKey::TrieLog(_) => 2,
        },
        k.as_slice().into(),
    )
}

/// The backing database for a bonsai storage view. This is used
/// to implement historical access (for storage proofs), by applying
/// changes from the trie-log without modifying the real database.
///
/// This is kind of a hack for now. This abstraction shouldn't look like
/// this at all ideally, and it should probably be an implementation
/// detail of bonsai-trie.
pub struct BonsaiTransaction {
    /// Backing snapshot. If the value has not been changed, it'll be queried from
    /// here.
    snapshot: SnapshotRef,
    /// The changes on top of the snapshot.
    /// Key is (column id, key) and value is Some(value) if the change is an insert, and None
    /// if the change is a deletion of the key.
    changed: BTreeMap<(u8, ByteVec), Option<ByteVec>>,
    column_mapping: DatabaseKeyMapping,
}

impl fmt::Debug for BonsaiTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BonsaiTransaction {{}}")
    }
}

// TODO: a lot of this is not really used yet, this whole abstraction does not really make sense anyway, this needs to be modified
// upstream in bonsai-trie
impl BonsaiDatabase for BonsaiTransaction {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Default::default()
    }

    #[tracing::instrument(skip(self, key))]
    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Getting from RocksDB: {:?}", key);
        if let Some(val) = self.changed.get(&to_changed_key(key)) {
            return Ok(val.clone());
        }
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.db.db.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn get_by_prefix(&self, _prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        unreachable!("unused for now")
    }

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.db.db.get_cf(&handle, key.as_slice())?.is_some())
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        self.changed.insert(to_changed_key(key), Some(value.into()));
        Ok(None)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        self.changed.insert(to_changed_key(key), None);
        Ok(None)
    }

    fn remove_by_prefix(&mut self, _prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        unreachable!("unused yet")
    }

    fn write_batch(&mut self, _batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        Ok(())
    }
}

impl BonsaiPersistentDatabase<BasicId> for BonsaiDB {
    type Transaction<'a>
        = BonsaiTransaction
    where
        Self: 'a;
    type DatabaseError = TrieError;

    /// this is called upstream, but we ignore it for now because we create the snapshot in [`crate::MadaraBackend::store_block`]
    #[tracing::instrument(skip(self))]
    fn snapshot(&mut self, id: BasicId) {}

    #[tracing::instrument(skip(self))]
    fn transaction(&self, requested_id: BasicId) -> Option<(BasicId, Self::Transaction<'_>)> {
        tracing::trace!("Generating RocksDB transaction");
        let (id, snapshot) = self.snapshots.get_closest(requested_id.as_u64());

        tracing::debug!("Snapshot for requested block_id={requested_id:?} => got block_id={id:?}");

        id.map(|id| {
            (
                BasicId::new(id),
                BonsaiTransaction {
                    snapshot,
                    column_mapping: self.column_mapping.clone(),
                    changed: Default::default(),
                },
            )
        })
    }

    fn merge<'a>(&mut self, _transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        unreachable!("unused for now")
    }
}
