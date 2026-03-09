use super::*;
use crate::rocksdb::column::Column;
use crate::rocksdb::rocksdb_snapshot::SnapshotWithDBArc;
use crate::rocksdb::trie::{
    BONSAI_CLASS_LOG_COLUMN, BONSAI_CONTRACT_FLAT_COLUMN, BONSAI_CONTRACT_LOG_COLUMN,
    BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
};
use crate::rocksdb::RocksDBStorage;
use crate::MadaraBackend;
use bonsai_trie::{BonsaiDatabase, ByteVec, DatabaseKey};
use mp_chain_config::ChainConfig;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, MigratedClassItem, NonceUpdate,
    ReplacedClassItem, StateDiff, StorageEntry,
};
use rocksdb::IteratorMode;
use starknet_types_core::felt::Felt;
use std::sync::Arc;

use crate::rocksdb::snapshots::SnapshotRef;

fn setup_backend() -> Arc<MadaraBackend> {
    MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()))
}

fn setup_snapshot_db() -> Arc<MadaraBackend> {
    setup_backend()
}

fn write_snapshot_value(backend: &RocksDBStorage, column: Column, key: &[u8], value: &[u8]) {
    let handle = backend.inner.get_column(column);
    backend.inner.db.put_cf(&handle, key, value).expect("write snapshot value");
}

fn fresh_snapshot(backend: &RocksDBStorage) -> SnapshotRef {
    Arc::new(SnapshotWithDBArc::new(Arc::clone(&backend.inner)))
}

fn count_column_entries(backend: &RocksDBStorage, column: Column) -> usize {
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

#[test]
fn in_memory_bonsai_overlay_hit_beats_snapshot() {
    let backend = setup_snapshot_db();
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
    let backend = setup_snapshot_db();
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
    let backend = setup_snapshot_db();
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
    use crate::rocksdb::WriteBatchWithTransaction;

    let backend = setup_snapshot_db();
    let snapshot = fresh_snapshot(&backend.db);
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());
    let key = b"not-persisted-key";
    let handle = backend.db.inner.get_column(BONSAI_CONTRACT_FLAT_COLUMN);

    db.insert(&DatabaseKey::Flat(key), b"overlay-value", None).expect("insert overlay value");
    use bonsai_trie::BonsaiDatabase;
    db.write_batch(WriteBatchWithTransaction::default()).expect("write batch no-op");

    let persisted = backend.db.inner.db.get_cf(&handle, key).expect("read rocksdb");
    assert_eq!(persisted, None, "overlay writes must not persist before explicit flush");
}

#[test]
fn in_memory_single_block_root_matches_sequential_apply() {
    let backend_seq = setup_backend();
    let backend_mem = setup_backend();
    let diff = synthetic_state_diff(0);

    let expected_root = sequential_roots(&backend_seq, std::slice::from_ref(&diff))[0];
    let snapshot = fresh_snapshot(&backend_mem.db);
    let computed = compute_root_from_snapshot(&backend_mem.db, None, snapshot, 0, &diff, false, TrieLogMode::Off)
        .expect("compute");

    assert_eq!(computed.state_root, expected_root);
    assert!(computed.overlay.is_none(), "overlay should be absent when include_overlay=false");
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

    let expected_roots = sequential_roots(&backend_seq, &diffs);
    let snapshot = fresh_snapshot(&backend_mem.db);
    let results = compute_roots_in_parallel_from_snapshot(
        &backend_mem.db,
        None,
        snapshot,
        0,
        &diffs,
        Some(2),
        TrieLogMode::Checkpoint,
    )
    .expect("parallel roots");

    let got_roots: Vec<_> = results.iter().map(|result| result.state_root).collect();
    assert_eq!(got_roots, expected_roots);
    assert_eq!(results.iter().filter(|result| result.overlay.is_some()).count(), 1);
    assert_eq!(results.iter().find(|result| result.overlay.is_some()).map(|result| result.block_n), Some(2));
}

