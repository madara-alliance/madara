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
const STAGED_PREFIX: &[u8] = b"committed_data_staged_v1/";
const STAGED_BY_BLOCK_PREFIX: &[u8] = b"committed_data_staged_block_v1/";
const ACTIVE_PREFIX: &[u8] = b"committed_data_active_v1/";
const RETIRED_BY_ROOT_PREFIX: &[u8] = b"committed_data_retired_root_v1/";
const RETIRED_BY_BLOCK_PREFIX: &[u8] = b"committed_data_retired_block_v1/";
const SCAN_CURSOR_PREFIX: &[u8] = b"committed_data_lifecycle_next_block_v1/";
const PRUNE_WATERMARK_KEY: &[u8] = b"committed_data_prune_watermark_v1";
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

fn root_key(prefix: &[u8], root: Felt) -> Vec<u8> {
    let mut key = prefix.to_vec();
    key.extend_from_slice(&root.to_bytes_be());
    key
}

fn active_key(oracle: Felt, slot: u8) -> Vec<u8> {
    let mut key = ACTIVE_PREFIX.to_vec();
    key.extend_from_slice(&oracle.to_bytes_be());
    key.push(slot);
    key
}

fn retired_block_key(block: u64, root: Felt) -> Vec<u8> {
    let mut key = RETIRED_BY_BLOCK_PREFIX.to_vec();
    key.extend_from_slice(&block.to_be_bytes());
    key.extend_from_slice(&root.to_bytes_be());
    key
}

fn staged_block_key(block: u64, root: Felt) -> Vec<u8> {
    let mut key = STAGED_BY_BLOCK_PREFIX.to_vec();
    key.extend_from_slice(&block.to_be_bytes());
    key.extend_from_slice(&root.to_bytes_be());
    key
}

fn prefix_upper_bound(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    let last = end.last_mut().expect("non-empty committed-data prefix");
    *last = last.checked_add(1).expect("committed-data prefix does not end in 0xff");
    end
}

