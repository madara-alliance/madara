use super::db::InMemoryBonsaiDb;
use super::overlay::BonsaiOverlay;
use super::state_diff::cumulative_squashed_state_diffs;
use crate::prelude::*;
use crate::rocksdb::global_trie::{ClassTrieTimings, ContractTrieTimings};
use crate::rocksdb::snapshots::SnapshotRef;
use crate::rocksdb::trie::BasicId;
use crate::rocksdb::RocksDBStorage;
use bitvec::{order::Msb0, vec::BitVec, view::AsBits};
use bonsai_trie::BonsaiStorageConfig;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use rayon::prelude::*;
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon, StarkHash},
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

type InMemoryTrie<H> = bonsai_trie::BonsaiStorage<BasicId, InMemoryBonsaiDb, H>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TrieLogMode {
    #[default]
    Off,
    Checkpoint,
}

#[derive(Debug, Clone)]
pub struct InMemoryRootComputation {
    pub block_n: u64,
    pub state_root: Felt,
    pub timings: crate::rocksdb::global_trie::MerklizationTimings,
    pub overlay: Option<BonsaiOverlay>,
}

#[derive(Debug, Default)]
struct ContractLeaf {
    class_hash: Option<Felt>,
    storage_root: Option<Felt>,
    nonce: Option<Felt>,
}

fn bonsai_storage_config_for_mode(backend: &RocksDBStorage, trie_log_mode: TrieLogMode) -> BonsaiStorageConfig {
    BonsaiStorageConfig {
        max_saved_trie_logs: match trie_log_mode {
            TrieLogMode::Off => Some(0),
            TrieLogMode::Checkpoint => backend.inner.config.max_saved_trie_logs,
        },
        max_saved_snapshots: Some(0),
        snapshot_interval: backend.inner.config.snapshot_interval,
    }
}

fn contract_state_leaf_hash(
    backend: &RocksDBStorage,
    contract_address: &Felt,
    contract_leaf: &ContractLeaf,
    block_number: u64,
) -> Result<Felt> {
    let nonce = contract_leaf
        .nonce
        .unwrap_or(backend.inner.get_contract_nonce_at(block_number, contract_address)?.unwrap_or(Felt::ZERO));

    let class_hash = if let Some(class_hash) = contract_leaf.class_hash {
        class_hash
    } else {
        backend.inner.get_contract_class_hash_at(block_number, contract_address)?.unwrap_or(Felt::ZERO)
    };
    let storage_root =
        contract_leaf.storage_root.context("Storage root needs to be set before contract leaf hashing")?;

    Ok(Pedersen::hash(&Pedersen::hash(&Pedersen::hash(&class_hash, &storage_root), &nonce), &Felt::ZERO))
}

fn in_memory_contract_trie_root(
    backend: &RocksDBStorage,
    contract_storage_trie: &mut InMemoryTrie<Pedersen>,
    contract_trie: &mut InMemoryTrie<Pedersen>,
    state_diff: &StateDiff,
    block_n: u64,
) -> Result<(Felt, ContractTrieTimings)> {
    let mut timings = ContractTrieTimings::default();
    let mut contract_leafs: HashMap<Felt, ContractLeaf> = HashMap::new();

    for ContractStorageDiffItem { address, storage_entries } in &state_diff.storage_diffs {
        for StorageEntry { key, value } in storage_entries {
            let bytes = key.to_bytes_be();
            let bitvec: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            contract_storage_trie
                .insert(&address.to_bytes_be(), &bitvec, value)
                .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
        }
        contract_leafs.insert(*address, ContractLeaf::default());
    }

    let storage_commit_start = Instant::now();
    contract_storage_trie.commit(BasicId::new(block_n)).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    timings.storage_commit = storage_commit_start.elapsed();

    for NonceUpdate { contract_address, nonce } in &state_diff.nonces {
        contract_leafs.entry(*contract_address).or_default().nonce = Some(*nonce);
    }
    for DeployedContractItem { address, class_hash } in &state_diff.deployed_contracts {
        contract_leafs.entry(*address).or_default().class_hash = Some(*class_hash);
    }
    for ReplacedClassItem { contract_address, class_hash } in &state_diff.replaced_classes {
        contract_leafs.entry(*contract_address).or_default().class_hash = Some(*class_hash);
    }

    let leaf_hashes: Vec<_> = contract_leafs
        .into_par_iter()
        .map(|(contract_address, mut leaf)| {
            let storage_root = contract_storage_trie
                .root_hash(&contract_address.to_bytes_be())
                .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
            leaf.storage_root = Some(storage_root);
            let leaf_hash = contract_state_leaf_hash(backend, &contract_address, &leaf, block_n)?;
            let bytes = contract_address.to_bytes_be();
            let bitvec: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
            anyhow::Ok((bitvec, leaf_hash))
        })
        .collect::<Result<_>>()?;

    for (key, value) in leaf_hashes {
        contract_trie
            .insert(super::super::bonsai_identifier::CONTRACT, &key, &value)
            .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    }

    let trie_commit_start = Instant::now();
    contract_trie.commit(BasicId::new(block_n)).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    timings.trie_commit = trie_commit_start.elapsed();

    let root = contract_trie
        .root_hash(super::super::bonsai_identifier::CONTRACT)
        .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    Ok((root, timings))
}

const CONTRACT_CLASS_HASH_VERSION: Felt = Felt::from_hex_unchecked("0x434f4e54524143545f434c4153535f4c4541465f5630");

fn compute_class_leaf_hash(compiled_class_hash: &Felt) -> Felt {
    Poseidon::hash(&CONTRACT_CLASS_HASH_VERSION, compiled_class_hash)
}

