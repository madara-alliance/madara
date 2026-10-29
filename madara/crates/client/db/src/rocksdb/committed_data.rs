//! Immutable shared Patricia nodes and namespace catalogs.
use super::*;
use crate::persistent_committed_data::{node_key, Tree, NEXT_KEY};
use crate::storage::CommittedDataPageEntries;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataSet, CommittedDataWitness, MAX_COMMITTED_DATA_VALUES,
};
use mp_gateway::committed_data::{CommittedDataVersion, ROOTS_PER_PAGE, VALUES_PER_PAGE};
use rocksdb::WriteBatch;

// Separate catalog: root-keyed data can be shared by multiple namespaces without copying nodes.
const VERSIONS_PREFIX: &[u8] = b"committed_data_versions/";

/// Lexical application/dataset/root key; namespace labels do not duplicate the shared value map.
fn version_key(version: CommittedDataVersion) -> Vec<u8> {
    let mut key = VERSIONS_PREFIX.to_vec();
    for value in [version.application_id, version.dataset_id, version.root] {
        key.extend_from_slice(&value.to_bytes_be());
    }
    key
}

pub(super) const PREFIX: &[u8] = b"committed_data_mpt/";
const FELT_BYTES: usize = 32;
const VERSION_KEY_BYTES: usize = 3 * FELT_BYTES;
const DATASET_OFFSET: usize = FELT_BYTES;
const ROOT_OFFSET: usize = 2 * FELT_BYTES;
const KEY_BYTES: usize = PREFIX.len() + FELT_BYTES + 1 + 4;
pub(super) const METADATA_LEVEL: u8 = u8::MAX;
pub(super) const METADATA_BYTES: usize = 13;

/// Root metadata key: root, kind 255, and offset zero.
pub(super) fn metadata_key(root: Felt, kind: u8, page: u32) -> Vec<u8> {
    let mut key = PREFIX.to_vec();
    key.extend_from_slice(&root.to_bytes_be());
    key.push(kind);
    key.extend_from_slice(&page.to_be_bytes());
    key
}
/// Fixed root record: count(u32), checkpoint presence(u8), checkpoint(u64), node ID(u64).
/// Node ID zero marks a pruned proof index; the permanent value archive still owns the dataset.
fn encode_metadata(count: u32, checkpoint: Option<u64>, node_id: u64) -> Vec<u8> {
    let mut metadata = Vec::with_capacity(METADATA_BYTES + size_of::<u64>());
    metadata.extend(count.to_be_bytes());
    metadata.push(u8::from(checkpoint.is_some()));
    metadata.extend(checkpoint.unwrap_or(0).to_be_bytes());
    metadata.extend(node_id.to_be_bytes());
    metadata
}

/// Reject out-of-field encodings instead of silently reducing bytes modulo the field.
fn canonical_felt(bytes: &[u8]) -> Result<Felt> {
    let raw: [u8; FELT_BYTES] = bytes.try_into()?;
    let value = Felt::from_bytes_be(&raw);
    anyhow::ensure!(value.to_bytes_be() == raw, "Noncanonical committed-data value");
    Ok(value)
}
/// Count plus first-import checkpoint. The checkpoint is not a publication or last-use block.
pub(super) fn decode_metadata(bytes: &[u8]) -> Result<(usize, Option<u64>)> {
    anyhow::ensure!(bytes.len() == METADATA_BYTES + 8, "Invalid committed-data metadata length");
    let count = u32::from_be_bytes(bytes[..4].try_into()?) as usize;
    anyhow::ensure!((1..=MAX_COMMITTED_DATA_VALUES).contains(&count), "Invalid committed-data size");
    let block = u64::from_be_bytes(bytes[5..13].try_into()?);
    let checkpoint = match bytes[4] {
        0 if block == 0 => None,
        1 => Some(block),
        _ => anyhow::bail!("Invalid committed-data import checkpoint"),
    };
    Ok((count, checkpoint))
}

