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
pub enum TrieError {
    #[error(transparent)]
    RocksDb(#[from] rocksdb::Error),
    #[error("Cannot delete an unbounded trie-log prefix")]
    UnboundedPrefix,
}
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
        self.get_bonsai_with_revert_mode(column_mapping, false)
    }

    /// Creates a Bonsai handle whose prefix removals and inverse writes are committed together.
    fn get_bonsai_for_revert<H: StarkHash + Send + Sync>(
        &self,
        column_mapping: DatabaseKeyMapping,
    ) -> BonsaiStorage<BasicId, BonsaiDB, H> {
        self.get_bonsai_with_revert_mode(column_mapping, true)
    }

    /// Creates a Bonsai handle with the requested database behavior.
    fn get_bonsai_with_revert_mode<H: StarkHash + Send + Sync>(
        &self,
        column_mapping: DatabaseKeyMapping,
        revert_mode: bool,
    ) -> BonsaiStorage<BasicId, BonsaiDB, H> {
        BonsaiStorage::new(
            BonsaiDB {
                backend: self.inner.clone(),
                column_mapping,
                snapshots: self.snapshots.clone(),
                deferred_prefix_deletes: revert_mode.then(Vec::new),
            },
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

    /// Opens the contract trie with atomic, history-free inverse writes for rollback.
    pub(crate) fn contract_trie_for_revert(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai_for_revert(DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_TRIE_COLUMN,
            log: BONSAI_CONTRACT_LOG_COLUMN,
        })
    }

    /// Opens the contract-storage trie with atomic, history-free inverse writes for rollback.
    pub(crate) fn contract_storage_trie_for_revert(&self) -> GlobalTrie<Pedersen> {
        self.get_bonsai_for_revert(DatabaseKeyMapping {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        })
    }

    /// Opens the class trie with atomic, history-free inverse writes for rollback.
    pub(crate) fn class_trie_for_revert(&self) -> GlobalTrie<Poseidon> {
        self.get_bonsai_for_revert(DatabaseKeyMapping {
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

/// Decodes the block revision carried by a trie-log prefix for observability.
fn trie_log_revision(prefix: &DatabaseKey) -> Option<u64> {
    let DatabaseKey::TrieLog(prefix) = prefix else { return None };
    let revision: [u8; 8] = prefix.get(..8)?.try_into().ok()?;
    Some(u64::from_be_bytes(revision))
}

/// Maps a range-delete write result to its stable metrics label.
fn prefix_prune_outcome(write_succeeded: bool) -> &'static str {
    if write_succeeded {
        "success"
    } else {
        "write_error"
    }
}

/// Records one logical revision-prefix deletion after its RocksDB write resolves.
struct PrefixPruneObservation {
    column: &'static str,
    revision: Option<u64>,
    started_at: Instant,
    entries_scanned: u64,
    batch_operations: u64,
    batch_bytes: u64,
    strategy: &'static str,
}

/// Holds a rollback log-range deletion until the inverse state batch is ready.
struct DeferredPrefixDelete {
    column: Column,
    start: ByteVec,
    end: ByteVec,
    observation: PrefixPruneObservation,
}

impl PrefixPruneObservation {
    /// Emits range-deletion cost and the latest successfully deleted revision.
    fn record(self, write_succeeded: bool) {
        let outcome = prefix_prune_outcome(write_succeeded);
        let attributes = [
            KeyValue::new("column", self.column),
            KeyValue::new("strategy", self.strategy),
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
    /// Rollback-only range deletions applied with the inverse-write batch.
    deferred_prefix_deletes: Option<Vec<DeferredPrefixDelete>>,
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
        let mut readopts = rocksdb::ReadOptions::default();
        readopts.set_prefix_same_as_start(true);
        let iter = self.backend.db.iterator_cf_opt(
            &handle,
            readopts,
            IteratorMode::From(prefix.as_slice(), Direction::Forward),
        );
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

        let old_value = if self.deferred_prefix_deletes.is_some() {
            None
        } else {
            self.backend.db.get_cf(&handle, key.as_slice())?.map(Into::into)
        };
        if let Some(batch) = batch {
            batch.put_cf(&handle, key.as_slice(), value);
        } else {
            self.backend.db.put_cf_opt(&handle, key.as_slice(), value, &self.backend.writeopts)?;
        }
        Ok(old_value)
    }

    #[tracing::instrument(skip(self, key, batch))]
    fn remove(
        &mut self,
        key: &DatabaseKey,
        batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        tracing::trace!("Removing from RocksDB: {:?}", key);
        let handle = self.backend.get_column(self.column_mapping.map(key).clone());
        let old_value = if self.deferred_prefix_deletes.is_some() {
            None
        } else {
            self.backend.db.get_cf(&handle, key.as_slice())?.map(Into::into)
        };
        if let Some(batch) = batch {
            batch.delete_cf(&handle, key.as_slice());
        } else {
            self.backend.db.delete_cf_opt(&handle, key.as_slice(), &self.backend.writeopts)?;
        }
        Ok(old_value)
    }

    #[tracing::instrument(skip(self, prefix))]
    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let started_at = Instant::now();
        let column = self.column_mapping.map(prefix).clone();
        if let Some(deferred_prefix_deletes) = self.deferred_prefix_deletes.as_mut() {
            let Some(end) = prefix_upper_bound(prefix.as_slice()) else {
                return Err(TrieError::UnboundedPrefix);
            };
            let batch_bytes = u64::try_from(prefix.as_slice().len() + end.len()).unwrap_or(u64::MAX);
            deferred_prefix_deletes.push(DeferredPrefixDelete {
                column: column.clone(),
                start: prefix.as_slice().into(),
                end: end.into(),
                observation: PrefixPruneObservation {
                    column: column.rocksdb_name,
                    revision: trie_log_revision(prefix),
                    started_at,
                    entries_scanned: 0,
                    batch_operations: 1,
                    batch_bytes,
                    strategy: "range_delete_deferred",
                },
            });
            return Ok(());
        }

        tracing::trace!("Deleting RocksDB prefix range: {:?}", prefix);
        let Some(end) = prefix_upper_bound(prefix.as_slice()) else {
            return Err(TrieError::UnboundedPrefix);
        };
        let handle = self.backend.get_column(column.clone());
        let mut batch = self.create_batch();
        batch.delete_range_cf(&handle, prefix.as_slice(), &end);
        let observation = PrefixPruneObservation {
            column: column.rocksdb_name,
            revision: trie_log_revision(prefix),
            started_at,
            entries_scanned: 0,
            batch_operations: 1,
            batch_bytes: u64::try_from(batch.size_in_bytes()).unwrap_or(u64::MAX),
            strategy: "range_delete",
        };
        drop(handle);
        let result = self.write_batch(batch);
        observation.record(result.is_ok());
        result
    }

    #[tracing::instrument(skip(self, batch))]
    fn write_batch(&mut self, mut batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        let deferred_prefix_deletes = self.deferred_prefix_deletes.as_mut().map(std::mem::take).unwrap_or_default();
        for deletion in &deferred_prefix_deletes {
            let handle = self.backend.get_column(deletion.column.clone());
            batch.delete_range_cf(&handle, &deletion.start, &deletion.end);
        }
        let result = self.backend.db.write_opt(batch, &self.backend.writeopts);
        for deletion in deferred_prefix_deletes {
            deletion.observation.record(result.is_ok());
        }
        Ok(result?)
    }
}

/// Returns the smallest lexicographic key strictly above every key with `prefix`.
fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    for index in (0..end.len()).rev() {
        if end[index] != u8::MAX {
            end[index] += 1;
            end.truncate(index + 1);
            return Some(end);
        }
    }
    None
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
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn get_by_prefix(&self, _prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        unreachable!("unused for now")
    }

    #[tracing::instrument(skip(self, key))]
    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        tracing::trace!("Checking if RocksDB contains: {:?}", key);
        if let Some(val) = self.changed.get(&to_changed_key(key)) {
            return Ok(val.is_some());
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
    fn prefix_prune_outcome_reports_write_result() {
        assert_eq!(prefix_prune_outcome(true), "success");
        assert_eq!(prefix_prune_outcome(false), "write_error");
    }

    #[test]
    fn prefix_upper_bound_covers_exact_prefix_only() {
        assert_eq!(prefix_upper_bound(&[0x00, 0x12, 0x34]), Some(vec![0x00, 0x12, 0x35]));
        assert_eq!(prefix_upper_bound(&[0x00, 0x12, 0xff]), Some(vec![0x00, 0x13]));
        assert_eq!(prefix_upper_bound(&[0xff, 0xff]), None);
    }

    #[test]
    fn remove_by_prefix_range_delete_removes_only_matching_revision() {
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
            deferred_prefix_deletes: None,
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

    #[test]
    fn deferred_range_delete_commits_with_inverse_writes() {
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
            deferred_prefix_deletes: Some(Vec::new()),
        };
        let log_handle = storage.inner.get_column(BONSAI_CONTRACT_LOG_COLUMN);
        let flat_handle = storage.inner.get_column(BONSAI_CONTRACT_FLAT_COLUMN);
        let revision = 7u64.to_be_bytes();
        let next_revision = 8u64.to_be_bytes();
        let reverted_log_key = [revision.as_slice(), b"entry"].concat();
        let retained_log_key = [next_revision.as_slice(), b"entry"].concat();
        storage.inner.db.put_cf(&log_handle, &reverted_log_key, b"change").unwrap();
        storage.inner.db.put_cf(&log_handle, &retained_log_key, b"change").unwrap();

        bonsai_db.remove_by_prefix(&DatabaseKey::TrieLog(&revision)).unwrap();
        assert!(
            storage.inner.db.get_cf(&log_handle, &reverted_log_key).unwrap().is_some(),
            "rollback log deletion must wait for the inverse-write batch"
        );

        let state_key = DatabaseKey::Flat(b"state-key");
        let mut batch = bonsai_db.create_batch();
        bonsai_db.insert(&state_key, b"old-state", Some(&mut batch)).unwrap();
        bonsai_db.write_batch(batch).unwrap();

        assert!(storage.inner.db.get_cf(&log_handle, reverted_log_key).unwrap().is_none());
        assert_eq!(storage.inner.db.get_cf(&log_handle, retained_log_key).unwrap().as_deref(), Some(&b"change"[..]));
        assert_eq!(storage.inner.db.get_cf(&flat_handle, b"state-key").unwrap().as_deref(), Some(&b"old-state"[..]));
    }
}
