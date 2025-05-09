use crate::*;
use bincode::{deserialize, serialize};
use std::collections::hash_map::DefaultHasher;
use tests::utils::create_filter;

#[test]
fn test_basic_atomic_serialization() {
    const NB_ELEM: u64 = 3;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);

    filter.add(&"test1");
    filter.add(&"test2");
    filter.add(&"test3");

    let fingerprint = filter.storage().fingerprint();
    let serialized = serialize(&filter).unwrap();
    let deserialized: BloomFilter<DefaultHasher, AtomicBitStore> = deserialize(&serialized).unwrap();
    assert_eq!(fingerprint, deserialized.storage().fingerprint(), "Fingerprint mismatch");

    let readonly = deserialized.finalize();

    assert!(readonly.might_contain(&"test1"));
    assert!(readonly.might_contain(&"test2"));
    assert!(readonly.might_contain(&"test3"));
    assert!(!readonly.might_contain(&"test4"));
}

#[test]
fn test_basic_readonly_serialization() {
    const NB_ELEM: u64 = 1;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);

    filter.add(&"test1");
    let readonly = filter.finalize();
    let fingerprint = readonly.storage().fingerprint();

    // Sérialiser
    let serialized = serialize(&readonly).unwrap();

    // Désérialiser
    let deserialized: BloomFilter<DefaultHasher, BitStore> = deserialize(&serialized).unwrap();

    assert_eq!(fingerprint, deserialized.storage().fingerprint(), "Fingerprint mismatch");

    assert!(deserialized.might_contain(&"test1"));
    assert!(!deserialized.might_contain(&"test2"));
}

#[test]
fn test_cross_type_serialization() {
    const NB_ELEM: u64 = 3;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);

    filter.add(&"test1");
    filter.add(&"test2");

    let serialized = serialize(&filter).unwrap();

    let deserialized: BloomFilter<DefaultHasher, BitStore> = deserialize(&serialized).unwrap();

    assert!(deserialized.might_contain(&"test1"));
    assert!(deserialized.might_contain(&"test2"));
    assert!(!deserialized.might_contain(&"test3"));
}

#[test]
fn test_large_dataset_serialization() {
    const NB_ELEM: u64 = 10_000;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);

    for i in 0..NB_ELEM {
        filter.add(&format!("test{}", i));
    }

    let serialized = serialize(&filter).unwrap();

    let _atomic: BloomFilter<DefaultHasher, AtomicBitStore> = deserialize(&serialized).unwrap();
    let readonly: BloomFilter<DefaultHasher, BitStore> = deserialize(&serialized).unwrap();

    for i in 0..NB_ELEM {
        let key = format!("test{}", i);
        assert!(readonly.might_contain(&key));
    }
}
