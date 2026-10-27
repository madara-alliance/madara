use super::*;
use crate::persistent_committed_data::{node_key, Tree};
use crate::{MadaraBackend, MadaraBackendConfig, MadaraStorageRead, MadaraStorageWrite};
use blockifier::execution::syscalls::committed_data::CommittedDataBatchProof;

fn open(path: &std::path::Path) -> Arc<MadaraBackend> {
    MadaraBackend::open_rocksdb(
        path,
        mp_chain_config::ChainConfig::madara_test().into(),
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        Default::default(),
    )
    .unwrap()
}
fn pointer(db: &RocksDBStorage, root: Felt) -> u64 {
    persistent_id(
        &db.inner
            .db
            .get_cf(&db.inner.get_column(meta::META_COLUMN), metadata_key(root, METADATA_LEVEL, 0))
            .unwrap()
            .unwrap(),
    )
    .unwrap()
}
type StoredRow = (Box<[u8]>, Box<[u8]>);

fn rows(db: &RocksDBStorage, prefix: &[u8]) -> Vec<StoredRow> {
    db.inner
        .db
        .iterator_cf(&db.inner.get_column(meta::META_COLUMN), IteratorMode::From(prefix, rocksdb::Direction::Forward))
        .map(|row| row.unwrap())
        .take_while(|(key, _)| key.starts_with(prefix))
        .collect()
}
fn drain(db: &RocksDBStorage, cutoff: Option<u64>) {
    for _ in 0..1000 {
        if !db.maintain_history(cutoff).unwrap() {
            return;
        }
    }
    panic!("Maintenance did not converge");
}
fn dataset(value: u64) -> CommittedDataSet {
    CommittedDataSet::new(
        (0..8u64).map(|key| (Felt::from(key), Felt::from(if key == 0 { value } else { key }))).collect(),
    )
    .unwrap()
}
fn assert_dataset(db: &RocksDBStorage, expected: &CommittedDataSet) {
    let root = expected.root();
    assert_eq!(db.get_committed_data_page(root, 0).unwrap().unwrap(), (8, expected.entries().to_vec()));
    for (key, value) in expected.entries() {
        let witness = db.get_committed_data_witness(root, *key).unwrap().unwrap();
        assert_eq!(witness.value, *value);
        assert!(witness.verify());
    }
    assert!(db.get_committed_data_witness(root, Felt::from(99)).unwrap().is_none());
    let keys = [Felt::ONE, Felt::ZERO, Felt::ONE, Felt::from(7)];
    assert_eq!(
        db.get_committed_data_batch(root, &keys).unwrap(),
        CommittedDataBatchProof::from_dataset(expected, &keys).unwrap()
    );
}

