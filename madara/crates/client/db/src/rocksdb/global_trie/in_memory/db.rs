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

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_lex_sorted(bytes: &[ByteVec]) -> bool {
    bytes.windows(2).all(|pair| pair[0].as_slice() <= pair[1].as_slice())
}

const OVERLAY_TRIE_COLUMN_ID: u8 = 0;
const OVERLAY_FLAT_COLUMN_ID: u8 = 1;
pub(super) const OVERLAY_TRIE_LOG_COLUMN_ID: u8 = 2;

pub type OverlayKey = (u8, ByteVec);
pub type OverlayMap = Arc<DashMap<OverlayKey, Option<ByteVec>>>;

#[derive(Clone, Debug)]
pub struct InMemoryColumnMapping {
    pub(super) flat: Column,
    pub(super) trie: Column,
    pub(super) log: Column,
}

impl InMemoryColumnMapping {
    pub fn contract() -> Self {
        Self { flat: BONSAI_CONTRACT_FLAT_COLUMN, trie: BONSAI_CONTRACT_TRIE_COLUMN, log: BONSAI_CONTRACT_LOG_COLUMN }
    }

    pub fn contract_storage() -> Self {
        Self {
            flat: BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie: BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            log: BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
        }
    }

    pub fn class() -> Self {
        Self { flat: BONSAI_CLASS_FLAT_COLUMN, trie: BONSAI_CLASS_TRIE_COLUMN, log: BONSAI_CLASS_LOG_COLUMN }
    }

    pub(super) fn map(&self, key: &DatabaseKey) -> &Column {
        match key {
            DatabaseKey::Trie(_) => &self.trie,
            DatabaseKey::Flat(_) => &self.flat,
            DatabaseKey::TrieLog(_) => &self.log,
        }
    }

    pub(super) fn map_from_column_id(&self, column_id: u8) -> Option<&Column> {
        match column_id {
            OVERLAY_TRIE_COLUMN_ID => Some(&self.trie),
            OVERLAY_FLAT_COLUMN_ID => Some(&self.flat),
            OVERLAY_TRIE_LOG_COLUMN_ID => Some(&self.log),
            _ => None,
        }
    }
}

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

#[derive(Clone)]
pub struct InMemoryBonsaiDb {
    snapshot: SnapshotRef,
    pub(super) changed: OverlayMap,
    pub(super) column_mapping: InMemoryColumnMapping,
}

impl fmt::Debug for InMemoryBonsaiDb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InMemoryBonsaiDb {{ changed_len: {} }}", self.changed.len())
    }
}

impl InMemoryBonsaiDb {
    pub fn with_mapping(snapshot: SnapshotRef, column_mapping: InMemoryColumnMapping, changed: OverlayMap) -> Self {
        Self { snapshot, changed, column_mapping }
    }

