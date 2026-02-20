use super::db::{InMemoryColumnMapping, OverlayMap};
use crate::prelude::*;
use crate::rocksdb::{RocksDBStorage, WriteBatchWithTransaction};

pub(crate) const OVERLAY_TRIE_LOG_COLUMN_ID: u8 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrieLogMode {
    #[default]
    Off,
    Checkpoint,
}

#[derive(Debug, Clone)]
pub struct BonsaiOverlay {
    pub contract_changed: OverlayMap,
    pub contract_storage_changed: OverlayMap,
    pub class_changed: OverlayMap,
}

impl BonsaiOverlay {
    pub(crate) fn apply_changed_map_to_batch(
        backend: &RocksDBStorage,
        mapping: &InMemoryColumnMapping,
        changed: &OverlayMap,
        trie_log_mode: TrieLogMode,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        for entry in changed.iter() {
            let ((column_id, key), value) = entry.pair();
            if trie_log_mode == TrieLogMode::Off && *column_id == OVERLAY_TRIE_LOG_COLUMN_ID {
                continue;
            }

            let column = mapping
                .map_from_column_id(*column_id)
                .with_context(|| format!("unknown in-memory overlay column_id={column_id}"))?;
            let handle = backend.inner.get_column(column.clone());
            match value {
                Some(value) => batch.put_cf(&handle, key.as_slice(), value.as_slice()),
                None => batch.delete_cf(&handle, key.as_slice()),
            }
        }
        Ok(())
    }

    pub(crate) fn put_all_to_batch(
        &self,
        backend: &RocksDBStorage,
        trie_log_mode: TrieLogMode,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        for (mapping, changed) in [
            (InMemoryColumnMapping::contract(), &self.contract_changed),
            (InMemoryColumnMapping::contract_storage(), &self.contract_storage_changed),
            (InMemoryColumnMapping::class(), &self.class_changed),
        ] {
            Self::apply_changed_map_to_batch(backend, &mapping, changed, trie_log_mode, batch)?;
        }
        Ok(())
    }

    pub fn flush_to_db(&self, backend: &RocksDBStorage, trie_log_mode: TrieLogMode) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        self.put_all_to_batch(backend, trie_log_mode, &mut batch)?;

        backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
        Ok(())
    }
}
