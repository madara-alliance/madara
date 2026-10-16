//! Paged immutable datasets. A cold read touches at most 21 small records.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    leaf, CommittedDataSet, CommittedDataWitness, COMMITTED_DATA_TREE_HEIGHT, MAX_COMMITTED_DATA_VALUES,
};
use rocksdb::WriteBatch;
use starknet_types_core::hash::{Poseidon, StarkHash};

// Caller-independent indexed leaves. The prefix isolates this schema from other records.
const PREFIX: &[u8] = b"committed_data_pages/";
const PAGE_VALUES: usize = 256;
const PAGE_BYTES: usize = PAGE_VALUES * 32;
const METADATA_LEVEL: u8 = u8::MAX;

/// Encodes a root/level/page key; metadata uses the reserved final level.
fn key(root: Felt, level: u8, page: u32) -> Vec<u8> {
    let mut key = PREFIX.to_vec();
    key.extend_from_slice(&root.to_bytes_be());
    key.push(level);
    key.extend_from_slice(&page.to_be_bytes());
    key
}

/// Decodes exactly one canonical field element without silently reducing corrupt bytes.
fn canonical_felt(bytes: &[u8]) -> Result<Felt> {
    let raw: [u8; 32] = bytes.try_into()?;
    let value = Felt::from_bytes_be(&raw);
    anyhow::ensure!(value.to_bytes_be() == raw, "Noncanonical committed-data value");
    Ok(value)
}

/// Metadata stores count, minimum tree height and the local confirmed head at first import admission.
/// That head is an ingestion checkpoint, not the block that published or last used this root.
fn decode_metadata(bytes: &[u8]) -> Result<(usize, usize, Option<u64>)> {
    anyhow::ensure!(bytes.len() == 14, "Invalid committed-data metadata length");
    let count = u32::from_be_bytes(bytes[..4].try_into()?) as usize;
    anyhow::ensure!((1..=MAX_COMMITTED_DATA_VALUES).contains(&count), "Invalid committed-data size");
    let height = usize::from(bytes[4]);
    anyhow::ensure!(
        height <= COMMITTED_DATA_TREE_HEIGHT && height == (count - 1).checked_ilog2().map_or(0, |log| log as usize + 1),
        "Invalid committed-data height"
    );
    let block = u64::from_be_bytes(bytes[6..14].try_into()?);
    let imported_after_block = match bytes[5] {
        0 if block == 0 => None,
        1 => Some(block),
        _ => anyhow::bail!("Invalid committed-data import checkpoint"),
    };
    Ok((count, height, imported_after_block))
}

fn count(bytes: &[u8]) -> Result<usize> {
    Ok(decode_metadata(bytes)?.0)
}

fn encode_metadata(dataset: &CommittedDataSet, imported_after_block: Option<u64>) -> Result<[u8; 14]> {
    let mut bytes = [0_u8; 14];
    bytes[..4].copy_from_slice(&u32::try_from(dataset.values().len())?.to_be_bytes());
    bytes[4] = dataset.height().try_into()?;
    if let Some(block) = imported_after_block {
        bytes[5] = 1;
        bytes[6..14].copy_from_slice(&block.to_be_bytes());
    }
    Ok(bytes)
}