#[test]
fn boundary_flush_updates_persisted_root_and_checkpoint() {
    let backend = setup_backend();
    let diffs: Vec<_> = (0_u64..3).map(synthetic_state_diff).collect();
    let snapshot = fresh_snapshot(&backend.db);

    let results = compute_roots_in_parallel_from_snapshot(
        &backend.db,
        None,
        snapshot,
        0,
        &diffs,
        Some(2),
        TrieLogMode::Checkpoint,
    )
    .expect("parallel roots");
    let boundary = results.last().expect("boundary result");
    let overlay = boundary.overlay.as_ref().expect("boundary overlay");

    flush_overlay_and_checkpoint(&backend.db, boundary.block_n, overlay, TrieLogMode::Checkpoint)
        .expect("flush and checkpoint");

    let persisted_root = crate::rocksdb::global_trie::get_state_root(&backend.db).expect("read persisted root");
    assert_eq!(persisted_root, boundary.state_root);
    assert_eq!(backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"), Some(2));
    assert!(backend.has_parallel_merkle_checkpoint(2).expect("checkpoint marker"));
}

#[test]
fn trie_log_mode_controls_log_column_flush_behavior() {
    let backend_off = setup_backend();
    let backend_checkpoint = setup_backend();
    let diff = synthetic_state_diff(0);

    let off_snapshot = fresh_snapshot(&backend_off.db);
    let off_result = compute_root_from_snapshot(&backend_off.db, None, off_snapshot, 0, &diff, true, TrieLogMode::Off)
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
        None,
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

    let off_log_entries = count_column_entries(&backend_off.db, BONSAI_CONTRACT_LOG_COLUMN)
        + count_column_entries(&backend_off.db, BONSAI_CONTRACT_STORAGE_LOG_COLUMN)
        + count_column_entries(&backend_off.db, BONSAI_CLASS_LOG_COLUMN);
    let checkpoint_log_entries = count_column_entries(&backend_checkpoint.db, BONSAI_CONTRACT_LOG_COLUMN)
        + count_column_entries(&backend_checkpoint.db, BONSAI_CONTRACT_STORAGE_LOG_COLUMN)
        + count_column_entries(&backend_checkpoint.db, BONSAI_CLASS_LOG_COLUMN);

    assert_eq!(off_log_entries, 0, "off mode should not persist trie logs");
    assert!(checkpoint_log_entries > 0, "checkpoint mode should persist trie logs");
}

#[test]
fn checkpoint_metadata_must_be_monotonic() {
    let backend = setup_backend();
    backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5");
    backend.write_parallel_merkle_checkpoint(8).expect("checkpoint 8");

    let err = backend.write_parallel_merkle_checkpoint(7).expect_err("checkpoint regression should be rejected");
    let message = format!("{err:#}");
    assert!(message.contains("must be monotonic"), "unexpected error: {message}");
}

#[test]
fn checkpoint_metadata_floor_and_cleanup_follow_revert_target() {
    let backend = setup_backend();
    backend.write_parallel_merkle_checkpoint(2).expect("checkpoint 2");
    backend.write_parallel_merkle_checkpoint(5).expect("checkpoint 5");
    backend.write_parallel_merkle_checkpoint(8).expect("checkpoint 8");

    assert_eq!(
        backend.db.get_parallel_merkle_checkpoint_floor(7).expect("floor"),
        Some(5),
        "floor should pick greatest checkpoint <= target"
    );

    backend.db.remove_parallel_merkle_checkpoints_above(5).expect("remove checkpoints above target");

    assert_eq!(
        backend.get_parallel_merkle_latest_checkpoint().expect("latest checkpoint"),
        Some(5),
        "latest checkpoint pointer should be rewound to target floor"
    );
    assert!(backend.has_parallel_merkle_checkpoint(2).expect("checkpoint 2 must remain"));
    assert!(backend.has_parallel_merkle_checkpoint(5).expect("checkpoint 5 must remain"));
    assert!(!backend.has_parallel_merkle_checkpoint(8).expect("checkpoint 8 must be removed"));
}