#[test]
fn committed_history_prunes_only_old_references_and_reopens_cold_values() {
    let dir = tempfile::tempdir().unwrap();
    let first = dataset(10);
    let second = dataset(20);
    {
        let backend = open(dir.path());
        let db = &backend.db;
        db.write_committed_data_dataset(&first, Some(10)).unwrap(); // last observed block 11
        db.patch_committed_data_dataset(first.root(), second.root(), vec![(Felt::ZERO, Felt::from(20))], Some(20))
            .unwrap();
        drain(db, None);
        let before = rows(db, b"cdm/n/").len();
        assert!(!db.maintain_history(Some(11)).unwrap(), "cutoff is exclusive");
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&db.inner));
        let old_id = pointer(db, first.root());
        assert!(db.maintain_history(Some(12)).unwrap());
        let cf = db.inner.get_column(meta::META_COLUMN);
        let mut concurrent =
            Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |id| Ok(snapshot.get_cf(&cf, node_key(id))?));
        assert!(concurrent.witness(old_id, first.root(), Felt::ZERO).unwrap().unwrap().verify());
        drop(concurrent);
        drop(snapshot);
        assert_eq!(pointer(db, first.root()), 0);
        assert_ne!(pointer(db, second.root()), 0);
        let after = rows(db, b"cdm/n/").len();
        assert!(after < before);
        assert_eq!(after, 15, "Exactly the surviving eight-leaf tree remains");
        for _ in 0..3 {
            assert_dataset(db, &first);
            assert_eq!(pointer(db, first.root()), 0, "Historical reads keep the root cold");
            assert_eq!(rows(db, b"cdm/n/").len(), after, "Rebuilds never persist nodes for a later sweep");
            assert!(!db.maintain_history(Some(12)).unwrap(), "A rebuild creates no new pruning work");
        }
        assert_dataset(db, &second);
        assert_eq!(rows(db, b"cdm/n/").len(), after, "Cold reads do not repopulate proof nodes");
        // Retention follows execution height, including an old root used in a new block.
        backend.committed_data_provider_at(100).value(second.root(), Felt::ZERO).unwrap();
        assert!(!db.maintain_history(Some(100)).unwrap());
        assert!(db.maintain_history(Some(101)).unwrap());
        assert!(rows(db, b"cdm/n/").is_empty());
        assert!(rows(db, b"cdh/ref/").is_empty());
    }
    let backend = open(dir.path());
    let db = &backend.db;
    assert_dataset(db, &first);
    assert_dataset(db, &second);
    let fork = dataset(30);
    db.patch_committed_data_dataset(first.root(), fork.root(), vec![(Felt::ZERO, Felt::from(30))], Some(200)).unwrap();
    assert_dataset(db, &fork);
    // Identical retries must not leak references or advance the allocator.
    let before = rows(db, b"cdh/ref/");
    db.patch_committed_data_dataset(first.root(), fork.root(), vec![(Felt::ZERO, Felt::from(30))], Some(200)).unwrap();
    assert_eq!(rows(db, b"cdh/ref/"), before);
    // An explicit full re-import restores its index and makes it eligible for later pruning.
    db.write_committed_data_dataset(&first, Some(10)).unwrap();
    assert_ne!(pointer(db, first.root()), 0);
    drain(db, Some(202));
    assert!(rows(db, b"cdm/n/").is_empty());
    assert_dataset(db, &fork);
}

#[test]
fn committed_history_batched_reclamation_preserves_shared_nodes_and_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let backend = open(dir.path());
    let db = &backend.db;
    // A complete 2048-leaf tree crosses the reclamation read-batch boundary.
    let values: Vec<_> = (0..2048u64).map(|key| (Felt::from(key), Felt::ONE)).collect();
    let first = CommittedDataSet::new(values.clone()).unwrap();
    let second = CommittedDataSet::new(values.iter().map(|(key, _)| (*key, Felt::TWO)).collect()).unwrap();
    let mut fork_values = second.entries().to_vec();
    fork_values[0].1 = Felt::from(3);
    let fork = CommittedDataSet::new(fork_values).unwrap();
    db.write_committed_data_dataset(&first, Some(0)).unwrap();
    db.patch_committed_data_dataset(first.root(), second.root(), second.entries().to_vec(), Some(1)).unwrap();
    db.patch_committed_data_dataset(second.root(), fork.root(), vec![(Felt::ZERO, Felt::from(3))], Some(2)).unwrap();
    let cf = db.inner.get_column(meta::META_COLUMN);
    let (key, original) = rows(db, b"cdh/ref/").remove(0);
    // A corrupt leaf is reached after branches have already been staged for removal.
    db.inner.db.put_cf(&cf, &key, [0u8; 24]).unwrap();
    let nodes = rows(db, b"cdm/n/");
    let references = rows(db, b"cdh/ref/");
    assert!(db.maintain_history(Some(3)).is_err());
    assert_eq!(rows(db, b"cdm/n/"), nodes);
    assert_eq!(rows(db, b"cdh/ref/"), references);
    assert_ne!(pointer(db, first.root()), 0);
    db.inner.db.put_cf(&cf, key, original).unwrap();
    drain(db, Some(3));
    assert_eq!(rows(db, b"cdm/n/").len(), 4095, "Only the shared fork tree survives");
    for tree in [&first, &second, &fork] {
        let keys = [Felt::ZERO, Felt::ONE, Felt::from(2047)];
        assert_eq!(
            db.get_committed_data_batch(tree.root(), &keys).unwrap(),
            CommittedDataBatchProof::from_dataset(tree, &keys).unwrap()
        );
    }
    drain(db, Some(4));
    assert!(rows(db, b"cdm/n/").is_empty());
    assert!(rows(db, b"cdh/ref/").is_empty());
}

