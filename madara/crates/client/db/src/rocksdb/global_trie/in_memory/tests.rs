use super::*;
use crate::rocksdb::column::Column;
use crate::rocksdb::rocksdb_snapshot::SnapshotWithDBArc;
use crate::rocksdb::state::{CONTRACT_CLASS_HASH_COLUMN, CONTRACT_NONCE_COLUMN};
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
use std::fs;
use std::mem::size_of;
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

fn load_state_diff_fixture(path: &str) -> StateDiff {
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap_or_else(|_| panic!("read fixture {path}")))
            .unwrap_or_else(|_| panic!("parse fixture json {path}"));
    let obj = value.as_object_mut().expect("state diff fixture must be an object");
    obj.entry("old_declared_contracts").or_insert_with(|| serde_json::json!([]));
    obj.entry("replaced_classes").or_insert_with(|| serde_json::json!([]));
    obj.entry("declared_classes").or_insert_with(|| serde_json::json!([]));
    obj.entry("migrated_compiled_classes").or_insert_with(|| serde_json::json!([]));
    serde_json::from_value(value).unwrap_or_else(|_| panic!("parse state diff fixture {path}"))
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

fn make_contract_history_key(contract_address: &Felt, block_n: u32) -> [u8; 32 + size_of::<u32>()] {
    let mut key = [0u8; 32 + size_of::<u32>()];
    key[..32].copy_from_slice(&contract_address.to_bytes_be());
    key[32..].copy_from_slice(&(u32::MAX - block_n).to_be_bytes());
    key
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
fn in_memory_single_block_root_preserves_existing_contract_storage() {
    let contract_address = Felt::from(777_u64);
    let class_hash = Felt::from(888_u64);

    let base_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry { key: Felt::from(1_u64), value: Felt::from(10_u64) },
                StorageEntry { key: Felt::from(2_u64), value: Felt::from(20_u64) },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(1_u64) }],
        migrated_compiled_classes: vec![],
    };
    let current_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry { key: Felt::from(3_u64), value: Felt::from(30_u64) },
                StorageEntry { key: Felt::from(4_u64), value: Felt::from(40_u64) },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let (expected_root, _timings) =
        backend_expected.write_access().apply_to_global_trie(1, [&current_diff]).expect("sequential apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("actual compute");

    assert_eq!(actual.state_root, expected_root, "in-memory trie must preserve historical storage");
}

