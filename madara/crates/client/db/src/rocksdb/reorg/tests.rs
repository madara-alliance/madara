use super::*;
use crate::{MadaraBackend, MadaraBackendConfig};
use mc_class_exec::config::NativeConfig;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_state_update::{ContractStorageDiffItem, DeployedContractItem, StorageEntry};

fn open_backend(path: &Path, wal: bool) -> Arc<MadaraBackend> {
    MadaraBackend::open_rocksdb(
        path,
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig::default(),
        // Recovery must not require genesis logs on a serial full node without checkpoints.
        RocksDBConfig {
            max_saved_trie_logs: Some(2),
            write_mode: DbWriteMode { wal, fsync: false },
            ..Default::default()
        },
        Arc::new(NativeConfig::default()),
    )
    .unwrap()
}

fn fill_chain(backend: &Arc<MadaraBackend>) {
    for block_number in 0..4 {
        let address = Felt::from(100 + block_number);
        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number, ..Default::default() },
                    state_diff: StateDiff {
                        deployed_contracts: vec![DeployedContractItem { address, class_hash: Felt::ONE }],
                        storage_diffs: vec![ContractStorageDiffItem {
                            address,
                            storage_entries: vec![StorageEntry { key: Felt::ZERO, value: address }],
                        }],
                        ..Default::default()
                    },
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .unwrap();
    }
    backend.write_latest_applied_trie_update(&Some(3)).unwrap();
}

enum CrashPhase {
    BeforeRollback,
    OneTrie,
    AllTries,
    HeadCommitted,
    RecoveryReplayed,
}

#[rstest::rstest]
#[case::before_rollback(CrashPhase::BeforeRollback)]
#[case::partial_trie_rollback(CrashPhase::OneTrie)]
#[case::before_head_commit(CrashPhase::AllTries)]
#[case::after_head_commit(CrashPhase::HeadCommitted)]
#[case::during_recovery(CrashPhase::RecoveryReplayed)]
fn full_node_restart_repairs_interrupted_reorg(
    #[case] phase: CrashPhase,
    #[values(true, false)] wal: bool,
    #[values(false, true)] migrated: bool,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let (expected_tip, expected_root) = {
        let backend = open_backend(dir.path(), wal);
        fill_chain(&backend);
        if migrated {
            backend.write_parallel_merkle_checkpoint(0).unwrap();
        }
        let target = backend.db.get_block_info(2).unwrap().unwrap();
        let context = backend.db.prepare_reorg(&target.block_hash).unwrap();
        let expected_tip = if matches!(phase, CrashPhase::HeadCommitted) { 2 } else { 3 };
        let expected_root = backend.db.get_block_info(expected_tip).unwrap().unwrap().header.global_state_root;

        match phase {
            CrashPhase::BeforeRollback | CrashPhase::OneTrie => {
                backend.db.begin_reorg_recovery(2).unwrap();
                if matches!(phase, CrashPhase::OneTrie) {
                    let mut contracts = backend.db.contract_trie_for_revert();
                    contracts.revert_to(BasicId::new(2), BasicId::new(3)).unwrap();
                    contracts.commit(BasicId::new(2)).unwrap();
                }
            }
            CrashPhase::AllTries | CrashPhase::HeadCommitted | CrashPhase::RecoveryReplayed => {
                // Use production phases, stopping before completion clears the recovery marker.
                backend.db.revert_tries(&context).unwrap();
                backend.db.verify_reorg_target_root(&context).unwrap();
                if matches!(phase, CrashPhase::HeadCommitted) {
                    backend.db.commit_reorg_head(2, &[], None).unwrap();
                } else if matches!(phase, CrashPhase::RecoveryReplayed) {
                    backend.db.inner.write_parallel_merkle_checkpoint(2).unwrap();
                    backend.db.reconcile_confirmed_parallel_merkle_state(Some(3), "test_interrupted_recovery").unwrap();
                }
            }
        }
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), Some(2));
        backend.flush().unwrap();
        (expected_tip, expected_root)
    };

    // Shared DB open must repair the interrupted transition without producer startup helpers.
    // Opening twice also verifies that completed recovery is durable and idempotent.
    for _ in 0..2 {
        let backend = open_backend(dir.path(), wal);
        assert_eq!(backend.latest_confirmed_block_n(), Some(expected_tip));
        assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(expected_tip));
        assert_eq!(backend.db.get_state_root_hash().unwrap(), expected_root);
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
        if expected_tip == 2 {
            assert!(backend.db.get_block_info(3).unwrap().is_none(), "startup must finish suffix cleanup");
        }
    }
}

