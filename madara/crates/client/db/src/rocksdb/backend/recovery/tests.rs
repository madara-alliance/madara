use super::*;
use crate::{MadaraBackend, MadaraBackendConfig};
use mc_class_exec::config::NativeConfig;
use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
use mp_chain_config::ChainConfig;
use mp_state_update::{ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, StorageEntry};

fn open_backend(path: &Path, wal: bool, retention: usize) -> Arc<MadaraBackend> {
    MadaraBackend::open_rocksdb(
        path,
        Arc::new(ChainConfig::madara_test()),
        MadaraBackendConfig::default(),
        RocksDBConfig {
            max_saved_trie_logs: Some(retention),
            write_mode: DbWriteMode { wal, fsync: false },
            ..Default::default()
        },
        Arc::new(NativeConfig::default()),
    )
    .unwrap()
}

/// Confirm through block 6, but leave the materialized tries at checkpoint 5.
/// Class state can be idle since genesis or include a declaration after checkpoint 3.
fn recovery_chain(backend: &Arc<MadaraBackend>, class_update: bool) -> Felt {
    for block_number in 0..=6 {
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
                        declared_classes: if block_number == 0 || (class_update && block_number == 4) {
                            vec![DeclaredClassItem {
                                class_hash: Felt::from(200 + block_number),
                                compiled_class_hash: Felt::from(300 + block_number),
                            }]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    transactions: vec![],
                    events: vec![],
                },
                &[],
                false,
            )
            .unwrap();
        if [0, 3, 5].contains(&block_number) {
            backend.write_parallel_merkle_checkpoint(block_number).unwrap();
        }
    }
    let expected = backend.db.get_state_root_hash().unwrap();
    backend.db.rollback_tries_to_checkpoint_floor(Some(5), "recovery_fixture").unwrap();
    expected
}

fn rollback_classes(backend: &MadaraBackend) {
    let mut classes = backend.db.class_trie_for_revert();
    classes.revert_to(BasicId::new(3), BasicId::new(4)).unwrap();
    classes.commit(BasicId::new(3)).unwrap();
}

#[rstest::rstest]
fn recovery_uses_recent_checkpoint_when_class_trie_is_unchanged(#[values(true, false)] wal: bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let expected = {
        let backend = open_backend(dir.path(), wal, 32);
        let expected = recovery_chain(&backend, false);
        assert_eq!(backend.db.trie_log_heads().unwrap().class, Some(0));
        let recovery = backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
        let (floor, _) = backend.db.select_verified_recovery_floor(6, recovery, "idle_class").unwrap();
        assert_eq!(floor, Some(5), "idle class must not cause replay from genesis");
        backend.reconcile_confirmed_parallel_merkle_state("idle_class").unwrap();
        assert_eq!(backend.db.trie_log_heads().unwrap().class, Some(0), "no synthetic class commits");
        expected
    };
    let backend = open_backend(dir.path(), wal, 32);
    assert_eq!(backend.db.get_state_root_hash().unwrap(), expected);
    assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(6));
    assert_eq!(backend.db.inner.get_confirmed_trie_recovery().unwrap(), None);
}

#[rstest::rstest]
fn recovery_falls_back_when_class_trie_was_partially_rolled_back(#[values(true, false)] wal: bool) {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), wal, 32);
    let expected = recovery_chain(&backend, true);
    let recovery = backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
    rollback_classes(&backend);
    let (floor, _) = backend.db.select_verified_recovery_floor(6, recovery, "partial_class").unwrap();
    assert_eq!(floor, Some(3), "checkpoint 5 has a missing class declaration");
    backend.reconcile_confirmed_parallel_merkle_state("partial_class").unwrap();
    assert_eq!(backend.db.get_state_root_hash().unwrap(), expected);
    assert_eq!(backend.db.inner.get_confirmed_trie_recovery().unwrap(), None);
}

#[test]
fn failed_recovery_keeps_checkpoints_and_durable_intent() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), true, 32);
    recovery_chain(&backend, true);
    let recovery = backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
    // Unlogged corruption cannot be repaired from any existing checkpoint.
    backend.db.clear_global_trie_columns().unwrap();
    let error = backend.db.select_verified_recovery_floor(6, recovery, "corrupt_base").unwrap_err();
    assert!(format!("{error:#}").contains("No verified checkpoint remains"));
    for floor in [0, 3, 5] {
        assert!(backend.db.has_parallel_merkle_checkpoint(floor).unwrap());
    }
    assert_eq!(backend.db.inner.get_confirmed_trie_recovery().unwrap(), Some(recovery));
    assert_eq!(backend.latest_confirmed_block_n(), Some(6));
}

