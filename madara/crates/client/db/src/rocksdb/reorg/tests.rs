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

/// Two immutable maps survive canonical rewinds: one archive-only, one leased on disk.
fn historical_datasets() -> [blockifier::execution::syscalls::committed_data::CommittedDataSet; 2] {
    use blockifier::execution::syscalls::committed_data::CommittedDataSet;
    [10u64, 20].map(|value| {
        CommittedDataSet::new((0..8u64).map(|key| (Felt::from(key), Felt::from(value + key))).collect()).unwrap()
    })
}

/// Separate process entry point. SIGKILL skips backend Drop, shutdown flush and service cleanup.
#[cfg(unix)]
#[test]
fn reorg_crash_child() {
    let Ok(path) = std::env::var("MADARA_REORG_CRASH_PATH") else { return };
    let phase = std::env::var("MADARA_REORG_CRASH_PHASE").unwrap();
    let wal = std::env::var("MADARA_REORG_CRASH_WAL").unwrap() == "true";
    let backend = open_backend(Path::new(&path), wal);
    fill_chain(&backend);
    if std::env::var("MADARA_REORG_CRASH_MIGRATED").unwrap() == "true" {
        backend.write_parallel_merkle_checkpoint(0).unwrap();
    }
    let datasets = historical_datasets();
    for dataset in &datasets {
        backend.db.write_committed_data_dataset(dataset, Some(0)).unwrap();
    }
    while backend.db.maintain_committed_data_history(Some(u64::MAX)).unwrap() {}
    assert!(backend.db.get_committed_data_witness(datasets[1].root(), Felt::ZERO).unwrap().unwrap().verify());
    let target = backend.db.get_block_info(2).unwrap().unwrap();
    let context = backend.db.prepare_reorg(&target.block_hash).unwrap();
    match phase.as_str() {
        "intent" | "one_trie" => {
            backend.db.begin_reorg_recovery(2).unwrap();
            if phase == "one_trie" {
                let mut contracts = backend.db.contract_trie_for_revert();
                contracts.revert_to(BasicId::new(2), BasicId::new(3)).unwrap();
                contracts.commit(BasicId::new(2)).unwrap();
            }
        }
        "all_tries" | "head" | "replayed" => {
            backend.db.revert_tries(&context).unwrap();
            backend.db.verify_reorg_target_root(&context).unwrap();
            if phase == "head" {
                backend.db.commit_reorg_head(2, &[], None).unwrap();
            } else if phase == "replayed" {
                backend.db.inner.write_parallel_merkle_checkpoint(2).unwrap();
                backend.db.reconcile_confirmed_parallel_merkle_state(Some(3), "killed_reorg_recovery").unwrap();
            }
        }
        _ => panic!("Unknown crash phase"),
    }
    assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), Some(2));
    if std::env::var("MADARA_REORG_CRASH_FLUSH").unwrap() == "true" {
        backend.flush().unwrap();
    }
    // SAFETY: this test-only child intentionally terminates itself at the selected boundary.
    unsafe { libc::raise(libc::SIGKILL) };
    unreachable!("SIGKILL must terminate the child");
}

