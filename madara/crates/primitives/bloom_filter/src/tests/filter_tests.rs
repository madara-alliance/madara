use crate::*;
use rand::{thread_rng, Rng};
use rayon::prelude::*;
use std::collections::{hash_map::DefaultHasher, HashSet};
use tests::utils::{create_filter, FALSE_POSITIF_RATE, HASH_COUNT};

#[test]
fn test_basic_operations() {
    const NB_ELEM: u64 = 2;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);
    let key1 = b"Hello";
    let key2 = b"World";

    // Test adding elements
    filter.add(key1);
    filter.add(key2);

    // Test conversion to read-only and lookups
    let ro_filter = filter.finalize();
    assert!(ro_filter.might_contain(key1), "Filter should contain key1");
    assert!(ro_filter.might_contain(key2), "Filter should contain key2");
    assert!(!ro_filter.might_contain(b"other"), "Filter should not contain 'other'");
}

#[test]
fn test_false_positive_rate() {
    const NB_ELEM: u64 = 100_000;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);

    // Add elements
    for i in 0..NB_ELEM {
        filter.add(&i);
    }

    let read_only_filter = filter.finalize();

    // Test false positive rate
    let fp_rate = read_only_filter.estimated_false_positive_rate();
    assert!(
        fp_rate <= FALSE_POSITIF_RATE * 1.1,
        "False positive rate {} exceeds expected rate {} by more than 10%",
        fp_rate,
        FALSE_POSITIF_RATE
    );
}

#[test]
fn test_parallel() {
    const NB_ELEM: u64 = 10_000;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);
    let elements: Vec<u64> = (0..NB_ELEM).collect();

    // Test parallel insertion
    elements.par_iter().for_each(|item| {
        filter.add(item);
    });

    let ro_filter = filter.finalize();

    // Test parallel lookup for false negatives
    let false_negatives: Vec<_> = elements.par_iter().filter(|&&item| !ro_filter.might_contain(&item)).collect();

    assert!(false_negatives.is_empty(), "Found {} false negatives which should be impossible", false_negatives.len());
}

#[test]
fn test_pre_calculated_hashes() {
    const NB_ELEM: u64 = 1_000;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);
    let test_item = "test_value";

    // Create pre-calculated hashes
    let pre_calc = PreCalculatedHashes::new::<DefaultHasher, _>(HASH_COUNT, &test_item);

    // Test that hash count matches
    assert_eq!(pre_calc.hash_count(), HASH_COUNT, "Pre-calculated hash count should match filter hash count");

    filter.add(&test_item);
    let ro_filter = filter.finalize();

    // Test lookup with pre-calculated hashes
    assert!(ro_filter.might_contain_hashes(&pre_calc), "Filter should find item using pre-calculated hashes");
}

#[test]
fn test_actual_false_positive_rate() {
    const NB_ELEM: u64 = 10_000;
    const TEST_SAMPLES: u64 = 100_000;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);
    let mut rng = thread_rng();

    // Create a set of known elements
    let mut known_elements = HashSet::new();
    for _ in 0..NB_ELEM {
        let value: u64 = rng.gen();
        known_elements.insert(value);
        filter.add(&value);
    }

    let ro_filter = filter.finalize();

    // Test random elements and count false positives
    let mut false_positives = 0;
    for _ in 0..TEST_SAMPLES {
        let test_value: u64 = rng.gen();
        if !known_elements.contains(&test_value) && ro_filter.might_contain(&test_value) {
            false_positives += 1;
        }
    }

    let actual_fp_rate = false_positives as f64 / TEST_SAMPLES as f64;
    println!("Actual false positive rate: {}", actual_fp_rate);
    assert!(
        actual_fp_rate <= FALSE_POSITIF_RATE * 1.1,
        "Actual false positive rate {} significantly exceeds expected rate {}",
        actual_fp_rate,
        FALSE_POSITIF_RATE
    );
}

#[test]
fn test_size_alignment() {
    // Test that the filter size is properly aligned to 64-bit boundaries
    let filter = BloomFilter::<DefaultHasher, AtomicBitStore>::new(100, HASH_COUNT);
    assert_eq!(filter.size() % 64, 0, "Filter size should be aligned to 64 bits");
}

#[test]
fn test_empty_filter() {
    let filter = create_filter::<DefaultHasher>(0);
    let ro_filter = filter.finalize();

    // Test various types with empty filter
    assert!(!ro_filter.might_contain(&42u64));
    assert!(!ro_filter.might_contain(&"test"));
    assert!(!ro_filter.might_contain(&vec![1, 2, 3]));
    assert!(!ro_filter.might_contain(&(21, "tuple")));
}

#[test]
fn test_different_types() {
    let filter = create_filter::<DefaultHasher>(10);

    // Test adding different types
    filter.add(&42u64);
    filter.add(&"string");
    filter.add(&vec![1, 2, 3]);
    filter.add(&(21, "tuple"));

    let ro_filter = filter.finalize();

    assert!(ro_filter.might_contain(&42u64));
    assert!(ro_filter.might_contain(&"string"));
    assert!(ro_filter.might_contain(&vec![1, 2, 3]));
    assert!(ro_filter.might_contain(&(21, "tuple")));
}

#[test]
fn test_concurrent_reads() {
    const NB_ELEM: u64 = 1_000;
    const NB_THREADS: usize = 8;

    let filter = create_filter::<DefaultHasher>(NB_ELEM);
    let test_value = "concurrent_test";
    filter.add(&test_value);

    let ro_filter = filter.finalize();
    let filter_arc = std::sync::Arc::new(ro_filter);

    // Spawn multiple threads to read concurrently
    let handles: Vec<_> = (0..NB_THREADS)
        .map(|_| {
            let filter_clone = std::sync::Arc::clone(&filter_arc);
            std::thread::spawn(move || {
                for _ in 0..1000 {
                    assert!(filter_clone.might_contain(&test_value));
                }
            })
        })
        .collect();

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }
}
