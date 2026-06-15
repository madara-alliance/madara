use crate::{
    prelude::*,
    rocksdb::{
        deserialize,
        global_trie::bonsai_identifier,
        serialize_to_smallvec,
        trie::{BONSAI_CLASS_TRIE_COLUMN, BONSAI_CONTRACT_STORAGE_TRIE_COLUMN, BONSAI_CONTRACT_TRIE_COLUMN},
        Column, RocksDBStorage, RocksDBStorageInner, WriteBatchWithTransaction,
    },
};
use bitvec::{order::Msb0, slice::BitSlice, vec::BitVec};
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
            self.collect_archive_proof_nodes(trie, root, &key, &mut proof)?;
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
        trie: ArchiveTrie,
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

            let Some(node) = self.archive_trie_node(trie, current_hash, &key[..path_len])? else {
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

    fn archive_trie_node(
        &self,
        trie: ArchiveTrie,
        hash: Felt,
        source_path: &BitSlice<u8, Msb0>,
    ) -> Result<Option<ProofNode>> {
        let col = trie.node_column();
        let col_handle = self.inner.get_column(col);
        let Some(encoded) = self.inner.db.get_cf(&col_handle, hash.to_bytes_be())? else {
            return self.lazy_archive_trie_node(trie, hash, source_path);
        };
        Ok(Some(self.decode_archive_trie_node(hash, &encoded)?))
    }

    fn lazy_archive_trie_node(
        &self,
        trie: ArchiveTrie,
        hash: Felt,
        source_path: &BitSlice<u8, Msb0>,
    ) -> Result<Option<ProofNode>> {
        let archive_col = trie.node_column();
        let Some(source_col) = source_column_for_archive_node_column(archive_col) else { return Ok(None) };
        let source_col = self.inner.get_column(source_col);
        let source_key = source_key_for_archive_trie(trie, source_path);
        let Some(encoded) = self.inner.db.get_cf(&source_col, &source_key)? else {
            tracing::debug!("archive trie lazy lookup missed source key hash={hash:#x} path_len={}", source_path.len());
            return Ok(None);
        };

        let Some(decoded_hash) = bonsai_trie::persisted_trie_node_hash(&encoded)? else {
            tracing::debug!("archive trie lazy lookup found non-finalized source node hash={hash:#x}");
            return Ok(None);
        };
        if decoded_hash != hash {
            tracing::debug!(
                "archive trie lazy lookup source hash mismatch expected={hash:#x} actual={decoded_hash:#x} path_len={}",
                source_path.len()
            );
            return Ok(None);
        }

        let archive_col = self.inner.get_column(archive_col);
        let mut batch = WriteBatchWithTransaction::default();
        batch.put_cf(&archive_col, hash.to_bytes_be(), &encoded);
        self.inner.db.write_opt(batch, &self.inner.writeopts)?;
        tracing::debug!(
            "lazily copied existing bonsai trie node into archive column hash={hash:#x} path_len={}",
            source_path.len()
        );

        Ok(Some(self.decode_archive_trie_node(hash, &encoded)?))
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

fn source_key_prefix_for_archive_trie(trie: ArchiveTrie) -> Vec<u8> {
    match trie {
        ArchiveTrie::Class => bonsai_identifier::CLASS.to_vec(),
        ArchiveTrie::Contract => bonsai_identifier::CONTRACT.to_vec(),
        ArchiveTrie::ContractStorage(contract_address) => contract_address.to_bytes_be().to_vec(),
    }
}

fn source_key_for_archive_trie(trie: ArchiveTrie, source_path: &BitSlice<u8, Msb0>) -> Vec<u8> {
    let mut source_key = source_key_prefix_for_archive_trie(trie);
    source_key.extend_from_slice(&encode_bonsai_path(source_path));
    source_key
}

fn encode_bonsai_path(source_path: &BitSlice<u8, Msb0>) -> Vec<u8> {
    debug_assert!(source_path.len() <= 251);

    let mut encoded = Vec::with_capacity(1 + source_path.len().div_ceil(8));
    encoded.push(source_path.len() as u8);

    let mut next_store = 0u8;
    let mut pos_in_next_store = 7u8;
    for bit in source_path {
        next_store |= u8::from(*bit) << pos_in_next_store;

        if pos_in_next_store == 0 {
            pos_in_next_store = 8;
            encoded.push(next_store);
            next_store = 0;
        }
        pos_in_next_store -= 1;
    }

    if pos_in_next_store < 7 {
        encoded.push(next_store);
    }

    encoded
}
