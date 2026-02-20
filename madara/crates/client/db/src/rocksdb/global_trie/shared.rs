use super::{bonsai_identifier, ClassTrieTimings, ContractTrieTimings};
use crate::prelude::*;
use crate::rocksdb::{
    trie::{BasicId, TrieError, WrappedBonsaiError},
    RocksDBStorage,
};
use bitvec::{order::Msb0, vec::BitVec, view::AsBits};
use bonsai_trie::{BonsaiDatabase, BonsaiPersistentDatabase, BonsaiStorage};
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StorageEntry,
};
use rayon::prelude::*;
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Default, Clone)]
pub(crate) struct ContractLeafState {
    pub class_hash: Option<Felt>,
    pub storage_root: Option<Felt>,
    pub nonce: Option<Felt>,
}

// "CONTRACT_CLASS_LEAF_V0"
pub(crate) const CONTRACT_CLASS_HASH_VERSION: Felt =
    Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f4c4541465f5630");

pub(crate) fn compute_class_leaf_hash(compiled_class_hash: &Felt) -> Felt {
    Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash)
}

pub(crate) fn felt_to_trie_bits(value: &Felt) -> BitVec<u8, Msb0> {
    value.to_bytes_be().as_bits()[5..].to_owned()
}

pub(crate) trait ContractStateReader {
    fn contract_nonce_at(&self, block_number: u64, contract_address: &Felt) -> Result<Option<Felt>>;

    fn contract_class_hash_at(&self, block_number: u64, contract_address: &Felt) -> Result<Option<Felt>>;
}

impl ContractStateReader for RocksDBStorage {
    fn contract_nonce_at(&self, block_number: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner.get_contract_nonce_at(block_number, contract_address)
    }

    fn contract_class_hash_at(&self, block_number: u64, contract_address: &Felt) -> Result<Option<Felt>> {
        self.inner.get_contract_class_hash_at(block_number, contract_address)
    }
}

pub(crate) fn class_trie_root_from_updates<DB>(
    class_trie: &mut BonsaiStorage<BasicId, DB, Poseidon>,
    updates: impl IntoIterator<Item = (Felt, Felt)>,
    block_n: u64,
) -> Result<(Felt, ClassTrieTimings)>
where
    DB: BonsaiDatabase<DatabaseError = TrieError> + BonsaiPersistentDatabase<BasicId>,
{
    for (key, value) in updates {
        let key_bits = felt_to_trie_bits(&key);
        class_trie.insert(bonsai_identifier::CLASS, &key_bits, &value).map_err(WrappedBonsaiError)?;
    }

    let mut timings = ClassTrieTimings::default();
    let commit_start = Instant::now();
    class_trie.commit(BasicId::new(block_n)).map_err(WrappedBonsaiError)?;
    timings.trie_commit = commit_start.elapsed();

    let root_hash = class_trie.root_hash(bonsai_identifier::CLASS).map_err(WrappedBonsaiError)?;

    Ok((root_hash, timings))
}

pub(crate) fn collect_class_updates(
    declared_classes: &[DeclaredClassItem],
    migrated_classes: &[MigratedClassItem],
) -> Vec<(Felt, Felt)> {
    let mut updates = Vec::with_capacity(declared_classes.len() + migrated_classes.len());

    updates.extend(declared_classes.iter().map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
        (*class_hash, compute_class_leaf_hash(compiled_class_hash))
    }));
    updates.extend(migrated_classes.iter().map(|MigratedClassItem { class_hash, compiled_class_hash }| {
        (*class_hash, compute_class_leaf_hash(compiled_class_hash))
    }));

    updates
}

fn resolve_contract_nonce(
    reader: &(impl ContractStateReader + ?Sized),
    block_number: u64,
    contract_address: &Felt,
    contract_leaf: &ContractLeafState,
) -> Result<Felt> {
    match contract_leaf.nonce {
        Some(nonce) => Ok(nonce),
        None => Ok(reader.contract_nonce_at(block_number, contract_address)?.unwrap_or(Felt::ZERO)),
    }
}

fn resolve_contract_class_hash(
    reader: &(impl ContractStateReader + ?Sized),
    block_number: u64,
    contract_address: &Felt,
    contract_leaf: &ContractLeafState,
) -> Result<Felt> {
    match contract_leaf.class_hash {
        Some(class_hash) => Ok(class_hash),
        None => Ok(reader.contract_class_hash_at(block_number, contract_address)?.unwrap_or(Felt::ZERO)),
    }
}

