//! Permanent values and a reclaimable proof index. All updates share the import write lock.
//! Reference counts count incoming node edges plus retained root pointers, never historical values.
use super::proof_references::ProofNodeReferences;
use super::{committed_data::*, meta, rocksdb_snapshot::SnapshotWithDBArc, *};
use crate::persistent_committed_data::Node;
use blockifier::execution::syscalls::committed_data::{CommittedDataSet, MAX_COMMITTED_DATA_VALUES};
use rocksdb::WriteBatch;
mod archive;
mod codec;
#[cfg(test)]
mod measurement;
#[cfg(test)]
mod tests;
use archive::{history_chain, reconstruct};
#[cfg(test)]
use archive::{ValueHistory, HISTORY};

const USED: &[u8] = b"cdh/used/";
const AGE: &[u8] = b"cdh/age/";
// Bounds cold replay work even when many updates occur in the same block.
const MAX_DELTAS: u32 = 64;

fn named(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    [prefix, suffix].concat()
}
fn root_key(prefix: &[u8], root: Felt) -> Vec<u8> {
    named(prefix, &root.to_bytes_be())
}
fn age_key(block: u64, root: Felt) -> Vec<u8> {
    [AGE, &block.to_be_bytes(), &root.to_bytes_be()].concat()
}
/// Every acknowledged committed-data write survives a process crash, independent of chain WAL policy.
pub(super) fn sync_options() -> WriteOptions {
    let mut options = WriteOptions::default();
    options.set_sync(true);
    options.disable_wal(false);
    options
}

/// Rebuild only in memory: an old proof request must not repopulate the durable proof window.
pub(super) fn with_reconstructed<T>(
    snapshot: &SnapshotWithDBArc,
    root: Felt,
    use_tree: impl FnOnce(&CommittedDataSet) -> Result<T>,
) -> Result<T> {
    // One cold rebuild per backend. Hold the permit through proof generation and tree destruction.
    let _permit = snapshot
        .db
        .committed_data
        .reconstruction
        .lock()
        .map_err(|_| anyhow::anyhow!("Historical reconstruction lock poisoned"))?;
    let tree = reconstruct(snapshot, root)?;
    use_tree(&tree)
}

impl RocksDBStorage {
    /// Transfer the retained root pointer in the same batch as reachable node/refcount changes.
    pub(super) fn stage_root_reference(
        &self,
        batch: &mut WriteBatch,
        old: Option<u64>,
        id: u64,
        first: u64,
        nodes: &[Node],
    ) -> Result<()> {
        let mut refs = ProofNodeReferences::new(&self.inner, first, nodes);
        refs.add(id)?;
        if let Some(old) = old {
            refs.remove(old)?;
        }
        refs.write(batch);
        Ok(())
    }
    /// Retain a root used by recent execution; reactivate its proof index if it was pruned.
    pub(super) fn note_root_use(&self, root: Felt, block: u64) -> Result<()> {
        let mut cache =
            self.inner.committed_data.tree.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let metadata = self
            .inner
            .db
            .get_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?
            .context("Cannot retain unknown dataset")?;
        if persistent_id(&metadata)? == 0 {
            // A genuinely recent execution reactivates a cold root once. Historical RPC proof
            // reads never take this path; the provider filters executions below the cutoff.
            let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
            let values = with_reconstructed(&snapshot, root, |tree| Ok(tree.entries().to_vec()))?;
            return self.store_persistent_patch_locked(
                None,
                root,
                values,
                Some(block.saturating_sub(1)),
                true,
                &mut cache,
            );
        }
        let mut batch = WriteBatch::default();
        self.stage_root_use(&mut batch, root, block)?;
        if !batch.is_empty() {
            self.inner.db.write_opt(batch, &sync_options())?;
        }
        Ok(())
    }
}

impl RocksDBStorage {
    /// One root per call lets the maintenance service check cancellation between roots.
    pub(super) fn maintain_history(&self, cutoff: Option<u64>) -> Result<bool> {
        let cache =
            self.inner.committed_data.tree.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let Some(cutoff) = cutoff else { return Ok(false) };
        let candidate =
            self.inner.db.iterator_cf(&cf, IteratorMode::From(AGE, rocksdb::Direction::Forward)).next().transpose()?;
        let Some((key, _)) = candidate else { return Ok(false) };
        if !key.starts_with(AGE) {
            return Ok(false);
        }
        anyhow::ensure!(key.len() == AGE.len() + 40, "Invalid historical age index");
        let used = u64::from_be_bytes(key[AGE.len()..AGE.len() + 8].try_into()?);
        if used >= cutoff {
            return Ok(false);
        }
        let root = Felt::from_bytes_be(&key[AGE.len() + 8..].try_into()?);
        let mut metadata =
            self.inner.db.get_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?.context("Missing retained root")?;
        let id = persistent_id(&metadata)?;
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
        // Archive authentication can be expensive. Do not block new imports while rebuilding.
        drop(cache);
        if id != 0 {
            let (_, archived_count) = history_chain(&snapshot, root)?;
            anyhow::ensure!(archived_count as usize == count(&metadata)?, "Historical value count mismatch");
        }
        drop(snapshot);
        let mut cache =
            self.inner.committed_data.tree.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        // Recheck under the write lock: a concurrent import/use/repair may have retained this root.
        if self.inner.db.get_cf(&cf, &key)?.is_none()
            || self.inner.db.get_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?.as_deref() != Some(metadata.as_slice())
        {
            return Ok(true);
        }
        let mut batch = WriteBatch::default();
        if id != 0 {
            let mut refs = ProofNodeReferences::new(&self.inner, u64::MAX, &[]);
            refs.remove(id)?;
            for &deleted in refs.deleted_node_ids() {
                cache.remove(deleted);
            }
            refs.write(&mut batch);
            metadata[METADATA_BYTES..].copy_from_slice(&0u64.to_be_bytes());
            batch.put_cf(&cf, metadata_key(root, METADATA_LEVEL, 0), metadata);
        }
        batch.delete_cf(&cf, key);
        self.inner.db.write_opt(batch, &sync_options())?;
        Ok(true)
    }

    /// Advance last-use monotonically and maintain its age index in the same caller-owned batch.
    pub(super) fn stage_root_use(&self, batch: &mut WriteBatch, root: Felt, block: u64) -> Result<()> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let key = root_key(USED, root);
        let previous = self
            .inner
            .db
            .get_cf(&cf, &key)?
            .map(|b| -> Result<u64> { Ok(u64::from_be_bytes(b.as_slice().try_into()?)) })
            .transpose()?;
        if let Some(previous) = previous.filter(|previous| *previous >= block) {
            let age = age_key(previous, root);
            if self.inner.db.get_cf(&cf, &age)?.is_none() {
                batch.put_cf(&cf, age, []);
            }
            return Ok(());
        }
        if let Some(previous) = previous {
            batch.delete_cf(&cf, age_key(previous, root));
        }
        batch.put_cf(&cf, key, block.to_be_bytes());
        batch.put_cf(&cf, age_key(block, root), []);
        Ok(())
    }
}
