//! Fault boundaries for durable reconstruction: bad clocks, damaged metadata and interrupted IO.
use super::*;
use crate::persistent_committed_data::NEXT_KEY;

/// Read through a held snapshot without refreshing its lease or changing the injected clock.
fn assert_snapshot(db: &RocksDBStorage, snapshot: &SnapshotWithDBArc, expected: &CommittedDataSet) {
    let cf = db.inner.get_column(meta::META_COLUMN);
    let metadata = snapshot.get_cf(&cf, metadata_key(expected.root(), METADATA_LEVEL, 0)).unwrap().unwrap();
    let id = persistent_id(&metadata).unwrap();
    assert_ne!(id, 0);
    let mut tree =
        Tree::new(u64::MAX, MAX_COMMITTED_DATA_VALUES as u32, |node| Ok(snapshot.get_cf(&cf, node_key(node))?));
    for (key, value) in expected.entries() {
        let proof = tree.witness(id, expected.root(), *key).unwrap().unwrap();
        assert_eq!(proof.value, *value);
        assert!(proof.verify());
    }
}

/// A backwards clock must neither shorten protection nor write on every repeated read.
#[test]
fn clock_rollback_and_same_second_reads_do_not_shorten_or_rewrite_lease() {
    let directory = tempfile::tempdir().unwrap();
    let backend = open_with_idle(directory.path(), 100);
    let db = &backend.db;
    let expected = dataset(10);
    db.write_committed_data_dataset(&expected, Some(0)).unwrap();
    drain(db, Some(2));
    db.historical_proof_snapshot_at(expected.root(), 1000).unwrap();
    let before = rows(db, b"");
    let sequence = db.inner.db.latest_sequence_number();
    for now in [0, 980, 1000, 999, 1000] {
        let snapshot = db.historical_proof_snapshot_at(expected.root(), now).unwrap();
        assert_snapshot(db, &snapshot, &expected);
    }
    assert_eq!(rows(db, b""), before);
    assert_eq!(db.inner.db.latest_sequence_number(), sequence);
    db.historical_proof_snapshot_at(expected.root(), 1050).unwrap();
    assert!(!db.maintain_history_at(Some(u64::MAX), 1100).unwrap());
    assert!(db.maintain_history_at(None, 1150).unwrap());
    assert_ne!(pointer(db, expected.root()), 0, "Expiry alone cannot prune without L1 cutoff");
    assert!(db.maintain_history_at(Some(2), 1150).unwrap());
    assert_eq!(pointer(db, expected.root()), 0);
    assert_dataset(db, &expected);
}

/// Changing configuration on restart affects new accesses, not a previously promised deadline.
#[rstest::rstest]
#[case::disable(0, 1100)]
#[case::shorten(10, 1100)]
#[case::extend(200, 1205)]
fn restart_configuration_preserves_existing_lease(#[case] idle: u64, #[case] expires: u64) {
    let directory = tempfile::tempdir().unwrap();
    let expected = dataset(10);
    let original_id = {
        let backend = open_with_idle(directory.path(), 100);
        backend.db.write_committed_data_dataset(&expected, Some(0)).unwrap();
        drain(&backend.db, Some(2));
        backend.db.historical_proof_snapshot_at(expected.root(), 1000).unwrap();
        pointer(&backend.db, expected.root())
    };
    let backend = open_with_idle(directory.path(), idle);
    let db = &backend.db;
    assert_snapshot(db, &db.historical_proof_snapshot_at(expected.root(), 1005).unwrap(), &expected);
    assert_eq!(pointer(db, expected.root()), original_id);
    assert!(!db.maintain_history_at(Some(2), expires - 1).unwrap());
    assert!(db.maintain_history_at(Some(2), expires).unwrap());
    assert!(db.maintain_history_at(Some(2), expires).unwrap());
    assert_eq!(pointer(db, expected.root()), 0);
    assert_dataset(db, &expected);
    assert_eq!(pointer(db, expected.root()) == 0, idle == 0);
}

