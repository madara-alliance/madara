use crate::{
    prelude::*,
    rocksdb::{
        deserialize, serialize_to_smallvec,
        trie::{BONSAI_CLASS_TRIE_COLUMN, BONSAI_CONTRACT_STORAGE_TRIE_COLUMN, BONSAI_CONTRACT_TRIE_COLUMN},
        Column, RocksDBStorage, RocksDBStorageInner, WriteBatchWithTransaction,
    },
};
use bitvec::{order::Msb0, vec::BitVec};
use bonsai_trie::ProofNode;
use mp_convert::Felt;
use rocksdb::{Direction, IteratorMode};
use std::collections::HashMap;

pub(crate) const ARCHIVE_CLASS_TRIE_NODE_COLUMN: Column =
    Column::new("archive_class_trie_node").set_point_lookup().use_contracts_mem_budget();
pub(crate) const ARCHIVE_CONTRACT_TRIE_NODE_COLUMN: Column =
    Column::new("archive_contract_trie_node").set_point_lookup().use_contracts_mem_budget();
pub(crate) const ARCHIVE_CONTRACT_STORAGE_TRIE_NODE_COLUMN: Column =
    Column::new("archive_contract_storage_trie_node").set_point_lookup().use_contracts_mem_budget();

pub(crate) const ARCHIVE_CLASS_ROOT_COLUMN: Column = Column::new("archive_class_root").set_point_lookup();
pub(crate) const ARCHIVE_CONTRACT_ROOT_COLUMN: Column = Column::new("archive_contract_root").set_point_lookup();
pub(crate) const ARCHIVE_CONTRACT_STORAGE_ROOT_COLUMN: Column =
    Column::new("archive_contract_storage_root").with_prefix_extractor_len(32);
pub(crate) const ARCHIVE_META_COLUMN: Column = Column::new("archive_meta").set_point_lookup();

fn block_key(block_n: u64) -> [u8; 8] {
    block_n.to_be_bytes()
}

fn contract_block_key(contract_address: &Felt, block_n: u64) -> [u8; 40] {
    let mut key = [0u8; 40];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    key[32..].copy_from_slice(&block_n.to_be_bytes());
    key
}

impl RocksDBStorageInner {
    pub(crate) fn archive_put_trie_node(
        &self,
        batch: &mut WriteBatchWithTransaction,
        col: Column,
        node_hash: &Felt,
        encoded_node: &[u8],
    ) {
        let col = self.get_column(col);
        batch.put_cf(&col, node_hash.to_bytes_be(), encoded_node);
    }

    pub(crate) fn archive_put_class_root(
        &self,
        batch: &mut WriteBatchWithTransaction,
        block_n: u64,
        root: &Felt,
    ) -> Result<()> {
        let col = self.get_column(ARCHIVE_CLASS_ROOT_COLUMN);
        batch.put_cf(&col, block_key(block_n), serialize_to_smallvec::<[u8; 64]>(root)?);
        Ok(())
    }

    pub(crate) fn archive_put_contract_root(
        &self,
        batch: &mut WriteBatchWithTransaction,
        block_n: u64,
        root: &Felt,
    ) -> Result<()> {
        let col = self.get_column(ARCHIVE_CONTRACT_ROOT_COLUMN);
        batch.put_cf(&col, block_key(block_n), serialize_to_smallvec::<[u8; 64]>(root)?);
        Ok(())
    }