#[test]
fn in_memory_single_block_root_preserves_existing_storage_across_multiple_contracts() {
    let contract_a = Felt::from(777_u64);
    let contract_b = Felt::from(888_u64);
    let contract_c = Felt::from(999_u64);

    let base_diff = StateDiff {
        storage_diffs: vec![
            ContractStorageDiffItem {
                address: contract_a,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(1_u64), value: Felt::from(10_u64) },
                    StorageEntry { key: Felt::from(2_u64), value: Felt::from(20_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_b,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(3_u64), value: Felt::from(30_u64) },
                    StorageEntry { key: Felt::from(4_u64), value: Felt::from(40_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_c,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(5_u64), value: Felt::from(50_u64) },
                    StorageEntry { key: Felt::from(6_u64), value: Felt::from(60_u64) },
                ],
            },
        ],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![
            DeployedContractItem { address: contract_a, class_hash: Felt::from(11_u64) },
            DeployedContractItem { address: contract_b, class_hash: Felt::from(22_u64) },
            DeployedContractItem { address: contract_c, class_hash: Felt::from(33_u64) },
        ],
        replaced_classes: vec![],
        nonces: vec![
            NonceUpdate { contract_address: contract_a, nonce: Felt::from(1_u64) },
            NonceUpdate { contract_address: contract_b, nonce: Felt::from(2_u64) },
            NonceUpdate { contract_address: contract_c, nonce: Felt::from(3_u64) },
        ],
        migrated_compiled_classes: vec![],
    };
    let current_diff = StateDiff {
        storage_diffs: vec![
            ContractStorageDiffItem {
                address: contract_a,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(7_u64), value: Felt::from(70_u64) },
                    StorageEntry { key: Felt::from(8_u64), value: Felt::from(80_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_b,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(9_u64), value: Felt::from(90_u64) },
                    StorageEntry { key: Felt::from(10_u64), value: Felt::from(100_u64) },
                ],
            },
            ContractStorageDiffItem {
                address: contract_c,
                storage_entries: vec![
                    StorageEntry { key: Felt::from(11_u64), value: Felt::from(110_u64) },
                    StorageEntry { key: Felt::from(12_u64), value: Felt::from(120_u64) },
                ],
            },
        ],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let (expected_root, _timings) =
        backend_expected.write_access().apply_to_global_trie(1, [&current_diff]).expect("sequential apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("actual compute");

    assert_eq!(
        actual.state_root, expected_root,
        "in-memory trie must preserve historical storage when multiple contract storage tries are updated together"
    );
}

#[test]
fn replay_regression_contract_2860_storage_root_matches_sequential_apply() {
    let contract_address =
        Felt::from_hex_unchecked("0x286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6");
    let class_hash = Felt::from_hex_unchecked("0x405f587ee8276e95a6466b37cad24e738ae0fcf2d56fffc94c26840d00a9833");

    let base_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1e2829f14592e9477a272088d9a12bfb3bcb689becc36ca875d36ebaa31cd93"),
                    value: Felt::from_hex_unchecked("0xf71e88194323"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b0"),
                    value: Felt::from_hex_unchecked("0xcc80e7c0"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x53b6948612042f19eec772c614ec9d0f2577bf2eda0c6fc61c627618616ec88"),
                    value: Felt::from_hex_unchecked("0x26f861fad69cd"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0f"),
                    value: Felt::from_hex_unchecked("0x107c03fffbe4"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5b2ea3498b24afb618de9112ce585215a324afd78c6214c4087552b4dfbaf53"),
                    value: Felt::from_hex_unchecked("0xbb64ab3501a9"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b1"),
                    value: Felt::from_hex_unchecked("0x48b8cdcb87"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0e"),
                    value: Felt::from_hex_unchecked(
                        "0x800000000000010ffffffffffffffffffffffffffffffffffffe547179ce483",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x224bea62583ba3072c880ababdcbe5d083407be91587ab7530899ba1455bf0"),
                    value: Felt::from_hex_unchecked("0x15471b350e5dacf"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b2"),
                    value: Felt::from_hex_unchecked("0x1af1ff9c4"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9234"),
                    value: Felt::from_hex_unchecked("0xb9d6e0b380"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9235"),
                    value: Felt::from_hex_unchecked("0x4d1289b62565"),
                },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let current_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1705a5bf42340e733815ddb6d42eabe16220907418e56599a6de3aed153ff15"),
                    value: Felt::from_hex_unchecked(
                        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9235"),
                    value: Felt::from_hex_unchecked("0x4d0d197fdefc"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5f7ffc919f3f967aeabf4a2b9aec07ff3fefadae6695dae4b3fcdd07be5439c"),
                    value: Felt::from_hex_unchecked(
                        "0x6f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5f9e8f374b7853af89720bf3039ae9e7ede5cce41bf02b8efb3612de3248b5c"),
                    value: Felt::from_hex_unchecked(
                        "0x6f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b2"),
                    value: Felt::from_hex_unchecked("0x1af1d307e"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b1"),
                    value: Felt::from_hex_unchecked("0x4d63217ed3"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5f7ffc919f3f967aeabf4a2b9aec07ff3fefadae6695dae4b3fcdd07be5439d"),
                    value: Felt::from_hex_unchecked("0x9184e72a000"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1e2829f14592e9477a272088d9a12bfb3bcb689becc36ca875d36ebaa31cd93"),
                    value: Felt::from_hex_unchecked("0xf71e8809f991"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9234"),
                    value: Felt::from_hex_unchecked("0xb9c9c3c480"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0e"),
                    value: Felt::from_hex_unchecked(
                        "0x800000000000010ffffffffffffffffffffffffffffffffffffe54651ba5166",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b0"),
                    value: Felt::from_hex_unchecked("0xd99dd6c0"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5b2ea3498b24afb618de9112ce585215a324afd78c6214c4087552b4dfbaf53"),
                    value: Felt::from_hex_unchecked("0xbb64ab38105f"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x224bea62583ba3072c880ababdcbe5d083407be91587ab7530899ba1455bf0"),
                    value: Felt::from_hex_unchecked("0x15471b2f4422699"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1705a5bf42340e733815ddb6d42eabe16220907418e56599a6de3aed153ff16"),
                    value: Felt::from_hex_unchecked(
                        "0x2f3f4d7c08fce6fe8ff30feb6a9dcbf88d2c787aeca673fca362eb2832dab72",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x2c11b97d01a63fdae176e288c5b93dc76d1274d50a5d570fe99ecdd14da6460"),
                    value: Felt::from_hex_unchecked(
                        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x53b6948612042f19eec772c614ec9d0f2577bf2eda0c6fc61c627618616ec88"),
                    value: Felt::from_hex_unchecked("0x26f861fb9a4a9"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x6a4a89c00e77bc38c421e7c89f665dd807f0315e361b9c0d5291fdb9918e6ca"),
                    value: Felt::from_hex_unchecked(
                        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0f"),
                    value: Felt::from_hex_unchecked("0x107c6d3edacb"),
                },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let (expected_root, _timings) =
        backend_expected.write_access().apply_to_global_trie(1, [&current_diff]).expect("sequential apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("actual compute");

    assert_eq!(actual.state_root, expected_root, "replay regression for contract 2860 should match sequential");
}

#[test]
fn replay_regression_contract_2860_persisted_base_snapshot_matches_sequential_apply() {
    let contract_address =
        Felt::from_hex_unchecked("0x286003f7c7bfc3f94e8f0af48b48302e7aee2fb13c23b141479ba00832ef2c6");
    let class_hash = Felt::from_hex_unchecked("0x405f587ee8276e95a6466b37cad24e738ae0fcf2d56fffc94c26840d00a9833");

    let base_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1e2829f14592e9477a272088d9a12bfb3bcb689becc36ca875d36ebaa31cd93"),
                    value: Felt::from_hex_unchecked("0xf71e88194323"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b0"),
                    value: Felt::from_hex_unchecked("0xcc80e7c0"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x53b6948612042f19eec772c614ec9d0f2577bf2eda0c6fc61c627618616ec88"),
                    value: Felt::from_hex_unchecked("0x26f861fad69cd"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0f"),
                    value: Felt::from_hex_unchecked("0x107c03fffbe4"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5b2ea3498b24afb618de9112ce585215a324afd78c6214c4087552b4dfbaf53"),
                    value: Felt::from_hex_unchecked("0xbb64ab3501a9"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b1"),
                    value: Felt::from_hex_unchecked("0x48b8cdcb87"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0e"),
                    value: Felt::from_hex_unchecked(
                        "0x800000000000010ffffffffffffffffffffffffffffffffffffe547179ce483",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x224bea62583ba3072c880ababdcbe5d083407be91587ab7530899ba1455bf0"),
                    value: Felt::from_hex_unchecked("0x15471b350e5dacf"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b2"),
                    value: Felt::from_hex_unchecked("0x1af1ff9c4"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9234"),
                    value: Felt::from_hex_unchecked("0xb9d6e0b380"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9235"),
                    value: Felt::from_hex_unchecked("0x4d1289b62565"),
                },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let current_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1705a5bf42340e733815ddb6d42eabe16220907418e56599a6de3aed153ff15"),
                    value: Felt::from_hex_unchecked(
                        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9235"),
                    value: Felt::from_hex_unchecked("0x4d0d197fdefc"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5f7ffc919f3f967aeabf4a2b9aec07ff3fefadae6695dae4b3fcdd07be5439c"),
                    value: Felt::from_hex_unchecked(
                        "0x6f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5f9e8f374b7853af89720bf3039ae9e7ede5cce41bf02b8efb3612de3248b5c"),
                    value: Felt::from_hex_unchecked(
                        "0x6f373b346561036d98ea10fb3e60d2f459c872b1933b50b21fe6ef4fda3b75e",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b2"),
                    value: Felt::from_hex_unchecked("0x1af1d307e"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b1"),
                    value: Felt::from_hex_unchecked("0x4d63217ed3"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5f7ffc919f3f967aeabf4a2b9aec07ff3fefadae6695dae4b3fcdd07be5439d"),
                    value: Felt::from_hex_unchecked("0x9184e72a000"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1e2829f14592e9477a272088d9a12bfb3bcb689becc36ca875d36ebaa31cd93"),
                    value: Felt::from_hex_unchecked("0xf71e8809f991"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x351fa422af4f1d90c3bd7681632c71b198df732bd8ccc704451ccb35d6d9234"),
                    value: Felt::from_hex_unchecked("0xb9c9c3c480"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0e"),
                    value: Felt::from_hex_unchecked(
                        "0x800000000000010ffffffffffffffffffffffffffffffffffffe54651ba5166",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x798816bc0d0cdd4d2ccbd7548fc58169b2200d880a5fdd20786697b56c829b0"),
                    value: Felt::from_hex_unchecked("0xd99dd6c0"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x5b2ea3498b24afb618de9112ce585215a324afd78c6214c4087552b4dfbaf53"),
                    value: Felt::from_hex_unchecked("0xbb64ab38105f"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x224bea62583ba3072c880ababdcbe5d083407be91587ab7530899ba1455bf0"),
                    value: Felt::from_hex_unchecked("0x15471b2f4422699"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x1705a5bf42340e733815ddb6d42eabe16220907418e56599a6de3aed153ff16"),
                    value: Felt::from_hex_unchecked(
                        "0x2f3f4d7c08fce6fe8ff30feb6a9dcbf88d2c787aeca673fca362eb2832dab72",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x2c11b97d01a63fdae176e288c5b93dc76d1274d50a5d570fe99ecdd14da6460"),
                    value: Felt::from_hex_unchecked(
                        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x53b6948612042f19eec772c614ec9d0f2577bf2eda0c6fc61c627618616ec88"),
                    value: Felt::from_hex_unchecked("0x26f861fb9a4a9"),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0x6a4a89c00e77bc38c421e7c89f665dd807f0315e361b9c0d5291fdb9918e6ca"),
                    value: Felt::from_hex_unchecked(
                        "0x7251cef9a80c41e67176978fc08f3d649b56559a391c889b0719fd671e4d781",
                    ),
                },
                StorageEntry {
                    key: Felt::from_hex_unchecked("0xd18a1105b691ce8d68944784622bbabb7964b73d1ab81c0240ef2eda493f0f"),
                    value: Felt::from_hex_unchecked("0x107c6d3edacb"),
                },
            ],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    backend_expected.write_access().apply_to_global_trie(0, [&base_diff]).expect("build persisted base trie");
    let (expected_root, _timings) = backend_expected
        .write_access()
        .apply_to_global_trie(1, [&current_diff])
        .expect("sequential apply from persisted base");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    backend_actual.write_access().apply_to_global_trie(0, [&base_diff]).expect("build persisted base trie");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        1,
        &current_diff,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("actual compute from persisted base");

    assert_eq!(
        actual.state_root, expected_root,
        "persisted-base replay regression for contract 2860 should match sequential"
    );
}

#[test]
fn debug_replay_regression_669160_full_window_matches_sequential_apply() {
    let base_diff = load_state_diff_fixture("/tmp/state_update_669158.json");
    let diff_159 = load_state_diff_fixture("/tmp/state_update_669159.json");
    let diff_160 = load_state_diff_fixture("/tmp/state_update_669160.json");

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    backend_expected.write_access().apply_to_global_trie(0, [&base_diff]).expect("build persisted base trie");
    backend_expected.db.inner.state_apply_state_diff(1, &diff_159).expect("seed 669159 history");
    backend_expected.write_access().apply_to_global_trie(1, [&diff_159]).expect("sequential 669159 apply");
    backend_expected.db.inner.state_apply_state_diff(2, &diff_160).expect("seed 669160 history");
    let (expected_root, _timings) =
        backend_expected.write_access().apply_to_global_trie(2, [&diff_160]).expect("sequential 669160 apply");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    backend_actual.write_access().apply_to_global_trie(0, [&base_diff]).expect("build persisted base trie");

    let squashed = cumulative_squashed_state_diffs([&diff_159, &diff_160]);
    let cumulative = squashed.last().expect("cumulative diff for 669160");
    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        fresh_snapshot(&backend_actual.db),
        2,
        cumulative,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("actual compute from cumulative diff");

    assert_eq!(actual.state_root, expected_root, "669159+669160 cumulative replay window should match sequential");
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
fn contract_leaf_fallback_reads_nonce_and_class_from_snapshot_state() {
    let contract_address = Felt::from(777_u64);
    let original_nonce = Felt::from(11_u64);
    let original_class_hash = Felt::from(22_u64);
    let mutated_nonce = Felt::from(33_u64);
    let mutated_class_hash = Felt::from(44_u64);

    let base_diff = StateDiff {
        storage_diffs: vec![],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash: original_class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: original_nonce }],
        migrated_compiled_classes: vec![],
    };
    let current_diff = StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(55_u64) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![],
        replaced_classes: vec![],
        nonces: vec![],
        migrated_compiled_classes: vec![],
    };

    let backend_expected = setup_backend();
    backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let expected = compute_root_from_snapshot(
        &backend_expected.db,
        Some(0),
        fresh_snapshot(&backend_expected.db),
        1,
        &current_diff,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("expected compute");

    let backend_actual = setup_backend();
    backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
    let snapshot = fresh_snapshot(&backend_actual.db);

    let nonce_handle = backend_actual.db.inner.get_column(CONTRACT_NONCE_COLUMN);
    let class_handle = backend_actual.db.inner.get_column(CONTRACT_CLASS_HASH_COLUMN);
    backend_actual
        .db
        .inner
        .db
        .put_cf(
            &nonce_handle,
            make_contract_history_key(&contract_address, 1),
            crate::rocksdb::serialize_to_smallvec::<[u8; 64]>(&mutated_nonce).expect("serialize nonce"),
        )
        .expect("mutate live nonce history");
    backend_actual
        .db
        .inner
        .db
        .put_cf(
            &class_handle,
            make_contract_history_key(&contract_address, 1),
            crate::rocksdb::serialize_to_smallvec::<[u8; 64]>(&mutated_class_hash).expect("serialize class hash"),
        )
        .expect("mutate live class history");

    let actual = compute_root_from_snapshot(
        &backend_actual.db,
        Some(0),
        snapshot,
        1,
        &current_diff,
        false,
        TrieLogMode::Checkpoint,
    )
    .expect("actual compute");

    assert_eq!(actual.state_root, expected.state_root, "snapshot-scoped fallback should ignore live DB mutation");
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

fn next_u64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 7;
    x ^= x >> 9;
    x ^= x << 8;
    *state = x;
    x
}

fn random_felt_251(state: &mut u64) -> Felt {
    let mut bytes = [0u8; 32];
    for chunk in bytes.chunks_mut(8) {
        chunk.copy_from_slice(&next_u64(state).to_be_bytes());
    }
    bytes[0] &= 0x07;
    Felt::from_bytes_be(&bytes)
}

#[test]
fn search_in_memory_mismatch_against_sequential_for_complex_storage_shapes() {
    for seed in 1_u64..=128 {
        let mut state = seed;
        let contracts: Vec<_> = (0..6).map(|_| random_felt_251(&mut state)).collect();

        let base_diff = StateDiff {
            storage_diffs: contracts
                .iter()
                .map(|address| ContractStorageDiffItem {
                    address: *address,
                    storage_entries: (0..8)
                        .map(|_| StorageEntry { key: random_felt_251(&mut state), value: random_felt_251(&mut state) })
                        .collect(),
                })
                .collect(),
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: contracts
                .iter()
                .map(|address| DeployedContractItem { address: *address, class_hash: random_felt_251(&mut state) })
                .collect(),
            replaced_classes: vec![],
            nonces: contracts
                .iter()
                .map(|address| NonceUpdate {
                    contract_address: *address,
                    nonce: Felt::from(next_u64(&mut state) & 0xff),
                })
                .collect(),
            migrated_compiled_classes: vec![],
        };
        let current_diff = StateDiff {
            storage_diffs: contracts
                .iter()
                .map(|address| ContractStorageDiffItem {
                    address: *address,
                    storage_entries: (0..6)
                        .map(|_| StorageEntry { key: random_felt_251(&mut state), value: random_felt_251(&mut state) })
                        .collect(),
                })
                .collect(),
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: contracts
                .iter()
                .take(3)
                .map(|address| NonceUpdate {
                    contract_address: *address,
                    nonce: Felt::from(next_u64(&mut state) & 0xff),
                })
                .collect(),
            migrated_compiled_classes: vec![],
        };

        let backend_expected = setup_backend();
        backend_expected.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
        let (expected_root, _timings) =
            backend_expected.write_access().apply_to_global_trie(1, [&current_diff]).expect("sequential apply");

        let backend_actual = setup_backend();
        backend_actual.db.inner.state_apply_state_diff(0, &base_diff).expect("seed base history");
        let actual = compute_root_from_snapshot(
            &backend_actual.db,
            Some(0),
            fresh_snapshot(&backend_actual.db),
            1,
            &current_diff,
            false,
            TrieLogMode::Checkpoint,
        )
        .expect("actual compute");

        assert_eq!(
            actual.state_root, expected_root,
            "seed {seed} produced a mismatch between in-memory and sequential trie computation"
        );
    }
}