/// Arithmetic exhaustion is an ordinary error: staging must leave every durable row unchanged.
#[rstest::rstest]
#[case::zero_allocator(Some(0), 1000)]
#[case::exhausted_allocator(Some(u64::MAX), 1000)]
#[case::partially_staged_tree(Some(u64::MAX - 4), 1000)]
#[case::lease_overflow(None, u64::MAX - 10)]
fn reconstruction_overflow_publishes_nothing(#[case] allocator: Option<u64>, #[case] now: u64) {
    let directory = tempfile::tempdir().unwrap();
    let backend = open_with_idle(directory.path(), 100);
    let db = &backend.db;
    let expected = dataset(10);
    db.write_committed_data_dataset(&expected, Some(0)).unwrap();
    drain(db, Some(2));
    let cf = db.inner.get_column(meta::META_COLUMN);
    let original = db.inner.db.get_cf(&cf, NEXT_KEY).unwrap().unwrap();
    if let Some(allocator) = allocator {
        db.inner.db.put_cf(&cf, NEXT_KEY, allocator.to_be_bytes()).unwrap();
    }
    let before = rows(db, b"");
    assert!(db.historical_proof_snapshot_at(expected.root(), now).is_err());
    assert_eq!(rows(db, b""), before);
    db.inner.db.put_cf(&cf, NEXT_KEY, original).unwrap();
    assert_dataset(db, &expected);
}

/// A corrupt allocator can point at a pruned hole before live IDs. Checking only that first
/// slot misses an overlapping allocation later in the same batch and can destroy another root.
#[rstest::rstest]
#[case::live_nodes(false)]
#[case::missing_node_payloads(true)]
fn rewound_allocator_cannot_fill_a_hole_into_another_roots_nodes(#[case] missing_payloads: bool) {
    let directory = tempfile::tempdir().unwrap();
    let backend = open_with_idle(directory.path(), 100);
    let db = &backend.db;
    let historical = dataset(10);
    let retained =
        CommittedDataSet::new((100..108u64).map(|key| (Felt::from(key), Felt::from(key))).collect()).unwrap();
    db.write_committed_data_dataset(&historical, Some(0)).unwrap();
    db.write_committed_data_dataset(&retained, Some(100)).unwrap();
    drain(db, Some(2));
    let cf = db.inner.get_column(meta::META_COLUMN);
    let original = db.inner.db.get_cf(&cf, NEXT_KEY).unwrap().unwrap();
    let retained_nodes = rows(db, b"cdm/n/");
    if missing_payloads {
        for (key, _) in &retained_nodes {
            db.inner.db.delete_cf(&cf, key).unwrap();
        }
    }
    db.inner.db.put_cf(&cf, NEXT_KEY, 8u64.to_be_bytes()).unwrap();
    let before = rows(db, b"");
    assert!(db.historical_proof_snapshot_at(historical.root(), 1000).is_err());
    assert_eq!(rows(db, b""), before, "Failed rebuild must not overwrite another root's ownership");
    db.inner.db.put_cf(&cf, NEXT_KEY, original).unwrap();
    for (key, value) in retained_nodes {
        db.inner.db.put_cf(&cf, key, value).unwrap();
    }
    assert_snapshot(db, &SnapshotWithDBArc::new(Arc::clone(&db.inner)), &retained);
    assert_dataset(db, &historical);
}

/// Corrupt lease scheduling must stop deletion before changing nodes, ownership or values.
#[rstest::rstest]
#[case::missing_lease(0)]
#[case::malformed_lease(1)]
#[case::mismatched_deadline(2)]
#[case::missing_execution_age(3)]
#[case::malformed_expiry_key(4)]
fn corrupt_lease_metadata_stops_pruning_atomically(#[case] damage: u8) {
    let directory = tempfile::tempdir().unwrap();
    let backend = open_with_idle(directory.path(), 100);
    let db = &backend.db;
    let expected = dataset(10);
    db.write_committed_data_dataset(&expected, Some(0)).unwrap();
    drain(db, Some(2));
    db.historical_proof_snapshot_at(expected.root(), 1000).unwrap();
    let cf = db.inner.get_column(meta::META_COLUMN);
    let lease = root_key(leases::LEASE, expected.root());
    let used = root_key(USED, expected.root());
    match damage {
        0 => db.inner.db.delete_cf(&cf, &lease).unwrap(),
        1 => db.inner.db.put_cf(&cf, &lease, [0]).unwrap(),
        2 => db.inner.db.put_cf(&cf, &lease, 1200u64.to_be_bytes()).unwrap(),
        3 => db.inner.db.delete_cf(&cf, &used).unwrap(),
        4 => db.inner.db.put_cf(&cf, b"cdh/expiry/", []).unwrap(),
        _ => unreachable!(),
    }
    let before = rows(db, b"");
    assert!(db.maintain_history_at(Some(u64::MAX), 2000).is_err());
    assert_eq!(rows(db, b""), before);
    assert_snapshot(db, &SnapshotWithDBArc::new(Arc::clone(&db.inner)), &expected);
}

