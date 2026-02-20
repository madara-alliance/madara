use super::db::{InMemoryBonsaiDb, InMemoryColumnMapping};
use super::*;
use crate::rocksdb::column::Column;
use crate::rocksdb::rocksdb_snapshot::SnapshotWithDBArc;
use crate::rocksdb::snapshots::SnapshotRef;
use crate::rocksdb::RocksDBStorage;
use crate::MadaraBackend;
use mp_chain_config::ChainConfig;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

fn setup_backend() -> Arc<MadaraBackend> {
    MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()))
}

fn write_snapshot_value(backend: &RocksDBStorage, column: Column, key: &[u8], value: &[u8]) {
    let handle = backend.inner.get_column(column);
    backend.inner.db.put_cf(&handle, key, value).expect("write snapshot value");
}

fn fresh_snapshot(backend: &RocksDBStorage) -> SnapshotRef {
    Arc::new(SnapshotWithDBArc::new(Arc::clone(&backend.inner)))
}

fn count_column_entries(backend: &RocksDBStorage, column: Column) -> usize {
    use rocksdb::IteratorMode;

    let handle = backend.inner.get_column(column);
    backend.inner.db.iterator_cf(&handle, IteratorMode::Start).map_while(|item| item.ok()).count()
}

fn synthetic_state_diff(index: u64) -> StateDiff {
    let contract_address = Felt::from(10_000 + index);
    let class_hash = Felt::from(20_000 + index);
    let compiled_class_hash = Felt::from(30_000 + index);

    StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(40_000 + index) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![DeclaredClassItem { class_hash, compiled_class_hash }],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(index + 1) }],
        migrated_compiled_classes: vec![],
    }
}

fn sequential_roots(backend: &Arc<MadaraBackend>, diffs: &[StateDiff]) -> Vec<Felt> {
    let mut roots = Vec::new();
    for (index, diff) in diffs.iter().enumerate() {
        let block_n = u64::try_from(index).expect("index fits into u64");
        let (root, _timings) = backend
            .write_access()
            .apply_to_global_trie(block_n, [diff])
            .expect("sequential apply_to_global_trie should succeed");
        roots.push(root);
    }
    roots
}

fn compute_parallel_results(
    backend: &Arc<MadaraBackend>,
    diffs: &[StateDiff],
    boundary_block_n: Option<u64>,
    trie_log_mode: TrieLogMode,
) -> Vec<InMemoryRootComputation> {
    let snapshot = fresh_snapshot(&backend.db);
    compute_roots_in_parallel_from_snapshot(&backend.db, snapshot, 0, diffs, boundary_block_n, trie_log_mode)
        .expect("parallel roots")
}

fn assert_parallel_roots_match_sequential(
    backend_seq: &Arc<MadaraBackend>,
    backend_parallel: &Arc<MadaraBackend>,
    diffs: &[StateDiff],
    boundary_block_n: Option<u64>,
    trie_log_mode: TrieLogMode,
) -> Vec<InMemoryRootComputation> {
    let expected_roots = sequential_roots(backend_seq, diffs);
    let results = compute_parallel_results(backend_parallel, diffs, boundary_block_n, trie_log_mode);
    let got_roots: Vec<_> = results.iter().map(|result| result.state_root).collect();
    assert_eq!(got_roots, expected_roots);
    results
}

fn assert_checkpoint_state(backend: &Arc<MadaraBackend>, block_n: u64) {
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(block_n));
    assert!(backend.has_parallel_merkle_checkpoint(block_n).expect("checkpoint marker"));
}

fn count_trie_log_entries(backend: &Arc<MadaraBackend>) -> usize {
    use crate::rocksdb::trie::{
        BONSAI_CLASS_LOG_COLUMN, BONSAI_CONTRACT_LOG_COLUMN, BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
    };

    count_column_entries(&backend.db, BONSAI_CONTRACT_LOG_COLUMN)
        + count_column_entries(&backend.db, BONSAI_CONTRACT_STORAGE_LOG_COLUMN)
        + count_column_entries(&backend.db, BONSAI_CLASS_LOG_COLUMN)
}

#[test]
fn in_memory_bonsai_overlay_hit_beats_snapshot() {
    use crate::rocksdb::trie::BONSAI_CONTRACT_FLAT_COLUMN;
    use bonsai_trie::{BonsaiDatabase, ByteVec, DatabaseKey};

    let backend = setup_backend();
    let key = b"overlay-hit-key";
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, key, b"snapshot-value");
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());

    let got_from_snapshot = db.get(&DatabaseKey::Flat(key)).expect("read from snapshot");
    assert_eq!(got_from_snapshot, Some(ByteVec::from(&b"snapshot-value"[..])));

    db.insert(&DatabaseKey::Flat(key), b"overlay-value", None).expect("insert overlay");
    let got = db.get(&DatabaseKey::Flat(key)).expect("read overlay");
    assert_eq!(got, Some(ByteVec::from(&b"overlay-value"[..])));
}