#[test]
fn successful_reorg_clears_recovery_marker() {
    let dir = tempfile::TempDir::new().unwrap();
    let expected_root = {
        let backend = open_backend(dir.path(), true);
        fill_chain(&backend);
        let target = backend.db.get_block_info(2).unwrap().unwrap();
        backend.revert_to(&target.block_hash).unwrap();
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
        target.header.global_state_root
    };
    let backend = open_backend(dir.path(), true);
    assert_eq!(backend.latest_confirmed_block_n(), Some(2));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), expected_root);
    assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
}

#[test]
fn failed_reorg_recovery_keeps_marker_for_retry() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), true);
    fill_chain(&backend);
    backend.db.begin_reorg_recovery(2).unwrap();
    // A missing authoritative header must stop recovery before clearing its durable intent.
    backend.db.inner.remove_all_blocks_starting_from(3).unwrap();
    assert!(backend.db.recover_interrupted_reorg(Some(3)).is_err());
    assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), Some(2));
}

#[test]
fn unfinished_reorg_keeps_its_original_recovery_floor() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), true);
    fill_chain(&backend);
    backend.db.begin_reorg_recovery(2).unwrap();
    backend.db.begin_reorg_recovery(2).unwrap();
    assert!(backend.db.begin_reorg_recovery(1).is_err());
    assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), Some(2));
}

#[test]
fn reorg_rejects_pruned_migration_floor_before_mutating_tries() {
    let dir = tempfile::TempDir::new().unwrap();
    let expected_root = {
        let backend = open_backend(dir.path(), true);
        fill_chain(&backend);
        // Migration's checkpoint survives while serial sync advances and prunes old logs.
        backend.write_parallel_merkle_checkpoint(0).unwrap();
        let expected_root = backend.db.get_state_root_hash().unwrap();
        let target = backend.db.get_block_info(0).unwrap().unwrap();

        let error = backend.revert_to(&target.block_hash).unwrap_err();
        assert!(format!("{error:#}").contains("checkpoint floor 0 predates first retained trie-log revision 2"));
        assert_eq!(backend.latest_confirmed_block_n(), Some(3));
        assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(3));
        assert_eq!(backend.db.get_state_root_hash().unwrap(), expected_root);
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
        expected_root
    };
    let backend = open_backend(dir.path(), true);
    assert_eq!(backend.latest_confirmed_block_n(), Some(3));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), expected_root);
}

#[rstest::rstest]
fn recent_reorg_uses_retained_serial_revision_after_migration(#[values(true, false)] wal: bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let expected_root = {
        let backend = open_backend(dir.path(), wal);
        fill_chain(&backend);
        backend.write_parallel_merkle_checkpoint(0).unwrap();
        let target = backend.db.get_block_info(2).unwrap().unwrap();
        backend.revert_to(&target.block_hash).unwrap();
        assert_eq!(backend.get_parallel_merkle_latest_checkpoint().unwrap(), Some(2));
        assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(2));
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
        target.header.global_state_root
    };
    for _ in 0..2 {
        let backend = open_backend(dir.path(), wal);
        assert_eq!(backend.latest_confirmed_block_n(), Some(2));
        assert_eq!(backend.db.get_state_root_hash().unwrap(), expected_root);
        assert!(backend.db.get_block_info(3).unwrap().is_none());
    }
}

#[test]
fn recent_reorg_uses_class_revision_when_other_tries_did_not_change() {
    let dir = tempfile::TempDir::new().unwrap();
    let expected_root = {
        let backend = open_backend(dir.path(), true);
        for block_number in 0..4 {
            let state_diff = if block_number == 2 {
                StateDiff {
                    declared_classes: vec![mp_state_update::DeclaredClassItem {
                        class_hash: Felt::from(200_u64),
                        compiled_class_hash: Felt::from(300_u64),
                    }],
                    ..Default::default()
                }
            } else {
                StateDiff {
                    deployed_contracts: vec![DeployedContractItem {
                        address: Felt::from(100 + block_number),
                        class_hash: Felt::ONE,
                    }],
                    ..Default::default()
                }
            };
            backend
                .write_access()
                .add_full_block_with_classes(
                    &FullBlockWithoutCommitments {
                        header: PreconfirmedHeader { block_number, ..Default::default() },
                        state_diff,
                        transactions: vec![],
                        events: vec![],
                    },
                    &[],
                    false,
                )
                .unwrap();
        }
        backend.write_parallel_merkle_checkpoint(0).unwrap();
        assert_eq!(backend.db.inner.bonsai_log_floor(trie::BONSAI_CLASS_LOG_COLUMN, 2).unwrap(), Some(2));
        assert_eq!(backend.db.inner.bonsai_log_floor(trie::BONSAI_CONTRACT_LOG_COLUMN, 2).unwrap(), None);
        let target = backend.db.get_block_info(2).unwrap().unwrap();
        backend.revert_to(&target.block_hash).unwrap();
        target.header.global_state_root
    };
    let backend = open_backend(dir.path(), true);
    assert_eq!(backend.latest_confirmed_block_n(), Some(2));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), expected_root);
}
