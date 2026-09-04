use super::*;
use crate::rocksdb::global_trie::bonsai_identifier;
use bitvec::{order::Msb0, vec::BitVec, view::AsBits};
use mp_convert::Felt;
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry};

fn contract_trie_key(key: Felt) -> BitVec<u8, Msb0> {
    let bytes = key.to_bytes_be();
    bytes.as_bits()[5..].to_owned()
}

#[test]
fn open_preserves_unknown_legacy_column_families() {
    const LEGACY_COLUMN: &str = "tainted_rebuild_carry";
    const LEGACY_KEY: &[u8] = b"legacy-key";
    const LEGACY_VALUE: &[u8] = b"legacy-value";

    let temp_dir = tempfile::TempDir::new().unwrap();
    let mut options = RocksDBOptions::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    {
        let db = DB::open_cf(&options, temp_dir.path(), [LEGACY_COLUMN]).unwrap();
        let legacy = db.cf_handle(LEGACY_COLUMN).unwrap();
        db.put_cf(&legacy, LEGACY_KEY, LEGACY_VALUE).unwrap();
    }

    let storage = RocksDBStorage::open(temp_dir.path(), RocksDBConfig::default()).unwrap();
    let legacy = storage.inner.db.cf_handle(LEGACY_COLUMN).expect("legacy column should remain open");
    assert_eq!(storage.inner.db.get_cf(&legacy, LEGACY_KEY).unwrap().as_deref(), Some(LEGACY_VALUE));
}

#[test]
fn trie_revert_action_handles_equal_older_and_missing_heads() {
    assert_eq!(trie_revert_action(Some(12), 8), TrieRevertAction::Revert { current: 12, target: 8 });
    assert_eq!(trie_revert_action(Some(8), 8), TrieRevertAction::AlreadyAtTarget(8));
    assert_eq!(trie_revert_action(Some(5), 8), TrieRevertAction::OlderThanTarget { current: 5, target: 8 });
    assert_eq!(trie_revert_action(None, 8), TrieRevertAction::Missing);
}

#[test]
fn parallel_merkle_revert_rejects_ranges_outside_retained_logs() {
    ensure_parallel_merkle_revert_is_retained(20_000, 10_001, 10_001, Some(10_000))
        .expect("the oldest retained target should remain revertible");
    ensure_parallel_merkle_revert_is_retained(20_000, 0, 0, None)
        .expect("unbounded retention should permit an older target");

    let error = ensure_parallel_merkle_revert_is_retained(20_000, 10_002, 9_999, Some(10_000))
        .expect_err("a checkpoint floor older than retained trie logs must be rejected");
    assert!(error.to_string().contains("checkpoint floor 9999 predates first retained trie-log revision 10001"));
}

#[test]
fn reorg_root_mismatch_is_fatal_before_head_advancement() {
    let expected_root = Felt::from(1_u64);
    let actual_root = Felt::from(2_u64);

    ensure_reorg_target_root_matches(8, expected_root, expected_root).expect("matching target root should be accepted");
    let error = ensure_reorg_target_root_matches(8, expected_root, actual_root)
        .expect_err("mismatched target root must stop the reorg");
    assert!(error.to_string().contains("refusing to advance head projection"));
}

#[test]
fn latest_bonsai_log_id_reads_latest_committed_revision() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let storage = RocksDBStorage::open(temp_dir.path(), RocksDBConfig::default()).unwrap();

    let mut trie = storage.contract_trie();
    let key_a = contract_trie_key(Felt::from(1u64));
    trie.insert(bonsai_identifier::CONTRACT, &key_a, &Felt::from(11u64)).unwrap();
    trie.commit(BasicId::new(2)).unwrap();

    let key_b = contract_trie_key(Felt::from(2u64));
    trie.insert(bonsai_identifier::CONTRACT, &key_b, &Felt::from(22u64)).unwrap();
    trie.commit(BasicId::new(5)).unwrap();

    assert_eq!(storage.inner.latest_bonsai_log_id(trie::BONSAI_CONTRACT_LOG_COLUMN).unwrap(), Some(5));
}

