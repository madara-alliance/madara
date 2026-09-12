use super::*;

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
fn historyless_in_memory_writes_skip_previous_values() {
    let backend = setup_snapshot_db();
    let key = b"historyless-key";
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, key, b"snapshot-value");
    let snapshot = fresh_snapshot(&backend.db);
    let mut db =
        InMemoryBonsaiDb::with_mapping(snapshot, InMemoryColumnMapping::contract(), Arc::new(DashMap::new()), false);

    let previous = db.insert(&DatabaseKey::Flat(key), b"overlay-value", None).expect("insert overlay");
    assert_eq!(previous, None, "historyless writes should not fetch the snapshot value");
    assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("read overlay"), Some(ByteVec::from(&b"overlay-value"[..])));

    let previous = db.remove(&DatabaseKey::Flat(key), None).expect("remove overlay");
    assert_eq!(previous, None, "historyless removals should not fetch the overlay value");
    assert_eq!(db.get(&DatabaseKey::Flat(key)).expect("read tombstone"), None);
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
fn prefix_operations_merge_overlay_with_pinned_snapshot() {
    let backend = setup_snapshot_db();
    for key in [b"p/a", b"p/b", b"p/c", b"q/a"] {
        write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, key, b"original");
    }
    let snapshot = fresh_snapshot(&backend.db);
    // Live writes after capture must not leak into iteration over this snapshot.
    write_snapshot_value(&backend.db, BONSAI_CONTRACT_FLAT_COLUMN, b"p/a", b"new-live-value");
    let mut db = InMemoryBonsaiDb::test_with_mapping(snapshot, InMemoryColumnMapping::contract());
    db.insert(&DatabaseKey::Flat(b"p/b"), b"replacement", None).unwrap();
    db.remove(&DatabaseKey::Flat(b"p/c"), None).unwrap();
    db.insert(&DatabaseKey::Flat(b"p/d"), b"overlay-only", None).unwrap();
    db.insert(&DatabaseKey::Trie(b"p/a"), b"other-column", None).unwrap();

    assert_eq!(
        db.get_by_prefix(&DatabaseKey::Flat(b"p/")).unwrap(),
        vec![
            (ByteVec::from(&b"p/a"[..]), ByteVec::from(&b"original"[..])),
            (ByteVec::from(&b"p/b"[..]), ByteVec::from(&b"replacement"[..])),
            (ByteVec::from(&b"p/d"[..]), ByteVec::from(&b"overlay-only"[..])),
        ]
    );
    db.remove_by_prefix(&DatabaseKey::Flat(b"p/")).unwrap();
    assert!(db.get_by_prefix(&DatabaseKey::Flat(b"p/")).unwrap().is_empty());
    assert_eq!(db.get(&DatabaseKey::Flat(b"q/a")).unwrap(), Some(ByteVec::from(&b"original"[..])));
    assert_eq!(db.get(&DatabaseKey::Trie(b"p/a")).unwrap(), Some(ByteVec::from(&b"other-column"[..])));
    let handle = backend.db.inner.get_column(BONSAI_CONTRACT_FLAT_COLUMN);
    assert_eq!(backend.db.inner.db.get_cf(&handle, b"p/a").unwrap().as_deref(), Some(&b"new-live-value"[..]));
}