/// Kill before/after each durable transition, with and without a final flush. Startup must
/// select one complete canonical head, retain historical maps, and permit another revert/branch.
#[cfg(unix)]
#[rstest::rstest]
fn full_node_restart_repairs_interrupted_reorg(
    #[values("intent", "one_trie", "all_tries", "head", "replayed")] phase: &str,
    #[values(true, false)] wal: bool,
    #[values(false, true)] migrated: bool,
    #[values(false, true)] flush_phase: bool,
) {
    use crate::rocksdb::committed_data::{metadata_key, persistent_id, METADATA_LEVEL};
    use blockifier::execution::syscalls::committed_data::CommittedDataBatchProof;
    use std::os::unix::process::ExitStatusExt;
    let dir = tempfile::TempDir::new().unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "rocksdb::reorg::tests::reorg_crash_child", "--nocapture"])
        .env("MADARA_REORG_CRASH_PATH", dir.path())
        .env("MADARA_REORG_CRASH_PHASE", phase)
        .env("MADARA_REORG_CRASH_WAL", wal.to_string())
        .env("MADARA_REORG_CRASH_MIGRATED", migrated.to_string())
        .env("MADARA_REORG_CRASH_FLUSH", flush_phase.to_string())
        .output()
        .unwrap();
    assert_eq!(child.status.signal(), Some(libc::SIGKILL), "{}", String::from_utf8_lossy(&child.stderr));
    let datasets = historical_datasets();
    for restart in 0..2 {
        let backend = open_backend(dir.path(), wal);
        let tip = backend.latest_confirmed_block_n().unwrap();
        if phase == "head" && !wal && !flush_phase {
            // Unsynced WAL-disabled metadata may keep either head, never a mixed state.
            assert!([2, 3].contains(&tip));
        } else {
            assert_eq!(tip, if phase == "head" { 2 } else { 3 });
        }
        assert_eq!(backend.get_latest_applied_trie_update().unwrap(), Some(tip));
        assert_eq!(
            backend.db.get_state_root_hash().unwrap(),
            backend.db.get_block_info(tip).unwrap().unwrap().header.global_state_root
        );
        assert_eq!(backend.db.inner.get_reorg_recovery_floor().unwrap(), None);
        if tip == 2 {
            assert!(backend.db.get_block_info(3).unwrap().is_none());
        }
        if restart == 0 {
            let cf = backend.db.inner.get_column(meta::META_COLUMN);
            for (index, dataset) in datasets.iter().enumerate() {
                let bytes =
                    backend.db.inner.db.get_cf(&cf, metadata_key(dataset.root(), METADATA_LEVEL, 0)).unwrap().unwrap();
                assert_eq!(persistent_id(&bytes).unwrap() == 0, index == 0, "Reorg must not rewrite proof indexes");
            }
        }
        for dataset in &datasets {
            let keys = [Felt::ZERO, Felt::ONE, Felt::ZERO, Felt::from(7)];
            assert_eq!(
                backend.db.get_committed_data_batch(dataset.root(), &keys).unwrap(),
                CommittedDataBatchProof::from_dataset(dataset, &keys).unwrap()
            );
        }
    }
    let backend = open_backend(dir.path(), wal);
    let target = backend.db.get_block_info(2).unwrap().unwrap();
    backend.revert_to(&target.block_hash).unwrap();
    backend.revert_to(&target.block_hash).unwrap(); // Retrying the operator action is harmless.
    backend
        .write_access()
        .add_full_block_with_classes(
            &FullBlockWithoutCommitments {
                header: PreconfirmedHeader { block_number: 3, ..Default::default() },
                state_diff: StateDiff {
                    deployed_contracts: vec![DeployedContractItem { address: Felt::from(999), class_hash: Felt::ONE }],
                    ..Default::default()
                },
                transactions: vec![],
                events: vec![],
            },
            &[],
            false,
        )
        .unwrap();
    backend.flush().unwrap();
    let replacement = backend.db.get_state_root_hash().unwrap();
    drop(backend);
    let backend = open_backend(dir.path(), wal);
    assert_eq!(backend.latest_confirmed_block_n(), Some(3));
    assert_eq!(backend.db.get_state_root_hash().unwrap(), replacement);
    for dataset in &datasets {
        assert!(backend.db.get_committed_data_witness(dataset.root(), Felt::ZERO).unwrap().unwrap().verify());
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

#[rstest::rstest]
fn reorg_rejects_pruned_migration_floor_before_mutating_tries(#[values(0, 3)] first_checkpoint: u64) {
    let dir = tempfile::TempDir::new().unwrap();
    let expected_root = {
        let backend = open_backend(dir.path(), true);
        fill_chain(&backend);
        // A checkpoint label does not make pruned inverse logs available again.
        backend.write_parallel_merkle_checkpoint(first_checkpoint).unwrap();
        let expected_root = backend.db.get_state_root_hash().unwrap();
        let target = backend.db.get_block_info(0).unwrap().unwrap();

        let error = backend.revert_to(&target.block_hash).unwrap_err();
        let expected = if first_checkpoint == 0 {
            "checkpoint floor 0 predates first retained trie-log revision 2"
        } else {
            "No retained trie revision or checkpoint at/before revert target 0"
        };
        assert!(format!("{error:#}").contains(expected), "{error:#}");
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
fn recent_reorg_uses_retained_serial_revision_after_mode_switch(
    #[values(true, false)] wal: bool,
    #[values(0, 3)] first_checkpoint: u64,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let expected_root = {
        let backend = open_backend(dir.path(), wal);
        fill_chain(&backend);
        // A migrated database can have an early checkpoint; a serial full node promoted
        // to sequencer can start checkpointing strictly after the requested revert target.
        backend.write_parallel_merkle_checkpoint(first_checkpoint).unwrap();
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
