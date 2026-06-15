use super::*;
use crate::rocksdb::global_trie::bonsai_identifier;
use bitvec::{order::Msb0, vec::BitVec, view::AsBits};
use mp_convert::Felt;

fn contract_trie_key(key: Felt) -> BitVec<u8, Msb0> {
    let bytes = key.to_bytes_be();
    bytes.as_bits()[5..].to_owned()
}

#[test]
fn trie_revert_action_handles_equal_older_and_missing_heads() {
    assert_eq!(trie_revert_action(Some(12), 8), TrieRevertAction::Revert { current: 12, target: 8 });
    assert_eq!(trie_revert_action(Some(8), 8), TrieRevertAction::AlreadyAtTarget(8));
    assert_eq!(trie_revert_action(Some(5), 8), TrieRevertAction::OlderThanTarget { current: 5, target: 8 });
    assert_eq!(trie_revert_action(None, 8), TrieRevertAction::Missing);
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
fn create_checkpoint_writes_openable_rocksdb_checkpoint() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let source_path = temp_dir.path().join("source");
    let checkpoint_path = temp_dir.path().join("checkpoint");
    let storage = RocksDBStorage::open(&source_path, RocksDBConfig::default()).unwrap();

    storage.create_checkpoint(&checkpoint_path).unwrap();
    assert!(checkpoint_path.join("CURRENT").is_file());

    let _checkpoint = RocksDBStorage::open(&checkpoint_path, RocksDBConfig::default()).unwrap();
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