#[test]
fn recovery_does_not_hide_missing_checkpoint_headers_as_root_mismatches() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), true, 32);
    recovery_chain(&backend, false);
    let recovery = backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
    let before = backend.db.get_state_root_hash().unwrap();
    backend.db.inner.remove_all_blocks_starting_from(5).unwrap();
    let error = backend.db.select_verified_recovery_floor(6, recovery, "missing_header").unwrap_err();
    assert!(format!("{error:#}").contains("Missing block info"));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), before);
    assert!(backend.db.has_parallel_merkle_checkpoint(5).unwrap());
}

#[test]
fn recovery_rejects_fallback_beyond_original_log_retention() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), true, 2);
    recovery_chain(&backend, false);
    let recovery = backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
    // Checkpoint 5 becomes unavailable: an older migration checkpoint must not
    // bypass the original retention window just because rollback lowered log heads.
    backend.db.rewind_parallel_merkle_checkpoints(Some(3)).unwrap();
    let before = backend.db.get_state_root_hash().unwrap();
    let error = backend.db.select_verified_recovery_floor(6, recovery, "pruned_floor").unwrap_err();
    assert!(format!("{error:#}").contains("predates first retained trie-log revision"));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), before);
}

#[test]
fn replay_root_mismatch_does_not_publish_overlay_or_checkpoint() {
    let dir = tempfile::TempDir::new().unwrap();
    let backend = open_backend(dir.path(), true, 32);
    recovery_chain(&backend, false);
    backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
    // Starting from the wrong base computes a bad root, which must never reach RocksDB.
    backend.db.rollback_tries_to_checkpoint_floor(Some(3), "wrong_replay_base").unwrap();
    let before = backend.db.get_state_root_hash().unwrap();
    let error = backend.db.replay_confirmed_trie(6, 6).unwrap_err();
    assert!(format!("{error:#}").contains("replay root mismatch"));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), before);
    assert!(!backend.db.has_parallel_merkle_checkpoint(6).unwrap());
}

/// Run in a separate process so exit skips RocksDB/backend destructors.
#[test]
fn confirmed_recovery_crash_child() {
    let Ok(path) = std::env::var("MADARA_TEST_RECOVERY_PATH") else { return };
    let phase = std::env::var("MADARA_TEST_RECOVERY_PHASE").unwrap();
    let wal = std::env::var("MADARA_TEST_RECOVERY_WAL").unwrap() == "true";
    let backend = open_backend(Path::new(&path), wal, 3);
    recovery_chain(&backend, true);
    let recovery = backend.db.begin_confirmed_trie_recovery(Some(6)).unwrap();
    if phase != "intent" {
        rollback_classes(&backend);
    }
    if ["selected", "mid_replay", "replayed", "unflushed", "completed"].contains(&phase.as_str()) {
        let (floor, _) = backend.db.select_verified_recovery_floor(6, recovery, "crash_child").unwrap();
        assert_eq!(floor, Some(3));
    }
    if phase == "mid_replay" {
        backend.db.replay_confirmed_trie(4, 5).unwrap();
    } else if phase == "replayed" || phase == "unflushed" {
        // Checkpoint 3 has fallen outside retention by block 6. The persisted
        // checkpoint must allow restart without returning to the original floor.
        backend.db.replay_confirmed_trie(4, 6).unwrap();
    } else if phase == "completed" {
        backend.db.replay_confirmed_trie(4, 6).unwrap();
        backend.db.finalize_confirmed_reconcile(6).unwrap();
    }
    if phase != "unflushed" {
        backend.flush().unwrap();
    }
    std::process::exit(73);
}