/// Readers race lease expiry, pruning and fork imports. Each returned proof is checked against
/// an independently built map; final reclamation must leave no leaked nodes or ownership rows.
#[test]
fn repeated_rebuild_prune_and_fork_races_preserve_every_historical_root() {
    let directory = tempfile::tempdir().unwrap();
    let backend = open_with_idle(directory.path(), 1);
    let db = &backend.db;
    let expected: Vec<_> = (10..26).map(dataset).collect();
    db.write_committed_data_dataset(&expected[0], Some(0)).unwrap();
    drain(db, Some(2));
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for tree in &expected[1..] {
                db.patch_committed_data_dataset(expected[0].root(), tree.root(), vec![tree.entries()[0]], Some(0))
                    .unwrap();
            }
        });
        scope.spawn(|| {
            for _ in 0..200 {
                db.maintain_history_at(Some(u64::MAX), leases::unix_seconds().unwrap() + 100).unwrap();
            }
        });
        for _ in 0..6 {
            scope.spawn(|| {
                for _ in 0..40 {
                    assert_dataset(db, &expected[0]);
                }
            });
        }
    });
    for tree in &expected {
        assert_dataset(db, tree);
    }
    for _ in 0..100 {
        if !db.maintain_history_at(Some(u64::MAX), leases::unix_seconds().unwrap() + 100).unwrap() {
            break;
        }
    }
    assert!(rows(db, b"cdm/n/").is_empty());
    assert!(rows(db, b"cdh/ref/").is_empty());
    assert!(rows(db, leases::LEASE).is_empty());
    assert_eq!(rows(db, HISTORY).len(), expected.len());
    println!("ADVERSARIAL_RACES elapsed_ms={} readers=6 reads=240 roots=16", started.elapsed().as_millis());
    drop(backend);
    let backend = open_with_idle(directory.path(), 100);
    for tree in &expected {
        assert_dataset(&backend.db, tree);
    }
}

/// Kill a process while it repeatedly publishes and deletes an index, including before an
/// operation acknowledges. Recovery may expose either complete state, never half a tree.
#[test]
fn killed_rebuild_and_prune_cycles_recover_atomic_indexes() {
    use std::io::{BufRead, Write};
    const DIRECTORY: &str = "MADARA_REBUILD_CYCLE_CRASH_DIRECTORY";
    const TEST: &str =
        "rocksdb::committed_history::tests::adversarial::killed_rebuild_and_prune_cycles_recover_atomic_indexes";
    let expected =
        CommittedDataSet::new((0..4097u32).map(|key| (Felt::from(key), Felt::from(key + 1))).collect()).unwrap();
    if let Ok(directory) = std::env::var(DIRECTORY) {
        let backend = open_with_idle(std::path::Path::new(&directory), 1);
        println!("REBUILD_CYCLE_READY");
        std::io::stdout().flush().unwrap();
        loop {
            backend.db.historical_proof_snapshot_at(expected.root(), 1000).unwrap();
            while backend.db.maintain_history_at(Some(2), 2000).unwrap() {}
        }
    }
    for retained in [false, true] {
        for delay_ms in [0, 5, 50, 250] {
            let directory = tempfile::tempdir().unwrap();
            {
                let backend = open_with_idle(directory.path(), 1);
                backend.db.write_committed_data_dataset(&expected, Some(0)).unwrap();
                drain(&backend.db, Some(2));
                if retained {
                    backend.db.historical_proof_snapshot_at(expected.root(), 1000).unwrap();
                }
            }
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(DIRECTORY, directory.path())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let output = std::io::BufReader::new(child.stdout.take().unwrap());
            let mut ready = false;
            for line in output.lines() {
                if line.unwrap() == "REBUILD_CYCLE_READY" {
                    ready = true;
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                    child.kill().unwrap();
                    break;
                }
            }
            assert!(!child.wait().unwrap().success());
            assert!(ready, "Child must enter the operation loop before SIGKILL");
            let backend = open_with_idle(directory.path(), 1);
            let db = &backend.db;
            let expected_nodes = if pointer(db, expected.root()) == 0 { 0 } else { 2 * expected.entries().len() - 1 };
            assert_eq!(rows(db, b"cdm/n/").len(), expected_nodes);
            assert_eq!(rows(db, b"cdh/ref/").len(), expected_nodes);
            let keys = [Felt::ZERO, Felt::from(2048u32), Felt::from(4096u32)];
            assert_eq!(
                db.get_committed_data_batch(expected.root(), &keys).unwrap(),
                CommittedDataBatchProof::from_dataset(&expected, &keys).unwrap()
            );
            while db.maintain_history_at(Some(2), u64::MAX).unwrap() {}
            assert!(rows(db, b"cdm/n/").is_empty());
            assert!(rows(db, b"cdh/ref/").is_empty());
            println!("ADVERSARIAL_SIGKILL retained={retained} delay_ms={delay_ms} recovered_nodes={expected_nodes}");
        }
    }
}