/// Reconstructs a minimum-height path from occupied-prefix pages and deterministic padding.
/// Missing metadata/unused indices return `None`; missing or malformed pages return an error.
/// The callback bounds record bytes. The caller must authenticate the returned path against root.
fn read_witness(
    root: Felt,
    index: u32,
    mut read: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<Option<CommittedDataWitness>> {
    anyhow::ensure!((index as usize) < MAX_COMMITTED_DATA_VALUES, "Committed-data index out of range");
    let Some(metadata) = read(&key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
    let (count, height, _) = decode_metadata(&metadata)?;
    if index as usize >= count {
        return Ok(None);
    }
    let mut read_node = |level: usize, node_index: usize| -> Result<Felt> {
        let node_count = count.div_ceil(1 << level);
        let page = node_index / PAGE_VALUES;
        let bytes = read(&key(root, level.try_into()?, page.try_into()?))?
            .ok_or_else(|| anyhow::anyhow!("Committed-data page unavailable"))?;
        let expected_bytes = (node_count - page * PAGE_VALUES).min(PAGE_VALUES) * 32;
        anyhow::ensure!(bytes.len() == expected_bytes, "Invalid committed-data page length");
        let offset = (node_index % PAGE_VALUES) * 32;
        canonical_felt(&bytes[offset..offset + 32])
    };
    let value = read_node(0, index as usize)?;
    let mut siblings = Vec::with_capacity(height);
    static EMPTY: std::sync::LazyLock<[Felt; COMMITTED_DATA_TREE_HEIGHT]> = std::sync::LazyLock::new(|| {
        let mut hashes = [Felt::ZERO; COMMITTED_DATA_TREE_HEIGHT];
        for height in 1..COMMITTED_DATA_TREE_HEIGHT {
            hashes[height] = Poseidon::hash(&hashes[height - 1], &hashes[height - 1]);
        }
        hashes
    });
    for level in 0..height {
        let sibling_index = ((index as usize) >> level) ^ 1;
        let sibling = if sibling_index < count.div_ceil(1 << level) {
            let node = read_node(level, sibling_index)?;
            if level == 0 {
                leaf(sibling_index.try_into()?, node)
            } else {
                node
            }
        } else {
            EMPTY[level]
        };
        siblings.push(sibling);
    }
    Ok(Some(CommittedDataWitness { root, index, value, siblings }))
}

/// Streams bounded value/hash pages; the root is encoded in each key rather than in a page.
fn records(dataset: &CommittedDataSet) -> impl Iterator<Item = (Vec<u8>, Vec<u8>)> + '_ {
    let levels = std::iter::once(dataset.values()).chain(dataset.internal_levels());
    levels.enumerate().flat_map(move |(level, nodes)| {
        nodes.chunks(PAGE_VALUES).enumerate().map(move |(page, values)| {
            let key = key(dataset.root(), level as u8, page as u32);
            (key, values.iter().flat_map(Felt::to_bytes_be).collect())
        })
    })
}

impl RocksDBStorage {
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
            anyhow::ensure!(raw.len() == PREFIX.len() + 37, "Invalid committed-data key");
            let root = canonical_felt(&raw[PREFIX.len()..PREFIX.len() + 32])?;
            let metadata = snapshot
                .get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))?
                .ok_or_else(|| anyhow::anyhow!("Incomplete committed-data dataset"))?;
            count(&metadata)?;
            roots.push(root);
            if roots.len() == 64 {
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
    pub(super) fn read_committed_data_page(&self, root: Felt, start: u32) -> Result<Option<(u32, Vec<Felt>)>> {
        anyhow::ensure!(
            start % 4096 == 0 && (start as usize) < MAX_COMMITTED_DATA_VALUES,
            "Invalid committed-data page offset"
        );
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let Some(metadata) = snapshot.get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
        let count = count(&metadata)?;
        anyhow::ensure!((start as usize) < count, "Committed-data page outside dataset");
        let end = (start as usize + 4096).min(count);
        let mut values = Vec::with_capacity(end - start as usize);
        for offset in (start as usize..end).step_by(PAGE_VALUES) {
            let bytes = snapshot
                .get_pinned_cf(&cf, key(root, 0, (offset / PAGE_VALUES) as u32))?
                .ok_or_else(|| anyhow::anyhow!("Committed-data page unavailable"))?;
            anyhow::ensure!(
                bytes.len() == (count - offset).min(PAGE_VALUES) * 32,
                "Invalid committed-data page length"
            );
            for raw in bytes.chunks_exact(32) {
                values.push(canonical_felt(raw)?);
            }
        }
        Ok(Some((count as u32, values)))
    }

    /// Reads at most one metadata record and 20 bounded page records, without rebuilding a tree.
    pub(super) fn read_committed_data_witness(&self, root: Felt, index: u32) -> Result<Option<CommittedDataWitness>> {
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
            let (count, height, _) = decode_metadata(existing)?;
            anyhow::ensure!(
                (count, height) == (dataset.values().len(), dataset.height()),
                "Conflicting committed-data metadata"
            );
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
            let values: Vec<_> = (0..size).map(Felt::from).collect();
            let dataset = CommittedDataSet::new(values).unwrap();
            let mut pages: HashMap<_, _> = records(&dataset).collect();
            pages.insert(key(dataset.root(), METADATA_LEVEL, 0), encode_metadata(&dataset, None).unwrap().to_vec());
            for index in [0, size / 2, size - 1] {
                let mut reads = 0;
                let mut bytes = 0;
                let witness = read_witness(dataset.root(), index as u32, |key| {
                    reads += 1;
                    let value = pages.get(key).cloned();
                    bytes += value.as_ref().map_or(0, Vec::len);
                    Ok(value)
                })
                .unwrap()
                .unwrap();
                assert_eq!(witness, dataset.witness(index as u32).unwrap());
                assert!(witness.verify());
                assert!(reads <= dataset.height() + 2);
                assert!(bytes <= (dataset.height() + 1) * PAGE_BYTES + 14);
            }
        }
    }

    #[test]
    fn committed_data_missing_page_is_an_error_not_an_absent_dataset() {
        let dataset = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
        let metadata = key(dataset.root(), METADATA_LEVEL, 0);
        let error = read_witness(dataset.root(), 0, |key| {
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
        let dataset = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
        let mut metadata = encode_metadata(&dataset, Some(42)).unwrap();
        assert_eq!(decode_metadata(&metadata).unwrap(), (1, 0, Some(42)));
        metadata[5] = 2;
        assert!(decode_metadata(&metadata).is_err());
    }
}
