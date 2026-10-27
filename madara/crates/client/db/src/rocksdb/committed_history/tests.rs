use super::*;
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
fn rows(db: &RocksDBStorage, prefix: &[u8]) -> Vec<(Box<[u8]>, Box<[u8]>)> {
    db.inner
        .db
        .iterator_cf(&db.inner.get_column(meta::META_COLUMN), IteratorMode::From(prefix, rocksdb::Direction::Forward))
        .map(|row| row.unwrap())
        .take_while(|(key, _)| key.starts_with(prefix))
        .collect()
}
fn drain(db: &RocksDBStorage, cutoff: Option<u64>) {
    for _ in 0..1000 {
        if !db.maintain_history(cutoff, 0).unwrap() {
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
        assert!(!db.maintain_history(Some(11), 0).unwrap(), "cutoff is exclusive");
        let snapshot = SnapshotWithDBArc::new(Arc::clone(&db.inner));
        let old_id = pointer(db, first.root());
        assert!(db.maintain_history(Some(12), 0).unwrap());
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
        assert_dataset(db, &first);
        assert_dataset(db, &second);
        assert_eq!(rows(db, b"cdm/n/").len(), after, "Cold reads do not repopulate proof nodes");
        // Retention follows execution height, including an old root used in a new block.
        backend.committed_data_provider_at(100).value(second.root(), Felt::ZERO).unwrap();
        assert!(!db.maintain_history(Some(100), 0).unwrap());
        assert!(db.maintain_history(Some(101), 0).unwrap());
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
fn committed_history_legacy_migration_resumes_and_corruption_blocks_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let datasets = [dataset(10), dataset(20), dataset(30)];
    {
        let backend = open(dir.path());
        let db = &backend.db;
        for tree in &datasets {
            db.write_committed_data_dataset(tree, Some(1)).unwrap();
        }
        // A pre-retention DB has only node/root records. Preserve those exact records.
        let cf = db.inner.get_column(meta::META_COLUMN);
        let mut batch = WriteBatch::default();
        for (key, _) in rows(db, b"cdh/") {
            batch.delete_cf(&cf, key);
        }
        db.inner.db.write(batch).unwrap();
        assert!(db.maintain_history(Some(100), 0).unwrap());
        assert_eq!(rows(db, REGISTERED).len(), 1);
        assert!(datasets.iter().all(|tree| pointer(db, tree.root()) != 0));
    }
    let backend = open(dir.path());
    let db = &backend.db;
    drain(db, None);
    assert_eq!(rows(db, REGISTERED).len(), 3);
    let cf = db.inner.get_column(meta::META_COLUMN);
    // The lexically first root is the first candidate: every root has the same age.
    let root = datasets.iter().map(CommittedDataSet::root).min().unwrap();
    let key = root_key(HISTORY, root);
    let original = db.inner.db.get_cf(&cf, &key).unwrap().unwrap();
    let mut history: ValueHistory = deserialize(&original).unwrap();
    history.record.chunks[0][0] ^= 1;
    db.inner.db.put_cf(&cf, &key, serialize(&history).unwrap()).unwrap();
    let nodes = rows(db, b"cdm/n/");
    assert!(db.maintain_history(Some(100), 0).is_err());
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
    drain(&backend.db, None); // Finish empty migration before observing newer blocks.
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