#[test]
fn in_memory_bonsai_tombstone_hides_snapshot_value() {
    use crate::rocksdb::trie::BONSAI_CONTRACT_FLAT_COLUMN;
    use bonsai_trie::{BonsaiDatabase, DatabaseKey};

    let backend = setup_backend();
    let key = b"tombstone-key";
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, key, b"snapshot-value");
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());

    assert!(db.contains(&DatabaseKey::Flat(key)).expect("contains before delete"));
    db.remove(&DatabaseKey::Flat(key), None).expect("remove key");

    assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("read tombstoned key"), None);
    assert!(!db.contains(&DatabaseKey::Flat(key)).expect("contains after delete"));
}

#[test]
fn in_memory_bonsai_insert_remove_contains_are_consistent() {
    use bonsai_trie::{BonsaiDatabase, ByteVec, DatabaseKey};

    let backend = setup_backend();
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());
    let key = b"in-memory-key";

    assert!(!db.contains(&DatabaseKey::Flat(key)).expect("contains before insert"));
    db.insert(&DatabaseKey::Flat(key), b"value-1", None).expect("insert");
    assert!(db.contains(&DatabaseKey::Flat(key)).expect("contains after insert"));
    assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("get after insert"), Some(ByteVec::from(&b"value-1"[..])));

    db.insert(&DatabaseKey::Flat(key), b"value-2", None).expect("overwrite");
    assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("get after overwrite"), Some(ByteVec::from(&b"value-2"[..])));

    db.remove(&DatabaseKey::Flat(key), None).expect("remove");
    assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("get after remove"), None);
    assert!(!db.contains(&DatabaseKey::Flat(key)).expect("contains after remove"));
}

#[test]
fn in_memory_bonsai_write_batch_does_not_persist_to_rocksdb() {
    use crate::rocksdb::trie::BONSAI_CONTRACT_FLAT_COLUMN;
    use crate::rocksdb::WriteBatchWithTransaction;
    use bonsai_trie::{BonsaiDatabase, DatabaseKey};

    let backend = setup_backend();
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());
    let key = b"not-persisted-key";
    let handle = backend.db.inner.get_column(BONSAI_CONTRACT_FLAT_COLUMN);

    db.insert(&DatabaseKey::Flat(key), b"overlay-value", None).expect("insert overlay value");
    db.write_batch(WriteBatchWithTransaction::default()).expect("write batch no-op");

    let persisted = backend.db.inner.db.get_cf(&handle, key).expect("read rocksdb");
    assert_eq!(persisted, None, "overlay writes must not persist before explicit flush");
}

#[test]
fn cumulative_squash_keeps_root_relevant_fields() {
    let contract_address = Felt::from(777_u64);
    let diff_a = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(11_u64) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![DeclaredClassItem {
            class_hash: Felt::from(100_u64),
            compiled_class_hash: Felt::from(200_u64),
        }],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash: Felt::from(100_u64) }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(1_u64) }],
        migrated_compiled_classes: vec![],
    };
    let diff_b = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(22_u64) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![ReplacedClassItem { contract_address, class_hash: Felt::from(101_u64) }],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(2_u64) }],
        migrated_compiled_classes: vec![MigratedClassItem {
            class_hash: Felt::from(101_u64),
            compiled_class_hash: Felt::from(201_u64),
        }],
    };

    let squashed = squash_state_diffs([&diff_a, &diff_b]);
    assert_eq!(squashed.storage_diffs.len(), 1);
    assert_eq!(
        squashed.storage_diffs[0].storage_entries,
        vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(22_u64) }]
    );
    assert_eq!(squashed.nonces, vec![NonceUpdate { contract_address, nonce: Felt::from(2_u64) }]);
    assert_eq!(
        squashed.replaced_classes,
        vec![ReplacedClassItem { contract_address, class_hash: Felt::from(101_u64) }]
    );
    assert_eq!(
        squashed.migrated_compiled_classes,
        vec![MigratedClassItem { class_hash: Felt::from(101_u64), compiled_class_hash: Felt::from(201_u64) }]
    );
}

#[test]
fn in_memory_parallel_roots_match_sequential_per_block() {
    let backend_seq = setup_backend();
    let backend_mem = setup_backend();
    let diffs: Vec<_> = (0_u64..5).map(synthetic_state_diff).collect();

    let results =
        assert_parallel_roots_match_sequential(&backend_seq, &backend_mem, &diffs, Some(2), TrieLogMode::Checkpoint);
    assert_eq!(results.iter().filter(|result| result.overlay.is_some()).count(), 1);
    assert_eq!(results.iter().find(|result| result.overlay.is_some()).map(|result| result.block_n), Some(2));
}