    pub(crate) fn archive_put_contract_storage_root(
        &self,
        batch: &mut WriteBatchWithTransaction,
        block_n: u64,
        contract_address: &Felt,
        root: &Felt,
    ) -> Result<()> {
        let col = self.get_column(ARCHIVE_CONTRACT_STORAGE_ROOT_COLUMN);
        batch.put_cf(&col, contract_block_key(contract_address, block_n), serialize_to_smallvec::<[u8; 64]>(root)?);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ArchiveTrie {
    Class,
    Contract,
    ContractStorage(Felt),
}

impl ArchiveTrie {
    fn node_column(self) -> Column {
        match self {
            Self::Class => ARCHIVE_CLASS_TRIE_NODE_COLUMN,
            Self::Contract => ARCHIVE_CONTRACT_TRIE_NODE_COLUMN,
            Self::ContractStorage(_) => ARCHIVE_CONTRACT_STORAGE_TRIE_NODE_COLUMN,
        }
    }

    fn root_column(self) -> Column {
        match self {
            Self::Class => ARCHIVE_CLASS_ROOT_COLUMN,
            Self::Contract => ARCHIVE_CONTRACT_ROOT_COLUMN,
            Self::ContractStorage(_) => ARCHIVE_CONTRACT_STORAGE_ROOT_COLUMN,
        }
    }
}

impl RocksDBStorage {
    pub fn archive_trie_proof(
        &self,
        trie: ArchiveTrie,
        block_n: u64,
        keys: impl IntoIterator<Item = BitVec<u8, Msb0>>,
    ) -> Result<Option<(Felt, Vec<(Felt, ProofNode)>)>> {
        let Some(root) = self.archive_root_at(trie, block_n)? else { return Ok(None) };
        if root == Felt::ZERO {
            return Ok(Some((root, Vec::new())));
        }

        let mut proof = HashMap::new();
        for key in keys {
            self.collect_archive_proof_nodes(trie.node_column(), root, &key, &mut proof)?;
        }

        Ok(Some((root, proof.into_iter().collect())))
    }

    fn archive_root_at(&self, trie: ArchiveTrie, block_n: u64) -> Result<Option<Felt>> {
        match trie {
            ArchiveTrie::Class | ArchiveTrie::Contract => self.archive_root_at_by_block(trie.root_column(), block_n),
            ArchiveTrie::ContractStorage(contract_address) => {
                self.archive_contract_storage_root_at(contract_address, block_n)
            }
        }
    }

    fn archive_root_at_by_block(&self, col: Column, block_n: u64) -> Result<Option<Felt>> {
        let col = self.inner.get_column(col);
        let mut iter = self.inner.db.iterator_cf(&col, IteratorMode::From(&block_key(block_n), Direction::Reverse));

        let Some(item) = iter.next() else { return Ok(None) };
        let (_key, value) = item?;
        Ok(Some(deserialize(value)?))
    }

    fn archive_contract_storage_root_at(&self, contract_address: Felt, block_n: u64) -> Result<Option<Felt>> {
        let col = self.inner.get_column(ARCHIVE_CONTRACT_STORAGE_ROOT_COLUMN);
        let contract_prefix = contract_address.to_bytes_be();
        let seek_key = contract_block_key(&contract_address, block_n);
        let mut iter = self.inner.db.iterator_cf(&col, IteratorMode::From(&seek_key, Direction::Reverse));

        let Some(item) = iter.next() else { return Ok(None) };
        let (key, value) = item?;
        if !key.starts_with(&contract_prefix) {
            return Ok(None);
        }
        Ok(Some(deserialize(value)?))
    }

    fn collect_archive_proof_nodes(
        &self,
        node_col: Column,
        root: Felt,
        key: &BitVec<u8, Msb0>,
        proof: &mut HashMap<Felt, ProofNode>,
    ) -> Result<()> {
        let mut path_len = 0usize;
        let mut current_hash = root;

        loop {
            if current_hash == Felt::ZERO || path_len >= key.len() {
                return Ok(());
            }

            let Some(node) = self.archive_trie_node(node_col, current_hash)? else {
                anyhow::bail!("archive trie node missing for hash {current_hash:#x}");
            };

            match node.clone() {
                ProofNode::Binary { left, right } => {
                    let next = if key[path_len] { right } else { left };
                    path_len += 1;
                    proof.insert(current_hash, node);
                    current_hash = next;
                }
                ProofNode::Edge { child, path } => {
                    let edge_len = path.len();
                    let end = path_len.saturating_add(edge_len);
                    proof.insert(current_hash, node);
                    if end > key.len() || key.get(path_len..end) != Some(path.0.as_bitslice()) {
                        return Ok(());
                    }
                    path_len = end;
                    current_hash = child;
                }
            }
        }
    }

    fn archive_trie_node(&self, col: Column, hash: Felt) -> Result<Option<ProofNode>> {
        let col_handle = self.inner.get_column(col);
        let Some(encoded) = self.inner.db.get_cf(&col_handle, hash.to_bytes_be())? else {
            return self.lazy_archive_trie_node(col, hash);
        };
        Ok(Some(self.decode_archive_trie_node(hash, &encoded)?))
    }

    fn lazy_archive_trie_node(&self, archive_col: Column, hash: Felt) -> Result<Option<ProofNode>> {
        let Some(source_col) = source_column_for_archive_node_column(archive_col) else { return Ok(None) };
        let source_col = self.inner.get_column(source_col);

        for item in self.inner.db.iterator_cf(&source_col, IteratorMode::Start) {
            let (_key, encoded) = item?;
            let Ok(Some(decoded_hash)) = bonsai_trie::persisted_trie_node_hash(&encoded) else { continue };
            if decoded_hash != hash {
                continue;
            }

            let archive_col = self.inner.get_column(archive_col);
            let mut batch = WriteBatchWithTransaction::default();
            batch.put_cf(&archive_col, hash.to_bytes_be(), &encoded);
            self.inner.db.write_opt(batch, &self.inner.writeopts)?;
            tracing::debug!("lazily copied existing bonsai trie node into archive column hash={hash:#x}");
            return Ok(Some(self.decode_archive_trie_node(hash, &encoded)?));
        }

        Ok(None)
    }

    fn decode_archive_trie_node(&self, hash: Felt, encoded: &[u8]) -> Result<ProofNode> {
        let Some((decoded_hash, proof_node)) = bonsai_trie::persisted_trie_node_to_proof_node(&encoded)? else {
            anyhow::bail!("archive trie node {hash:#x} did not contain a finalized proof node");
        };
        anyhow::ensure!(
            decoded_hash == hash,
            "archive trie node hash mismatch: key={hash:#x}, decoded={decoded_hash:#x}"
        );
        Ok(proof_node)
    }
}

fn source_column_for_archive_node_column(archive_col: Column) -> Option<Column> {
    match archive_col.rocksdb_name {
        name if name == ARCHIVE_CLASS_TRIE_NODE_COLUMN.rocksdb_name => Some(BONSAI_CLASS_TRIE_COLUMN),
        name if name == ARCHIVE_CONTRACT_TRIE_NODE_COLUMN.rocksdb_name => Some(BONSAI_CONTRACT_TRIE_COLUMN),
        name if name == ARCHIVE_CONTRACT_STORAGE_TRIE_NODE_COLUMN.rocksdb_name => {
            Some(BONSAI_CONTRACT_STORAGE_TRIE_COLUMN)
        }
        _ => None,
    }
}