#[test]
fn revert_single_trie_reverts_and_commits_on_the_same_handle() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let storage = RocksDBStorage::open(temp_dir.path(), RocksDBConfig::default()).unwrap();

    let key_a = contract_trie_key(Felt::from(1u64));
    let key_b = contract_trie_key(Felt::from(2u64));

    let mut trie = storage.contract_trie();
    trie.insert(bonsai_identifier::CONTRACT, &key_a, &Felt::from(11u64)).unwrap();
    let root_at_2 = trie.root_hash_staged(bonsai_identifier::CONTRACT).unwrap();
    trie.commit(BasicId::new(2)).unwrap();

    trie.insert(bonsai_identifier::CONTRACT, &key_b, &Felt::from(22u64)).unwrap();
    let root_at_5 = trie.root_hash_staged(bonsai_identifier::CONTRACT).unwrap();
    trie.commit(BasicId::new(5)).unwrap();

    let mut trie = storage.contract_trie();
    assert!(revert_single_trie("contract", &mut trie, Some(5), 2).unwrap());
    trie.commit(BasicId::new(2)).unwrap();

    let latest_head = storage.inner.latest_bonsai_log_id(trie::BONSAI_CONTRACT_LOG_COLUMN).unwrap();
    assert_eq!(latest_head, Some(2));

    let current_root = storage.contract_trie().root_hash_staged(bonsai_identifier::CONTRACT).unwrap();
    assert_eq!(current_root, root_at_2);
    assert_ne!(current_root, root_at_5);
}

#[test]
fn revert_single_trie_skipped_paths_do_not_fabricate_target_revisions() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let storage = RocksDBStorage::open(temp_dir.path(), RocksDBConfig::default()).unwrap();

    let key = contract_trie_key(Felt::from(1u64));
    let mut trie = storage.contract_trie();
    trie.insert(bonsai_identifier::CONTRACT, &key, &Felt::from(11u64)).unwrap();
    trie.commit(BasicId::new(5)).unwrap();

    let mut older_than_target = storage.contract_trie();
    assert!(!revert_single_trie("contract", &mut older_than_target, Some(5), 8).unwrap());
    assert_eq!(storage.inner.latest_bonsai_log_id(trie::BONSAI_CONTRACT_LOG_COLUMN).unwrap(), Some(5));

    let mut already_at_target = storage.contract_trie();
    assert!(!revert_single_trie("contract", &mut already_at_target, Some(5), 5).unwrap());
    assert_eq!(storage.inner.latest_bonsai_log_id(trie::BONSAI_CONTRACT_LOG_COLUMN).unwrap(), Some(5));

    let mut missing = storage.class_trie();
    assert!(!revert_single_trie("class", &mut missing, None, 8).unwrap());
    assert_eq!(storage.inner.latest_bonsai_log_id(trie::BONSAI_CLASS_LOG_COLUMN).unwrap(), None);
}

fn create_test_storage(config: RocksDBConfig) -> (tempfile::TempDir, RocksDBStorage) {
    let temp_dir = tempfile::TempDir::with_prefix("rocksdb-exact-base-test").unwrap();
    let storage = RocksDBStorage::open(temp_dir.path(), config).unwrap();
    (temp_dir, storage)
}

fn synthetic_state_diff(index: u64) -> StateDiff {
    let contract_address = Felt::from(10_000 + index);
    let class_hash = Felt::from(20_000 + index);
    StateDiff {
        storage_diffs: vec![ContractStorageDiffItem {
            address: contract_address,
            storage_entries: vec![StorageEntry { key: Felt::from(1_u64), value: Felt::from(40_000 + index) }],
        }],
        old_declared_contracts: vec![],
        declared_classes: vec![],
        deployed_contracts: vec![DeployedContractItem { address: contract_address, class_hash }],
        replaced_classes: vec![],
        nonces: vec![NonceUpdate { contract_address, nonce: Felt::from(index + 1) }],
        migrated_compiled_classes: vec![],
    }
}