#[test]
fn in_memory_storage_only_diff_reads_nonce_and_class_from_db() {
    let backend_seq = setup_backend();
    let backend_mem = setup_backend();
    let contract_address = Felt::from(42_u64);
    let class_hash = Felt::from(99_u64);
    let init_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(10_u64) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![DeclaredClassItem { class_hash, compiled_class_hash: Felt::from(777_u64) }],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(5_u64) }],
        migrated_compiled_classes: vec![],
    };
    let storage_only_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(2_u64), value: Felt::from(20_u64) }],
        }],
        ..Default::default()
    };

    backend_seq.write_access().apply_to_global_trie(0, [&init_diff]).expect("sequential init should succeed");
    backend_mem.write_access().apply_to_global_trie(0, [&init_diff]).expect("snapshot init should succeed");
    let (expected_root, _timings) = backend_seq
        .write_access()
        .apply_to_global_trie(1, [&storage_only_diff])
        .expect("sequential block #1 should succeed");

    let snapshot = fresh_snapshot(&backend_mem.db);
    let computed =
        compute_root_from_snapshot(&backend_mem.db, snapshot, 1, &storage_only_diff, false, TrieLogMode::Checkpoint)
            .expect("parallel in-memory compute should succeed");

    assert_eq!(computed.state_root, expected_root);
    assert!(computed.overlay.is_none(), "overlay should be absent when include_overlay=false");
}

#[test]
fn boundary_flush_updates_persisted_root_and_checkpoint() {
    let backend = setup_backend();
    let diffs: Vec<_> = (0_u64..3).map(synthetic_state_diff).collect();
    let results = compute_parallel_results(&backend, &diffs, Some(2), TrieLogMode::Checkpoint);
    let boundary = results.last().expect("boundary result");
    let overlay = boundary.overlay.as_ref().expect("boundary overlay");

    flush_overlay_and_checkpoint(&backend.db, boundary.block_n, overlay, TrieLogMode::Checkpoint)
        .expect("flush and checkpoint");

    let persisted_root = crate::rocksdb::global_trie::get_state_root(&backend.db).expect("read persisted root");
    assert_eq!(persisted_root, boundary.state_root);
    assert_checkpoint_state(&backend, 2);
}

#[test]
fn in_memory_get_by_prefix_prefers_overlay_entries() {
    use crate::rocksdb::trie::BONSAI_CONTRACT_FLAT_COLUMN;
    use bonsai_trie::{BonsaiDatabase, DatabaseKey};
    use std::collections::BTreeMap;

    let backend = setup_backend();
    let prefix = b"prefix/";

    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, b"prefix/a", b"snapshot-a");
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, b"prefix/b", b"snapshot-b");
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());

    db.insert(&DatabaseKey::Flat(b"prefix/a"), b"overlay-a", None).expect("overlay update should succeed");
    db.insert(&DatabaseKey::Flat(b"prefix/c"), b"overlay-c", None).expect("overlay insert should succeed");

    let merged = db.get_by_prefix(&DatabaseKey::Flat(prefix)).expect("prefix read should succeed");
    let merged_map: BTreeMap<Vec<u8>, Vec<u8>> =
        merged.into_iter().map(|(k, v)| (k.as_slice().to_vec(), v.as_slice().to_vec())).collect();

    assert_eq!(merged_map.get(b"prefix/a".as_slice()), Some(&b"overlay-a".to_vec()));
    assert_eq!(merged_map.get(b"prefix/b".as_slice()), Some(&b"snapshot-b".to_vec()));
    assert_eq!(merged_map.get(b"prefix/c".as_slice()), Some(&b"overlay-c".to_vec()));
}

#[test]
fn in_memory_remove_by_prefix_tombstones_snapshot_and_overlay() {
    use crate::rocksdb::trie::BONSAI_CONTRACT_FLAT_COLUMN;
    use bonsai_trie::{BonsaiDatabase, DatabaseKey};

    let backend = setup_backend();
    let prefix = b"wipe/";

    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, b"wipe/a", b"snapshot-a");
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, b"wipe/b", b"snapshot-b");
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, b"keep/z", b"snapshot-z");
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());

    db.insert(&DatabaseKey::Flat(b"wipe/c"), b"overlay-c", None).expect("overlay insert should succeed");
    db.insert(&DatabaseKey::Flat(b"keep/y"), b"overlay-y", None).expect("overlay insert should succeed");

    db.remove_by_prefix(&DatabaseKey::Flat(prefix)).expect("remove by prefix should succeed");

    assert_eq!(db.get(&DatabaseKey::Flat(b"wipe/a")).expect("read a"), None);
    assert_eq!(db.get(&DatabaseKey::Flat(b"wipe/b")).expect("read b"), None);
    assert_eq!(db.get(&DatabaseKey::Flat(b"wipe/c")).expect("read c"), None);
    assert_eq!(
        db.get(&DatabaseKey::Flat(b"keep/z")).expect("read untouched snapshot key"),
        Some((&b"snapshot-z"[..]).into())
    );
    assert_eq!(
        db.get(&DatabaseKey::Flat(b"keep/y")).expect("read untouched overlay key"),
        Some((&b"overlay-y"[..]).into())
    );
}

