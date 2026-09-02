use super::db::{InMemoryColumnMapping, OverlayMap};
use crate::prelude::*;
use crate::rocksdb::trie::{BONSAI_CLASS_LOG_COLUMN, BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_LOG_COLUMN};
use crate::rocksdb::{RocksDBStorage, WriteBatchWithTransaction};
use rocksdb::{Direction, IteratorMode};

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
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        for entry in changed.iter() {
            let ((column_id, key), value) = entry.pair();
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

    pub fn flush_to_db(&self, backend: &RocksDBStorage) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::contract(),
            &self.contract_changed,
            &mut batch,
        )?;
        Self::apply_changed_map_to_batch(
            backend,
            &InMemoryColumnMapping::contract_storage(),
            &self.contract_storage_changed,
            &mut batch,
        )?;
        Self::apply_changed_map_to_batch(backend, &InMemoryColumnMapping::class(), &self.class_changed, &mut batch)?;

        backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
        Ok(())
    }
}

fn prune_expired_boundary_logs_in_batch(
    backend: &RocksDBStorage,
    block_n: u64,
    boundary_interval: u64,
    batch: &mut WriteBatchWithTransaction,
) -> Result<Option<(u64, usize)>> {
    ensure!(boundary_interval > 0, "Parallel Merkle boundary interval must be greater than zero");
    let Some(max_saved_trie_logs) = backend.inner.config.max_saved_trie_logs else {
        return Ok(None);
    };
    let retention_distance = boundary_interval
        .checked_mul(u64::try_from(max_saved_trie_logs).context("Converting trie-log retention to u64")?)
        .context("Parallel Merkle trie-log retention distance overflow")?;
    let Some(expired_revision) = block_n.checked_sub(retention_distance) else {
        return Ok(None);
    };
    let prefix = expired_revision.to_be_bytes();
    let mut deleted_entries = 0;

    for column in [BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_LOG_COLUMN, BONSAI_CLASS_LOG_COLUMN] {
        let handle = backend.inner.get_column(column);
        let iter = backend.inner.db.iterator_cf(&handle, IteratorMode::From(prefix.as_slice(), Direction::Forward));
        for item in iter {
            let (key, _) = item.context("Scanning expired boundary trie-log revision")?;
            if !key.starts_with(&prefix) {
                break;
            }
            batch.delete_cf(&handle, key);
            deleted_entries += 1;
        }
    }

    Ok(Some((expired_revision, deleted_entries)))
}

pub fn flush_overlay_and_checkpoint(
    backend: &RocksDBStorage,
    block_n: u64,
    boundary_interval: u64,
    overlay: &BonsaiOverlay,
) -> Result<()> {
    let mut batch = WriteBatchWithTransaction::default();
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::contract(),
        &overlay.contract_changed,
        &mut batch,
    )?;
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::contract_storage(),
        &overlay.contract_storage_changed,
        &mut batch,
    )?;
    BonsaiOverlay::apply_changed_map_to_batch(
        backend,
        &InMemoryColumnMapping::class(),
        &overlay.class_changed,
        &mut batch,
    )?;
    let pruned = prune_expired_boundary_logs_in_batch(backend, block_n, boundary_interval, &mut batch)?;
    backend.inner.parallel_merkle_mark_checkpoint_in_batch(block_n, &mut batch)?;
    backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
    if let Some((expired_revision, deleted_entries)) = pruned {
        tracing::debug!(
            "parallel_merkle_boundary_logs_pruned block_number={block_n} boundary_interval={boundary_interval} expired_revision={expired_revision} deleted_entries={deleted_entries} max_saved_trie_logs={:?}",
            backend.inner.config.max_saved_trie_logs
        );
    }
    Ok(())
}
