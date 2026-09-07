use crate::rocksdb::column::Column;
use crate::rocksdb::snapshots::SnapshotRef;
use crate::rocksdb::trie::{
    BasicId, TrieError, BONSAI_CLASS_FLAT_COLUMN, BONSAI_CLASS_LOG_COLUMN, BONSAI_CLASS_TRIE_COLUMN,
    BONSAI_CONTRACT_FLAT_COLUMN, BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
    BONSAI_CONTRACT_STORAGE_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_TRIE_COLUMN, BONSAI_CONTRACT_TRIE_COLUMN,
};
use crate::rocksdb::WriteBatchWithTransaction;
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, ByteVec, DatabaseKey};
use dashmap::DashMap;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

const OVERLAY_TRIE_COLUMN_ID: u8 = 0;
const OVERLAY_FLAT_COLUMN_ID: u8 = 1;
pub(super) const OVERLAY_TRIE_LOG_COLUMN_ID: u8 = 2;

/// Logical Bonsai column identifier and raw key bytes.
pub type OverlayKey = (u8, ByteVec);
/// Shared per-job changes: absent keys fall back to the snapshot, `None` values are tombstones.
pub type OverlayMap = Arc<DashMap<OverlayKey, Option<ByteVec>>>;

/// Maps one logical Bonsai trie to its durable flat, trie, and log columns.
#[derive(Clone, Debug)]
pub struct InMemoryColumnMapping {
    pub(super) flat: Column,
    pub(super) trie: Column,
    pub(super) log: Column,
}

impl InMemoryColumnMapping {
    /// Returns the Bonsai column mapping for the global contract trie.
    /// Trie, flat, and log identifiers retain the durable RocksDB layout.
    pub fn contract() -> Self {
        Self { flat: BONSAI_CONTRACT_FLAT_COLUMN, trie: BONSAI_CONTRACT_TRIE_COLUMN, log: BONSAI_CONTRACT_LOG_COLUMN }
    }

    /// Returns the Bonsai column mapping for per-contract storage tries.
    /// Each in-memory overlay key can therefore be resolved back to its durable column.
    pub fn contract_storage() -> Self {
        Self {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        }
    }

    /// Returns the Bonsai column mapping for the global class trie.
    /// Trie, flat, and log identifiers retain the durable RocksDB layout.
    pub fn class() -> Self {
        Self { flat: BONSAI_CLASS_FLAT_COLUMN, trie: BONSAI_CLASS_TRIE_COLUMN, log: BONSAI_CLASS_LOG_COLUMN }
    }

    /// Maps a logical Bonsai key variant to its durable RocksDB column.
    /// All overlay reads use this mapping before consulting the pinned snapshot.
    pub(super) fn map(&self, key: &DatabaseKey) -> &Column {
        match key {
            DatabaseKey::Trie(_) => &self.trie,
            DatabaseKey::Flat(_) => &self.flat,
            DatabaseKey::TrieLog(_) => &self.log,
        }
    }

    /// Resolves the compact overlay column identifier into its durable RocksDB column.
    /// Unknown identifiers return `None` so corrupted overlay data cannot be misrouted.
    pub(super) fn map_from_column_id(&self, column_id: u8) -> Option<&Column> {
        match column_id {
            OVERLAY_TRIE_COLUMN_ID => Some(&self.trie),
            OVERLAY_FLAT_COLUMN_ID => Some(&self.flat),
            OVERLAY_TRIE_LOG_COLUMN_ID => Some(&self.log),
            _ => None,
        }
    }
}

/// Converts a Bonsai database key into the overlay's compact column-and-bytes representation.
/// The encoded column identifier is later resolved through [`InMemoryColumnMapping`].
pub(super) fn to_changed_key(key: &DatabaseKey) -> OverlayKey {
    (
        match key {
            DatabaseKey::Trie(_) => OVERLAY_TRIE_COLUMN_ID,
            DatabaseKey::Flat(_) => OVERLAY_FLAT_COLUMN_ID,
            DatabaseKey::TrieLog(_) => OVERLAY_TRIE_LOG_COLUMN_ID,
        },
        key.as_slice().into(),
    )
}

/// Snapshot-backed Bonsai database whose clones share one root job's mutable overlay.
/// Separate root jobs must construct separate overlays.
#[derive(Clone)]
pub struct InMemoryBonsaiDb {
    snapshot: SnapshotRef,
    pub(super) changed: OverlayMap,
    pub(super) column_mapping: InMemoryColumnMapping,
    track_old_values: bool,
}

impl fmt::Debug for InMemoryBonsaiDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InMemoryBonsaiDb {{ changed_len: {} }}", self.changed.len())
    }
}

impl InMemoryBonsaiDb {
    /// Creates an overlay DB and controls whether writes retain values needed by trie logs.
    pub fn with_mapping(
        snapshot: SnapshotRef,
        column_mapping: InMemoryColumnMapping,
        changed: OverlayMap,
        track_old_values: bool,
    ) -> Self {
        Self { snapshot, changed, column_mapping, track_old_values }
    }

