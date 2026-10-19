//! Immutable Patricia datasets: sorted key/value pages and compact postorder branch pages.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    key_bit, leaf, CommittedDataBranch, CommittedDataSet, CommittedDataSibling, CommittedDataWitness,
    COMMITTED_DATA_TREE_HEIGHT, MAX_COMMITTED_DATA_VALUES,
};
use mp_gateway::committed_data::{CommittedDataVersion, ROOTS_PER_PAGE, VALUES_PER_PAGE};
use rocksdb::WriteBatch;

// Separate catalog: root-keyed data can be shared by multiple namespaces without copying pages.
const VERSIONS_PREFIX: &[u8] = b"committed_data_versions/";

fn version_key(version: CommittedDataVersion) -> Vec<u8> {
    let mut key = VERSIONS_PREFIX.to_vec();
    for value in [version.application_id, version.dataset_id, version.root] {
        key.extend_from_slice(&value.to_bytes_be());
    }
    key
}

const PREFIX: &[u8] = b"committed_data_mpt/";
const PAGE_VALUES: usize = 256;
const FELT_BYTES: usize = 32;
const KEY_BYTES: usize = PREFIX.len() + FELT_BYTES + 1 + 4;
const ENTRY_BYTES: usize = 2 * FELT_BYTES;
const BRANCH_BYTES: usize = FELT_BYTES + 1 + 2 * 4;
const PAGE_BYTES: usize = PAGE_VALUES * ENTRY_BYTES;
const METADATA_LEVEL: u8 = u8::MAX;
const METADATA_BYTES: usize = 13;

/// Root, record kind (entries=0, branches=1, metadata=255), then page number.
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
    anyhow::ensure!(bytes.len() == METADATA_BYTES, "Invalid committed-data metadata length");
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
fn count(bytes: &[u8]) -> Result<usize> {
    Ok(decode_metadata(bytes)?.0)
}
fn encode_metadata(dataset: &CommittedDataSet, checkpoint: Option<u64>) -> Result<[u8; METADATA_BYTES]> {
    let mut bytes = [0; METADATA_BYTES];
    bytes[..4].copy_from_slice(&u32::try_from(dataset.entries().len())?.to_be_bytes());
    if let Some(block) = checkpoint {
        bytes[4] = 1;
        bytes[5..].copy_from_slice(&block.to_be_bytes());
    }
    Ok(bytes)
}

