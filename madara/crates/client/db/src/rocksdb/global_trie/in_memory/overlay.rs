use super::db::{InMemoryColumnMapping, OverlayMap};
use crate::prelude::*;
use crate::rocksdb::trie::{BONSAI_CLASS_LOG_COLUMN, BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_LOG_COLUMN};
use crate::rocksdb::{RocksDBStorage, WriteBatchWithTransaction};
use std::time::{Duration, Instant};

const SLOW_BOUNDARY_FLUSH_WARNING: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct BonsaiOverlay {
    pub contract_changed: OverlayMap,
    pub contract_storage_changed: OverlayMap,
    pub class_changed: OverlayMap,
}

/// Result of attempting to publish a computed boundary overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryFlushOutcome {
    /// The overlay base matched durable state, so the checkpoint was published.
    Persisted,
    /// A newer checkpoint already exists, so applying this older-base delta was unsafe.
    StaleBaseSkipped { latest_checkpoint: u64 },
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

/// Adds one range tombstone per trie-log column for revisions outside the configured block window.
///
/// `max_saved_trie_logs` is a number of block revisions, even though parallel Merkle only writes
/// logs at boundary revisions. Range deletion keeps pruning work bounded regardless of how many
/// keys exist in an expired revision and leaves physical space reclamation to RocksDB compaction.
fn prune_expired_boundary_logs_in_batch(
    backend: &RocksDBStorage,
    block_n: u64,
    batch: &mut WriteBatchWithTransaction,
) -> Result<Option<u64>> {
    let Some(max_saved_trie_logs) = backend.inner.config.max_saved_trie_logs else {
        return Ok(None);
    };
    let retained_block_revisions =
        u64::try_from(max_saved_trie_logs).context("Converting trie-log retention to u64")?;
    let first_retained_revision = if retained_block_revisions == 0 {
        block_n.checked_add(1).context("Computing trie-log range end after the maximum block number")?
    } else {
        block_n.saturating_sub(retained_block_revisions - 1)
    };
    if first_retained_revision == 0 {
        return Ok(None);
    }

    let first_revision = 0_u64.to_be_bytes();
    let first_retained_revision_key = first_retained_revision.to_be_bytes();

    for column in [BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_LOG_COLUMN, BONSAI_CLASS_LOG_COLUMN] {
        let handle = backend.inner.get_column(column);
        batch.delete_range_cf(&handle, first_revision, first_retained_revision_key);
    }

    Ok(Some(first_retained_revision))
}

/// Rejects a future base and identifies an older base that must be skipped.
fn stale_boundary_outcome(
    block_n: u64,
    latest_checkpoint: Option<u64>,
    overlay_base_block_n: Option<u64>,
) -> Result<Option<BoundaryFlushOutcome>> {
    if latest_checkpoint == overlay_base_block_n {
        return Ok(None);
    }

    match (latest_checkpoint, overlay_base_block_n) {
        (Some(latest_checkpoint), None) => {
            Ok(Some(BoundaryFlushOutcome::StaleBaseSkipped { latest_checkpoint }))
        }
        (Some(latest_checkpoint), Some(overlay_base_block_n)) if latest_checkpoint > overlay_base_block_n => {
            Ok(Some(BoundaryFlushOutcome::StaleBaseSkipped { latest_checkpoint }))
        }
        _ => anyhow::bail!(
            "parallel merkle overlay base is newer than durable state: block={block_n}, overlay_base={overlay_base_block_n:?}, latest_checkpoint={latest_checkpoint:?}"
        ),
    }
}

/// Publishes an overlay only when it was computed from the current durable checkpoint.
///
/// Workers may compute cumulative roots from an older durable floor while a previous
/// boundary is committing. Such roots remain valid, but their overlays are deltas
/// relative to that older floor and must not be applied on top of a newer checkpoint.
pub fn flush_overlay_and_checkpoint(
    backend: &RocksDBStorage,
    block_n: u64,
    boundary_interval: u64,
    overlay_base_block_n: Option<u64>,
    overlay: &BonsaiOverlay,
) -> Result<BoundaryFlushOutcome> {
    let latest_checkpoint = backend.inner.get_parallel_merkle_latest_checkpoint()?;
    if let Some(outcome) = stale_boundary_outcome(block_n, latest_checkpoint, overlay_base_block_n)? {
        return Ok(outcome);
    }

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
    let pruned_before_revision = prune_expired_boundary_logs_in_batch(backend, block_n, &mut batch)?;
    backend.inner.parallel_merkle_mark_checkpoint_in_batch(block_n, &mut batch)?;
    let write_started = Instant::now();
    backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
    let write_duration = write_started.elapsed();

    if write_duration >= SLOW_BOUNDARY_FLUSH_WARNING {
        tracing::warn!(
            block_number = block_n,
            boundary_interval,
            write_duration_ms = write_duration.as_secs_f64() * 1000.0,
            ?pruned_before_revision,
            "parallel_merkle_boundary_flush_slow"
        );
    }
    if let Some(first_retained_revision) = pruned_before_revision {
        tracing::debug!(
            "parallel_merkle_boundary_logs_pruned block_number={block_n} boundary_interval={boundary_interval} first_retained_revision={first_retained_revision} range_tombstones=3 max_saved_trie_logs={:?}",
            backend.inner.config.max_saved_trie_logs
        );
    }
    Ok(BoundaryFlushOutcome::Persisted)
}
