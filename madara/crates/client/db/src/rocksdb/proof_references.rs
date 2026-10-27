//! Shared-node ownership. Root pointers and branch edges each hold one reference.
use super::*;
use crate::persistent_committed_data::{node_key, Node};
use rocksdb::WriteBatch;
use std::collections::HashMap;
const REFERENCES: &[u8] = b"cdh/ref/";

// Refcounts are staged atomically with nodes and root metadata. Missing durable counts are corruption.
#[derive(Clone, Copy)]
struct Reference {
    count: u64,
    children: Option<(u64, u64)>,
}
impl Reference {
    fn encode(self) -> [u8; 24] {
        let mut bytes = [0; 24];
        bytes[..8].copy_from_slice(&self.count.to_be_bytes());
        if let Some((left, right)) = self.children {
            bytes[8..16].copy_from_slice(&left.to_be_bytes());
            bytes[16..].copy_from_slice(&right.to_be_bytes());
        }
        bytes
    }
    fn decode(id: u64, bytes: &[u8]) -> Result<Self> {
        anyhow::ensure!(bytes.len() == 24, "Invalid proof reference");
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
pub(super) struct References<'a> {
    db: &'a RocksDBStorageInner,
    changes: HashMap<u64, Reference>,
    first: u64,
    new_nodes: &'a [Node],
    deleted: Vec<u64>,
}
impl<'a> References<'a> {
    pub(super) fn new(db: &'a RocksDBStorageInner, first: u64, nodes: &'a [Node]) -> Self {
        Self { db, changes: HashMap::new(), first, new_nodes: nodes, deleted: Vec::new() }
    }
    fn reference(&self, id: u64) -> Result<Option<Reference>> {
        if let Some(value) = self.changes.get(&id) {
            return Ok(Some(*value));
        }
        // Allocator IDs in this batch cannot already have durable references.
        if id.checked_sub(self.first).is_some_and(|offset| offset < self.new_nodes.len() as u64) {
            return Ok(None);
        }
        self.db
            .db
            .get_cf(&self.db.get_column(meta::META_COLUMN), [REFERENCES, &id.to_be_bytes()].concat())?
            .map(|bytes| Reference::decode(id, &bytes))
            .transpose()
    }
    fn node(&self, id: u64) -> Result<Node> {
        id.checked_sub(self.first)
            .and_then(|offset| self.new_nodes.get(offset as usize))
            .copied()
            .context("Missing durable proof reference")
    }
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
                Reference { count: 0, children }
            }
        };
        reference.count = reference.count.checked_add(1).context("Proof reference overflow")?;
        self.changes.insert(id, reference);
        Ok(())
    }
    pub(super) fn remove(&mut self, id: u64) -> Result<()> {
        let cf = self.db.get_column(meta::META_COLUMN);
        let mut pending = vec![id];
        while !pending.is_empty() {
            // Bound temporary read buffers while amortizing RocksDB lookups across a frontier.
            let ids = pending.split_off(pending.len().saturating_sub(1024));
            let missing: Vec<_> = ids.iter().copied().filter(|id| !self.changes.contains_key(id)).collect();
            let values =
                self.db.db.multi_get_cf(missing.iter().map(|id| (&cf, [REFERENCES, &id.to_be_bytes()].concat())));
            for (id, value) in missing.into_iter().zip(values) {
                let bytes = value?.context("Unregistered proof node")?;
                self.changes.insert(id, Reference::decode(id, &bytes)?);
            }
            for id in ids {
                let reference = self.changes.get_mut(&id).context("Unregistered proof node")?;
                reference.count = reference.count.checked_sub(1).context("Proof reference underflow")?;
                if reference.count == 0 {
                    self.deleted.push(id);
                    // Separate edges also allow a full import to repair a damaged node.
                    if let Some((left, right)) = reference.children {
                        pending.extend([left, right]);
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn deleted(&self) -> &[u64] {
        &self.deleted
    }
    pub(super) fn write(self, batch: &mut WriteBatch) {
        let cf = self.db.get_column(meta::META_COLUMN);
        for (offset, node) in self.new_nodes.iter().enumerate() {
            let id = self.first + offset as u64;
            if self.changes.get(&id).is_some_and(|reference| reference.count > 0) {
                batch.put_cf(&cf, node_key(id), node.encode());
            }
        }
        for (id, reference) in self.changes {
            let key = [REFERENCES, &id.to_be_bytes()].concat();
            if reference.count == 0 {
                batch.delete_cf(&cf, key);
            } else {
                batch.put_cf(&cf, key, reference.encode());
            }
        }
        for id in self.deleted {
            batch.delete_cf(&cf, node_key(id));
        }
    }
}
