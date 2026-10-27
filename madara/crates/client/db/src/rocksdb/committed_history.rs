//! Permanent values and a reclaimable proof index. All updates share the import write lock.
//! Reference counts count incoming node edges plus retained root pointers, never historical values.
use super::proof_references::References;
use super::{committed_data::*, meta, rocksdb_snapshot::SnapshotWithDBArc, *};
use crate::{
    committed_data_recovery::RecoveryRecord,
    persistent_committed_data::{node_key, Node, Tree},
};
use blockifier::execution::syscalls::committed_data::{CommittedDataSet, MAX_COMMITTED_DATA_VALUES};
use rocksdb::WriteBatch;
use std::collections::BTreeMap;

const HISTORY: &[u8] = b"cdh/v1/";
const REGISTERED: &[u8] = b"cdh/registered/";
const USED: &[u8] = b"cdh/used/";
const AGE: &[u8] = b"cdh/age/";
const MIGRATION: &[u8] = b"cdh/migration";
// Bounds cold replay work even when many updates occur in the same block.
const MAX_DELTAS: u32 = 64;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ValueHistory {
    base: Option<Felt>,
    depth: u32,
    record: RecoveryRecord,
}
fn named(prefix: &[u8], suffix: &[u8]) -> Vec<u8> {
    [prefix, suffix].concat()
}
fn root_key(prefix: &[u8], root: Felt) -> Vec<u8> {
    named(prefix, &root.to_bytes_be())
}
fn age_key(block: u64, root: Felt) -> Vec<u8> {
    [AGE, &block.to_be_bytes(), &root.to_bytes_be()].concat()
}
fn sync_options() -> WriteOptions {
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
        .committed_data_reconstruction
        .lock()
        .map_err(|_| anyhow::anyhow!("Historical reconstruction lock poisoned"))?;
    let tree = reconstruct(snapshot, root)?;
    use_tree(&tree)
}
fn reconstruct(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<CommittedDataSet> {
    let cf = snapshot.db.get_column(meta::META_COLUMN);
    let mut next = Some(root);
    let mut records = Vec::new();
    let mut expected_depth = None;
    while let Some(current) = next {
        anyhow::ensure!(records.len() <= MAX_DELTAS as usize, "Historical value chain exceeds bound");
        let bytes = snapshot.get_pinned_cf(&cf, root_key(HISTORY, current))?.context("Missing historical values")?;
        let history: ValueHistory = deserialize(bytes)?;
        anyhow::ensure!(history.record.manifest.root == current, "Historical root identity mismatch");
        anyhow::ensure!(
            history.depth <= MAX_DELTAS && expected_depth.is_none_or(|d| d == history.depth),
            "Invalid historical depth"
        );
        anyhow::ensure!(history.base.is_some() == (history.depth > 0), "Invalid historical base");
        expected_depth = history.depth.checked_sub(1);
        next = history.base;
        records.push(current);
    }
    let mut values = BTreeMap::new();
    for current in records.into_iter().rev() {
        let bytes = snapshot.get_pinned_cf(&cf, root_key(HISTORY, current))?.context("Missing historical values")?;
        let history: ValueHistory = deserialize(bytes)?;
        values.extend(history.record.decode()?);
        anyhow::ensure!(values.len() <= MAX_COMMITTED_DATA_VALUES, "Historical dataset exceeds bound");
    }
    let tree = CommittedDataSet::new(values.into_iter().collect())?;
    anyhow::ensure!(tree.root() == root, "Reconstructed historical root mismatch");
    Ok(tree)
}

impl RocksDBStorage {
    /// Called with the import lock held. Returns true when the old pointer had a root reference.
    pub(super) fn history_registered(&self, root: Felt) -> Result<bool> {
        Ok(self.inner.db.get_cf(&self.inner.get_column(meta::META_COLUMN), root_key(REGISTERED, root))?.is_some())
    }
    /// Archive submitted deltas, with periodic full checkpoints bounding reconstruction work.
    pub(super) fn stage_value_history<F: FnMut(u64) -> Result<Option<Vec<u8>>>>(
        &self,
        batch: &mut WriteBatch,
        root: Felt,
        base: Option<Felt>,
        entries: &[(Felt, Felt)],
        tree: &mut Tree<F>,
        tree_root: (u64, u32),
    ) -> Result<()> {
        let (id, count) = tree_root;
        let cf = self.inner.get_column(meta::META_COLUMN);
        if self.inner.db.get_cf(&cf, root_key(HISTORY, root))?.is_some() {
            return Ok(());
        }
        let parent = base
            .filter(|base| *base != root)
            .map(|base| -> Result<Option<ValueHistory>> {
                Ok(self.inner.db.get_cf(&cf, root_key(HISTORY, base))?.map(deserialize).transpose()?)
            })
            .transpose()?
            .flatten();
        let (base, depth, values) = if let Some(parent) = parent.filter(|p| p.depth < MAX_DELTAS) {
            (base, parent.depth + 1, entries.to_vec())
        } else {
            let mut values = Vec::with_capacity(count as usize);
            tree.page(id, 0, count, &mut values)?;
            (None, 0, values)
        };
        // This is an independent local value archive, not the temporary Mongo export record.
        let history =
            ValueHistory { base, depth, record: RecoveryRecord::encode(Felt::ZERO, None, root, None, values)? };
        batch.put_cf(&cf, root_key(HISTORY, root), serialize(&history)?);
        Ok(())
    }
    pub(super) fn stage_root_reference(
        &self,
        batch: &mut WriteBatch,
        root: Felt,
        old: Option<u64>,
        id: u64,
        first: u64,
        nodes: &[Node],
    ) -> Result<()> {
        let registered = self.history_registered(root)?;
        let mut refs = References::new(&self.inner, first, nodes);
        refs.add(id)?;
        if let Some(old) = old.filter(|_| registered) {
            refs.remove(old)?;
        }
        refs.write(batch);
        batch.put_cf(&self.inner.get_column(meta::META_COLUMN), root_key(REGISTERED, root), []);
        Ok(())
    }
    pub(super) fn note_root_use(&self, root: Felt, block: u64) -> Result<()> {
        let mut cache =
            self.inner.committed_data_write.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
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
    /// One root per call bounds migration/pruning work and lets the service check cancellation.
    /// Legacy roots are all adopted before any refcounts can be used for deletion.
    pub(super) fn maintain_history(&self, cutoff: Option<u64>, observed_head: u64) -> Result<bool> {
        let mut cache =
            self.inner.committed_data_write.lock().map_err(|_| anyhow::anyhow!("Committed-data lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let cursor = self.inner.db.get_cf(&cf, MIGRATION)?;
        if cursor.as_deref() != Some(&[2]) {
            let after = cursor
                .as_ref()
                .map(|bytes| -> Result<Felt> {
                    anyhow::ensure!(bytes.len() == 33 && bytes[0] == 1, "Invalid history migration cursor");
                    Ok(Felt::from_bytes_be(&bytes[1..].try_into()?))
                })
                .transpose()?;
            let mut batch = WriteBatch::default();
            if let Some(root) = self.read_committed_data_roots(after)?.first().copied() {
                let metadata = self
                    .inner
                    .db
                    .get_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?
                    .context("Missing migration root")?;
                let id = persistent_id(&metadata)?;
                if !self.history_registered(root)? {
                    anyhow::ensure!(id != 0, "Legacy root cannot have a cold pointer");
                    let mut tree = Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| {
                        Ok(self.inner.db.get_cf(&cf, node_key(id))?)
                    });
                    tree.check_root(id, root, count(&metadata)? as u32)?;
                    self.stage_value_history(&mut batch, root, None, &[], &mut tree, (id, count(&metadata)? as u32))?;
                    self.stage_root_reference(&mut batch, root, None, id, u64::MAX, &[])?;
                }
                // Unknown historical usage is conservatively retained from this observed head.
                self.stage_root_use(&mut batch, root, observed_head)?;
                batch.put_cf(&cf, MIGRATION, [&[1], root.to_bytes_be().as_slice()].concat());
            } else {
                batch.put_cf(&cf, MIGRATION, [2]);
            }
            self.inner.db.write_opt(batch, &sync_options())?;
            return Ok(true);
        }
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
        let mut batch = WriteBatch::default();
        if id != 0 {
            // Authenticate reconstruction before releasing any node. Failure leaves everything intact.
            let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
            with_reconstructed(&snapshot, root, |tree| {
                anyhow::ensure!(tree.entries().len() == count(&metadata)?, "Historical value count mismatch");
                Ok(())
            })?;
            let mut refs = References::new(&self.inner, u64::MAX, &[]);
            refs.remove(id)?;
            refs.write(&mut batch);
            metadata[METADATA_BYTES..].copy_from_slice(&0u64.to_be_bytes());
            batch.put_cf(&cf, metadata_key(root, METADATA_LEVEL, 0), metadata);
        }
        batch.delete_cf(&cf, key);
        self.inner.db.write_opt(batch, &sync_options())?;
        // A cached node must not hide a missing/corrupt durable node after reclamation.
        cache.clear();
        Ok(true)
    }

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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod measurement;