#[test]
fn trie_log_mode_controls_log_column_flush_behavior() {
    let backend_off = setup_backend();
    let backend_checkpoint = setup_backend();
    let diff = synthetic_state_diff(0);

    let off_snapshot = fresh_snapshot(&backend_off.db);
    let off_result = compute_root_from_snapshot(&backend_off.db, off_snapshot, 0, &diff, true, TrieLogMode::Off)
        .expect("off mode compute");
    flush_overlay_and_checkpoint(
        &backend_off.db,
        0,
        off_result.overlay.as_ref().expect("off overlay"),
        TrieLogMode::Off,
    )
    .expect("off mode flush");

    let checkpoint_snapshot = fresh_snapshot(&backend_checkpoint.db);
    let checkpoint_result = compute_root_from_snapshot(
        &backend_checkpoint.db,
        checkpoint_snapshot,
        0,
        &diff,
        true,
        TrieLogMode::Checkpoint,
    )
    .expect("checkpoint mode compute");
    flush_overlay_and_checkpoint(
        &backend_checkpoint.db,
        0,
        checkpoint_result.overlay.as_ref().expect("checkpoint overlay"),
        TrieLogMode::Checkpoint,
    )
    .expect("checkpoint mode flush");

    let off_log_entries = count_trie_log_entries(&backend_off);
    let checkpoint_log_entries = count_trie_log_entries(&backend_checkpoint);

    assert_eq!(off_log_entries, 0, "off mode should not persist trie logs");
    assert!(checkpoint_log_entries > 0, "checkpoint mode should persist trie logs");
}

#[test]
fn flush_overlay_and_checkpoint_is_atomic_on_checkpoint_regression() {
    let backend = setup_backend();
    backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5 should succeed");
    let initial_root = crate::rocksdb::global_trie::get_state_root(&backend.db).expect("read initial root");
    let initial_log_entries = count_trie_log_entries(&backend);
    let diff = synthetic_state_diff(0);
    let snapshot = fresh_snapshot(&backend.db);
    let computed = compute_root_from_snapshot(&backend.db, snapshot, 0, &diff, true, TrieLogMode::Checkpoint)
        .expect("parallel compute should succeed");
    let overlay = computed.overlay.as_ref().expect("overlay should be present when include_overlay=true");

    let err = flush_overlay_and_checkpoint(&backend.db, 4, overlay, TrieLogMode::Checkpoint)
        .expect_err("regressing checkpoint should fail");
    let message = format!("{err:#}");
    assert!(message.contains("must be monotonic"), "unexpected error: {message}");
    assert_eq!(
        crate::rocksdb::global_trie::get_state_root(&backend.db).expect("read root after failed flush"),
        initial_root
    );
    assert_eq!(count_trie_log_entries(&backend), initial_log_entries);
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(5));
    assert!(!backend.has_parallel_merkle_checkpoint(4).expect("checkpoint marker for regressed block"));
}

#[test]
fn compute_roots_boundary_and_empty_input_behavior() {
    let diffs: Vec<_> = (0_u64..3).map(synthetic_state_diff).collect();

    let none_boundary_results = assert_parallel_roots_match_sequential(
        &setup_backend(),
        &setup_backend(),
        &diffs,
        None,
        TrieLogMode::Checkpoint,
    );
    assert!(none_boundary_results.iter().all(|result| result.overlay.is_none()));

    let out_of_range_results = assert_parallel_roots_match_sequential(
        &setup_backend(),
        &setup_backend(),
        &diffs,
        Some(999),
        TrieLogMode::Checkpoint,
    );
    assert!(out_of_range_results.iter().all(|result| result.overlay.is_none()));

    let backend = setup_backend();
    let snapshot = fresh_snapshot(&backend.db);
    let empty_results =
        compute_roots_in_parallel_from_snapshot(&backend.db, snapshot, 0, &[], Some(0), TrieLogMode::Checkpoint)
            .expect("empty state diff list should succeed");
    assert!(empty_results.is_empty());
}