fn in_memory_class_trie_root(
    class_trie: &mut InMemoryTrie<Poseidon>,
    state_diff: &StateDiff,
    block_n: u64,
) -> Result<(Felt, ClassTrieTimings)> {
    let mut timings = ClassTrieTimings::default();

    let declared_updates: Vec<_> = state_diff
        .declared_classes
        .par_iter()
        .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
            (*class_hash, compute_class_leaf_hash(compiled_class_hash))
        })
        .collect();

    let migrated_updates: Vec<_> = state_diff
        .migrated_compiled_classes
        .par_iter()
        .map(|MigratedClassItem { class_hash, compiled_class_hash }| {
            (*class_hash, compute_class_leaf_hash(compiled_class_hash))
        })
        .collect();

    for (key, value) in declared_updates.into_iter().chain(migrated_updates) {
        let bytes = key.to_bytes_be();
        let bitvec: BitVec<u8, Msb0> = bytes.as_bits()[5..].to_owned();
        class_trie
            .insert(super::super::bonsai_identifier::CLASS, &bitvec, &value)
            .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    }

    let trie_commit_start = Instant::now();
    class_trie.commit(BasicId::new(block_n)).map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    timings.trie_commit = trie_commit_start.elapsed();
    let root = class_trie
        .root_hash(super::super::bonsai_identifier::CLASS)
        .map_err(crate::rocksdb::trie::WrappedBonsaiError)?;
    Ok((root, timings))
}

pub fn compute_root_from_snapshot(
    backend: &RocksDBStorage,
    snapshot_block: Option<u64>,
    snapshot: SnapshotRef,
    block_n: u64,
    state_diff: &StateDiff,
    include_overlay: bool,
    trie_log_mode: TrieLogMode,
) -> Result<InMemoryRootComputation> {
    let config = bonsai_storage_config_for_mode(backend, trie_log_mode);
    let (contract_db, contract_changed) = InMemoryBonsaiDb::contract(Arc::clone(&snapshot));
    let (contract_storage_db, contract_storage_changed) = InMemoryBonsaiDb::contract_storage(Arc::clone(&snapshot));
    let (class_db, class_changed) = InMemoryBonsaiDb::class(snapshot);

    let mut contract_trie: InMemoryTrie<Pedersen> = bonsai_trie::BonsaiStorage::new(contract_db, config.clone(), 251);
    let mut contract_storage_trie: InMemoryTrie<Pedersen> =
        bonsai_trie::BonsaiStorage::new(contract_storage_db, config.clone(), 251);
    let mut class_trie: InMemoryTrie<Poseidon> = bonsai_trie::BonsaiStorage::new(class_db, config, 251);

    let block_start = Instant::now();
    let ((contract_result, contract_duration), (class_result, class_duration)) = rayon::join(
        || {
            let start = Instant::now();
            let result = in_memory_contract_trie_root(
                backend,
                &mut contract_storage_trie,
                &mut contract_trie,
                state_diff,
                block_n,
            );
            (result, start.elapsed())
        },
        || {
            let start = Instant::now();
            let result = in_memory_class_trie_root(&mut class_trie, state_diff, block_n);
            (result, start.elapsed())
        },
    );

    let (contract_root, contract_trie_timings) = contract_result?;
    let (class_root, class_trie_timings) = class_result?;
    let state_root = super::super::calculate_state_root(contract_root, class_root);
    tracing::info!(
        "parallel_root_computed block_number={} source_snapshot_block={snapshot_block:?} source_snapshot_is_future={} contract_root={:#x} class_root={:#x} state_root={:#x} storage_diff_contracts={} deployed_contracts={} replaced_classes={} nonces={} declared_classes={} migrated_compiled_classes={} include_overlay={} trie_log_mode={:?}",
        block_n,
        snapshot_block.is_some_and(|selected| selected > block_n),
        contract_root,
        class_root,
        state_root,
        state_diff.storage_diffs.len(),
        state_diff.deployed_contracts.len(),
        state_diff.replaced_classes.len(),
        state_diff.nonces.len(),
        state_diff.declared_classes.len(),
        state_diff.migrated_compiled_classes.len(),
        include_overlay,
        trie_log_mode
    );

    let timings = crate::rocksdb::global_trie::MerklizationTimings {
        total: block_start.elapsed(),
        contract_trie_root: contract_duration,
        class_trie_root: class_duration,
        contract_trie: contract_trie_timings,
        class_trie: class_trie_timings,
    };

    let overlay =
        include_overlay.then_some(BonsaiOverlay { contract_changed, contract_storage_changed, class_changed });
    Ok(InMemoryRootComputation { block_n, state_root, timings, overlay })
}

pub fn compute_roots_in_parallel_from_snapshot(
    backend: &RocksDBStorage,
    snapshot_block: Option<u64>,
    snapshot: SnapshotRef,
    start_block_n: u64,
    state_diffs: &[StateDiff],
    boundary_block_n: Option<u64>,
    trie_log_mode: TrieLogMode,
) -> Result<Vec<InMemoryRootComputation>> {
    let cumulative = cumulative_squashed_state_diffs(state_diffs.iter());
    let mut roots: Vec<_> = cumulative
        .into_par_iter()
        .enumerate()
        .map(|(index, state_diff)| {
            let block_n = start_block_n + u64::try_from(index).expect("index fits into u64");
            compute_root_from_snapshot(
                backend,
                snapshot_block,
                Arc::clone(&snapshot),
                block_n,
                &state_diff,
                boundary_block_n == Some(block_n),
                trie_log_mode,
            )
        })
        .collect::<Result<_>>()?;

    roots.sort_by_key(|result| result.block_n);
    Ok(roots)
}
