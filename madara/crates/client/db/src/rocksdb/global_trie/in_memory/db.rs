use super::overlay::OVERLAY_TRIE_LOG_COLUMN_ID;
use crate::prelude::*;
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
use rocksdb::{Direction, IteratorMode};
use std::fmt;
use std::sync::Arc;

pub(crate) const OVERLAY_TRIE_COLUMN_ID: u8 = 0;
pub(crate) const OVERLAY_FLAT_COLUMN_ID: u8 = 1;

pub(crate) type OverlayKey = (u8, ByteVec);
pub(crate) type OverlayMap = Arc<DashMap<OverlayKey, Option<ByteVec>>>;

#[derive(Clone, Debug)]
pub(crate) struct InMemoryColumnMapping {
    flat: Column,
    trie: Column,
    log: Column,
}

impl InMemoryColumnMapping {
    pub(crate) fn contract() -> Self {
        Self { flat: BONSAI_CONTRACT_FLAT_COLUMN, trie: BONSAI_CONTRACT_TRIE_COLUMN, log: BONSAI_CONTRACT_LOG_COLUMN }
    }

    pub(crate) fn contract_storage() -> Self {
        Self {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        }
    }

    pub(crate) fn class() -> Self {
        Self { flat: BONSAI_CLASS_FLAT_COLUMN, trie: BONSAI_CLASS_TRIE_COLUMN, log: BONSAI_CLASS_LOG_COLUMN }
    }

    fn map(&self, key: &DatabaseKey) -> &Column {
        match key {
            DatabaseKey::Trie(_) => &self.trie,
            DatabaseKey::Flat(_) => &self.flat,
            DatabaseKey::TrieLog(_) => &self.log,
        }
    }

    pub(crate) fn map_from_column_id(&self, column_id: u8) -> Option<&Column> {
        match column_id {
            OVERLAY_TRIE_COLUMN_ID => Some(&self.trie),
            OVERLAY_FLAT_COLUMN_ID => Some(&self.flat),
            OVERLAY_TRIE_LOG_COLUMN_ID => Some(&self.log),
            _ => None,
        }
    }
}

fn to_changed_key(key: &DatabaseKey) -> OverlayKey {
    (
        match key {
            DatabaseKey::Trie(_) => OVERLAY_TRIE_COLUMN_ID,
            DatabaseKey::Flat(_) => OVERLAY_FLAT_COLUMN_ID,
            DatabaseKey::TrieLog(_) => OVERLAY_TRIE_LOG_COLUMN_ID,
        },
        key.as_slice().into(),
    )
}

#[derive(Clone)]
pub(crate) struct InMemoryBonsaiDb {
    snapshot: SnapshotRef,
    changed: OverlayMap,
    column_mapping: InMemoryColumnMapping,
}

impl fmt::Debug for InMemoryBonsaiDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InMemoryBonsaiDb {{ changed_len: {} }}", self.changed.len())
    }
}

impl InMemoryBonsaiDb {
    pub(crate) fn with_mapping(
        snapshot: SnapshotRef,
        column_mapping: InMemoryColumnMapping,
        changed: OverlayMap,
    ) -> Self {
        Self { snapshot, changed, column_mapping }
    }

    pub(crate) fn contract(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::contract(), Arc::clone(&changed)), changed)
    }

    pub(crate) fn contract_storage(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::contract_storage(), Arc::clone(&changed)), changed)
    }

    pub(crate) fn class(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::class(), Arc::clone(&changed)), changed)
    }

    #[cfg(test)]
    pub(crate) fn test_with_mapping(snapshot: SnapshotRef, column_mapping: InMemoryColumnMapping) -> Self {
        Self::with_mapping(snapshot, column_mapping, Arc::new(DashMap::new()))
    }

    fn get_from_snapshot(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, TrieError> {
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn changed_value(&self, key: &DatabaseKey) -> Option<Option<ByteVec>> {
        self.changed.get(&to_changed_key(key)).map(|v| v.value().clone())
    }

    fn read_overlay_or_snapshot(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, TrieError> {
        if let Some(value) = self.changed_value(key) {
            return Ok(value);
        }
        self.get_from_snapshot(key)
    }

    fn scan_snapshot_prefix(&self, column: &Column, prefix_bytes: &[u8]) -> Vec<(ByteVec, ByteVec)> {
        let handle = self.snapshot.db.get_column(column.clone());
        self.snapshot
            .db
            .db
            .iterator_cf(&handle, IteratorMode::From(prefix_bytes, Direction::Forward))
            .map_while(|kv| {
                if let Ok((key, value)) = kv {
                    if key.starts_with(prefix_bytes) {
                        Some((key.to_vec().into(), value.to_vec().into()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    fn merge_overlay_prefix(&self, prefix_col: u8, prefix_bytes: &[u8], out: &mut Vec<(ByteVec, ByteVec)>) {
        for entry in self.changed.iter() {
            let ((column_id, key), value) = entry.pair();
            if *column_id != prefix_col || !key.starts_with(prefix_bytes) {
                continue;
            }

            match value {
                Some(v) => {
                    if let Some((_, existing)) =
                        out.iter_mut().find(|(existing_key, _)| existing_key.as_slice() == key.as_slice())
                    {
                        *existing = v.clone();
                    } else {
                        out.push((key.clone(), v.clone()));
                    }
                }
                None => out.retain(|(existing_key, _)| existing_key.as_slice() != key.as_slice()),
            }
        }
    }

    fn overlay_keys_for_prefix(&self, prefix_col: u8, prefix_bytes: &[u8]) -> Vec<OverlayKey> {
        self.changed
            .iter()
            .map(|entry| entry.key().clone())
            .filter(|(column_id, key)| *column_id == prefix_col && key.starts_with(prefix_bytes))
            .collect()
    }
}

impl BonsaiDatabase for InMemoryBonsaiDb {
    type Batch = WriteBatchWithTransaction;
    type DatabaseError = TrieError;

    fn create_batch(&self) -> Self::Batch {
        Self::Batch::default()
    }

    fn get(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, Self::DatabaseError> {
        self.read_overlay_or_snapshot(key)
    }

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        // We only need this for trie-log pruning in commit paths. We read from live DB here,
        // and rely on overlay-first entries to override current values for matching keys.
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(Vec::new());
        };
        let mut out = self.scan_snapshot_prefix(column, prefix_bytes.as_slice());
        self.merge_overlay_prefix(prefix_col, prefix_bytes.as_slice(), &mut out);

        Ok(out)
    }

    fn contains(&self, key: &DatabaseKey) -> Result<bool, Self::DatabaseError> {
        Ok(self.read_overlay_or_snapshot(key)?.is_some())
    }

    fn insert(
        &mut self,
        key: &DatabaseKey,
        value: &[u8],
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.read_overlay_or_snapshot(key)?;
        self.changed.insert(to_changed_key(key), Some(value.into()));
        Ok(previous)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.read_overlay_or_snapshot(key)?;
        self.changed.insert(to_changed_key(key), None);
        Ok(previous)
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(());
        };

        for (key, _) in self.scan_snapshot_prefix(column, prefix_bytes.as_slice()) {
            self.changed.insert((prefix_col, key), None);
        }

        for key in self.overlay_keys_for_prefix(prefix_col, prefix_bytes.as_slice()) {
            self.changed.insert(key, None);
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