/// Read one fixed-size node record from a bounded page. Leaves occupy IDs 0..count;
/// branches occupy count..2*count-1 in postorder, so every child precedes its parent.
fn read_record(
    root: Felt,
    count: usize,
    id: u32,
    read: &mut impl FnMut(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<Vec<u8>> {
    let id = id as usize;
    anyhow::ensure!(id < 2 * count - 1, "Invalid committed-data node ID");
    let (kind, offset, nodes, width) =
        if id < count { (0, id, count, ENTRY_BYTES) } else { (1, id - count, count - 1, BRANCH_BYTES) };
    let page = offset / PAGE_VALUES;
    let bytes =
        read(&key(root, kind, page as u32))?.ok_or_else(|| anyhow::anyhow!("Committed-data page unavailable"))?;
    anyhow::ensure!(
        bytes.len() == (nodes - page * PAGE_VALUES).min(PAGE_VALUES) * width,
        "Invalid committed-data page length"
    );
    let start = offset % PAGE_VALUES * width;
    Ok(bytes[start..start + width].to_vec())
}
fn decode_branch(bytes: &[u8], id: u32) -> Result<CommittedDataBranch> {
    anyhow::ensure!(bytes.len() == BRANCH_BYTES, "Invalid committed-data branch length");
    let node = CommittedDataBranch {
        hash: canonical_felt(&bytes[..32])?,
        bit: bytes[32],
        left: u32::from_be_bytes(bytes[33..37].try_into()?),
        right: u32::from_be_bytes(bytes[37..41].try_into()?),
    };
    anyhow::ensure!(
        usize::from(node.bit) < COMMITTED_DATA_TREE_HEIGHT
            && node.left < id
            && node.right < id
            && node.left != node.right,
        "Invalid committed-data branch"
    );
    Ok(node)
}
/// Walk only the requested path. Missing metadata/key returns None; malformed records fail.
/// The backend authenticates the assembled path before returning it to execution.
fn read_witness(
    root: Felt,
    requested: Felt,
    mut read: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<Option<CommittedDataWitness>> {
    let Some(metadata) = read(&key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
    let count = count(&metadata)?;
    let mut id = (2 * count - 2) as u32;
    let mut previous = COMMITTED_DATA_TREE_HEIGHT;
    let mut siblings = Vec::new();
    while id as usize >= count {
        let node = decode_branch(&read_record(root, count, id, &mut read)?, id)?;
        anyhow::ensure!(usize::from(node.bit) < previous, "Unordered committed-data branches");
        previous = usize::from(node.bit);
        let (child, sibling) =
            if key_bit(requested, node.bit) { (node.right, node.left) } else { (node.left, node.right) };
        let bytes = read_record(root, count, sibling, &mut read)?;
        let hash = if (sibling as usize) < count {
            leaf(canonical_felt(&bytes[..32])?, canonical_felt(&bytes[32..])?)
        } else {
            decode_branch(&bytes, sibling)?.hash
        };
        siblings.push(CommittedDataSibling { bit: node.bit, hash });
        id = child;
    }
    let bytes = read_record(root, count, id, &mut read)?;
    let stored_key = canonical_felt(&bytes[..32])?;
    if stored_key != requested {
        return Ok(None);
    }
    siblings.reverse();
    Ok(Some(CommittedDataWitness { root, key: requested, value: canonical_felt(&bytes[32..])?, siblings }))
}
/// Fixed-size encoding keeps storage independent of Rust/serde layouts.
fn records(dataset: &CommittedDataSet) -> impl Iterator<Item = (Vec<u8>, Vec<u8>)> + '_ {
    let entries = dataset.entries().chunks(PAGE_VALUES).enumerate().map(|(page, entries)| {
        (
            key(dataset.root(), 0, page as u32),
            entries
                .iter()
                .flat_map(|(key, value)| [key.to_bytes_be(), value.to_bytes_be()].into_iter().flatten())
                .collect(),
        )
    });
    let branches = dataset.branches().chunks(PAGE_VALUES).enumerate().map(|(page, nodes)| {
        let mut bytes = Vec::with_capacity(nodes.len() * BRANCH_BYTES);
        for node in nodes {
            bytes.extend_from_slice(&node.hash.to_bytes_be());
            bytes.push(node.bit);
            bytes.extend_from_slice(&node.left.to_be_bytes());
            bytes.extend_from_slice(&node.right.to_be_bytes());
        }
        (key(dataset.root(), 1, page as u32), bytes)
    });
    entries.chain(branches)
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

    /// Seek over complete roots without scanning their value/hash pages. Each response is bounded.
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
    pub(super) fn read_committed_data_page(&self, root: Felt, start: u32) -> Result<Option<(u32, Vec<(Felt, Felt)>)>> {
        anyhow::ensure!(
            (start as usize) % VALUES_PER_PAGE == 0 && (start as usize) < MAX_COMMITTED_DATA_VALUES,
            "Invalid committed-data page offset"
        );
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let Some(metadata) = snapshot.get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
        let count = count(&metadata)?;
        anyhow::ensure!((start as usize) < count, "Committed-data page outside dataset");
        // The feeder response spans several on-disk pages, but remains a
        // protocol-bounded slice under one RocksDB snapshot.
        let end = (start as usize + VALUES_PER_PAGE).min(count);
        let mut values = Vec::with_capacity(end - start as usize);
        for offset in (start as usize..end).step_by(PAGE_VALUES) {
            let bytes = snapshot
                .get_pinned_cf(&cf, key(root, 0, (offset / PAGE_VALUES) as u32))?
                .ok_or_else(|| anyhow::anyhow!("Committed-data page unavailable"))?;
            anyhow::ensure!(
                bytes.len() == (count - offset).min(PAGE_VALUES) * ENTRY_BYTES,
                "Invalid committed-data page length"
            );
            for raw in bytes.chunks_exact(ENTRY_BYTES) {
                values.push((canonical_felt(&raw[..FELT_BYTES])?, canonical_felt(&raw[FELT_BYTES..])?));
            }
        }
        Ok(Some((count as u32, values)))
    }

    /// Reads metadata and at most two pages per Patricia branch, plus the leaf page.
    pub(super) fn read_committed_data_witness(&self, root: Felt, index: Felt) -> Result<Option<CommittedDataWitness>> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        read_witness(root, index, |key| {
            snapshot
                .get_pinned_cf(&cf, key)?
                .map(|bytes| {
                    anyhow::ensure!(bytes.len() <= PAGE_BYTES, "Oversized committed-data record");
                    Ok(bytes.to_vec())
                })
                .transpose()
        })
    }

    /// Atomically writes all root-keyed pages and metadata with synchronous WAL.
    /// Re-import repairs pages while preserving the original import checkpoint.
    pub(super) fn store_committed_data_dataset(
        &self,
        dataset: &CommittedDataSet,
        imported_after_block: Option<u64>,
    ) -> Result<()> {
        let _write = self
            .inner
            .committed_data_write
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let metadata_key = key(dataset.root(), METADATA_LEVEL, 0);
        let existing = self.inner.db.get_pinned_cf(&cf, &metadata_key)?;
        if let Some(existing) = &existing {
            let (count, _) = decode_metadata(existing)?;
            anyhow::ensure!(count == dataset.entries().len(), "Conflicting committed-data metadata");
        }
        let mut batch = WriteBatch::default();
        let metadata = match existing {
            Some(existing) => existing.to_vec(),
            None => encode_metadata(dataset, imported_after_block)?.to_vec(),
        };
        for (key, bytes) in std::iter::once((metadata_key, metadata)).chain(records(dataset)) {
            // Re-import repairs corrupt pages with the same authenticated logical dataset.
            batch.put_cf(&cf, key, bytes);
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn committed_data_pages_match_the_tree_and_bound_read_work() {
        for size in [1_usize, 2, 255, 256, 257, 513] {
            let values: Vec<_> = (0..size).map(|i| (Felt::from(i), Felt::from(i))).collect();
            let dataset = CommittedDataSet::new(values).unwrap();
            let mut pages: HashMap<_, _> = records(&dataset).collect();
            pages.insert(key(dataset.root(), METADATA_LEVEL, 0), encode_metadata(&dataset, None).unwrap().to_vec());
            for index in [0, size / 2, size - 1] {
                let mut reads = 0;
                let mut bytes = 0;
                let witness = read_witness(dataset.root(), Felt::from(index), |key| {
                    reads += 1;
                    let value = pages.get(key).cloned();
                    bytes += value.as_ref().map_or(0, Vec::len);
                    Ok(value)
                })
                .unwrap()
                .unwrap();
                assert_eq!(witness, dataset.witness(Felt::from(index)).unwrap());
                assert!(witness.verify());
                assert!(reads <= 2 * witness.siblings.len() + 2);
                assert!(bytes <= (2 * witness.siblings.len() + 1) * PAGE_BYTES + METADATA_BYTES);
            }
        }
    }

    #[test]
    fn committed_data_missing_page_is_an_error_not_an_absent_dataset() {
        let dataset = CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap();
        let metadata = key(dataset.root(), METADATA_LEVEL, 0);
        let error = read_witness(dataset.root(), Felt::ZERO, |key| {
            Ok((key == metadata.as_slice()).then(|| encode_metadata(&dataset, None).unwrap().to_vec()))
        })
        .unwrap_err();
        assert!(error.to_string().contains("page unavailable"));
    }

    #[test]
    fn committed_data_records_reject_noncanonical_and_invalid_lengths() {
        assert!(canonical_felt(&[0_u8; 31]).is_err());
        assert!(canonical_felt(&[0xff_u8; 32]).is_err());
        assert_eq!(canonical_felt(&Felt::MAX.to_bytes_be()).unwrap(), Felt::MAX);
        assert!(count(&[]).is_err());
        assert!(count(&0_u32.to_be_bytes()).is_err());
        assert!(count(&u32::MAX.to_be_bytes()).is_err());
        assert!(decode_metadata(&[0, 0, 0, 1, 19]).is_err());
        let dataset = CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap();
        let mut metadata = encode_metadata(&dataset, Some(42)).unwrap();
        assert_eq!(decode_metadata(&metadata).unwrap(), (1, Some(42)));
        metadata[4] = 2;
        assert!(decode_metadata(&metadata).is_err());
    }
}