    pub fn contract(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::contract(), Arc::clone(&changed)), changed)
    }

    pub fn contract_storage(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::contract_storage(), Arc::clone(&changed)), changed)
    }

    pub fn class(snapshot: SnapshotRef) -> (Self, OverlayMap) {
        let changed = Arc::new(DashMap::new());
        (Self::with_mapping(snapshot, InMemoryColumnMapping::class(), Arc::clone(&changed)), changed)
    }

    #[cfg(test)]
    pub(super) fn test_with_mapping(snapshot: SnapshotRef, column_mapping: InMemoryColumnMapping) -> Self {
        Self::with_mapping(snapshot, column_mapping, Arc::new(DashMap::new()))
    }

    fn get_from_snapshot(&self, key: &DatabaseKey) -> Result<Option<ByteVec>, TrieError> {
        let handle = self.snapshot.db.get_column(self.column_mapping.map(key).clone());
        Ok(self.snapshot.get_cf(&handle, key.as_slice())?.map(Into::into))
    }

    fn changed_value(&self, key: &DatabaseKey) -> Option<Option<ByteVec>> {
        self.changed.get(&to_changed_key(key)).map(|v| v.value().clone())
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

    fn get_by_prefix(&self, prefix: &DatabaseKey) -> Result<Vec<(ByteVec, ByteVec)>, Self::DatabaseError> {
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(Vec::new());
        };
        let handle = self.snapshot.db.get_column(column.clone());
        let readopts = self.snapshot.read_options_with_snapshot();

        let mut out: Vec<(ByteVec, ByteVec)> = Vec::new();
        for item in self.snapshot.db.db.iterator_cf_opt(
            &handle,
            readopts,
            IteratorMode::From(prefix_bytes.as_slice(), Direction::Forward),
        ) {
            let (key, value) = item?;
            if !key.starts_with(prefix_bytes.as_slice()) {
                break;
            }
            out.push((key.to_vec().into(), value.to_vec().into()));
        }
        tracing::info!(
            "parallel_bonsai_get_by_prefix_snapshot column_id={} prefix={} snapshot_matches={} snapshot_keys={:?} snapshot_keys_sorted={}",
            prefix_col,
            bytes_to_hex(prefix_bytes.as_slice()),
            out.len(),
            out.iter().map(|(key, _)| bytes_to_hex(key.as_slice())).collect::<Vec<_>>(),
            is_lex_sorted(&out.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>())
        );

        for entry in self.changed.iter() {
            let ((column_id, key), value) = entry.pair();
            if *column_id != prefix_col || !key.starts_with(prefix_bytes.as_slice()) {
                continue;
            }

            match value {
                Some(v) => {
                    tracing::info!(
                        "parallel_bonsai_get_by_prefix_overlay_upsert column_id={} key={} value_len={}",
                        column_id,
                        bytes_to_hex(key.as_slice()),
                        v.len()
                    );
                    if let Some((_, existing)) =
                        out.iter_mut().find(|(existing_key, _)| existing_key.as_slice() == key.as_slice())
                    {
                        *existing = v.clone();
                    } else {
                        out.push((key.clone(), v.clone()));
                    }
                }
                None => {
                    tracing::info!(
                        "parallel_bonsai_get_by_prefix_overlay_delete column_id={} key={}",
                        column_id,
                        bytes_to_hex(key.as_slice())
                    );
                    out.retain(|(existing_key, _)| existing_key.as_slice() != key.as_slice())
                }
            }
        }

        tracing::info!(
            "parallel_bonsai_get_by_prefix_result column_id={} prefix={} merged_matches={} keys={:?} keys_sorted={} values_lens={:?}",
            prefix_col,
            bytes_to_hex(prefix_bytes.as_slice()),
            out.len(),
            out.iter().map(|(key, _)| bytes_to_hex(key.as_slice())).collect::<Vec<_>>(),
            is_lex_sorted(&out.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>()),
            out.iter().map(|(_, value)| value.len()).collect::<Vec<_>>()
        );

        Ok(out)
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
        let previous = self.get(key)?;
        self.changed.insert(to_changed_key(key), Some(value.into()));
        Ok(previous)
    }

    fn remove(
        &mut self,
        key: &DatabaseKey,
        _batch: Option<&mut Self::Batch>,
    ) -> Result<Option<ByteVec>, Self::DatabaseError> {
        let previous = self.get(key)?;
        self.changed.insert(to_changed_key(key), None);
        Ok(previous)
    }

    fn remove_by_prefix(&mut self, prefix: &DatabaseKey) -> Result<(), Self::DatabaseError> {
        let prefix_key = to_changed_key(prefix);
        let (prefix_col, prefix_bytes) = (prefix_key.0, prefix_key.1);

        let Some(column) = self.column_mapping.map_from_column_id(prefix_col) else {
            return Ok(());
        };
        let handle = self.snapshot.db.get_column(column.clone());
        let readopts = self.snapshot.read_options_with_snapshot();

        for item in self.snapshot.db.db.iterator_cf_opt(
            &handle,
            readopts,
            IteratorMode::From(prefix_bytes.as_slice(), Direction::Forward),
        ) {
            let (key, _value) = item?;
            if !key.starts_with(prefix_bytes.as_slice()) {
                break;
            }
            self.changed.insert((prefix_col, key.to_vec().into()), None);
        }

        for key in self
            .changed
            .iter()
            .map(|entry| entry.key().clone())
            .filter(|(column_id, key)| *column_id == prefix_col && key.starts_with(prefix_bytes.as_slice()))
            .collect::<Vec<_>>()
        {
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
