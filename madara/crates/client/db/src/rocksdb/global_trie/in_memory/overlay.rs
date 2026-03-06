use super::db::{InMemoryColumnMapping, OverlayMap, OVERLAY_TRIE_LOG_COLUMN_ID};
use super::TrieLogMode;
use crate::prelude::*;
use crate::rocksdb::{RocksDBStorage, WriteBatchWithTransaction};

#[derive(Debug, Clone)]
pub struct BonsaiOverlay {
    pub contract_changed: OverlayMap,
    pub contract_storage_changed: OverlayMap,
    pub class_changed: OverlayMap,
}

impl BonsaiOverlay {
    pub(super) fn apply_changed_map_to_batch(
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

    pub fn flush_to_db(&self, backend: &RocksDBStorage, trie_log_mode: TrieLogMode) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::contract(),
            &self.contract_changed,
            trie_log_mode,
            &mut batch,
        )?;
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::contract_storage(),
            &self.contract_storage_changed,
            trie_log_mode,
            &mut batch,
        )?;
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::class(),
            &self.class_changed,
            trie_log_mode,
            &mut batch,
        )?;

        backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
        Ok(())
    }
}

pub fn flush_overlay_and_checkpoint(
    backend: &RocksDBStorage,
    block_n: u64,
    overlay: &BonsaiOverlay,
    trie_log_mode: TrieLogMode,
) -> Result<()> {
    let mut batch = WriteBatchWithTransaction::default();
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::contract(),
        &overlay.contract_changed,
        trie_log_mode,
        &mut batch,
    )?;
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::contract_storage(),
        &overlay.contract_storage_changed,
        trie_log_mode,
        &mut batch,
    )?;
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::class(),
        &overlay.class_changed,
        trie_log_mode,
        &mut batch,
    )?;
    backend.inner.parallel_merkle_mark_checkpoint_in_batch(block_n, &mut batch)?;
    backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
    Ok(())
}