#[test]
fn committed_history_checkpoints_bound_replay_without_expiring_values() {
    let dir = tempfile::tempdir().unwrap();
    let backend = open(dir.path());
    let db = &backend.db;
    let mut versions = vec![dataset(10)];
    db.write_committed_data_dataset(&versions[0], None).unwrap();
    for tick in 1..=70u64 {
        let next = dataset(tick + 10);
        db.patch_committed_data_dataset(
            versions.last().unwrap().root(),
            next.root(),
            vec![(Felt::ZERO, Felt::from(tick + 10))],
            Some(tick),
        )
        .unwrap();
        versions.push(next);
    }
    let history = |root| -> ValueHistory {
        deserialize(
            db.inner.db.get_cf(&db.inner.get_column(meta::META_COLUMN), root_key(HISTORY, root)).unwrap().unwrap(),
        )
        .unwrap()
    };
    assert_eq!(history(versions[64].root()).depth, 64);
    assert_eq!(history(versions[65].root()).depth, 0);
    drain(db, Some(u64::MAX));
    assert!(rows(db, b"cdm/n/").is_empty());
    assert!(rows(db, b"cdh/ref/").is_empty());
    assert_eq!(rows(db, HISTORY).len(), versions.len());
    for expected in &versions {
        assert_dataset(db, expected);
    }
}

#[test]
fn committed_history_archive_corruption_blocks_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let datasets = [dataset(10), dataset(20), dataset(30)];
    let backend = open(dir.path());
    let db = &backend.db;
    for tree in &datasets {
        db.write_committed_data_dataset(tree, Some(1)).unwrap();
    }
    let cf = db.inner.get_column(meta::META_COLUMN);
    // The lexically first root is the first candidate: every root has the same age.
    let root = datasets.iter().map(CommittedDataSet::root).min().unwrap();
    let key = root_key(HISTORY, root);
    let original = db.inner.db.get_cf(&cf, &key).unwrap().unwrap();
    let mut history: ValueHistory = deserialize(&original).unwrap();
    history.data[0] ^= 1;
    db.inner.db.put_cf(&cf, &key, serialize(&history).unwrap()).unwrap();
    let nodes = rows(db, b"cdm/n/");
    assert!(db.maintain_history(Some(100)).is_err());
    assert_eq!(rows(db, b"cdm/n/"), nodes);
    assert_ne!(pointer(db, root), 0);
    db.inner.db.put_cf(&cf, key, original).unwrap();
    drain(db, Some(100));
    for tree in &datasets {
        assert_dataset(db, tree);
    }
    // Cold requests fail closed on missing/corrupt archives too.
    db.inner.db.delete_cf(&cf, root_key(HISTORY, root)).unwrap();
    assert!(db.get_committed_data_witness(root, Felt::ZERO).is_err());
    assert!(db.get_committed_data_page(root, 0).is_err());
    assert!(db.get_committed_data_batch(root, &[Felt::ZERO]).is_err());
}