/// A real exhausted filesystem exercises RocksDB/WAL errors, rather than mocking write errors.
/// CI explicitly runs this on its own bounded tmpfs; never fill the developer's normal disk.
#[rstest::rstest]
#[case::rebuild(0)]
#[case::renew_lease(1)]
#[case::prune(2)]
#[ignore = "requires a dedicated size-limited filesystem; explicitly run by Madara CI"]
fn disk_full_during_reconstruction_preserves_archive_and_recovers_after_restart(#[case] operation: u8) {
    use std::io::Write;
    let location = std::env::var("MADARA_COMMITTED_DATA_FAULT_DISK").expect("dedicated tmpfs path");
    let filesystem = std::process::Command::new("stat").args(["-f", "-c", "%T", &location]).output().unwrap();
    assert!(filesystem.status.success());
    assert_eq!(String::from_utf8(filesystem.stdout).unwrap().trim(), "tmpfs", "Fault injection requires tmpfs");
    let directory = tempfile::tempdir_in(location).unwrap();
    let expected =
        CommittedDataSet::new((0..4097u32).map(|key| (Felt::from(key), Felt::from(key + 1))).collect()).unwrap();
    let backend = open_with_idle(&directory.path().join("db"), 100);
    let db = &backend.db;
    db.write_committed_data_dataset(&expected, Some(0)).unwrap();
    drain(db, Some(2));
    if operation != 0 {
        db.historical_proof_snapshot_at(expected.root(), 1000).unwrap();
        if operation == 2 {
            assert!(db.maintain_history_at(None, 1100).unwrap());
        }
    }
    let archives = rows(db, HISTORY);
    let filler_path = directory.path().join("filler");
    let mut filler = std::fs::File::create(&filler_path).unwrap();
    let mut exhausted = false;
    // Bound collateral space even if someone mistakenly supplies a larger tmpfs.
    for _ in 0..128 {
        if let Err(error) = filler.write_all(&[0u8; 1024 * 1024]) {
            assert_eq!(error.raw_os_error(), Some(28), "Expected Linux ENOSPC");
            exhausted = true;
            break;
        }
    }
    assert!(exhausted, "Fault filesystem must be no larger than 128 MiB");
    // A tiny renewal can still fit the already allocated tail of a WAL page. Advance the
    // clock until an append needs a new page, rather than mistaking spare tail bytes for a bug.
    let attempts = if operation == 1 { 4096 } else { 1 };
    let mut result = Ok(());
    for offset in 0..attempts {
        result = if operation == 2 {
            db.maintain_history_at(Some(2), 1200).map(|_| ())
        } else {
            db.historical_proof_snapshot_at(expected.root(), 1050 + offset).map(drop)
        };
        if result.is_err() {
            break;
        }
    }
    let failure = result.expect_err("The durable operation must fail on the exhausted filesystem");
    println!("ADVERSARIAL_ENOSPC operation={operation} error={failure:#}");
    assert_eq!(rows(db, HISTORY), archives);
    drop(filler);
    std::fs::remove_file(filler_path).unwrap();
    // RocksDB may latch an IO background error. Restart is the supported recovery boundary.
    drop(backend);
    let backend = open_with_idle(&directory.path().join("db"), 100);
    let db = &backend.db;
    assert_eq!(rows(db, HISTORY), archives);
    let keys = [Felt::ZERO, Felt::from(4096u32)];
    assert_eq!(
        db.get_committed_data_batch(expected.root(), &keys).unwrap(),
        CommittedDataBatchProof::from_dataset(&expected, &keys).unwrap()
    );
    while db.maintain_history_at(Some(2), u64::MAX).unwrap() {}
    assert!(rows(db, b"cdm/n/").is_empty());
    assert!(rows(db, b"cdh/ref/").is_empty());
}
