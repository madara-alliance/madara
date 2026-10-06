//! Versioned, paged immutable datasets. A cold read touches at most 21 small records.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    leaf, CommittedDataSet, CommittedDataWitness, COMMITTED_DATA_TREE_HEIGHT, MAX_COMMITTED_DATA_VALUES,
};
use rocksdb::WriteBatch;
use starknet_types_core::hash::{Poseidon, StarkHash};

// Caller-independent leaves have different roots. Never interpret legacy publisher-bound pages
// using this schema; old roots must be rebuilt and explicitly re-imported.
const PREFIX: &[u8] = b"committed_data_pages_v2/";
// Retain the shared quota counter so preserved legacy pages still count toward storage usage.
const USAGE_KEY: &[u8] = b"committed_data_pages_usage_v1";
const PAGE_VALUES: usize = 256;
const PAGE_BYTES: usize = PAGE_VALUES * 32;
const METADATA_LEVEL: u8 = u8::MAX;

/// Encodes a versioned root/level/page key; metadata uses the reserved final level.
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

/// Decodes and bounds the occupied-leaf count before any page arithmetic.
fn count(bytes: &[u8]) -> Result<usize> {
    let count = u32::from_be_bytes(bytes.try_into()?) as usize;
    anyhow::ensure!((1..=MAX_COMMITTED_DATA_VALUES).contains(&count), "Invalid committed-data size");
    Ok(count)
}