#[test]
fn committed_history_no_settlement_stalls_and_boundary_controls_pruning() {
    let dir = tempfile::tempdir().unwrap();
    let mut chain = mp_chain_config::ChainConfig::madara_test();
    chain.block_time = std::time::Duration::from_secs(5);
    let config = MadaraBackendConfig {
        committed_data_retention: crate::committed_data_retention::CommittedDataRetentionConfig {
            proof_retention_secs: Some(10),
            ..Default::default()
        },
        ..Default::default()
    };
    let backend =
        MadaraBackend::open_rocksdb(dir.path(), chain.into(), config, RocksDBConfig::default(), Default::default())
            .unwrap();
    let tree = dataset(10);
    backend.db.write_committed_data_dataset(&tree, Some(97)).unwrap();
    backend.chain_head_state.send_modify(|state| state.confirmed_tip = Some(100));
    assert!(!backend.maintain_committed_data_history().unwrap());
    backend.set_latest_l1_confirmed(Some(100)).unwrap();
    assert!(!backend.maintain_committed_data_history().unwrap()); // cutoff 98 == last use
    backend.set_latest_l1_confirmed(Some(200)).unwrap();
    assert!(!backend.maintain_committed_data_history().unwrap()); // clamp malformed future tip
    backend.set_latest_l1_confirmed(Some(100)).unwrap();
    backend.chain_head_state.send_modify(|state| state.confirmed_tip = Some(101));
    assert!(!backend.maintain_committed_data_history().unwrap()); // stalled L1
    backend.set_latest_l1_confirmed(Some(101)).unwrap();
    assert!(backend.maintain_committed_data_history().unwrap());
    assert_eq!(pointer(&backend.db, tree.root()), 0);
    // An old execution remains cold; recent execution reactivates the index once.
    assert_eq!(
        backend.committed_data_provider_at(98).value(tree.root(), Felt::ZERO).unwrap().unwrap().0,
        Felt::from(10)
    );
    assert_eq!(pointer(&backend.db, tree.root()), 0);
    assert_eq!(
        backend.committed_data_provider_at(102).value(tree.root(), Felt::ZERO).unwrap().unwrap().0,
        Felt::from(10)
    );
    assert_ne!(pointer(&backend.db, tree.root()), 0);
    assert!(!backend.maintain_committed_data_history().unwrap());
    backend.set_latest_l1_confirmed(Some(90)).unwrap(); // regression cannot remove values
    assert_dataset(&backend.db, &tree);
}

#[test]
fn committed_history_concurrent_imports_pruning_and_reads_preserve_roots() {
    let dir = tempfile::tempdir().unwrap();
    let backend = open(dir.path());
    drain(&backend.db, None);
    let first = dataset(10);
    backend.db.write_committed_data_dataset(&first, None).unwrap();
    let start = std::sync::Barrier::new(2);
    let done = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let pruner = scope.spawn(|| {
            start.wait();
            while !done.load(std::sync::atomic::Ordering::Acquire) {
                backend.db.maintain_history(Some(u64::MAX)).unwrap();
                std::thread::yield_now();
            }
        });
        let writer = scope.spawn(|| {
            start.wait();
            let mut root = first.root();
            for tick in 11..30u64 {
                let next = dataset(tick);
                backend
                    .db
                    .patch_committed_data_dataset(root, next.root(), vec![(Felt::ZERO, Felt::from(tick))], Some(tick))
                    .unwrap();
                assert_dataset(&backend.db, &next);
                root = next.root();
            }
        });
        let result = writer.join();
        done.store(true, std::sync::atomic::Ordering::Release);
        pruner.join().unwrap();
        result.unwrap();
    });
    drain(&backend.db, Some(u64::MAX));
    assert!(rows(&backend.db, b"cdm/n/").is_empty());
    for value in 10..30 {
        assert_dataset(&backend.db, &dataset(value));
    }
}

