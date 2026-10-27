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

fn version_key(version: CommittedDataVersion) -> Vec<u8> {
    let mut key = VERSIONS_PREFIX.to_vec();
    for value in [version.application_id, version.dataset_id, version.root] {
        key.extend_from_slice(&value.to_bytes_be());
    }
    key
}

const PREFIX: &[u8] = b"committed_data_mpt/";
const FELT_BYTES: usize = 32;
const KEY_BYTES: usize = PREFIX.len() + FELT_BYTES + 1 + 4;
const METADATA_LEVEL: u8 = u8::MAX;
const METADATA_BYTES: usize = 13;

/// Root metadata retains its original key layout: root, kind 255, and offset zero.
fn key(root: Felt, kind: u8, page: u32) -> Vec<u8> {
    let mut key = PREFIX.to_vec();
    key.extend_from_slice(&root.to_bytes_be());
    key.push(kind);
    key.extend_from_slice(&page.to_be_bytes());
    key
}
fn canonical_felt(bytes: &[u8]) -> Result<Felt> {
    let raw: [u8; FELT_BYTES] = bytes.try_into()?;
    let value = Felt::from_bytes_be(&raw);
    anyhow::ensure!(value.to_bytes_be() == raw, "Noncanonical committed-data value");
    Ok(value)
}
/// Count plus first-import checkpoint. The checkpoint is not a publication or last-use block.
fn decode_metadata(bytes: &[u8]) -> Result<(usize, Option<u64>)> {
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

fn persistent_id(metadata: &[u8]) -> Result<u64> {
    decode_metadata(metadata)?;
    let id = u64::from_be_bytes(metadata[METADATA_BYTES..].try_into()?);
    anyhow::ensure!(id > 0 && id < u64::MAX, "Invalid persistent root pointer");
    Ok(id)
}

fn count(bytes: &[u8]) -> Result<usize> {
    Ok(decode_metadata(bytes)?.0)
}
impl RocksDBStorage {
    pub(super) fn contains_committed_data_version(&self, version: CommittedDataVersion) -> Result<bool> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let record = self.inner.db.get_pinned_cf(&cf, version_key(version))?;
        if let Some(record) = &record {
            anyhow::ensure!(record.is_empty(), "Invalid committed-data version record");
        }
        Ok(record.is_some())
    }

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
            anyhow::ensure!(raw.len() == 3 * FELT_BYTES, "Invalid committed-data version key");
            versions.push(CommittedDataVersion {
                application_id: canonical_felt(&raw[..FELT_BYTES])?,
                dataset_id: canonical_felt(&raw[FELT_BYTES..2 * FELT_BYTES])?,
                root: canonical_felt(&raw[2 * FELT_BYTES..])?,
            });
            if versions.len() == ROOTS_PER_PAGE {
                break;
            }
        }
        Ok(versions)
    }

    /// Data is already durable and immutable. This second fsynced write publishes its label.
    /// No deletion exists, so a label can never race a data-pruning operation.
    pub(super) fn store_committed_data_version(&self, version: CommittedDataVersion, initializing: bool) -> Result<()> {
        // Serialize the check with every label write, including replication. Concurrent initial
        // requests cannot both create distinct first roots. Existing-root retries remain harmless.
        let _write = self
            .inner
            .committed_data_write
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
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
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
                &key(root, METADATA_LEVEL, u32::MAX),
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
                .get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))?
                .ok_or_else(|| anyhow::anyhow!("Incomplete committed-data dataset"))?;
            count(&metadata)?;
            roots.push(root);
            if roots.len() == ROOTS_PER_PAGE {
                break;
            }
            iterator.set_mode(rocksdb::IteratorMode::From(
                &key(root, METADATA_LEVEL, u32::MAX),
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
            .get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))?
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
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let Some(metadata) = snapshot.get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
        let count = count(&metadata)?;
        anyhow::ensure!((start as usize) < count, "Committed-data page outside dataset");
        let id = persistent_id(&metadata)?;
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
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let Some(metadata) = snapshot.get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
        let id = persistent_id(&metadata)?;
        let mut tree = Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| {
            Ok(snapshot.get_pinned_cf(&cf, node_key(id))?.map(|b| b.to_vec()))
        });
        tree.check_root(id, root, count(&metadata)? as u32)?;
        tree.witness(id, root, index)
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

    // ponytail: one shipping format; missing bases fail instead of rebuilding legacy snapshots.
    pub(super) fn store_persistent_patch(
        &self,
        base: Option<Felt>,
        root: Felt,
        mut entries: Vec<(Felt, Felt)>,
        checkpoint: Option<u64>,
        full_import: bool,
    ) -> Result<()> {
        let mut state = self
            .inner
            .committed_data_write
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let old_metadata = self.inner.db.get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))?;
        let base_root = base.unwrap_or(root);
        let base_metadata = self.inner.db.get_pinned_cf(&cf, key(base_root, METADATA_LEVEL, 0))?;
        let base_id = base_metadata.as_ref().map(|b| persistent_id(b)).transpose()?;
        anyhow::ensure!(full_import || base_id.is_some(), "Persistent base unavailable");
        let first_id = self
            .inner
            .db
            .get_pinned_cf(&cf, NEXT_KEY)?
            .map(|b| -> Result<u64> { Ok(u64::from_be_bytes(b.as_ref().try_into()?)) })
            .transpose()?
            .unwrap_or(1);
        anyhow::ensure!(first_id > 0, "Invalid persistent allocator");
        // No nodes are deleted in this schema. A missing/rewound allocator must never overwrite
        // an immutable node that older roots may still reference.
        anyhow::ensure!(
            self.inner.db.get_pinned_cf(&cf, node_key(first_id))?.is_none(),
            "Persistent allocator overlaps existing nodes"
        );
        let reader =
            |id| -> Result<Option<Vec<u8>>> { Ok(self.inner.db.get_pinned_cf(&cf, node_key(id))?.map(|b| b.to_vec())) };
        // Full imports deliberately bypass the cache so they can repair corrupt disk records.
        let cache = if full_import { Default::default() } else { std::mem::take(&mut *state) };
        let mut tree = Tree::with_cache(first_id, MAX_COMMITTED_DATA_VALUES as u32, reader, cache);
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
            // Only a full, independently authenticated map is allowed to repair damaged storage.
            Err(_) if full_import => {
                tree = Tree::new(first_id, MAX_COMMITTED_DATA_VALUES as u32, reader);
                tree.build(&mut entries)?
            }
            Err(error) => return Err(error),
        };
        let (actual_root, count) = tree.root(id)?;
        anyhow::ensure!(actual_root == root, "Committed-data root does not match updated dataset");
        // Retry/fork from an older base may reproduce a root already present. Authenticate the
        // request first, then discard its staged nodes instead of duplicating that version.
        // Full imports retain their explicit repair path, including damaged descendants.
        if !full_import {
            if let Some(existing_id) = old_metadata.as_ref().map(|b| persistent_id(b)).transpose()? {
                tree.check_root(existing_id, root, count)?;
                *state = tree.into_read_cache();
                return Ok(());
            }
        }
        let checkpoint =
            old_metadata.as_ref().map(|b| decode_metadata(b).map(|(_, c)| c)).transpose()?.unwrap_or(checkpoint);
        let mut metadata = Vec::with_capacity(METADATA_BYTES + 8);
        metadata.extend(count.to_be_bytes());
        metadata.push(u8::from(checkpoint.is_some()));
        metadata.extend(checkpoint.unwrap_or(0).to_be_bytes());
        metadata.extend(id.to_be_bytes());
        let mut batch = WriteBatch::default();
        for (key, value) in tree.records() {
            batch.put_cf(&cf, key, value);
        }
        batch.put_cf(&cf, NEXT_KEY, tree.next_id().to_be_bytes());
        batch.put_cf(&cf, key(root, METADATA_LEVEL, 0), metadata);
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
        *state = tree.into_committed_cache();
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
