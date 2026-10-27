//! Shared-node ownership. Root pointers and branch edges each hold one reference.
use super::*;
use crate::persistent_committed_data::{node_key, Node};
use rocksdb::WriteBatch;
use std::collections::HashMap;
pub(super) const REFERENCES: &[u8] = b"cdh/ref/";

const REFERENCE_BYTES: usize = 3 * size_of::<u64>();
/// Maximum reference records read in one pruning frontier.
const PRUNE_READ_BATCH_SIZE: usize = 1024;

// Refcounts are staged atomically with nodes and root metadata. Missing durable counts are corruption.
#[derive(Clone, Copy)]
struct ProofNodeReference {
    count: u64,
    children: Option<(u64, u64)>,
}
impl ProofNodeReference {
    /// Persist incoming count and child IDs, retaining edges even if node payloads need repair.
    fn encode(self) -> [u8; REFERENCE_BYTES] {
        let mut bytes = [0; REFERENCE_BYTES];
        bytes[..8].copy_from_slice(&self.count.to_be_bytes());
        if let Some((left, right)) = self.children {
            bytes[8..16].copy_from_slice(&left.to_be_bytes());
            bytes[16..].copy_from_slice(&right.to_be_bytes());
        }
        bytes
    }
    /// Reject zero counts and non-ancestral edges; child IDs always precede their parent.
    fn decode(id: u64, bytes: &[u8]) -> Result<Self> {
        anyhow::ensure!(bytes.len() == REFERENCE_BYTES, "Invalid proof reference");
        let count = u64::from_be_bytes(bytes[..8].try_into()?);
        let left = u64::from_be_bytes(bytes[8..16].try_into()?);
        let right = u64::from_be_bytes(bytes[16..].try_into()?);
        anyhow::ensure!(count > 0, "Empty persisted reference");
        let children = if left == 0 && right == 0 {
            None
        } else {
            anyhow::ensure!(
                left > 0 && right > 0 && left < id && right < id && left != right,
                "Invalid reference edges"
            );
            Some((left, right))
        };
        Ok(Self { count, children })
    }
}
/// Stages ownership changes for one atomic node/root write. No method commits independently.
/// A reference is an incoming branch edge or a retained root pointer; historical values own none.
pub(super) struct ProofNodeReferences<'a> {
    db: &'a RocksDBStorageInner,
    staged_references: HashMap<u64, ProofNodeReference>,
    first_new_node_id: u64,
    new_nodes: &'a [Node],
    deleted_node_ids: Vec<u64>,
}
impl<'a> ProofNodeReferences<'a> {
    pub(super) fn new(db: &'a RocksDBStorageInner, first_new_node_id: u64, nodes: &'a [Node]) -> Self {
        Self {
            db,
            staged_references: HashMap::new(),
            first_new_node_id,
            new_nodes: nodes,
            deleted_node_ids: Vec::new(),
        }
    }
    /// Prefer staged counts; only pre-existing IDs may load durable references.
    fn reference(&self, id: u64) -> Result<Option<ProofNodeReference>> {
        if let Some(value) = self.staged_references.get(&id) {
            return Ok(Some(*value));
        }
        // Allocator IDs in this batch cannot already have durable references.
        if id.checked_sub(self.first_new_node_id).is_some_and(|offset| offset < self.new_nodes.len() as u64) {
            return Ok(None);
        }
        self.db
            .db
            .get_cf(&self.db.get_column(meta::META_COLUMN), [REFERENCES, &id.to_be_bytes()].concat())?
            .map(|bytes| ProofNodeReference::decode(id, &bytes))
            .transpose()
    }
    /// Resolve a newly allocated node from this batch; missing durable ownership is corruption.
    fn node(&self, id: u64) -> Result<Node> {
        id.checked_sub(self.first_new_node_id)
            .and_then(|offset| self.new_nodes.get(offset as usize))
            .copied()
            .context("Missing durable proof reference")
    }
    /// Retain a root/edge, recursively registering children only for a newly allocated node.
    pub(super) fn add(&mut self, id: u64) -> Result<()> {
        let mut reference = match self.reference(id)? {
            Some(reference) => reference,
            None => {
                let children = match self.node(id)? {
                    Node::Branch { left, right, .. } => {
                        self.add(left)?;
                        self.add(right)?;
                        Some((left, right))
                    }
                    Node::Leaf { .. } => None,
                };
                ProofNodeReference { count: 0, children }
            }
        };
        reference.count = reference.count.checked_add(1).context("Proof reference overflow")?;
        self.staged_references.insert(id, reference);
        Ok(())
    }
    /// Release a root/edge and reclaim descendants only when their final owner disappears.
    pub(super) fn remove(&mut self, id: u64) -> Result<()> {
        let cf = self.db.get_column(meta::META_COLUMN);
        let mut pending = vec![id];
        while !pending.is_empty() {
            // Bound temporary read buffers while amortizing RocksDB lookups across a frontier.
            let ids = pending.split_off(pending.len().saturating_sub(PRUNE_READ_BATCH_SIZE));
            let missing: Vec<_> = ids.iter().copied().filter(|id| !self.staged_references.contains_key(id)).collect();
            let values =
                self.db.db.multi_get_cf(missing.iter().map(|id| (&cf, [REFERENCES, &id.to_be_bytes()].concat())));
            for (id, value) in missing.into_iter().zip(values) {
                let bytes = value?.context("Unregistered proof node")?;
                self.staged_references.insert(id, ProofNodeReference::decode(id, &bytes)?);
            }
            for id in ids {
                let reference = self.staged_references.get_mut(&id).context("Unregistered proof node")?;
                reference.count = reference.count.checked_sub(1).context("Proof reference underflow")?;
                if reference.count == 0 {
                    self.deleted_node_ids.push(id);
                    // Separate edges also allow a full import to repair a damaged node.
                    if let Some((left, right)) = reference.children {
                        pending.extend([left, right]);
                    }
                }
            }
        }
        Ok(())
    }
    /// Expose reclaimed IDs so the caller evicts stale decoded nodes before publication.
    pub(super) fn deleted_node_ids(&self) -> &[u64] {
        &self.deleted_node_ids
    }
    /// Append reachable new nodes, reference changes and deletions to the caller's atomic batch.
    pub(super) fn write(self, batch: &mut WriteBatch) {
        let cf = self.db.get_column(meta::META_COLUMN);
        for (offset, node) in self.new_nodes.iter().enumerate() {
            let id = self.first_new_node_id + offset as u64;
            if self.staged_references.get(&id).is_some_and(|reference| reference.count > 0) {
                batch.put_cf(&cf, node_key(id), node.encode());
            }
        }
        for (id, reference) in self.staged_references {
            let key = [REFERENCES, &id.to_be_bytes()].concat();
            if reference.count == 0 {
                batch.delete_cf(&cf, key);
            } else {
                batch.put_cf(&cf, key, reference.encode());
            }
        }
        for id in self.deleted_node_ids {
            batch.delete_cf(&cf, node_key(id));
        }
    }
}