#[test]
fn committed_history_dictionary_key_additions_forks_and_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let first = dataset(10);
    let mut added = first.entries().to_vec();
    added.push((Felt::MAX, Felt::MAX));
    let second = CommittedDataSet::new(added).unwrap();
    let fork = dataset(999);
    let mut wide_values = second.entries().to_vec();
    wide_values.last_mut().unwrap().1 = Felt::MAX - Felt::ONE;
    let wide = CommittedDataSet::new(wide_values.clone()).unwrap();
    wide_values.last_mut().unwrap().1 = Felt::ZERO;
    let zero = CommittedDataSet::new(wide_values).unwrap();
    {
        let backend = open(dir.path());
        let db = &backend.db;
        db.write_committed_data_dataset(&first, None).unwrap();
        db.patch_committed_data_dataset(first.root(), second.root(), vec![(Felt::MAX, Felt::MAX)], Some(1)).unwrap();
        db.patch_committed_data_dataset(first.root(), fork.root(), vec![(Felt::ZERO, Felt::from(999))], Some(2))
            .unwrap();
        db.patch_committed_data_dataset(second.root(), wide.root(), vec![(Felt::MAX, Felt::MAX - Felt::ONE)], Some(3))
            .unwrap();
        db.patch_committed_data_dataset(wide.root(), zero.root(), vec![(Felt::MAX, Felt::ZERO)], Some(4)).unwrap();
        assert_eq!(rows(db, archive::DICTIONARY).len(), 2, "Fork reuses original dictionary");
        let cf = db.inner.get_column(meta::META_COLUMN);
        let (key, bytes) = rows(db, archive::DICTIONARY).remove(0);
        let mut corrupt = bytes.to_vec();
        corrupt[0] ^= 1;
        db.inner.db.put_cf(&cf, &key, &corrupt).unwrap();
        // Both cold reconstruction and reclamation authenticate dictionary bytes.
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&db.inner));
        assert!([first.root(), second.root()].into_iter().any(|root| reconstruct(&snapshot, root).is_err()));
        drain(db, None);
        let mut rejected = false;
        for _ in 0..3 {
            if db.maintain_history(Some(u64::MAX)).is_err() {
                rejected = true;
                break;
            }
        }
        assert!(rejected);
        db.inner.db.put_cf(&cf, key, bytes).unwrap();
        drain(db, Some(u64::MAX));
    }
    let backend = open(dir.path());
    for expected in [&first, &second, &fork, &wide, &zero] {
        let keys: Vec<_> = expected.entries().iter().map(|(key, _)| *key).collect();
        assert_eq!(
            backend.db.get_committed_data_batch(expected.root(), &keys).unwrap(),
            CommittedDataBatchProof::from_dataset(expected, &keys).unwrap()
        );
        assert_eq!(backend.db.get_committed_data_page(expected.root(), 0).unwrap().unwrap().1, expected.entries());
    }
}

#[test]
fn committed_data_catalog_and_outbox_do_not_wait_for_proof_pruning() {
    use crate::committed_data_recovery::RecoveryRecord;
    use mp_gateway::committed_data::CommittedDataVersion;
    let directory = tempfile::tempdir().unwrap();
    let backend = open(directory.path());
    let tree = dataset(10);
    let root = tree.root();
    backend.db.write_committed_data_dataset(&tree, None).unwrap();
    let record =
        RecoveryRecord::encode(backend.chain_config().chain_id.to_felt(), None, root, None, tree.entries().to_vec())
            .unwrap();
    let version = CommittedDataVersion { application_id: Felt::ONE, dataset_id: Felt::TWO, root };
    std::thread::scope(|scope| {
        let pruning = backend.db.inner.committed_data_write.lock().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let db = &backend.db;
        let writer = scope.spawn(move || {
            let result = (|| -> Result<()> {
                db.write_committed_data_version(version, true)?;
                let id = db.store_recovery_record(&record)?;
                anyhow::ensure!(db.recovery_pending_bytes()? > 0);
                db.mark_recovery_exported(&id)?;
                anyhow::ensure!(db.recovery_pending_bytes()? == 0);
                anyhow::ensure!(db.get_recovery_manifest(&id)?.is_some());
                Ok(())
            })();
            send.send(result).unwrap();
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        // Release before joining so a regression fails instead of deadlocking the test.
        drop(pruning);
        writer.join().unwrap();
        result.expect("Catalog and outbox writes must complete while the proof arena is locked").unwrap();
    });
    assert!(backend.db.has_committed_data_version(version).unwrap());
    assert_dataset(&backend.db, &tree);
}