#[test]
fn latest_snapshot_path_uses_exact_checkpoint_base() {
    let (_temp_expected, expected_storage) = create_test_storage(RocksDBConfig::default());
    let diff0 = synthetic_state_diff(0);
    let diff1 = synthetic_state_diff(1);

    expected_storage.apply_to_global_trie(0, [&diff0], StarknetVersion::LATEST).expect("apply block 0");
    expected_storage.on_new_confirmed_head(0).expect("confirm block 0");
    let (expected_root, _) =
        expected_storage.apply_to_global_trie(1, [&diff1], StarknetVersion::LATEST).expect("apply block 1");

    let (_temp_actual, storage) = create_test_storage(RocksDBConfig::default());
    storage.apply_to_global_trie(0, [&diff0], StarknetVersion::LATEST).expect("apply block 0");
    storage.write_parallel_merkle_checkpoint(0).expect("checkpoint 0");
    storage.on_new_confirmed_head(0).expect("confirm block 0");

    let results = storage
        .compute_roots_in_parallel_from_latest_snapshot(1, &[diff1], StarknetVersion::LATEST, None)
        .expect("parallel roots");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].state_root, expected_root);
}

#[test]
fn latest_snapshot_path_rejects_snapshot_at_first_diff_block() {
    let (_temp_dir, storage) = create_test_storage(RocksDBConfig::default());
    let diff0 = synthetic_state_diff(0);
    let diff1 = synthetic_state_diff(1);

    storage.apply_to_global_trie(0, [&diff0], StarknetVersion::LATEST).expect("apply block 0");
    storage.on_new_confirmed_head(0).expect("confirm block 0");
    storage.apply_to_global_trie(1, [&diff1], StarknetVersion::LATEST).expect("apply block 1");
    storage.on_new_confirmed_head(1).expect("confirm block 1");

    let err = storage
        .compute_roots_in_parallel_from_latest_snapshot(1, &[diff1], StarknetVersion::LATEST, None)
        .expect_err("missing exact base snapshot should fail");
    let message = format!("{err:#}");
    assert!(message.contains("Missing exact base snapshot"), "unexpected error: {message}");
}

#[test]
fn empty_base_snapshot_survives_head_advance_for_precheckpoint_batches() {
    let (_temp_expected, expected_storage) = create_test_storage(RocksDBConfig::default());
    let diffs: Vec<_> = (0_u64..3).map(synthetic_state_diff).collect();
    let mut expected_roots = Vec::new();
    for (block_n, diff) in diffs.iter().enumerate() {
        let block_n = block_n as u64;
        let (root, _) =
            expected_storage.apply_to_global_trie(block_n, [diff], StarknetVersion::LATEST).expect("sequential apply");
        expected_storage.on_new_confirmed_head(block_n).expect("confirm block");
        expected_roots.push(root);
    }

    let (_temp_actual, storage) = create_test_storage(RocksDBConfig::default());
    storage.apply_to_global_trie(0, [&diffs[0]], StarknetVersion::LATEST).expect("apply block 0");
    storage.on_new_confirmed_head(0).expect("confirm block 0");
    storage.apply_to_global_trie(1, [&diffs[1]], StarknetVersion::LATEST).expect("apply block 1");
    storage.on_new_confirmed_head(1).expect("confirm block 1");

    let results = storage
        .compute_roots_in_parallel_from_latest_snapshot(0, &diffs, StarknetVersion::LATEST, None)
        .expect("parallel roots from empty base");
    let got_roots: Vec<_> = results.into_iter().map(|result| result.state_root).collect();
    assert_eq!(got_roots, expected_roots);
}
