use crate::rocksdb::column::Column;
use crate::rocksdb::snapshots::{SnapshotRef, Snapshots};
use crate::rocksdb::{RocksDBStorage, RocksDBStorageInner, WriteBatchWithTransaction};
use bonsai_trie::{
    id::Id, BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage, BonsaiStorageConfig, ByteVec, DBError, DatabaseKey,
};
use rocksdb::{Direction, IteratorMode};
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub const BONSAI_CONTRACT_FLAT_COLUMN: Column = Column::new("bonsai_contract_flat").set_point_lookup();
pub const BONSAI_CONTRACT_TRIE_COLUMN: Column = Column::new("bonsai_contract_trie").set_point_lookup();
pub const BONSAI_CONTRACT_LOG_COLUMN: Column = Column::new("bonsai_contract_log").set_log_cf();
pub const BONSAI_CONTRACT_STORAGE_FLAT_COLUMN: Column = Column::new("bonsai_contract_storage_flat").set_point_lookup();
pub const BONSAI_CONTRACT_STORAGE_TRIE_COLUMN: Column = Column::new("bonsai_contract_storage_trie").set_point_lookup();
pub const BONSAI_CONTRACT_STORAGE_LOG_COLUMN: Column = Column::new("bonsai_contract_storage_log").set_log_cf();
pub const BONSAI_CLASS_FLAT_COLUMN: Column = Column::new("bonsai_class_flat").set_point_lookup();
pub const BONSAI_CLASS_TRIE_COLUMN: Column = Column::new("bonsai_class_trie").set_point_lookup();
pub const BONSAI_CLASS_LOG_COLUMN: Column = Column::new("bonsai_class_log").set_log_cf();

pub type GlobalTrie<H> = BonsaiStorage<BasicId, BonsaiDB, H>;

pub use bonsai_trie::id::BasicId;
pub use bonsai_trie::ProofNode;

const TRIE_LOG_KEY_SEPARATOR: u8 = 0x00;
const TRIE_LOG_NEW_VALUE: u8 = 0x00;
const TRIE_LOG_OLD_VALUE: u8 = 0x01;
const TRIE_KEY_TYPE_TRIE: u8 = 0x00;
const TRIE_KEY_TYPE_FLAT: u8 = 0x01;

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
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        Ok(self.backend.db.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    #[tracing::instrument(skip(self, prefix))]
    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        tracing::trace!("Getting by prefix from RocksDB: {:?}", prefix);
        let handle = self.backend.get_column(self.column_mapping.map(prefix).clone());
        let iter = self.backend.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        Ok(iter
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix.as_slice()) {
                        // nb: to_vec on a Box<[u8]> is a noop conversion
                        Some((key.to_vec().into(), value.to_vec().into()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect())
    }

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        Ok(self.backend.db.get_cf(&handle, key.as_slice()).map(|value| value.is_some())?)
    }

    #[tracing::instrument(skip(self, key, value, batch))]
    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Inserting into RocksDB: {:?} {:?}", key, value);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());

        let old_value = self.backend.db.get_cf(&handle, key.as_slice())?;
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
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        let old_value = self.backend.db.get_cf(&handle, key.as_slice())?;
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
        for kv in iter {
            if let Ok((key, _)) = kv {
                if key.starts_with(prefix.as_slice()) {
                    batch.delete_cf(&handle, &key);
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        drop(handle);
        self.write_batch(batch)?;
        Ok(())
    }

    #[tracing::instrument(skip(self, batch))]
    fn write_batch(&mut self, batch: Self::Batch) -> Result<(), Self::DatabaseError> {
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

fn parse_trie_log_key(id_len: usize, key: &[u8]) -> Option<(u8, ByteVec, u8)> {
    if key.len() < id_len + 3 || key.get(id_len) != Some(&TRIE_LOG_KEY_SEPARATOR) {
        return None;
    }

    let change_type = *key.last()?;
    let key_type = key[key.len() - 2];
    let trie_key = key[id_len + 1..key.len() - 2].into();

    Some((key_type, trie_key, change_type))
}

fn apply_trie_log_revert(transaction: &mut BonsaiTransaction, id_len: usize, changes: Vec<(ByteVec, ByteVec)>) -> bool {
    let mut grouped_changes: BTreeMap<(u8, ByteVec), (Option<ByteVec>, bool)> = BTreeMap::new();

    for (key, value) in changes {
        let Some((key_type, trie_key, change_type)) = parse_trie_log_key(id_len, &key) else {
            tracing::warn!("Invalid trie log key format");
            return false;
        };

        let (old_value, has_new_value) = grouped_changes.entry((key_type, trie_key)).or_default();
        match change_type {
            TRIE_LOG_OLD_VALUE => *old_value = Some(value),
            TRIE_LOG_NEW_VALUE => *has_new_value = true,
            _ => {
                tracing::warn!("Invalid trie log change type: {change_type}");
                return false;
            }
        }
    }

    for ((key_type, trie_key), (old_value, has_new_value)) in grouped_changes {
        let key = match key_type {
            TRIE_KEY_TYPE_TRIE => DatabaseKey::Trie(&trie_key),
            TRIE_KEY_TYPE_FLAT => DatabaseKey::Flat(&trie_key),
            _ => {
                tracing::warn!("Invalid trie key type: {key_type}");
                return false;
            }
        };

        match old_value {
            Some(old_value) => {
                if transaction.insert(&key, &old_value, None).is_err() {
                    return false;
                }
            }
            None if has_new_value => {
                if transaction.remove(&key, None).is_err() {
                    return false;
                }
            }
            None => {}
        }
    }

    true
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
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn get_by_prefix(&self, _prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        unreachable!("unused for now")
    }

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        if let Some(value) = self.changed.get(&to_changed_key(key)) {
            return Ok(value.is_some());
        }
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.is_some())
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
        let requested_block_n = requested_id.as_u64();
        let (snapshot_id, snapshot) = self.snapshots.get_closest(requested_block_n);

        tracing::debug!("Snapshot for requested block_id={requested_id:?} => got block_id={snapshot_id:?}");

        let mut transaction =
            BonsaiTransaction { snapshot, column_mapping: self.column_mapping.clone(), changed: Default::default() };

        let snapshot_block_n = snapshot_id?;
        if snapshot_block_n < requested_block_n {
            tracing::warn!(
                "Snapshot for requested block_id={requested_id:?} is older than the requested block: {snapshot_block_n}"
            );
            return None;
        }

        for block_n in ((requested_block_n + 1)..=snapshot_block_n).rev() {
            let id = BasicId::new(block_n);
            let id_bytes = id.to_bytes();
            let changes = self.get_by_prefix(&DatabaseKey::TrieLog(&id_bytes)).ok()?;
            if !apply_trie_log_revert(&mut transaction, id_bytes.len(), changes) {
                return None;
            }
        }

        Some((requested_id, transaction))
    }

    fn merge<'a>(&mut self, _transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        unreachable!("unused for now")
    }
}
