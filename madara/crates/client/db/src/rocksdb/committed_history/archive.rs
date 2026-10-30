//! Permanent value history, independent from the reclaimable Patricia proof index.
//!
//! Each root stores either a complete checkpoint or a delta against a preceding root. Keys
//! live once in a content-addressed sorted dictionary. Upserts cannot delete keys: equal counts
//! imply the same key set; adding keys or reaching MAX_DELTAS starts a new checkpoint.
//! `codec` packs index gaps and exact value changes, then Zstd compresses that stream.
//! Reads verify record/dictionary checksums, bound decompression, replay oldest-first, and
//! authenticate the reconstructed MPT root. These records are never deleted by proof pruning.
use super::*;
use crate::committed_data_recovery::digest;
use crate::persistent_committed_data::Tree;
use sha2::{Digest, Sha256};
use std::io::Read;

const FELT_BYTES: usize = 32;
const ARCHIVE_COMPRESSION_LEVEL: i32 = 3;
// At most 524288 entries: a gap needs 3 LEB128 bytes, an absolute felt needs 2 + 32.
const MAX_ENCODED_ENTRY_BYTES: u64 = 3 + 2 + FELT_BYTES as u64;

pub(super) const HISTORY: &[u8] = b"cdh/v1/";
pub(super) const HISTORY_HASH: &[u8] = b"cdh/hash/";
pub(super) const DICTIONARY: &[u8] = b"cdh/keys/";
// An optimization hint only: full imports still authenticate every value and the resulting root.
const REUSE_ROOT: &[u8] = b"cdh/reuse/";

/// Find the last locally imported map with exactly this sorted key dictionary.
/// The caller holds the tree writer lock and checks that its proof nodes are still retained.
pub(in crate::rocksdb) fn reusable_root(db: &RocksDBStorage, entries: &[(Felt, Felt)]) -> Result<Option<Felt>> {
    let mut hasher = Sha256::new();
    for (key, _) in entries {
        hasher.update(key.to_bytes_be());
    }
    let dictionary_id: [u8; 32] = hasher.finalize().into();
    let cf = db.inner.get_column(meta::META_COLUMN);
    let Some(bytes) = db.inner.db.get_pinned_cf(&cf, named(REUSE_ROOT, &dictionary_id))? else {
        return Ok(None);
    };
    let root = codec::felt(&bytes)?;
    let snapshot = SnapshotWithDBArc::new(Arc::clone(&db.inner));
    let history = read_history(&snapshot, root)?;
    anyhow::ensure!(history.dictionary == dictionary_id, "Historical reuse dictionary mismatch");
    Ok(Some(root))
}