    /// Creates the contract-trie overlay used by one root computation.
    pub fn contract(snapshot: SnapshotRef, track_old_values: bool) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (
            Self::with_mapping(snapshot, InMemoryColumnMapping::contract(), Arc::clone(&changed), track_old_values),
            changed,
        )
    }

    /// Creates the contract-storage-trie overlay used by one root computation.
    pub fn contract_storage(snapshot: SnapshotRef, track_old_values: bool) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (
            Self::with_mapping(
                snapshot,
                InMemoryColumnMapping::contract_storage(),
                Arc::clone(&changed),
                track_old_values,
            ),
            changed,
        )
    }

    /// Creates the class-trie overlay used by one root computation.
    pub fn class(snapshot: SnapshotRef, track_old_values: bool) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::class(), Arc::clone(&changed), track_old_values), changed)
    }

    #[cfg(test)]
    pub(super) fn test_with_mapping(snapshot: SnapshotRef, column_mapping: InMemoryColumnMapping) -> Self {
        Self::with_mapping(snapshot, column_mapping, Arc::new(DashMap::new()), true)
    }

    /// Reads a key from the immutable snapshot backing this overlay.
    /// Overlay changes are intentionally ignored by this lower-level lookup.
    fn get_from_snapshot(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, TrieError> {
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(ByteVec::from))
    }

    /// Returns the overlay's tri-state value for a key: absent, deleted, or replaced.
    /// Callers use the outer option to distinguish an untouched key from a deletion.
    fn changed_value(&self, key: &DatabaseKey) -> Option<Option<ByteVec>> {
        self.changed.get(&to_changed_key(key)).map(|v| v.value().clone())
    }

    /// Reads a previous value only when the caller will retain a rollback log.
    fn previous_value_for_write(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, TrieError> {
        if self.track_old_values {
            self.get(key)
        } else {
            Ok(None)
        }
    }
}

impl BonsaiDatabase for InMemoryBonsaiDb {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        if let Some(value) = self.changed_value(key) {
            return Ok(value);
        }
        self.get_from_snapshot(key)
    }

    /// Merges snapshot rows and in-memory overlay changes for one logical Bonsai prefix.
    /// Overlay deletions remove snapshot rows, and the returned key order is deterministic.
    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(Vec::new());
        };
        let mut readopts = ReadOptions::default();
        readopts.set_prefix_same_as_start(true);

        // ponytail: BTreeMap handles replacement, deletion, and sorted output without repeated scans.
        let mut out = BTreeMap::<ByteVec, ByteVec>::new();
        for item in self
            .snapshot
            .iterator_cf(column.clone(), readopts, IteratorMode::From(prefix_bytes.as_slice(), Direction::Forward))
            .into_iter_items(|(key, value)| (ByteVec::from(key), ByteVec::from(value)))
        {
            let (key, value) = item?;
            if !key.starts_with(prefix_bytes.as_slice()) {
                break;
            }
            out.insert(key, value);
        }
        for entry in self.changed.iter() {
            let ((column_id, key), value) = entry.pair();
            if *column_id != prefix_col || !key.starts_with(prefix_bytes.as_slice()) {
                continue;
            }

            match value {
                Some(value) => {
                    out.insert(key.clone(), value.clone());
                }
                None => {
                    out.remove(key);
                }
            }
        }

        Ok(out.into_iter().collect())
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        if let Some(value) = self.changed_value(key) {
            return Ok(value.is_some());
        }
        Ok(self.get_from_snapshot(key)?.is_some())
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.previous_value_for_write(key)?;
        self.changed.insert(to_changed_key(key), Some(value.into()));
        Ok(previous)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.previous_value_for_write(key)?;
        self.changed.insert(to_changed_key(key), None);
        Ok(previous)
    }

    /// Marks every snapshot and overlay key under a logical prefix as deleted in memory.
    /// No RocksDB mutation occurs until the completed overlay is explicitly flushed.
    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(());
        };
        let mut readopts = ReadOptions::default();
        readopts.set_prefix_same_as_start(true);

        for item in self
            .snapshot
            .iterator_cf(column.clone(), readopts, IteratorMode::From(prefix_bytes.as_slice(), Direction::Forward))
            .into_iter_keys(|key| ByteVec::from(key))
        {
            let key = item?;
            if !key.starts_with(prefix_bytes.as_slice()) {
                break;
            }
            self.changed.insert((prefix_col, key), None);
        }

        for mut entry in self.changed.iter_mut() {
            let (column_id, key) = entry.key();
            if *column_id == prefix_col && key.starts_with(prefix_bytes.as_slice()) {
                *entry.value_mut() = None;
            }
        }

        Ok(())
    }

    fn write_batch(&mut self, _batch: Self::Batch) -> Result<(), Self::DatabaseError> {
        // Intentionally a no-op: all writes stay in overlay until explicit flush.
        Ok(())
    }
}

impl BonsaiPersistentDatabase<BasicId> for InMemoryBonsaiDb {
    type Transaction<'a>
        = Self
    where
        Self: 'a;
    type DatabaseError = TrieError;

    fn snapshot(&mut self, _id: BasicId) {}

    fn transaction(&self, _id: BasicId) -> Option<(BasicId, Self::Transaction<'_>)> {
        None
    }

    fn merge<'a>(&mut self, _transaction: Self::Transaction<'a>) -> Result<(), Self::DatabaseError>
    where
        Self: 'a,
    {
        unreachable!("merge is not supported for in-memory overlay db")
    }
}