/// Validate the complete root record and read its proof pointer; zero means archive-only.
pub(super) fn persistent_id(metadata: &[u8]) -> Result<u64> {
    decode_metadata(metadata)?;
    let id = u64::from_be_bytes(metadata[METADATA_BYTES..].try_into()?);
    anyhow::ensure!(id < u64::MAX, "Invalid persistent root pointer");
    Ok(id)
}

/// Read the authenticated-format count used to bound page assembly and tree traversal.
pub(super) fn count(bytes: &[u8]) -> Result<usize> {
    Ok(decode_metadata(bytes)?.0)
}
impl RocksDBStorage {
    /// Check an immutable namespace label, rejecting malformed catalog values.
    pub(super) fn contains_committed_data_version(&self, version: CommittedDataVersion) -> Result<bool> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let record = self.inner.db.get_pinned_cf(&cf, version_key(version))?;
        if let Some(record) = &record {
            anyhow::ensure!(record.is_empty(), "Invalid committed-data version record");
        }
        Ok(record.is_some())
    }

    /// Return one exclusive-cursor page ordered by application, dataset, then root.
    pub(super) fn read_committed_data_versions(
        &self,
        after: Option<CommittedDataVersion>,
    ) -> Result<Vec<CommittedDataVersion>> {
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let start = after.map(version_key).unwrap_or_else(|| VERSIONS_PREFIX.to_vec());
        let mut iterator = snapshot.iterator_cf(
            meta::META_COLUMN,
            rocksdb::ReadOptions::default(),
            rocksdb::IteratorMode::From(&start, rocksdb::Direction::Forward),
        );
        let mut versions = Vec::new();
        while iterator.next()? {
            let Some(raw) = iterator.key().filter(|raw| raw.starts_with(VERSIONS_PREFIX)) else { break };
            if after.is_some() && raw == start {
                continue;
            }
            let raw = &raw[VERSIONS_PREFIX.len()..];
            anyhow::ensure!(raw.len() == VERSION_KEY_BYTES, "Invalid committed-data version key");
            versions.push(CommittedDataVersion {
                application_id: canonical_felt(&raw[..FELT_BYTES])?,
                dataset_id: canonical_felt(&raw[DATASET_OFFSET..ROOT_OFFSET])?,
                root: canonical_felt(&raw[ROOT_OFFSET..])?,
            });
            if versions.len() == ROOTS_PER_PAGE {
                break;
            }
        }
        Ok(versions)
    }

    /// Data is already durable and immutable. This second fsynced write publishes its label.
    /// Value archives and metadata survive proof pruning, so labels remain resolvable.
    pub(super) fn store_committed_data_version(&self, version: CommittedDataVersion, initializing: bool) -> Result<()> {
        // Serialize the check with every label write, including replication. Concurrent initial
        // requests cannot both create distinct first roots. Existing-root retries remain harmless.
        let _write = self
            .inner
            .committed_data
            .records
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        if initializing && !self.contains_committed_data_version(version)? {
            let first = CommittedDataVersion { root: Felt::ZERO, ..version };
            let initialized = self.contains_committed_data_version(first)?
                || self.read_committed_data_versions(Some(first))?.first().is_some_and(|found| {
                    (found.application_id, found.dataset_id) == (version.application_id, version.dataset_id)
                });
            anyhow::ensure!(!initialized, "Dataset already initialized; supply its base root");
        }
        anyhow::ensure!(self.read_committed_data_count(version.root)?.is_some(), "Version dataset unavailable");
        let cf = self.inner.get_column(meta::META_COLUMN);
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, version_key(version), []);
        self.inner.db.write_opt(batch, &super::committed_history::sync_options())?;
        Ok(())
    }

    /// Seek over root metadata without scanning tree nodes. Each response is bounded.
    /// Cursors are lexical, not a durable high watermark: readers must rescan from the start.
    pub(super) fn read_committed_data_roots(&self, after: Option<Felt>) -> Result<Vec<Felt>> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let mut iterator =
            snapshot.iterator_cf(meta::META_COLUMN, rocksdb::ReadOptions::default(), rocksdb::IteratorMode::Start);
        match after {
            Some(root) => iterator.set_mode(rocksdb::IteratorMode::From(
                &metadata_key(root, METADATA_LEVEL, u32::MAX),
                rocksdb::Direction::Forward,
            )),
            None => iterator.set_mode(rocksdb::IteratorMode::From(PREFIX, rocksdb::Direction::Forward)),
        }
        let mut roots = Vec::new();
        while iterator.next()? {
            let Some(raw) = iterator.key().filter(|raw| raw.starts_with(PREFIX)) else { break };
            anyhow::ensure!(raw.len() == KEY_BYTES, "Invalid committed-data key");
            let root = canonical_felt(&raw[PREFIX.len()..PREFIX.len() + FELT_BYTES])?;
            let metadata = snapshot
                .get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?
                .ok_or_else(|| anyhow::anyhow!("Incomplete committed-data dataset"))?;
            count(&metadata)?;
            roots.push(root);
            if roots.len() == ROOTS_PER_PAGE {
                break;
            }
            iterator.set_mode(rocksdb::IteratorMode::From(
                &metadata_key(root, METADATA_LEVEL, u32::MAX),
                rocksdb::Direction::Forward,
            ));
        }
        Ok(roots)
    }

    /// Presence probes are a single small metadata read, independent of dataset size.
    pub(super) fn read_committed_data_count(&self, root: Felt) -> Result<Option<u32>> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        self.inner
            .db
            .get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?
            .map(|bytes| count(&bytes).map(|count| count as u32))
            .transpose()
    }

    /// Reads a bounded slice of raw values. Replicas authenticate the assembled dataset before import.
    pub(super) fn read_committed_data_page(&self, root: Felt, start: u32) -> Result<Option<CommittedDataPageEntries>> {
        anyhow::ensure!(
            (start as usize).is_multiple_of(VALUES_PER_PAGE) && (start as usize) < MAX_COMMITTED_DATA_VALUES,
            "Invalid committed-data page offset"
        );
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = self.historical_proof_snapshot(root)?;
        let Some(metadata) = snapshot.get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))? else {
            return Ok(None);
        };
        let count = count(&metadata)?;
        anyhow::ensure!((start as usize) < count, "Committed-data page outside dataset");
        let id = persistent_id(&metadata)?;
        if id == 0 {
            let _permit = self
                .inner
                .committed_data
                .reconstruction
                .lock()
                .map_err(|_| anyhow::anyhow!("Historical reconstruction lock poisoned"))?;
            let entries = super::committed_history::historical_values(&snapshot, root)?;
            anyhow::ensure!(entries.len() == count, "Historical value count mismatch");
            return Ok(Some((
                count as u32,
                entries[start as usize..(start as usize + VALUES_PER_PAGE).min(entries.len())].to_vec(),
            )));
        }
        let mut tree = Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| {
            Ok(snapshot.get_pinned_cf(&cf, node_key(id))?.map(|b| b.to_vec()))
        });
        tree.check_root(id, root, count as u32)?;
        let mut values = Vec::new();
        tree.page(id, start, VALUES_PER_PAGE as u32, &mut values)?;
        Ok(Some((count as u32, values)))
    }

    /// Reads and authenticates only the requested path under one snapshot.
    pub(super) fn read_committed_data_witness(&self, root: Felt, index: Felt) -> Result<Option<CommittedDataWitness>> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = self.historical_proof_snapshot(root)?;
        let Some(metadata) = snapshot.get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))? else {
            return Ok(None);
        };
        let id = persistent_id(&metadata)?;
        if id == 0 {
            return super::committed_history::with_reconstructed(&snapshot, root, |tree| Ok(tree.witness(index)));
        }
        let mut tree = Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| {
            Ok(snapshot.get_pinned_cf(&cf, node_key(id))?.map(|b| b.to_vec()))
        });
        tree.check_root(id, root, count(&metadata)? as u32)?;
        tree.witness(id, root, index)
    }

    /// Serve a transport batch from one snapshot and at most one cold reconstruction.
    pub(super) fn read_committed_data_witnesses(&self, root: Felt, keys: &[Felt]) -> Result<Vec<CommittedDataWitness>> {
        anyhow::ensure!(
            !keys.is_empty()
                && keys.len() <= blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_READ_KEYS,
            "Invalid batch size"
        );
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = self.historical_proof_snapshot(root)?;
        let metadata =
            snapshot.get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?.context("Missing dataset")?;
        let id = persistent_id(&metadata)?;
        if id == 0 {
            return super::committed_history::with_reconstructed(&snapshot, root, |tree| {
                keys.iter().map(|key| tree.witness(*key).context("Missing committed-data key")).collect()
            });
        }
        let mut tree = Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| {
            Ok(snapshot.get_pinned_cf(&cf, node_key(id))?.map(|bytes| bytes.to_vec()))
        });
        tree.check_root(id, root, count(&metadata)? as u32)?;
        keys.iter().map(|key| tree.witness(id, root, *key)?.context("Missing committed-data key")).collect()
    }

    /// Authenticate a bounded batch under one RocksDB snapshot; reuse reconstructed indexes on disk.
    pub(super) fn read_committed_data_batch(
        &self,
        root: Felt,
        keys: &[Felt],
    ) -> Result<blockifier::execution::syscalls::committed_data::CommittedDataBatchProof> {
        use blockifier::execution::syscalls::committed_data::{CommittedDataBatchProof, CommittedDataError};
        anyhow::ensure!(
            !keys.is_empty()
                && keys.len() <= blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_READ_KEYS,
            "Invalid batch size"
        );
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = self.historical_proof_snapshot(root)?;
        let metadata =
            snapshot.get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?.context("Missing dataset")?;
        let id = persistent_id(&metadata)?;
        if id == 0 {
            return super::committed_history::with_reconstructed(&snapshot, root, |tree| {
                Ok(CommittedDataBatchProof::from_dataset(tree, keys)?)
            });
        }
        let mut tree = Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| {
            Ok(snapshot.get_pinned_cf(&cf, node_key(id))?.map(|b| b.to_vec()))
        });
        tree.check_root(id, root, count(&metadata)? as u32)?;
        Ok(CommittedDataBatchProof::new(root, keys, |key| {
            tree.witness(id, root, key).map_err(|error| CommittedDataError::Provider(error.to_string()))
        })?)
    }

    /// Imports an authenticated full map into the immutable arena.
    /// Identical imports reuse nodes; an explicit full import can repair a corrupt root by rebuilding it.
    pub(super) fn store_committed_data_dataset(
        &self,
        dataset: &CommittedDataSet,
        imported_after_block: Option<u64>,
    ) -> Result<()> {
        self.store_persistent_patch(None, dataset.root(), dataset.entries().to_vec(), imported_after_block, true)
    }

    /// Lock-owning entry point for independent full imports and patches. Recent-use reactivation
    /// already owns this lock and calls the locked variant directly, avoiding recursive locking.
    pub(super) fn store_persistent_patch(
        &self,
        base: Option<Felt>,
        root: Felt,
        entries: Vec<(Felt, Felt)>,
        checkpoint: Option<u64>,
        full_import: bool,
    ) -> Result<()> {
        let mut state =
            self.inner.committed_data.tree.lock().map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        self.store_persistent_patch_locked(base, root, entries, checkpoint, full_import, &mut state)
    }

    /// Allocate monotonically; a rewound allocator must never overwrite another root's nodes.
    pub(super) fn next_committed_node_id(&self) -> Result<u64> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let first_id = self
            .inner
            .db
            .get_pinned_cf(&cf, NEXT_KEY)?
            .map(|b| -> Result<u64> { Ok(u64::from_be_bytes(b.as_ref().try_into()?)) })
            .transpose()?
            .unwrap_or(1);
        anyhow::ensure!(first_id > 0, "Invalid persistent allocator");
        // IDs are never reused, including after pruning. A rewound allocator must not
        // overwrite an immutable node that another root may still reference.
        anyhow::ensure!(
            self.inner.db.get_pinned_cf(&cf, node_key(first_id))?.is_none(),
            "Persistent allocator overlaps existing nodes"
        );
        Ok(first_id)
    }

    /// Build/authenticate in memory, then publish nodes, archive, ownership and metadata atomically.
    /// Caller holds the tree lock throughout. The cache is published only after a synced WAL write.
    pub(super) fn store_persistent_patch_locked(
        &self,
        base: Option<Felt>,
        root: Felt,
        mut entries: Vec<(Felt, Felt)>,
        checkpoint: Option<u64>,
        full_import: bool,
        state: &mut crate::persistent_committed_data::NodeCache,
    ) -> Result<()> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let old_metadata = self.inner.db.get_pinned_cf(&cf, metadata_key(root, METADATA_LEVEL, 0))?;
        let mut base = base;
        if full_import && base.is_none() && old_metadata.is_none() {
            entries.sort_unstable_by_key(|(key, _)| *key);
            // Replicas submit complete maps without a declared parent. Reuse a local map with
            // the same keys; this is storage sharing, never namespace or publication authority.
            if let Some(candidate) = super::committed_history::reusable_root(self, &entries)? {
                let metadata = self.inner.db.get_pinned_cf(&cf, metadata_key(candidate, METADATA_LEVEL, 0))?;
                if metadata.as_ref().map(|bytes| persistent_id(bytes)).transpose()?.is_some_and(|id| id != 0) {
                    base = Some(candidate);
                }
            }
        }
        let base_root = base.unwrap_or(root);
        let base_metadata = self.inner.db.get_pinned_cf(&cf, metadata_key(base_root, METADATA_LEVEL, 0))?;
        let base_id = base_metadata.as_ref().map(|b| persistent_id(b)).transpose()?.filter(|id| *id != 0);
        anyhow::ensure!(full_import || base_metadata.is_some(), "Persistent base unavailable");
        let first_id = self.next_committed_node_id()?;
        let reader =
            |id| -> Result<Option<Vec<u8>>> { Ok(self.inner.db.get_pinned_cf(&cf, node_key(id))?.map(|b| b.to_vec())) };
        // Full imports deliberately bypass the cache so they can repair corrupt disk records.
        let mut cache = if full_import { Default::default() } else { std::mem::take(&mut *state) };
        cache.configure(
            &*self.inner.committed_data.config.read().map_err(|_| anyhow::anyhow!("Committed-data config poisoned"))?,
            checkpoint.unwrap_or(0),
        );
        let mut tree = Tree::with_cache(first_id, MAX_COMMITTED_DATA_VALUES as u32, reader, cache);
        let base_id = if base_id.is_none() && base_metadata.is_some() && !full_import {
            let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
            Some(super::committed_history::with_reconstructed(&snapshot, base_root, |restored| {
                tree.build(&mut restored.entries().to_vec())
            })?)
        } else {
            base_id
        };
        let attempt = if let Some(id) = base_id {
            tree.check_root(
                id,
                base_root,
                count(base_metadata.as_ref().context("Persistent base metadata missing")?)? as u32,
            )
            .and_then(|_| tree.update(id, &mut entries))
        } else {
            tree.build(&mut entries)
        };
        let id = match attempt {
            Ok(id) => id,
            // A full map can rebuild damaged storage; authenticate the rebuilt root below.
            Err(_) if full_import => {
                // Rebuilding no longer describes a delta against the rejected storage base.
                base = None;
                tree = Tree::new(first_id, MAX_COMMITTED_DATA_VALUES as u32, reader);
                tree.build(&mut entries)?
            }
            Err(error) => return Err(error),
        };
        let (actual_root, count) = tree.root(id)?;
        anyhow::ensure!(actual_root == root, "Committed-data root does not match updated dataset");
        // A full retry can reuse an existing tree, but must still supply every entry. Otherwise
        // an unchanged subset could pass the root check through upsert semantics.
        anyhow::ensure!(!full_import || count as usize == entries.len(), "Incomplete committed-data full import");
        // Retry/fork from an older base may reproduce a root already present. Authenticate the
        // request first, then discard its staged nodes instead of duplicating that version.
        // Full imports retain their explicit repair path, including damaged descendants.
        if !full_import {
            if let Some(existing_id) =
                old_metadata.as_ref().map(|b| persistent_id(b)).transpose()?.filter(|id| *id != 0)
            {
                tree.check_root(existing_id, root, count)?;
                let mut batch = WriteBatch::default();
                self.stage_root_use(&mut batch, root, checkpoint.unwrap_or(0).saturating_add(1))?;
                let mut options = WriteOptions::default();
                options.set_sync(true);
                if !batch.is_empty() {
                    self.inner.db.write_opt(batch, &options)?;
                }
                *state = tree.into_read_cache();
                return Ok(());
            }
        }
        self.publish_persistent_tree(
            root,
            base,
            checkpoint,
            old_metadata.as_deref(),
            first_id,
            &mut tree,
            (id, count),
        )?;
        *state = tree.into_committed_cache();
        Ok(())
    }
    /// Publish an already authenticated tree in one WAL-synced batch. Archive and ownership
    /// changes must precede the root pointer in the same batch, never in separate writes.
    #[allow(clippy::too_many_arguments)]
    fn publish_persistent_tree<F: FnMut(u64) -> Result<Option<Vec<u8>>>>(
        &self,
        root: Felt,
        base: Option<Felt>,
        checkpoint: Option<u64>,
        old_metadata: Option<&[u8]>,
        first_id: u64,
        tree: &mut Tree<F>,
        tree_root: (u64, u32),
    ) -> Result<()> {
        let (id, count) = tree_root;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let last_used = checkpoint.unwrap_or(0).saturating_add(1);
        let checkpoint = old_metadata.map(|b| decode_metadata(b).map(|(_, c)| c)).transpose()?.unwrap_or(checkpoint);
        let metadata = encode_metadata(count, checkpoint, id);
        let mut batch = WriteBatch::default();
        self.stage_value_history(&mut batch, root, base, tree, (id, count))?;
        let old_id = old_metadata.map(persistent_id).transpose()?.filter(|id| *id != 0);
        self.stage_root_reference(&mut batch, old_id, id, first_id, &tree.nodes)?;
        self.stage_root_use(&mut batch, root, last_used)?;
        batch.put_cf(&cf, NEXT_KEY, tree.next_id().to_be_bytes());
        batch.put_cf(&cf, metadata_key(root, METADATA_LEVEL, 0), metadata);
        self.inner.db.write_opt(batch, &super::committed_history::sync_options())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_data_records_reject_noncanonical_and_invalid_lengths() {
        assert!(canonical_felt(&[0_u8; 31]).is_err());
        assert!(canonical_felt(&[0xff_u8; 32]).is_err());
        assert_eq!(canonical_felt(&Felt::MAX.to_bytes_be()).unwrap(), Felt::MAX);
        assert!(count(&[]).is_err());
        assert!(count(&0_u32.to_be_bytes()).is_err());
        assert!(count(&u32::MAX.to_be_bytes()).is_err());
        assert!(decode_metadata(&[0, 0, 0, 1, 19]).is_err());
        let mut metadata = [0u8; METADATA_BYTES + 8];
        metadata[..4].copy_from_slice(&1u32.to_be_bytes());
        metadata[4] = 1;
        metadata[5..13].copy_from_slice(&42u64.to_be_bytes());
        metadata[13..].copy_from_slice(&1u64.to_be_bytes());
        assert!(decode_metadata(&metadata[..METADATA_BYTES]).is_err());
        assert_eq!(persistent_id(&metadata).unwrap(), 1);
        assert_eq!(decode_metadata(&metadata).unwrap(), (1, Some(42)));
        metadata[4] = 2;
        assert!(decode_metadata(&metadata).is_err());
    }
}
