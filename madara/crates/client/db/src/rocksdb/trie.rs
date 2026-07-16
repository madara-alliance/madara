use crate::metrics::metrics;
use crate::rocksdb::column::Column;
use crate::rocksdb::snapshots::{SnapshotRef, Snapshots};
use crate::rocksdb::{RocksDBStorage, RocksDBStorageInner, WriteBatchWithTransaction};
use bonsai_trie::id::Id;
use bonsai_trie::{
    BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage, BonsaiStorageConfig, ByteVec, DBError, DatabaseKey,
};
use opentelemetry::KeyValue;
use rocksdb::{Direction, IteratorMode};
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

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

fn trie_log_revision(prefix: &DatabaseKey) -> Option<u64> {
    let DatabaseKey::TrieLog(prefix) = prefix else { return None };
    let revision: [u8; 8] = prefix.get(..8)?.try_into().ok()?;
    Some(u64::from_be_bytes(revision))
}

struct PrefixPruneObservation {
    column: &'static str,
    revision: Option<u64>,
    started_at: Instant,
    entries_scanned: u64,
    batch_operations: u64,
    batch_bytes: u64,
    scan_error: bool,
}

impl PrefixPruneObservation {
    fn record(self, write_succeeded: bool) {
        let outcome = if !write_succeeded {
            "write_error"
        } else if self.scan_error {
            "scan_error"
        } else {
            "success"
        };
        let attributes = [
            KeyValue::new("column", self.column),
            KeyValue::new("strategy", "point_delete"),
            KeyValue::new("outcome", outcome),
        ];

        metrics().bonsai_prefix_prune_operations.add(1, &attributes);
        metrics().bonsai_prefix_prune_entries_scanned.record(self.entries_scanned, &attributes);
        metrics().bonsai_prefix_prune_batch_operations.record(self.batch_operations, &attributes);
        metrics().bonsai_prefix_prune_batch_bytes.record(self.batch_bytes, &attributes);
        metrics().bonsai_prefix_prune_duration.record(self.started_at.elapsed().as_secs_f64(), &attributes);

        if outcome == "success" {
            if let Some(revision) = self.revision {
                metrics().bonsai_prefix_prune_last_revision.record(revision, &[KeyValue::new("column", self.column)]);
            }
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
        let started_at = Instant::now();
        let column = self.column_mapping.map(prefix);
        let handle = self.backend.get_column(column.clone());
        let iter = self.backend.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        let mut batch = self.create_batch();
        let mut entries_scanned = 0u64;
        let mut scan_error = false;
        for kv in iter {
            if let Ok((key, _)) = kv {
                entries_scanned = entries_scanned.saturating_add(1);
                if key.starts_with(prefix.as_slice()) {
                    batch.delete_cf(&handle, &key);
                } else {
                    break;
                }
            } else {
                scan_error = true;
                break;
            }
        }
        let observation = PrefixPruneObservation {
            column: column.rocksdb_name,
            revision: trie_log_revision(prefix),
            started_at,
            entries_scanned,
            batch_operations: u64::try_from(batch.len()).unwrap_or(u64::MAX),
            batch_bytes: u64::try_from(batch.size_in_bytes()).unwrap_or(u64::MAX),
            scan_error,
        };
        drop(handle);
        let result = self.write_batch(batch);
        observation.record(result.is_ok());
        result
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::RocksDBConfig;

    #[test]
    fn trie_log_revision_valid_prefix_returns_revision() {
        let revision = 42u64.to_be_bytes();

        assert_eq!(trie_log_revision(&DatabaseKey::TrieLog(&revision)), Some(42));
        assert_eq!(trie_log_revision(&DatabaseKey::Trie(&revision)), None);
        assert_eq!(trie_log_revision(&DatabaseKey::TrieLog(&revision[..7])), None);
    }

    #[test]
    fn remove_by_prefix_point_delete_removes_only_matching_revision() {
        let directory = tempfile::tempdir().unwrap();
        let storage = RocksDBStorage::open(directory.path(), RocksDBConfig::default()).unwrap();
        let mapping = DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_TRIE_COLUMN,
            log: BONSAI_CONTRACT_LOG_COLUMN,
        };
        let mut bonsai_db = BonsaiDB {
            backend: Arc::clone(&storage.inner),
            snapshots: Arc::clone(&storage.snapshots),
            column_mapping: mapping,
        };
        let handle = storage.inner.get_column(BONSAI_CONTRACT_LOG_COLUMN);

        let revision = 7u64.to_be_bytes();
        let next_revision = 8u64.to_be_bytes();
        let first_key = [revision.as_slice(), b"first"].concat();
        let second_key = [revision.as_slice(), b"second"].concat();
        let next_key = [next_revision.as_slice(), b"next"].concat();
        storage.inner.db.put_cf(&handle, &first_key, b"value").unwrap();
        storage.inner.db.put_cf(&handle, &second_key, b"value").unwrap();
        storage.inner.db.put_cf(&handle, &next_key, b"value").unwrap();

        bonsai_db.remove_by_prefix(&DatabaseKey::TrieLog(&revision)).unwrap();

        assert!(storage.inner.db.get_cf(&handle, first_key).unwrap().is_none());
        assert!(storage.inner.db.get_cf(&handle, second_key).unwrap().is_none());
        assert_eq!(storage.inner.db.get_cf(&handle, next_key).unwrap().as_deref(), Some(b"value".as_slice()));
    }
}