fn u64_value(bytes: &[u8], name: &str) -> Result<u64> {
    Ok(u64::from_be_bytes(bytes.try_into().with_context(|| format!("Invalid {name}"))?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommittedDataLifecycleCursor {
    pub(crate) next_block: u64,
    pub(crate) last_scanned_block_hash: Option<Felt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommittedDataPruneWatermark {
    pub(crate) block: u64,
    pub(crate) block_hash: Felt,
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

/// Exact logical bytes charged by the v2 page layout, derived without reading page contents.
fn dataset_logical_bytes(occupied: usize) -> Result<u64> {
    let key_bytes = u64::try_from(PREFIX.len() + 37)?;
    let mut total = key_bytes + 4; // Metadata key and occupied-count value.
    let mut nodes = occupied;
    for _ in 0..COMMITTED_DATA_TREE_HEIGHT {
        let pages = nodes.div_ceil(PAGE_VALUES);
        let page_bytes = u64::try_from(pages)?.checked_mul(key_bytes).context("Committed-data size overflow")?;
        let node_bytes = u64::try_from(nodes)?.checked_mul(32).context("Committed-data size overflow")?;
        total = total
            .checked_add(page_bytes)
            .and_then(|value| value.checked_add(node_bytes))
            .context("Committed-data size overflow")?;
        nodes = nodes.div_ceil(2);
    }
    Ok(total)
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

    /// Atomically writes all pages and the shared logical-byte quota with synchronous WAL.
    /// Existing roots are repaired in place without a second quota charge. Serialization at
    /// the shared storage owner prevents concurrent backend wrappers from overspending quota.
    pub(super) fn store_committed_data_dataset(
        &self,
        dataset: &CommittedDataSet,
        max_bytes: u64,
        staged_at_block: u64,
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
            anyhow::ensure!(count(existing)? == dataset.values().len(), "Conflicting committed-data metadata");
        }
        let used = self.inner.db.get_pinned_cf(&cf, USAGE_KEY)?;
        let used = used.map(|raw| <[u8; 8]>::try_from(&*raw).map(u64::from_be_bytes)).transpose()?.unwrap_or(0);
        let mut next = used;
        let mut dataset_bytes = 0_u64;
        let mut batch = WriteBatch::default();
        let metadata = u32::try_from(dataset.values().len())?.to_be_bytes().to_vec();
        for (key, bytes) in std::iter::once((metadata_key, metadata)).chain(records(dataset)) {
            let record_bytes = u64::try_from(key.len() + bytes.len())?;
            dataset_bytes = dataset_bytes.checked_add(record_bytes).context("Committed-data size overflow")?;
            if existing.is_none() {
                next =
                    next.checked_add(record_bytes).ok_or_else(|| anyhow::anyhow!("Committed-data quota overflow"))?;
                anyhow::ensure!(
                    next <= max_bytes,
                    "Committed-data storage quota exceeded; archive safely or increase the configured limit"
                );
            }
            // Re-import repairs corrupt pages with the same authenticated logical dataset.
            batch.put_cf(&cf, key, bytes);
        }
        let root = dataset.root();
        debug_assert_eq!(dataset_bytes, dataset_logical_bytes(dataset.values().len())?);
        let staged_key = root_key(STAGED_PREFIX, root);
        if let Some(previous) = self.inner.db.get_pinned_cf(&cf, &staged_key)? {
            batch.delete_cf(&cf, staged_block_key(u64_value(&previous, "committed-data staging block")?, root));
        }
        // The reverse index makes abandoned imports discoverable; re-import atomically renews the lease.
        batch.put_cf(&cf, &staged_key, staged_at_block.to_be_bytes());
        batch.put_cf(&cf, staged_block_key(staged_at_block, root), []);
        batch.put_cf(&cf, USAGE_KEY, next.to_be_bytes());
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
        Ok(())
    }

    /// Applies one canonical Oracle root rotation. Active slots are keyed by Oracle contract,
    /// while retirement is root-global so a root shared by slots/contracts stays pinned.
    pub(crate) fn record_committed_data_publication(
        &self,
        oracle: Felt,
        block: u64,
        price_root: Felt,
        funding_root: Felt,
    ) -> Result<()> {
        anyhow::ensure!(price_root != Felt::ZERO && funding_root != Felt::ZERO, "Empty committed-data root event");
        let _write = self
            .inner
            .committed_data_write
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);

        let mut active_counts = std::collections::HashMap::<Felt, usize>::new();
        let mut iterator = self.inner.db.raw_iterator_cf(&cf);
        iterator.seek(ACTIVE_PREFIX);
        while let Some(raw_key) = iterator.key().filter(|key| key.starts_with(ACTIVE_PREFIX)) {
            anyhow::ensure!(raw_key.len() == ACTIVE_PREFIX.len() + 33, "Invalid committed-data active key");
            let raw_root = iterator.value().context("Missing committed-data active root")?;
            *active_counts.entry(canonical_felt(raw_root)?).or_default() += 1;
            iterator.next();
        }
        iterator.status()?;
        drop(iterator);

        let previous_price =
            self.inner.db.get_pinned_cf(&cf, active_key(oracle, 0))?.map(|root| canonical_felt(&root)).transpose()?;
        let previous_funding =
            self.inner.db.get_pinned_cf(&cf, active_key(oracle, 1))?.map(|root| canonical_felt(&root)).transpose()?;
        let previous = [previous_price, previous_funding];
        for root in previous.into_iter().flatten() {
            let count = active_counts.get_mut(&root).context("Missing committed-data active reference")?;
            *count -= 1;
        }
        for root in [price_root, funding_root] {
            *active_counts.entry(root).or_default() += 1;
        }

        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, active_key(oracle, 0), price_root.to_bytes_be());
        batch.put_cf(&cf, active_key(oracle, 1), funding_root.to_bytes_be());
        for root in [price_root, funding_root] {
            let staged_key = root_key(STAGED_PREFIX, root);
            if let Some(staged_at) = self
                .inner
                .db
                .get_pinned_cf(&cf, &staged_key)?
                .map(|value| u64_value(&value, "committed-data staging block"))
                .transpose()?
            {
                if block >= staged_at {
                    batch.delete_cf(&cf, staged_block_key(staged_at, root));
                    batch.delete_cf(&cf, staged_key);
                }
            }
            let retired_key = root_key(RETIRED_BY_ROOT_PREFIX, root);
            if let Some(retired_at) = self
                .inner
                .db
                .get_pinned_cf(&cf, &retired_key)?
                .map(|value| u64_value(&value, "committed-data retirement block"))
                .transpose()?
            {
                if block > retired_at {
                    batch.delete_cf(&cf, retired_block_key(retired_at, root));
                    batch.delete_cf(&cf, retired_key);
                }
            }
        }
        for root in previous.into_iter().flatten() {
            if active_counts.get(&root).copied().unwrap_or_default() == 0 {
                let staged_key = root_key(STAGED_PREFIX, root);
                if let Some(staged_at) = self
                    .inner
                    .db
                    .get_pinned_cf(&cf, &staged_key)?
                    .map(|value| u64_value(&value, "committed-data staging block"))
                    .transpose()?
                {
                    if block >= staged_at {
                        batch.delete_cf(&cf, staged_block_key(staged_at, root));
                        batch.delete_cf(&cf, staged_key);
                    }
                }
                let retired_key = root_key(RETIRED_BY_ROOT_PREFIX, root);
                let previous_retirement = self
                    .inner
                    .db
                    .get_pinned_cf(&cf, &retired_key)?
                    .map(|value| u64_value(&value, "committed-data retirement block"))
                    .transpose()?;
                let retirement = previous_retirement.map_or(block, |previous| previous.max(block));
                if previous_retirement != Some(retirement) {
                    if let Some(previous) = previous_retirement {
                        batch.delete_cf(&cf, retired_block_key(previous, root));
                    }
                    batch.put_cf(&cf, &retired_key, retirement.to_be_bytes());
                    batch.put_cf(&cf, retired_block_key(retirement, root), []);
                }
            }
        }
        // Lifecycle rows precede a synchronous cursor write. Keep them in the WAL even when the
        // general database setting disables it, so a durable cursor can never skip lost rows.
        let mut options = WriteOptions::default();
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
        Ok(())
    }

    pub(crate) fn committed_data_lifecycle_cursor(&self, oracle: Felt) -> Result<CommittedDataLifecycleCursor> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let Some(value) = self.inner.db.get_pinned_cf(&cf, root_key(SCAN_CURSOR_PREFIX, oracle))? else {
            return Ok(CommittedDataLifecycleCursor { next_block: 0, last_scanned_block_hash: None });
        };
        let next_block = u64_value(
            value.get(..8).context("Invalid committed-data lifecycle cursor")?,
            "committed-data lifecycle cursor",
        )?;
        let last_scanned_block_hash = match value.len() {
            8 => None, // Legacy unanchored cursor; the caller rebuilds lifecycle metadata.
            40 => Some(canonical_felt(&value[8..])?),
            _ => anyhow::bail!("Invalid committed-data lifecycle cursor"),
        };
        Ok(CommittedDataLifecycleCursor { next_block, last_scanned_block_hash })
    }

    pub(crate) fn write_committed_data_lifecycle_cursor(
        &self,
        oracle: Felt,
        next_block: u64,
        last_scanned_block_hash: Felt,
    ) -> Result<()> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let mut value = Vec::with_capacity(40);
        value.extend_from_slice(&next_block.to_be_bytes());
        value.extend_from_slice(&last_scanned_block_hash.to_bytes_be());
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.put_cf_opt(&cf, root_key(SCAN_CURSOR_PREFIX, oracle), value, &options)?;
        Ok(())
    }

    pub(crate) fn committed_data_prune_watermark(&self) -> Result<Option<CommittedDataPruneWatermark>> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        let Some(value) = self.inner.db.get_pinned_cf(&cf, PRUNE_WATERMARK_KEY)? else { return Ok(None) };
        anyhow::ensure!(value.len() == 40, "Invalid committed-data prune watermark");
        Ok(Some(CommittedDataPruneWatermark {
            block: u64_value(&value[..8], "committed-data prune watermark block")?,
            block_hash: canonical_felt(&value[8..])?,
        }))
    }

    /// Clears only chain-derived lifecycle rows. Imported datasets and staging leases survive and
    /// are protected while canonical publication events are replayed from genesis.
    pub(crate) fn reset_committed_data_lifecycle(&self) -> Result<()> {
        let _write = self
            .inner
            .committed_data_write
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let mut batch = WriteBatch::default();
        for prefix in [ACTIVE_PREFIX, RETIRED_BY_ROOT_PREFIX, RETIRED_BY_BLOCK_PREFIX, SCAN_CURSOR_PREFIX] {
            batch.delete_range_cf(&cf, prefix.to_vec(), prefix_upper_bound(prefix));
        }
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
        Ok(())
    }

    /// Deletes a tiny bounded batch using range tombstones. It never forces compaction, so normal
    /// RocksDB background policy controls physical reclamation without blocking sequencing.
    pub(crate) fn prune_committed_data(
        &self,
        cutoff: u64,
        cutoff_hash: Felt,
        limit: usize,
        current_roots: &std::collections::HashSet<Felt>,
    ) -> Result<Vec<Felt>> {
        let _write = self
            .inner
            .committed_data_write
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data write lock poisoned"))?;
        let cf = self.inner.get_column(meta::META_COLUMN);

        let mut active = std::collections::HashSet::new();
        let mut iterator = self.inner.db.raw_iterator_cf(&cf);
        iterator.seek(ACTIVE_PREFIX);
        while let Some(raw_key) = iterator.key().filter(|key| key.starts_with(ACTIVE_PREFIX)) {
            anyhow::ensure!(raw_key.len() == ACTIVE_PREFIX.len() + 33, "Invalid committed-data active key");
            active.insert(canonical_felt(iterator.value().context("Missing committed-data active root")?)?);
            iterator.next();
        }
        iterator.status()?;
        drop(iterator);

        let scan_limit = limit.saturating_mul(16).max(1);
        let mut candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut stale_index_keys = Vec::new();
        let mut iterator = self.inner.db.raw_iterator_cf(&cf);
        iterator.seek(RETIRED_BY_BLOCK_PREFIX);
        let mut examined = 0;
        while examined < scan_limit {
            let Some(raw_key) = iterator.key().filter(|key| key.starts_with(RETIRED_BY_BLOCK_PREFIX)) else { break };
            examined += 1;
            anyhow::ensure!(
                raw_key.len() == RETIRED_BY_BLOCK_PREFIX.len() + 40,
                "Invalid committed-data retirement key"
            );
            let offset = RETIRED_BY_BLOCK_PREFIX.len();
            let block = u64_value(&raw_key[offset..offset + 8], "committed-data retirement index block")?;
            if block > cutoff {
                break;
            }
            let root = canonical_felt(&raw_key[offset + 8..])?;
            let reverse = self
                .inner
                .db
                .get_pinned_cf(&cf, root_key(RETIRED_BY_ROOT_PREFIX, root))?
                .map(|value| u64_value(&value, "committed-data retirement block"))
                .transpose()?;
            if reverse != Some(block) {
                stale_index_keys.push(raw_key.to_vec());
            } else if seen.insert(root) {
                candidates.push((block, root));
            }
            iterator.next();
        }
        iterator.status()?;
        drop(iterator);

        let mut iterator = self.inner.db.raw_iterator_cf(&cf);
        iterator.seek(STAGED_BY_BLOCK_PREFIX);
        examined = 0;
        while examined < scan_limit {
            let Some(raw_key) = iterator.key().filter(|key| key.starts_with(STAGED_BY_BLOCK_PREFIX)) else { break };
            examined += 1;
            anyhow::ensure!(
                raw_key.len() == STAGED_BY_BLOCK_PREFIX.len() + 40,
                "Invalid committed-data staging index key"
            );
            let offset = STAGED_BY_BLOCK_PREFIX.len();
            let block = u64_value(&raw_key[offset..offset + 8], "committed-data staging index block")?;
            if block > cutoff {
                break;
            }
            let root = canonical_felt(&raw_key[offset + 8..])?;
            let reverse = self
                .inner
                .db
                .get_pinned_cf(&cf, root_key(STAGED_PREFIX, root))?
                .map(|value| u64_value(&value, "committed-data staging block"))
                .transpose()?;
            if reverse != Some(block) {
                stale_index_keys.push(raw_key.to_vec());
            } else if seen.insert(root) {
                candidates.push((block, root));
            }
            iterator.next();
        }
        iterator.status()?;
        drop(iterator);

        let used = self.inner.db.get_pinned_cf(&cf, USAGE_KEY)?;
        let mut used = used.map(|raw| u64_value(&raw, "committed-data quota usage")).transpose()?.unwrap_or(0);
        let mut deleted = Vec::new();
        let mut batch = WriteBatch::default();
        for stale_key in stale_index_keys {
            batch.delete_cf(&cf, stale_key);
        }
        for (_, root) in candidates {
            if deleted.len() == limit {
                break;
            }
            // Do not clean an active root's older indexes here: Oracle histories are replayed
            // independently, so a lower-block activation can be observed after a later retirement.
            if active.contains(&root) {
                continue;
            }
            if current_roots.contains(&root) {
                continue;
            }

            let staged_key = root_key(STAGED_PREFIX, root);
            let staged_at = self
                .inner
                .db
                .get_pinned_cf(&cf, &staged_key)?
                .map(|value| u64_value(&value, "committed-data staging block"))
                .transpose()?;
            if staged_at.is_some_and(|block| block > cutoff) {
                continue;
            }

            let retired_key = root_key(RETIRED_BY_ROOT_PREFIX, root);
            let retired_at = self
                .inner
                .db
                .get_pinned_cf(&cf, &retired_key)?
                .map(|value| u64_value(&value, "committed-data retirement block"))
                .transpose()?;
            if !retired_at.is_some_and(|block| block <= cutoff) && !staged_at.is_some_and(|block| block <= cutoff) {
                continue;
            }

            let metadata_key = key(root, METADATA_LEVEL, 0);
            if self.inner.db.get_pinned_cf(&cf, &metadata_key)?.is_none() {
                if let Some(block) = retired_at {
                    batch.delete_cf(&cf, retired_block_key(block, root));
                    batch.delete_cf(&cf, &retired_key);
                }
                if let Some(block) = staged_at {
                    batch.delete_cf(&cf, staged_block_key(block, root));
                    batch.delete_cf(&cf, &staged_key);
                }
                continue;
            }
            let metadata = self.inner.db.get_pinned_cf(&cf, &metadata_key)?.expect("metadata presence checked");
            let size = dataset_logical_bytes(count(&metadata)?)?;
            used = used.checked_sub(size).context("Committed-data quota usage underflow")?;

            let range_start = root_key(PREFIX, root);
            let mut range_end = range_start.clone();
            range_end.extend_from_slice(&[u8::MAX; 6]);
            batch.delete_range_cf(&cf, range_start, range_end);
            if let Some(block) = retired_at {
                batch.delete_cf(&cf, retired_block_key(block, root));
                batch.delete_cf(&cf, &retired_key);
            }
            if let Some(block) = staged_at {
                batch.delete_cf(&cf, staged_block_key(block, root));
                batch.delete_cf(&cf, &staged_key);
            }
            deleted.push(root);
        }
        if !deleted.is_empty() {
            batch.put_cf(&cf, USAGE_KEY, used.to_be_bytes());
            let previous = self.committed_data_prune_watermark()?;
            if let Some(previous) = previous.filter(|watermark| watermark.block == cutoff) {
                anyhow::ensure!(previous.block_hash == cutoff_hash, "Committed-data prune watermark hash changed");
            }
            let watermark = previous
                .filter(|watermark| watermark.block > cutoff)
                .unwrap_or(CommittedDataPruneWatermark { block: cutoff, block_hash: cutoff_hash });
            let mut value = Vec::with_capacity(40);
            value.extend_from_slice(&watermark.block.to_be_bytes());
            value.extend_from_slice(&watermark.block_hash.to_bytes_be());
            batch.put_cf(&cf, PRUNE_WATERMARK_KEY, value);
        }
        if !batch.is_empty() {
            let mut options = WriteOptions::default();
            options.set_sync(true);
            options.disable_wal(false);
            self.inner.db.write_opt(batch, &options)?;
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MadaraBackend, MadaraStorageRead, MadaraStorageWrite};
    use mp_chain_config::ChainConfig;
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
    fn committed_data_snapshot_keeps_an_in_progress_witness_read_alive_during_pruning() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let old = CommittedDataSet::new((0..513_u32).map(Felt::from).collect()).unwrap();
        let replacement = CommittedDataSet::new(vec![Felt::from(900_u64)]).unwrap();
        backend.db.write_committed_data_dataset(&old, u64::MAX, 0).unwrap();
        backend.db.write_committed_data_dataset(&replacement, u64::MAX, 0).unwrap();
        backend.db.record_committed_data_publication(Felt::ONE, 0, old.root(), old.root()).unwrap();
        backend.db.record_committed_data_publication(Felt::ONE, 1, replacement.root(), replacement.root()).unwrap();

        let snapshot = rocksdb_snapshot::SnapshotWithDBArc::new(Arc::clone(&backend.db.inner));
        let cf = backend.db.inner.get_column(meta::META_COLUMN);
        let mut pruned = false;
        let witness = read_witness(old.root(), 512, |key| {
            let value = snapshot.get_pinned_cf(&cf, key)?.map(|bytes| bytes.to_vec());
            if !pruned {
                pruned = true;
                assert_eq!(
                    backend.db.prune_committed_data(1, Felt::ONE, 8, &std::collections::HashSet::new())?,
                    vec![old.root()]
                );
            }
            Ok(value)
        })
        .unwrap()
        .unwrap();

        assert!(witness.verify());
        assert!(backend.db.get_committed_data_witness(old.root(), 512).unwrap().is_none());
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
