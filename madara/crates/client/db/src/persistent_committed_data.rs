//! Immutable Patricia nodes shared by root versions. No deletion or retention policy lives here.
//!
//! IDs are append-only arena offsets, not protocol identifiers. Hashing and witnesses retain the
//! existing committed-data format. A writer publishes nodes, allocator and root pointer atomically.
use anyhow::{ensure, Context, Result};
use blockifier::execution::syscalls::committed_data::{
    branch, key_bit, leaf, CommittedDataSibling, CommittedDataWitness,
};
use lru::LruCache;
use starknet_types_core::felt::Felt;
use std::num::NonZeroUsize;

/// At most 2M nodes across ticks (roughly a few hundred MiB, lazily allocated). This cache is
/// database-local, is discarded on reopen and never supplies data not already durable.
pub(crate) struct NodeCache(LruCache<u64, (Node, bool)>);
impl Default for NodeCache {
    fn default() -> Self {
        let mut cache = LruCache::unbounded();
        cache.resize(NonZeroUsize::new(2_097_152).expect("Node-cache capacity is nonzero"));
        Self(cache)
    }
}

pub(crate) const NEXT_KEY: &[u8] = b"cdm/next";
pub(crate) fn node_key(id: u64) -> [u8; 14] {
    let mut key = [0; 14];
    key[..6].copy_from_slice(b"cdm/n/");
    key[6..].copy_from_slice(&id.to_be_bytes());
    key
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Node {
    Leaf { key: Felt, value: Felt, hash: Felt },
    Branch { bit: u8, left: u64, right: u64, hash: Felt, count: u32 },
}
impl Node {
    fn new_leaf(key: Felt, value: Felt) -> Self {
        Self::Leaf { key, value, hash: leaf(key, value) }
    }
    pub(crate) fn hash(self) -> Felt {
        match self {
            Self::Leaf { hash, .. } => hash,
            Self::Branch { hash, .. } => hash,
        }
    }
    fn count(self) -> u32 {
        match self {
            Self::Leaf { .. } => 1,
            Self::Branch { count, .. } => count,
        }
    }
    fn bit(self) -> i16 {
        match self {
            Self::Leaf { .. } => -1,
            Self::Branch { bit, .. } => bit.into(),
        }
    }
    pub(crate) fn encode(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(65);
        match self {
            Self::Leaf { key, value, .. } => {
                out.push(0);
                out.extend(key.to_bytes_be());
                out.extend(value.to_bytes_be());
            }
            Self::Branch { bit, left, right, hash, count } => {
                out.extend([1, bit]);
                out.extend(left.to_be_bytes());
                out.extend(right.to_be_bytes());
                out.extend(hash.to_bytes_be());
                out.extend(count.to_be_bytes());
            }
        }
        out
    }
    fn decode(id: u64, bytes: &[u8]) -> Result<Self> {
        fn felt(bytes: &[u8]) -> Result<Felt> {
            let raw: [u8; 32] = bytes.try_into()?;
            let f = Felt::from_bytes_be(&raw);
            ensure!(f.to_bytes_be() == raw, "Noncanonical persistent felt");
            Ok(f)
        }
        match bytes.first() {
            Some(0) if bytes.len() == 65 => Ok(Self::new_leaf(felt(&bytes[1..33])?, felt(&bytes[33..])?)),
            Some(1) if bytes.len() == 54 => {
                let bit = bytes[1];
                let left = u64::from_be_bytes(bytes[2..10].try_into()?);
                let right = u64::from_be_bytes(bytes[10..18].try_into()?);
                let count = u32::from_be_bytes(bytes[50..54].try_into()?);
                ensure!(
                    bit < 252 && left > 0 && right > 0 && left < id && right < id && left != right && count >= 2,
                    "Invalid persistent branch"
                );
                Ok(Self::Branch { bit, left, right, hash: felt(&bytes[18..50])?, count })
            }
            _ => anyhow::bail!("Invalid persistent node encoding"),
        }
    }
}

fn differing_bit(a: Felt, b: Felt) -> i16 {
    a.to_bytes_be()
        .iter()
        .zip(b.to_bytes_be())
        .enumerate()
        .find_map(|(i, (a, b))| {
            let x = a ^ b;
            (x != 0).then(|| ((31 - i) * 8 + 7 - x.leading_zeros() as usize) as i16)
        })
        .unwrap_or(-1)
}

/// One bounded operation. The caller serializes writers and discards this plan on any failure.
/// Writers may carry the durable-node cache between operations. Staged nodes stay in this plan
/// until commit; independent readers cannot observe them through the cache.
pub(crate) struct Tree<F> {
    read: F,
    pub(crate) first_id: u64,
    pub(crate) nodes: Vec<Node>,
    cache: NodeCache,
    replaced: Vec<u64>,
    limit: u32,
}
impl<F: FnMut(u64) -> Result<Option<Vec<u8>>>> Tree<F> {
    pub(crate) fn new(first_id: u64, limit: u32, read: F) -> Self {
        Self { read, first_id, nodes: Vec::new(), cache: NodeCache::default(), replaced: Vec::new(), limit }
    }
    pub(crate) fn with_cache(first_id: u64, limit: u32, read: F, cache: NodeCache) -> Self {
        Self { read, first_id, nodes: Vec::new(), cache, replaced: Vec::new(), limit }
    }
    pub(crate) fn into_read_cache(self) -> NodeCache {
        self.cache
    }
    /// Call only after the nodes and root pointer have been successfully committed.
    pub(crate) fn into_committed_cache(mut self) -> NodeCache {
        // Prior versions remain on disk. Prefer the current tree in RAM so obsolete paths do
        // not displace its still-live nodes merely because each tick creates a new version.
        for id in self.replaced {
            self.cache.0.pop(&id);
        }
        for (i, node) in self.nodes.into_iter().enumerate() {
            self.cache.0.put(self.first_id + i as u64, (node, true));
        }
        self.cache
    }
    fn raw(&mut self, id: u64) -> Result<Node> {
        ensure!(id != 0, "Invalid persistent node ID");
        if id >= self.first_id {
            return self.nodes.get(usize::try_from(id - self.first_id)?).copied().context("Missing staged node");
        }
        if let Some((node, _)) = self.cache.0.get(&id) {
            return Ok(*node);
        }
        let bytes = (self.read)(id)?.context("Persistent node unavailable")?;
        let node = Node::decode(id, &bytes)?;
        ensure!(node.count() <= self.limit, "Persistent subtree exceeds bound");
        self.cache.0.put(id, (node, false));
        Ok(node)
    }
    fn get(&mut self, id: u64) -> Result<Node> {
        let node = self.raw(id)?;
        if id >= self.first_id || self.cache.0.get(&id).is_some_and(|(_, checked)| *checked) {
            return Ok(node);
        }
        if let Node::Branch { bit, left, right, hash, count } = node {
            let l = self.raw(left)?;
            let r = self.raw(right)?;
            ensure!(l.bit() < bit.into() && r.bit() < bit.into(), "Invalid persistent path order");
            ensure!(l.count().checked_add(r.count()) == Some(count), "Invalid persistent subtree count");
            ensure!(branch(bit, l.hash(), r.hash()) == hash, "Persistent branch hash mismatch");
        }
        if let Some(entry) = self.cache.0.get_mut(&id) {
            entry.1 = true;
        }
        Ok(node)
    }
    pub(crate) fn check_root(&mut self, id: u64, root: Felt, count: u32) -> Result<()> {
        let node = self.get(id)?;
        ensure!(node.hash() == root && node.count() == count, "Persistent root mismatch");
        Ok(())
    }
    fn push(&mut self, node: Node) -> Result<u64> {
        ensure!(node.count() <= self.limit, "Persistent dataset exceeds bound");
        let id = self.first_id.checked_add(self.nodes.len().try_into()?).context("Node ID overflow")?;
        ensure!(id < u64::MAX, "Node ID exhaustion");
        self.nodes.push(node);
        Ok(id)
    }
    fn join(&mut self, bit: u8, left: u64, right: u64) -> Result<u64> {
        let l = self.get(left)?;
        let r = self.get(right)?;
        ensure!(l.bit() < bit.into() && r.bit() < bit.into(), "Invalid new path order");
        let count = l.count().checked_add(r.count()).context("Subtree count overflow")?;
        self.push(Node::Branch { bit, left, right, count, hash: branch(bit, l.hash(), r.hash()) })
    }
    fn build_sorted(&mut self, entries: &[(Felt, Felt)]) -> Result<u64> {
        if entries.len() == 1 {
            return self.push(Node::new_leaf(entries[0].0, entries[0].1));
        }
        let bit = differing_bit(entries[0].0, entries[entries.len() - 1].0) as u8;
        let split = entries.partition_point(|e| !key_bit(e.0, bit));
        let left = self.build_sorted(&entries[..split])?;
        let right = self.build_sorted(&entries[split..])?;
        self.join(bit, left, right)
    }
    pub(crate) fn prepare(entries: &mut [(Felt, Felt)], limit: u32) -> Result<()> {
        ensure!(!entries.is_empty() && entries.len() <= limit as usize, "Invalid persistent patch size");
        entries.sort_unstable_by_key(|e| e.0);
        ensure!(entries.windows(2).all(|p| p[0].0 != p[1].0), "Duplicate persistent key");
        Ok(())
    }
    pub(crate) fn build(&mut self, entries: &mut [(Felt, Felt)]) -> Result<u64> {
        Self::prepare(entries, self.limit)?;
        self.build_sorted(entries)
    }
    fn first_key(&mut self, mut id: u64) -> Result<Felt> {
        for _ in 0..=252 {
            match self.get(id)? {
                Node::Leaf { key, .. } => return Ok(key),
                Node::Branch { left, .. } => id = left,
            }
        }
        anyhow::bail!("Persistent path too long")
    }
    /// Recurses once through the union of changed paths, rather than rebuilding once per key.
    /// Omitted leaves and explicit zeros are retained. Unchanged values allocate no nodes.
    pub(crate) fn update(&mut self, base: u64, entries: &mut [(Felt, Felt)]) -> Result<u64> {
        Self::prepare(entries, self.limit)?;
        self.merge(base, entries)
    }
    fn merge(&mut self, id: u64, patch: &[(Felt, Felt)]) -> Result<u64> {
        if patch.is_empty() {
            return Ok(id);
        }
        let old = self.get(id)?;
        let first = self.first_key(id)?;
        let bit = old.bit().max(differing_bit(first, patch[0].0)).max(differing_bit(first, patch[patch.len() - 1].0));
        if bit == -1 {
            let Node::Leaf { value, .. } = old else { anyhow::bail!("Invalid leaf merge") };
            if value == patch[0].1 {
                return Ok(id);
            }
            self.replaced.push(id);
            return self.push(Node::new_leaf(first, patch[0].1));
        }
        let split = patch.partition_point(|e| !key_bit(e.0, bit as u8));
        if bit == old.bit() {
            let Node::Branch { left, right, .. } = old else { anyhow::bail!("Invalid branch merge") };
            let l = self.merge(left, &patch[..split])?;
            let r = self.merge(right, &patch[split..])?;
            if l == left && r == right {
                return Ok(id);
            }
            self.replaced.push(id);
            return self.join(bit as u8, l, r);
        }
        let (l, r) = if key_bit(first, bit as u8) {
            (self.build_sorted(&patch[..split])?, self.merge(id, &patch[split..])?)
        } else {
            (self.merge(id, &patch[..split])?, self.build_sorted(&patch[split..])?)
        };
        self.join(bit as u8, l, r)
    }
    pub(crate) fn root(&mut self, id: u64) -> Result<(Felt, u32)> {
        let n = self.get(id)?;
        Ok((n.hash(), n.count()))
    }
    pub(crate) fn witness(&mut self, mut id: u64, root: Felt, key: Felt) -> Result<Option<CommittedDataWitness>> {
        ensure!(self.get(id)?.hash() == root, "Persistent root mismatch");
        let mut siblings = Vec::new();
        loop {
            match self.get(id)? {
                Node::Leaf { key: found, value, .. } => {
                    if found != key {
                        return Ok(None);
                    }
                    siblings.reverse();
                    let witness = CommittedDataWitness { root, key, value, siblings };
                    ensure!(witness.verify(), "Persistent witness mismatch");
                    return Ok(Some(witness));
                }
                Node::Branch { bit, left, right, .. } => {
                    ensure!(siblings.len() < 252, "Persistent proof too long");
                    let (child, sibling) = if key_bit(key, bit) { (right, left) } else { (left, right) };
                    siblings.push(CommittedDataSibling { bit, hash: self.raw(sibling)?.hash() });
                    id = child;
                }
            }
        }
    }
    /// Rank-based feeder pagination uses subtree counts to skip unrelated subtrees.
    pub(crate) fn page(&mut self, id: u64, start: u32, length: u32, out: &mut Vec<(Felt, Felt)>) -> Result<()> {
        let node = self.get(id)?;
        if length == 0 || start >= node.count() {
            return Ok(());
        }
        match node {
            Node::Leaf { key, value, .. } => out.push((key, value)),
            Node::Branch { left, right, .. } => {
                let n = self.raw(left)?.count();
                let before = out.len();
                if start < n {
                    self.page(left, start, length, out)?;
                }
                self.page(right, start.saturating_sub(n), length - u32::try_from(out.len() - before)?, out)?;
            }
        }
        Ok(())
    }
    pub(crate) fn records(&self) -> impl Iterator<Item = ([u8; 14], Vec<u8>)> + '_ {
        self.nodes.iter().enumerate().map(|(i, n)| (node_key(self.first_id + i as u64), n.encode()))
    }
    pub(crate) fn next_id(&self) -> u64 {
        self.first_id + self.nodes.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    use std::collections::{BTreeMap, HashMap};

    #[test]
    fn persistent_updates_match_full_rebuilds_and_keep_every_historical_proof() -> Result<()> {
        let mut disk = HashMap::<u64, Vec<u8>>::new();
        let mut next = 1;
        let mut reference = BTreeMap::new();
        let mut roots = Vec::new();
        let mut base = None;
        // Mixed insertions, zero upserts, repeated values and branches above/below existing roots.
        for tick in 0..40u64 {
            let mut patch = BTreeMap::new();
            for i in 0..17u64 {
                let key = Felt::from((i * 97 + tick * 43) % 503);
                patch.insert(key, Felt::from(if i % 5 == 0 { 0 } else { tick + i }));
            }
            if tick == 1 {
                patch.insert(Felt::MAX, Felt::ZERO);
            }
            reference.extend(patch.clone());
            let mut entries: Vec<_> = patch.into_iter().collect();
            let mut tree = Tree::new(next, 1024, |id| Ok(disk.get(&id).cloned()));
            let id = match base {
                Some(id) => tree.update(id, &mut entries)?,
                None => tree.build(&mut entries)?,
            };
            let expected = CommittedDataSet::new(reference.iter().map(|(k, v)| (*k, *v)).collect())?;
            let (root, count) = tree.root(id)?;
            assert_eq!(root, expected.root());
            assert_eq!(count as usize, reference.len());
            let written: Vec<_> = tree.nodes.iter().enumerate().map(|(i, n)| (next + i as u64, n.encode())).collect();
            next = tree.next_id();
            drop(tree);
            disk.extend(written);
            roots.push((id, root, reference.clone()));
            base = Some(id);
        }
        // Drop operation caches and read all versions from encoded records.
        for (id, root, map) in &roots {
            let mut tree = Tree::new(next, 1024, |id| Ok(disk.get(&id).cloned()));
            tree.check_root(*id, *root, map.len() as u32)?;
            for (&key, &value) in map {
                let proof = tree.witness(*id, *root, key)?.unwrap();
                assert_eq!(proof.value, value);
                assert!(proof.verify());
            }
            assert!(tree.witness(*id, *root, Felt::from(999))?.is_none());
            let expected: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
            for start in (0..map.len()).step_by(11) {
                let mut page = Vec::new();
                tree.page(*id, start as u32, 11, &mut page)?;
                assert_eq!(page, expected[start..(start + 11).min(map.len())]);
            }
        }
        let (id, _, map) = roots.last().unwrap();
        let mut tree = Tree::new(next, 1024, |id| Ok(disk.get(&id).cloned()));
        let mut same: Vec<_> = map.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(tree.update(*id, &mut same)?, *id);
        assert!(tree.nodes.is_empty());
        assert!(tree.update(*id, &mut [(Felt::ONE, Felt::ZERO); 2]).is_err());
        Ok(())
    }

    #[test]
    fn committed_cache_retains_current_frontier_not_obsolete_versions() -> Result<()> {
        let mut disk = HashMap::new();
        let mut values: Vec<_> = (0..128u32).map(|k| (Felt::from(k), Felt::from(k))).collect();
        let mut tree = Tree::new(1, 256, |_| Ok(None));
        let mut id = tree.build(&mut values)?;
        let mut next = tree.next_id();
        for (i, n) in tree.nodes.iter().enumerate() {
            disk.insert(i as u64 + 1, n.encode());
        }
        let mut cache = tree.into_committed_cache();
        let mut prior = Vec::new();
        for tick in 0..30u32 {
            let old_id = id;
            let reads = std::cell::Cell::new(0);
            let mut tree = Tree::with_cache(
                next,
                256,
                |id| {
                    reads.set(reads.get() + 1);
                    Ok(disk.get(&id).cloned())
                },
                cache,
            );
            let old_root = tree.root(old_id)?.0;
            prior.push((old_id, old_root));
            values[(tick % 128) as usize].1 = Felt::from(1000 + tick);
            id = tree.update(old_id, &mut [values[(tick % 128) as usize]])?;
            assert_eq!(tree.root(id)?.0, CommittedDataSet::new(values.clone())?.root());
            assert_eq!(reads.get(), 0, "Warm updates must reuse authenticated immutable nodes");
            let records: Vec<_> = tree.nodes.iter().enumerate().map(|(i, n)| (next + i as u64, n.encode())).collect();
            next = tree.next_id();
            cache = tree.into_committed_cache();
            disk.extend(records);
            assert_eq!(cache.0.len(), 255, "Obsolete nodes must not displace the current frontier");
        }
        // Eviction is only a performance change; persisted history remains authoritative.
        let mut tiny = NodeCache::default();
        tiny.0.resize(NonZeroUsize::new(2).unwrap());
        let mut tree = Tree::with_cache(next, 256, |id| Ok(disk.get(&id).cloned()), tiny);
        for (id, root) in prior {
            assert!(tree.witness(id, root, Felt::from(5))?.unwrap().verify());
        }
        Ok(())
    }

    #[test]
    fn corrupt_records_and_allocation_limits_fail_closed() -> Result<()> {
        let mut tree = Tree::new(1, 4, |_| Ok(None));
        let mut entries = vec![(Felt::ONE, Felt::TWO), (Felt::from(8), Felt::ZERO)];
        let id = tree.build(&mut entries)?;
        let (root, count) = tree.root(id)?;
        let mut disk: HashMap<_, _> = tree.nodes.iter().enumerate().map(|(i, n)| (i as u64 + 1, n.encode())).collect();
        let next = tree.next_id();
        drop(tree);
        // Tamper a value without changing the expected external root.
        disk.get_mut(&1).unwrap()[64] ^= 1;
        let mut tree = Tree::new(next, 4, |id| Ok(disk.get(&id).cloned()));
        assert!(tree.check_root(id, root, count).is_err());
        assert!(Node::decode(3, &[1; 54]).is_err());
        let mut tree = Tree::new(1, 1, |_| Ok(None));
        assert!(tree.build(&mut entries).is_err());
        let mut tree = Tree::new(u64::MAX, 4, |_| Ok(None));
        assert!(tree.build(&mut entries[..1]).is_err());
        Ok(())
    }
}