#[rstest::rstest]
fn restart_resumes_confirmed_recovery_after_process_exit(
    #[values("intent", "partial", "selected", "mid_replay", "replayed", "unflushed", "completed")] phase: &str,
    #[values(true, false)] wal: bool,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "rocksdb::backend::recovery::tests::confirmed_recovery_crash_child", "--nocapture"])
        .env("MADARA_TEST_RECOVERY_PATH", dir.path())
        .env("MADARA_TEST_RECOVERY_PHASE", phase)
        .env("MADARA_TEST_RECOVERY_WAL", wal.to_string())
        .output()
        .unwrap();
    assert_eq!(child.status.code(), Some(73), "child failed: {}", String::from_utf8_lossy(&child.stderr));
    for _ in 0..2 {
        let backend = open_backend(dir.path(), wal, 3);
        assert_eq!(backend.latest_confirmed_block_n(), Some(6));
        let expected = backend.db.get_block_info(6).unwrap().unwrap().header.global_state_root;
        assert_eq!(backend.db.get_state_root_hash().unwrap(), expected);
        assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(6));
        assert_eq!(backend.get_parallel_merkle_latest_checkpoint().unwrap(), Some(6));
        assert_eq!(backend.db.inner.get_confirmed_trie_recovery().unwrap(), None);
    }
}

#[rstest::rstest]
fn restart_resumes_long_replay_after_original_floor_logs_are_pruned(
    #[values(true, false)] wal: bool,
    #[values(true, false)] reorg: bool,
    #[values(true, false)] finish_inner: bool,
) {
    use crate::preconfirmed::PreconfirmedBlock;
    let dir = tempfile::TempDir::new().unwrap();
    let expected = {
        let backend = open_backend(dir.path(), wal, 3);
        recovery_chain(&backend, false);
        let mut diffs = vec![backend.db.get_block_state_diff(6).unwrap().unwrap()];
        let mut expected = Felt::ZERO;
        // Confirm a suffix without materializing it, exactly as the parallel finalizer does.
        for block_n in 7..=10 {
            let diff = StateDiff {
                deployed_contracts: vec![DeployedContractItem {
                    address: Felt::from(100 + block_n),
                    class_hash: Felt::ONE,
                }],
                ..Default::default()
            };
            diffs.push(diff.clone());
            let header = PreconfirmedHeader { block_number: block_n, ..Default::default() };
            let snapshot = Arc::new(SnapshotWithDBArc::new(Arc::clone(&backend.db.inner)));
            let cumulative = global_trie::in_memory::squash_state_diffs(&diffs);
            let computed = backend
                .db
                .compute_root_from_selected_snapshot(
                    Some(5),
                    snapshot,
                    block_n,
                    &cumulative,
                    header.protocol_version,
                    false,
                    false,
                )
                .unwrap();
            expected = computed.state_root;
            backend.write_access().new_preconfirmed(PreconfirmedBlock::new(header)).unwrap();
            backend
                .write_access()
                .write_preconfirmed_with_precomputed_root(false, block_n, diff, computed.state_root, computed.timings)
                .unwrap();
            backend.write_access().new_confirmed_block(block_n).unwrap();
        }
        if reorg {
            // Reorg recovery has already verified floor 5 before starting reconciliation.
            backend.db.inner.write_reorg_recovery_floor(Some(5)).unwrap();
        }
        let recovery = backend.db.begin_confirmed_trie_recovery(Some(10)).unwrap();
        assert_eq!(backend.db.select_verified_recovery_floor(10, recovery, "long_replay").unwrap().0, Some(5));
        backend.db.replay_confirmed_trie(6, 9).unwrap();
        assert_eq!(backend.get_parallel_merkle_latest_checkpoint().unwrap(), Some(9));
        assert_eq!(backend.db.inner.bonsai_log_floor(trie::BONSAI_CONTRACT_LOG_COLUMN, 6).unwrap(), None);
        assert_ne!(backend.db.get_state_root_hash().unwrap(), expected);
        if finish_inner {
            backend.reconcile_confirmed_parallel_merkle_state("finish_inner_recovery").unwrap();
            assert_eq!(backend.db.inner.get_confirmed_trie_recovery().unwrap().is_some(), reorg);
        }
        backend.flush().unwrap();
        expected
    };
    for _ in 0..2 {
        let backend = open_backend(dir.path(), wal, 3);
        assert_eq!(backend.db.get_state_root_hash().unwrap(), expected);
        assert_eq!(backend.latest_confirmed_block_n(), Some(10));
        assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(10));
        assert_eq!(backend.db.inner.get_confirmed_trie_recovery().unwrap(), None);
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
    }
}