fn resolve_contract_storage_root(contract_leaf: &ContractLeafState) -> Result<Felt> {
    contract_leaf.storage_root.context("Storage root needs to be set before contract leaf hashing")
}

pub(crate) fn contract_state_leaf_hash(
    reader: &(impl ContractStateReader + ?Sized),
    contract_address: &Felt,
    contract_leaf: &ContractLeafState,
    block_number: u64,
) -> Result<Felt> {
    let nonce = resolve_contract_nonce(reader, block_number, contract_address, contract_leaf)?;
    let class_hash = resolve_contract_class_hash(reader, block_number, contract_address, contract_leaf)?;
    let storage_root = resolve_contract_storage_root(contract_leaf)?;

    Ok(Pedersen::hash(&Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce), &Felt::ZERO))
}

fn apply_contract_leaf_overrides(
    contract_leafs: &mut HashMap<Felt, ContractLeafState>,
    deployed_contracts: &[DeployedContractItem],
    replaced_classes: &[ReplacedClassItem],
    nonces: &[NonceUpdate],
) {
    for NonceUpdate { contract_address, nonce } in nonces {
        contract_leafs.entry(*contract_address).or_default().nonce = Some(*nonce);
    }
    for DeployedContractItem { address, class_hash } in deployed_contracts {
        contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
    }
    for ReplacedClassItem { contract_address, class_hash } in replaced_classes {
        contract_leafs.entry(*contract_address).or_default().class_hash = Some(*class_hash);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ContractTrieInputRefs<'a> {
    pub deployed_contracts: &'a [DeployedContractItem],
    pub replaced_classes: &'a [ReplacedClassItem],
    pub nonces: &'a [NonceUpdate],
    pub storage_diffs: &'a [ContractStorageDiffItem],
    pub block_n: u64,
}

pub(crate) fn contract_trie_root_from_parts<StorageDB, ContractDB>(
    reader: &(impl ContractStateReader + Sync),
    contract_storage_trie: &mut BonsaiStorage<BasicId, StorageDB, Pedersen>,
    contract_trie: &mut BonsaiStorage<BasicId, ContractDB, Pedersen>,
    inputs: ContractTrieInputRefs<'_>,
) -> Result<(Felt, ContractTrieTimings)>
where
    StorageDB: BonsaiDatabase<DatabaseError = TrieError> + BonsaiPersistentDatabase<BasicId> + Sync,
    ContractDB: BonsaiDatabase<DatabaseError = TrieError> + BonsaiPersistentDatabase<BasicId>,
{
    let mut timings = ContractTrieTimings::default();
    let mut contract_leafs: HashMap<Felt, ContractLeafState> = HashMap::new();

    for ContractStorageDiffItem { address, storage_entries } in inputs.storage_diffs {
        for StorageEntry { key, value } in storage_entries {
            let key_bits = felt_to_trie_bits(key);
            contract_storage_trie.insert(&address.to_bytes_be(), &key_bits, value).map_err(WrappedBonsaiError)?;
        }
        contract_leafs.entry(*address).or_default();
    }

    let storage_commit_start = Instant::now();
    contract_storage_trie.commit(BasicId::new(inputs.block_n)).map_err(WrappedBonsaiError)?;
    timings.storage_commit = storage_commit_start.elapsed();

    apply_contract_leaf_overrides(
        &mut contract_leafs,
        inputs.deployed_contracts,
        inputs.replaced_classes,
        inputs.nonces,
    );

    let leaf_updates: Vec<_> = contract_leafs
        .into_par_iter()
        .map(|(contract_address, mut leaf)| {
            let storage_root =
                contract_storage_trie.root_hash(&contract_address.to_bytes_be()).map_err(WrappedBonsaiError)?;
            leaf.storage_root = Some(storage_root);
            let leaf_hash = contract_state_leaf_hash(reader, &contract_address, &leaf, inputs.block_n)?;
            anyhow::Ok((contract_address, leaf_hash))
        })
        .collect::<Result<_>>()?;

    for (contract_address, leaf_hash) in leaf_updates {
        let key_bits = felt_to_trie_bits(&contract_address);
        contract_trie.insert(bonsai_identifier::CONTRACT, &key_bits, &leaf_hash).map_err(WrappedBonsaiError)?;
    }

    let trie_commit_start = Instant::now();
    contract_trie.commit(BasicId::new(inputs.block_n)).map_err(WrappedBonsaiError)?;
    timings.trie_commit = trie_commit_start.elapsed();
    let root_hash = contract_trie.root_hash(bonsai_identifier::CONTRACT).map_err(WrappedBonsaiError)?;

    Ok((root_hash, timings))
}
