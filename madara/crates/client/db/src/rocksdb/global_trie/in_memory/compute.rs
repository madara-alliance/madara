use super::super::shared;
use super::db::InMemoryBonsaiDb;
use super::overlay::{BonsaiOverlay, TrieLogMode};
use super::state_diff::cumulative_squashed_state_diffs;
use crate::metrics::metrics;
use crate::prelude::*;
use crate::rocksdb::global_trie::{ClassTrieTimings, ContractTrieTimings, MerklizationTimings};
use crate::rocksdb::snapshots::SnapshotRef;
use crate::rocksdb::trie::BasicId;
use crate::rocksdb::{RocksDBStorage, WriteBatchWithTransaction};
use bonsai_trie::BonsaiStorageConfig;
use mp_state_update::StateDiff;
use rayon::prelude::*;
use starknet_types_core::{
    felt::Felt,
    hash::{Pedersen, Poseidon},
};
use std::sync::Arc;
use std::time::Instant;

type InMemoryTrie<H> = bonsai_trie::BonsaiStorage<BasicId, InMemoryBonsaiDb, H>;

#[derive(Debug, Clone)]
pub struct InMemoryRootComputation {
    pub block_n: u64,
    pub state_root: Felt,
    pub timings: MerklizationTimings,
    pub overlay: Option<BonsaiOverlay>,
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

fn in_memory_contract_trie_root(
    backend: &RocksDBStorage,
    contract_storage_trie: &mut InMemoryTrie<Pedersen>,
    contract_trie: &mut InMemoryTrie<Pedersen>,
    state_diff: &StateDiff,
    block_n: u64,
) -> Result<(Felt, ContractTrieTimings)> {
    shared::contract_trie_root_from_parts(
        backend,
        contract_storage_trie,
        contract_trie,
        shared::ContractTrieInputRefs {
            deployed_contracts: &state_diff.deployed_contracts,
            replaced_classes: &state_diff.replaced_classes,
            nonces: &state_diff.nonces,
            storage_diffs: &state_diff.storage_diffs,
            block_n,
        },
    )
}

fn in_memory_class_trie_root(
    class_trie: &mut InMemoryTrie<Poseidon>,
    state_diff: &StateDiff,
    block_n: u64,
) -> Result<(Felt, ClassTrieTimings)> {
    shared::class_trie_root_from_updates(
        class_trie,
        shared::collect_class_updates(&state_diff.declared_classes, &state_diff.migrated_compiled_classes),
        block_n,
    )
}

pub fn compute_root_from_snapshot(
    backend: &RocksDBStorage,
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

    let timings = MerklizationTimings {
        total: block_start.elapsed(),
        contract_trie_root: contract_duration,
        class_trie_root: class_duration,
        contract_trie: contract_trie_timings,
        class_trie: class_trie_timings,
    };
    let contract_root_secs = timings.contract_trie_root.as_secs_f64();
    let class_root_secs = timings.class_trie_root.as_secs_f64();
    let total_secs = timings.total.as_secs_f64();
    let contract_storage_commit_secs = timings.contract_trie.storage_commit.as_secs_f64();
    let contract_commit_secs = timings.contract_trie.trie_commit.as_secs_f64();
    let class_commit_secs = timings.class_trie.trie_commit.as_secs_f64();
    metrics().contract_trie_root_duration.record(contract_root_secs, &[]);
    metrics().contract_trie_root_last.record(contract_root_secs, &[]);
    metrics().class_trie_root_duration.record(class_root_secs, &[]);
    metrics().class_trie_root_last.record(class_root_secs, &[]);
    metrics().apply_to_global_trie_duration.record(total_secs, &[]);
    metrics().apply_to_global_trie_last.record(total_secs, &[]);
    metrics().contract_storage_trie_commit_duration.record(contract_storage_commit_secs, &[]);
    metrics().contract_storage_trie_commit_last.record(contract_storage_commit_secs, &[]);
    metrics().contract_trie_commit_duration.record(contract_commit_secs, &[]);
    metrics().contract_trie_commit_last.record(contract_commit_secs, &[]);
    metrics().class_trie_commit_duration.record(class_commit_secs, &[]);
    metrics().class_trie_commit_last.record(class_commit_secs, &[]);

    let overlay =
        include_overlay.then_some(BonsaiOverlay { contract_changed, contract_storage_changed, class_changed });
    Ok(InMemoryRootComputation { block_n, state_root, timings, overlay })
}

pub fn compute_roots_in_parallel_from_snapshot(
    backend: &RocksDBStorage,
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

pub fn flush_overlay_and_checkpoint(
    backend: &RocksDBStorage,
    block_n: u64,
    overlay: &BonsaiOverlay,
    trie_log_mode: TrieLogMode,
) -> Result<()> {
    let mut batch = WriteBatchWithTransaction::default();
    overlay.put_all_to_batch(backend, trie_log_mode, &mut batch)?;
    backend.inner.parallel_merkle_mark_checkpoint_in_batch(block_n, &mut batch)?;
    backend.inner.db.write_opt(batch, &backend.inner.writeopts)?;
    Ok(())
}