/// One immutable archive record. `base=None, depth=0` is a self-contained checkpoint.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ValueHistory {
    pub(super) base: Option<Felt>,
    pub(super) depth: u32,
    count: u32,
    root: Felt,
    dictionary: [u8; 32],
    /// Zstd frame containing canonical codec tokens, not serialized felts or MPT nodes.
    pub(super) data: Vec<u8>,
}
fn exists(db: &RocksDBStorage, root: Felt) -> Result<bool> {
    Ok(db.inner.db.get_pinned_cf(&db.inner.get_column(meta::META_COLUMN), root_key(HISTORY, root))?.is_some())
}
/// Verify record bytes and identity before following its base link or decoding its payload.
fn read_history(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<ValueHistory> {
    let cf = snapshot.db.get_column(meta::META_COLUMN);
    let bytes = snapshot.get_pinned_cf(&cf, root_key(HISTORY, root))?.context("Missing historical values")?;
    let hash = snapshot.get_pinned_cf(&cf, root_key(HISTORY_HASH, root))?.context("Missing historical checksum")?;
    anyhow::ensure!(hash.as_ref() == digest(&bytes).as_bytes(), "Historical archive checksum mismatch");
    let history: ValueHistory = deserialize(bytes)?;
    anyhow::ensure!(history.root == root, "Historical root identity mismatch");
    anyhow::ensure!((1..=MAX_COMMITTED_DATA_VALUES).contains(&(history.count as usize)), "Invalid historical count");
    Ok(history)
}
/// Authenticate canonical, strictly sorted keys shared by every record in a delta chain.
fn dictionary(snapshot: &SnapshotWithDBArc, hash: &[u8; 32], count: u32) -> Result<Vec<Felt>> {
    let cf = snapshot.db.get_column(meta::META_COLUMN);
    let bytes = snapshot.get_pinned_cf(&cf, named(DICTIONARY, hash))?.context("Missing historical dictionary")?;
    anyhow::ensure!(bytes.len() == count as usize * FELT_BYTES, "Historical dictionary length mismatch");
    anyhow::ensure!(Sha256::digest(&bytes).as_slice() == hash, "Historical dictionary checksum mismatch");
    let keys: Vec<_> = bytes.chunks_exact(FELT_BYTES).map(codec::felt).collect::<Result<_>>()?;
    anyhow::ensure!(keys.windows(2).all(|w| w[0] < w[1]), "Unordered historical dictionary");
    Ok(keys)
}

/// Verify stored checksums, dictionary identity and decreasing depth before proof GC.
/// This checks archive integrity; `reconstruct` additionally decodes values and checks the MPT root.
pub(super) fn history_chain(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<(Vec<Felt>, u32)> {
    let mut next = Some(root);
    let mut records = Vec::new();
    let mut expected_depth = None;
    let mut count = 0;
    let mut dictionary_id = None;
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
        match dictionary_id {
            None => {
                dictionary(snapshot, &history.dictionary, history.count)?;
                dictionary_id = Some(history.dictionary);
            }
            Some(id) => anyhow::ensure!(
                id == history.dictionary && history.count == count,
                "Historical dictionary changed within delta chain"
            ),
        }
        expected_depth = history.depth.checked_sub(1);
        next = history.base;
        records.push(current);
    }
    Ok((records, count))
}
/// Replay a bounded chain oldest-first into a temporary map, then authenticate its final root.
/// No durable proof nodes are recreated, so historical reads do not undo retention.
pub(super) fn reconstruct(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<CommittedDataSet> {
    let tree = CommittedDataSet::new(values(snapshot, root)?)?;
    anyhow::ensure!(tree.root() == root, "Reconstructed historical root mismatch");
    Ok(tree)
}

/// Decode checksummed archive values without constructing a proof index. Replication consumers
/// authenticate the complete map against its requested root; pages are not membership proofs.
pub(in crate::rocksdb) fn values(snapshot: &SnapshotWithDBArc, root: Felt) -> Result<Vec<(Felt, Felt)>> {
    let (records, count) = history_chain(snapshot, root)?;
    let mut keys = Vec::new();
    let mut values = Vec::new();
    let mut dictionary_id = None;
    for current in records.into_iter().rev() {
        let history = read_history(snapshot, current)?;
        if history.base.is_none() {
            keys = dictionary(snapshot, &history.dictionary, history.count)?;
            values = vec![Felt::ZERO; keys.len()];
            dictionary_id = Some(history.dictionary);
        } else {
            anyhow::ensure!(
                dictionary_id == Some(history.dictionary) && values.len() == history.count as usize,
                "Historical dictionary changed within delta chain"
            );
        }
        // Each entry needs at most a three-byte index gap and a 34-byte absolute felt.
        let limit = history.count as u64 * MAX_ENCODED_ENTRY_BYTES;
        let mut raw = Vec::new();
        zstd::stream::read::Decoder::new(history.data.as_slice())?.take(limit + 1).read_to_end(&mut raw)?;
        anyhow::ensure!(raw.len() as u64 <= limit, "Historical values exceed decode bound");
        let applied = codec::apply(&raw, &mut values)?;
        anyhow::ensure!(history.base.is_some() || applied == values.len(), "Incomplete historical checkpoint");
    }
    let entries: Vec<_> = keys.into_iter().zip(values).collect();
    anyhow::ensure!(entries.len() == count as usize, "Historical count mismatch");
    Ok(entries)
}

impl RocksDBStorage {
    /// Stage a checkpoint or delta alongside the caller's root/node update; never commit separately.
    /// Existing immutable archives are reused. A new key set or full chain starts a checkpoint.
    pub(in crate::rocksdb) fn stage_value_history<F: FnMut(u64) -> Result<Option<Vec<u8>>>>(
        &self,
        batch: &mut WriteBatch,
        root: Felt,
        base: Option<Felt>,
        tree: &mut Tree<F>,
        tree_root: (u64, u32),
    ) -> Result<()> {
        let (id, count) = tree_root;
        let cf = self.inner.get_column(meta::META_COLUMN);
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&self.inner));
        if exists(self, root)? {
            let history = read_history(&snapshot, root)?;
            batch.put_cf(&cf, named(REUSE_ROOT, &history.dictionary), root.to_bytes_be());
            return Ok(());
        }
        let parent =
            base.filter(|base| *base != root).map(|base| -> Result<_> { read_history(&snapshot, base) }).transpose()?;
        let parent = parent.filter(|p| p.depth < MAX_DELTAS && p.count == count);
        let (base, depth, dictionary_id, raw) =
            if let Some(ValueHistory { depth, dictionary: dictionary_id, .. }) = parent {
                let keys = dictionary(&snapshot, &dictionary_id, count)?;
                let changes: Vec<_> = tree
                    .changes
                    .iter()
                    .map(|&(key, old, new)| {
                        keys.binary_search(&key)
                            .map(|index| (index, old, new))
                            .map_err(|_| anyhow::anyhow!("Changed key missing from dictionary"))
                    })
                    .collect::<Result<_>>()?;
                (base, depth + 1, dictionary_id, codec::encode(changes))
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
        let history = ValueHistory {
            base,
            depth,
            count,
            root,
            dictionary: dictionary_id,
            data: zstd::stream::encode_all(raw.as_slice(), ARCHIVE_COMPRESSION_LEVEL)?,
        };
        let bytes = serialize(&history)?;
        batch.put_cf(&cf, named(REUSE_ROOT, &dictionary_id), root.to_bytes_be());
        batch.put_cf(&cf, root_key(HISTORY_HASH, root), digest(&bytes));
        batch.put_cf(&cf, root_key(HISTORY, root), bytes);
        Ok(())
    }
}
