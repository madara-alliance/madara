//! Immutable Patricia nodes shared by root versions. No deletion or retention policy lives here.
//!
//! IDs are append-only arena offsets, not protocol identifiers. Hashing and witnesses retain the
//! existing committed-data format. A writer publishes nodes, allocator and root pointer atomically.
#[cfg(test)]
mod tests;

use anyhow::{ensure, Context, Result};
use blockifier::execution::syscalls::committed_data::{
    branch, key_bit, leaf, CommittedDataSibling, CommittedDataWitness,
};
use lru::LruCache;
use starknet_types_core::felt::Felt;
use std::num::NonZeroUsize;

/// Bounded decoded-node cache. Block age controls eligibility; a byte estimate caps occupancy.
/// The estimate includes hash-table/list bookkeeping, but is not a process RSS guarantee.
pub(crate) struct NodeCache {
    entries: LruCache<u64, (Node, bool, u64)>,
    observed_block: u64,
    memory_blocks: u64,
    enabled: bool,
}
impl Default for NodeCache {
    fn default() -> Self {
        let mut cache =
            Self { entries: LruCache::unbounded(), observed_block: 0, memory_blocks: u64::MAX, enabled: true };
        cache.configure(&Default::default(), 0);
        cache
    }
}
impl NodeCache {
    pub(crate) const ENTRY_BYTES: usize = std::mem::size_of::<Node>() + 128;
    pub(crate) fn configure(
        &mut self,
        config: &crate::committed_data_retention::CommittedDataRetentionConfig,
        block: u64,
    ) {
        self.observed_block = self.observed_block.max(block);
        self.memory_blocks = config.memory_blocks;
        self.enabled = config.cache_bytes / 2 >= Self::ENTRY_BYTES && config.memory_blocks > 0;
        self.entries
            .resize(NonZeroUsize::new((config.cache_bytes / 2 / Self::ENTRY_BYTES).max(1)).expect("nonzero capacity"));
        if !self.enabled {
            self.entries.clear();
        }
        // Stamps use a monotonic observed head, so LRU order is also block-age order.
        while self
            .entries
            .peek_lru()
            .is_some_and(|(_, (_, _, used))| self.observed_block.saturating_sub(*used) >= self.memory_blocks)
        {
            self.entries.pop_lru();
        }
    }
    fn disabled() -> Self {
        Self { entries: LruCache::unbounded(), observed_block: 0, memory_blocks: 0, enabled: false }
    }
    pub(crate) fn remove(&mut self, id: u64) {
        self.entries.pop(&id);
    }
    fn put(&mut self, id: u64, node: Node, checked: bool) {
        if self.enabled {
            self.entries.put(id, (node, checked, self.observed_block));
        }
    }
}

pub(crate) const NODE_PREFIX: &[u8] = b"cdm/n/";
pub(crate) const NODE_KEY_BYTES: usize = NODE_PREFIX.len() + size_of::<u64>();
pub(crate) const NEXT_KEY: &[u8] = b"cdm/next";
pub(crate) fn node_key(id: u64) -> [u8; NODE_KEY_BYTES] {
    let mut key = [0; NODE_KEY_BYTES];
    key[..NODE_PREFIX.len()].copy_from_slice(NODE_PREFIX);
    key[NODE_PREFIX.len()..].copy_from_slice(&id.to_be_bytes());
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
    pub(crate) fn decode(id: u64, bytes: &[u8]) -> Result<Self> {
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
    /// Changed existing leaves, in key order; consumed by the permanent value archive.
    pub(crate) changes: Vec<(Felt, Felt, Felt)>,
    limit: u32,
}
impl<F: FnMut(u64) -> Result<Option<Vec<u8>>>> Tree<F> {
    pub(crate) fn new(first_id: u64, limit: u32, read: F) -> Self {
        Self {
            read,
            first_id,
            nodes: Vec::new(),
            cache: NodeCache::disabled(),
            replaced: Vec::new(),
            changes: Vec::new(),
            limit,
        }
    }
    pub(crate) fn with_cache(first_id: u64, limit: u32, read: F, cache: NodeCache) -> Self {
        Self { read, first_id, nodes: Vec::new(), cache, replaced: Vec::new(), changes: Vec::new(), limit }
    }
    pub(crate) fn into_read_cache(self) -> NodeCache {
        self.cache
    }
    /// Call only after the nodes and root pointer have been successfully committed.
    pub(crate) fn into_committed_cache(mut self) -> NodeCache {
        // Prior versions remain on disk. Prefer the current tree in RAM so obsolete paths do
        // not displace its still-live nodes merely because each tick creates a new version.
        for id in self.replaced {
            self.cache.entries.pop(&id);
        }
        for (i, node) in self.nodes.into_iter().enumerate() {
            self.cache.put(self.first_id + i as u64, node, true);
        }
        self.cache
    }
    fn raw(&mut self, id: u64) -> Result<Node> {
        ensure!(id != 0, "Invalid persistent node ID");
        if id >= self.first_id {
            return self.nodes.get(usize::try_from(id - self.first_id)?).copied().context("Missing staged node");
        }
        if let Some((node, _, used)) = self.cache.entries.get_mut(&id) {
            *used = self.cache.observed_block;
            return Ok(*node);
        }
        let bytes = (self.read)(id)?.context("Persistent node unavailable")?;
        let node = Node::decode(id, &bytes)?;
        ensure!(node.count() <= self.limit, "Persistent subtree exceeds bound");
        self.cache.put(id, node, false);
        Ok(node)
    }
    fn get(&mut self, id: u64) -> Result<Node> {
        let node = self.raw(id)?;
        if id >= self.first_id || self.cache.entries.get(&id).is_some_and(|(_, checked, _)| *checked) {
            return Ok(node);
        }
        if let Node::Branch { bit, left, right, hash, count } = node {
            let l = self.raw(left)?;
            let r = self.raw(right)?;
            ensure!(l.bit() < bit.into() && r.bit() < bit.into(), "Invalid persistent path order");
            ensure!(l.count().checked_add(r.count()) == Some(count), "Invalid persistent subtree count");
            ensure!(branch(bit, l.hash(), r.hash()) == hash, "Persistent branch hash mismatch");
        }
        if let Some(entry) = self.cache.entries.get_mut(&id) {
            entry.1 = true;
        }
        Ok(node)
    }
    /// Authenticate the requested root identity and count before traversal or mutation.
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
            self.changes.push((first, value, patch[0].1));
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
    pub(crate) fn next_id(&self) -> u64 {
        self.first_id + self.nodes.len() as u64
    }
}
