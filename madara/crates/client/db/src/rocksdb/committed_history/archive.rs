//! Immutable key dictionaries and bounded, lossless value-delta chains.
use super::*;
use crate::committed_data_recovery::digest;
use crate::persistent_committed_data::Tree;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Read};

pub(super) const HISTORY: &[u8] = b"cdh/v1/";
pub(super) const HISTORY_HASH: &[u8] = b"cdh/hash/";
pub(super) const DICTIONARY: &[u8] = b"cdh/keys/";

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ValueHistory {
    pub(super) base: Option<Felt>,
    pub(super) depth: u32,
    count: u32,
    pub(super) record: CompactValues,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CompactValues {
    root: Felt,
    dictionary: [u8; 32],
    pub(super) data: Vec<u8>,
}
fn exists(db: &RocksDBStorage, root: Felt) -> Result<bool> {
    Ok(db.inner.db.get_pinned_cf(&db.inner.get_column(meta::META_COLUMN), root_key(HISTORY, root))?.is_some())
}
fn read_history(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<ValueHistory> {
    let cf = snapshot.db.get_column(meta::META_COLUMN);
    let bytes = snapshot.get_pinned_cf(&cf, root_key(HISTORY, root))?.context("Missing historical values")?;
    let hash = snapshot.get_pinned_cf(&cf, root_key(HISTORY_HASH, root))?.context("Missing historical checksum")?;
    anyhow::ensure!(hash.as_ref() == digest(&bytes).as_bytes(), "Historical archive checksum mismatch");
    let history: ValueHistory = deserialize(bytes)?;
    anyhow::ensure!(history.record.root == root, "Historical root identity mismatch");
    anyhow::ensure!((1..=MAX_COMMITTED_DATA_VALUES).contains(&(history.count as usize)), "Invalid historical count");
    Ok(history)
}
fn dictionary(snapshot: &SnapshotWithDBArc, hash: &[u8; 32], count: u32) -> Result<Vec<Felt>> {
    let cf = snapshot.db.get_column(meta::META_COLUMN);
    let bytes = snapshot.get_pinned_cf(&cf, named(DICTIONARY, hash))?.context("Missing historical dictionary")?;
    anyhow::ensure!(bytes.len() == count as usize * 32, "Historical dictionary length mismatch");
    anyhow::ensure!(Sha256::digest(&bytes).as_slice() == hash, "Historical dictionary checksum mismatch");
    let keys: Vec<_> = bytes.chunks_exact(32).map(codec::felt).collect::<Result<_>>()?;
    anyhow::ensure!(keys.windows(2).all(|w| w[0] < w[1]), "Unordered historical dictionary");
    Ok(keys)
}

/// Authenticate the complete immutable chain, including its shared dictionaries, before GC.
pub(super) fn history_chain(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<(Vec<Felt>, u32)> {
    let mut next = Some(root);
    let mut records = Vec::new();
    let mut expected_depth = None;
    let mut count = 0;
    let mut dictionaries = HashSet::new();
    while let Some(current) = next {
        anyhow::ensure!(records.len() <= MAX_DELTAS as usize, "Historical value chain exceeds bound");
        let history = read_history(snapshot, current)?;
        if records.is_empty() {
            count = history.count;
        }
        anyhow::ensure!(
            history.depth <= MAX_DELTAS && expected_depth.is_none_or(|d| d == history.depth),
            "Invalid historical depth"
        );
        anyhow::ensure!(history.base.is_some() == (history.depth > 0), "Invalid historical base");
        if dictionaries.insert((history.record.dictionary, history.count)) {
            dictionary(snapshot, &history.record.dictionary, history.count)?;
        }
        expected_depth = history.depth.checked_sub(1);
        next = history.base;
        records.push(current);
    }
    Ok((records, count))
}
pub(super) fn reconstruct(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<CommittedDataSet> {
    let (records, count) = history_chain(snapshot, root)?;
    let mut keys = Vec::new();
    let mut values = Vec::new();
    let mut dictionary_id = None;
    for current in records.into_iter().rev() {
        let history = read_history(snapshot, current)?;
        let record = history.record;
        if history.base.is_none() {
            keys = dictionary(snapshot, &record.dictionary, history.count)?;
            values = vec![Felt::ZERO; keys.len()];
            dictionary_id = Some(record.dictionary);
        } else {
            anyhow::ensure!(
                dictionary_id == Some(record.dictionary) && values.len() == history.count as usize,
                "Historical dictionary changed within delta chain"
            );
        }
        // Each entry needs at most a three-byte index gap and a 34-byte absolute felt.
        let limit = history.count as u64 * 37;
        let mut raw = Vec::new();
        zstd::stream::read::Decoder::new(record.data.as_slice())?.take(limit + 1).read_to_end(&mut raw)?;
        anyhow::ensure!(raw.len() as u64 <= limit, "Historical values exceed decode bound");
        let applied = codec::apply(&raw, &mut values)?;
        anyhow::ensure!(history.base.is_some() || applied == values.len(), "Incomplete historical checkpoint");
    }
    let entries: Vec<_> = keys.into_iter().zip(values).collect();
    anyhow::ensure!(entries.len() == count as usize, "Historical count mismatch");
    let tree = CommittedDataSet::new(entries)?;
    anyhow::ensure!(tree.root() == root, "Reconstructed historical root mismatch");
    Ok(tree)
}

impl RocksDBStorage {
    pub(in crate::rocksdb) fn stage_value_history<F: FnMut(u64) -> Result<Option<Vec<u8>>>>(
        &self,
        batch: &mut WriteBatch,
        root: Felt,
        base: Option<Felt>,
        tree: &mut Tree<F>,
        tree_root: (u64, u32),
    ) -> Result<()> {
        let (id, count) = tree_root;
        if exists(self, root)? {
            return Ok(());
        }
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
        let parent = base
            .filter(|base| *base != root)
            .map(|base| -> Result<_> {
                if exists(self, base)? {
                    Ok(Some(read_history(&snapshot, base)?))
                } else {
                    Ok(None)
                }
            })
            .transpose()?
            .flatten();
        let parent = parent.filter(|p| p.depth < MAX_DELTAS && p.count == count);
        let (base, depth, dictionary_id, raw) = if let Some(ValueHistory { depth, record, .. }) = parent {
            let keys = dictionary(&snapshot, &record.dictionary, count)?;
            let changes: Vec<_> = tree
                .changes
                .iter()
                .map(|&(key, old, new)| {
                    keys.binary_search(&key)
                        .map(|index| (index, old, new))
                        .map_err(|_| anyhow::anyhow!("Changed key missing from dictionary"))
                })
                .collect::<Result<_>>()?;
            (base, depth + 1, record.dictionary, codec::encode(changes))
        } else {
            // A changed key set or depth limit starts a self-contained checkpoint.
            let mut values = Vec::with_capacity(count as usize);
            tree.page(id, 0, count, &mut values)?;
            let keys: Vec<u8> = values.iter().flat_map(|(key, _)| key.to_bytes_be()).collect();
            let hash: [u8; 32] = Sha256::digest(&keys).into();
            let key = named(DICTIONARY, &hash);
            if let Some(existing) = self.inner.db.get_pinned_cf(&cf, &key)? {
                anyhow::ensure!(existing.as_ref() == keys, "Historical dictionary identity mismatch");
            } else {
                batch.put_cf(&cf, key, keys);
            }
            let raw = codec::encode(values.iter().enumerate().map(|(i, (_, value))| (i, Felt::ZERO, *value)));
            (None, 0, hash, raw)
        };
        let record =
            CompactValues { root, dictionary: dictionary_id, data: zstd::stream::encode_all(raw.as_slice(), 3)? };
        let bytes = serialize(&ValueHistory { base, depth, count, record })?;
        batch.put_cf(&cf, root_key(HISTORY_HASH, root), digest(&bytes));
        batch.put_cf(&cf, root_key(HISTORY, root), bytes);
        Ok(())
    }
}