/// Reconstructs a fixed-height path from occupied-prefix pages and deterministic padding.
/// Missing metadata/unused indices return `None`; missing or malformed pages return an error.
/// The callback bounds record bytes. The caller must authenticate the returned path against root.
fn read_witness(
    root: Felt,
    index: u32,
    mut read: impl FnMut(&[u8]) -> Result<Option<Vec<u8>>>,
) -> Result<Option<CommittedDataWitness>> {
    anyhow::ensure!((index as usize) < MAX_COMMITTED_DATA_VALUES, "Committed-data index out of range");
    let Some(metadata) = read(&key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
    let count = count(&metadata)?;
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
    let mut siblings = [Felt::ZERO; COMMITTED_DATA_TREE_HEIGHT];
    static EMPTY: std::sync::LazyLock<[Felt; COMMITTED_DATA_TREE_HEIGHT]> = std::sync::LazyLock::new(|| {
        let mut hashes = [Felt::ZERO; COMMITTED_DATA_TREE_HEIGHT];
        for height in 1..COMMITTED_DATA_TREE_HEIGHT {
            hashes[height] = Poseidon::hash(&hashes[height - 1], &hashes[height - 1]);
        }
        hashes
    });
    for (height, sibling) in siblings.iter_mut().enumerate() {
        let sibling_index = ((index as usize) >> height) ^ 1;
        if sibling_index < count.div_ceil(1 << height) {
            let node = read_node(height, sibling_index)?;
            *sibling = if height == 0 { leaf(sibling_index.try_into()?, node) } else { node };
        } else {
            *sibling = EMPTY[height];
        }
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
        let mut iterator = self.inner.db.raw_iterator_cf(&cf);
        match after {
            Some(root) => iterator.seek(key(root, METADATA_LEVEL, u32::MAX)),
            None => iterator.seek(PREFIX),
        }
        let mut roots = Vec::new();
        while let Some(raw) = iterator.key().filter(|raw| raw.starts_with(PREFIX)) {
            anyhow::ensure!(raw.len() == PREFIX.len() + 37, "Invalid committed-data key");
            let root = canonical_felt(&raw[PREFIX.len()..PREFIX.len() + 32])?;
            let metadata = self
                .inner
                .db
                .get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))?
                .ok_or_else(|| anyhow::anyhow!("Incomplete committed-data dataset"))?;
            count(&metadata)?;
            roots.push(root);
            if roots.len() == 64 {
                break;
            }
            iterator.seek(key(root, METADATA_LEVEL, u32::MAX));
        }
        iterator.status()?;
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
        let Some(metadata) = self.inner.db.get_pinned_cf(&cf, key(root, METADATA_LEVEL, 0))? else { return Ok(None) };
        let count = count(&metadata)?;
        anyhow::ensure!((start as usize) < count, "Committed-data page outside dataset");
        let end = (start as usize + 4096).min(count);
        let mut values = Vec::with_capacity(end - start as usize);
        for offset in (start as usize..end).step_by(PAGE_VALUES) {
            let bytes = self
                .inner
                .db
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
        read_witness(root, index, |key| {
            self.inner
                .db
                .get_pinned_cf(&cf, key)?
                .map(|bytes| {
                    anyhow::ensure!(bytes.len() <= PAGE_BYTES, "Oversized committed-data record");
                    Ok(bytes.to_vec())
                })
                .transpose()
        })
    }

    /// Atomically writes all pages and the shared logical-byte quota with synchronous WAL.
    /// Existing roots are repaired in place without a second quota charge. Serialization at
    /// the shared storage owner prevents concurrent backend wrappers from overspending quota.
    pub(super) fn store_committed_data_dataset(&self, dataset: &CommittedDataSet, max_bytes: u64) -> Result<()> {
        let _write = self
            .inner
            .committed_data_write
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let metadata_key = key(dataset.root(), METADATA_LEVEL, 0);
        let existing = self.inner.db.get_pinned_cf(&cf, &metadata_key)?;
        if let Some(existing) = &existing {
            anyhow::ensure!(count(existing)? == dataset.values().len(), "Conflicting committed-data metadata");
        }
        let used = self.inner.db.get_pinned_cf(&cf, USAGE_KEY)?;
        let used = used.map(|raw| <[u8; 8]>::try_from(&*raw).map(u64::from_be_bytes)).transpose()?.unwrap_or(0);
        let mut next = used;
        let mut batch = WriteBatch::default();
        let metadata = u32::try_from(dataset.values().len())?.to_be_bytes().to_vec();
        for (key, bytes) in std::iter::once((metadata_key, metadata)).chain(records(dataset)) {
            if existing.is_none() {
                next = next
                    .checked_add(u64::try_from(key.len() + bytes.len())?)
                    .ok_or_else(|| anyhow::anyhow!("Committed-data quota overflow"))?;
                anyhow::ensure!(
                    next <= max_bytes,
                    "Committed-data storage quota exceeded; archive safely or increase the configured limit"
                );
            }
            // Re-import repairs corrupt pages with the same authenticated logical dataset.
            batch.put_cf(&cf, key, bytes);
        }
        batch.put_cf(&cf, USAGE_KEY, next.to_be_bytes());
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
            let dataset = CommittedDataSet::new((0..size).map(Felt::from).collect()).unwrap();
            let mut pages: HashMap<_, _> = records(&dataset).collect();
            pages.insert(key(dataset.root(), METADATA_LEVEL, 0), (size as u32).to_be_bytes().to_vec());
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
                assert!(reads <= COMMITTED_DATA_TREE_HEIGHT + 2);
                assert!(bytes <= (COMMITTED_DATA_TREE_HEIGHT + 1) * PAGE_BYTES + 4);
            }
        }
    }

    #[test]
    fn committed_data_pages_do_not_read_legacy_publisher_bound_records() {
        let dataset = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
        let mut legacy_key = b"committed_data_pages_v1/".to_vec();
        legacy_key.extend_from_slice(&Felt::ONE.to_bytes_be()); // Old publisher.
        legacy_key.extend_from_slice(&dataset.root().to_bytes_be());
        legacy_key.push(METADATA_LEVEL);
        legacy_key.extend_from_slice(&0_u32.to_be_bytes());
        let legacy = HashMap::from([(legacy_key, 1_u32.to_be_bytes().to_vec())]);
        assert!(read_witness(dataset.root(), 0, |key| Ok(legacy.get(key).cloned())).unwrap().is_none());
    }

    #[test]
    fn committed_data_missing_page_is_an_error_not_an_absent_dataset() {
        let dataset = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
        let metadata = key(dataset.root(), METADATA_LEVEL, 0);
        let error = read_witness(dataset.root(), 0, |key| {
            Ok((key == metadata.as_slice()).then(|| 1_u32.to_be_bytes().to_vec()))
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
    }
}
