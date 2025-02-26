use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rayon::prelude::*;

use mp_bloom_filter::{AtomicBitStore, BitStore, BloomFilter};

use ahash::AHasher;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use twox_hash::XxHash64;

/// Configuration constants for benchmarks
const KEY_SIZE: usize = 32;
const HASH_COUNT: u8 = 7;
const BITS_PER_ELEMENT: f64 = 9.6;

const TEST_DATA_SIZE: usize = 100_000;
const LOOKUP_FILL_RATIO: f64 = 0.5;
const SAMPLE_SIZE: usize = 50;
const MEASUREMENT_TIME: u64 = 5;

/// Type alias for test data
type TestData = [u8; KEY_SIZE];

type BenchFn = for<'a, 'b, 'c> fn(&'a mut Criterion, &'b [TestData], &'c str);

struct HasherBenchmarks {
    name: &'static str,
    sequential_insertion: BenchFn,
    parallel_insertion: BenchFn,
    lookup: BenchFn,
    parallel_lookup: BenchFn,
}

fn get_hasher_benchmarks<H: 'static + std::hash::Hasher + Default + Sync>(name: &'static str) -> HasherBenchmarks {
    HasherBenchmarks {
        name,
        sequential_insertion: bench_sequential_insertion::<H> as BenchFn,
        parallel_insertion: bench_parallel_insertion::<H> as BenchFn,
        lookup: bench_lookup::<H> as BenchFn,
        parallel_lookup: bench_parallel_lookup::<H> as BenchFn,
    }
}

/// Generate test data for benchmarking
fn generate_test_data(size: usize) -> Vec<TestData> {
    let mut rng = StdRng::seed_from_u64(42);
    (0..size)
        .map(|_| {
            let mut key = [0u8; KEY_SIZE];
            rng.fill(&mut key);
            key
        })
        .collect()
}

/// Create a pre-filled atomic filter for mutable lookups
fn create_atomic_filter<H: std::hash::Hasher + Default>(test_data: &[TestData]) -> BloomFilter<H, AtomicBitStore> {
    let size = (test_data.len() as f64 * BITS_PER_ELEMENT).ceil() as u64;
    let filter = BloomFilter::<H, AtomicBitStore>::new(size, HASH_COUNT);

    let items_to_insert = (test_data.len() as f64 * LOOKUP_FILL_RATIO) as usize;
    for item in test_data.iter().take(items_to_insert) {
        filter.add(item);
    }
    filter
}

/// Create a pre-filled readonly filter
fn create_readonly_filter<H: std::hash::Hasher + Default>(test_data: &[TestData]) -> BloomFilter<H, BitStore> {
    create_atomic_filter::<H>(test_data).finalize()
}

/// Benchmark sequential insertion
fn bench_sequential_insertion<H: std::hash::Hasher + Default>(c: &mut Criterion, test_data: &[TestData], name: &str) {
    let size = (test_data.len() as f64 * BITS_PER_ELEMENT).ceil() as u64;

    let mut group = c.benchmark_group(format!("Sequential Insertion/{}", name));
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(std::time::Duration::from_secs(MEASUREMENT_TIME));

    group.bench_function("atomic", |b| {
        b.iter(|| {
            let filter = BloomFilter::<H, AtomicBitStore>::new(size, HASH_COUNT);
            for item in test_data {
                filter.add(black_box(item));
            }
        });
    });

    group.finish();
}

/// Benchmark parallel insertion using rayon
fn bench_parallel_insertion<H: Hasher + Default + Sync>(c: &mut Criterion, test_data: &[TestData], name: &str) {
    let size = (test_data.len() as f64 * BITS_PER_ELEMENT).ceil() as u64;

    let mut group = c.benchmark_group(format!("Parallel Insertion/{}", name));
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(std::time::Duration::from_secs(MEASUREMENT_TIME));

    group.bench_function("atomic", |b| {
        b.iter(|| {
            let filter = BloomFilter::<H, AtomicBitStore>::new(size, HASH_COUNT);
            test_data.par_iter().for_each(|item| {
                filter.add(black_box(item));
            });
        });
    });

    group.finish();
}

/// Benchmark lookups
fn bench_lookup<H: std::hash::Hasher + Default + Sync>(c: &mut Criterion, test_data: &[TestData], name: &str) {
    let mut group = c.benchmark_group(format!("Lookup/{}", name));
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(std::time::Duration::from_secs(MEASUREMENT_TIME));

    let readonly_filter = create_readonly_filter::<H>(test_data);

    // Benchmark readonly filter lookups
    group.bench_function("non atomic", |b| {
        b.iter(|| {
            for item in test_data {
                black_box(readonly_filter.might_contain(item));
            }
        });
    });

    group.finish();
}

/// Benchmark parallel lookups using rayon
fn bench_parallel_lookup<H: std::hash::Hasher + Default + Sync>(c: &mut Criterion, test_data: &[TestData], name: &str) {
    let mut group = c.benchmark_group(format!("Parallel Lookup/{}", name));
    group.sample_size(SAMPLE_SIZE);
    group.measurement_time(std::time::Duration::from_secs(MEASUREMENT_TIME));

    let readonly_filter = create_readonly_filter::<H>(test_data);

    // Benchmark parallel readonly filter lookups
    group.bench_function("non atomic", |b| {
        b.iter(|| {
            test_data.par_iter().for_each(|item| {
                black_box(readonly_filter.might_contain(item));
            });
        });
    });

    group.finish();
}

fn bench_bloom_filters(c: &mut Criterion) {
    let test_data = generate_test_data(TEST_DATA_SIZE);

    // Test different hash functions
    let hashers = [
        get_hasher_benchmarks::<DefaultHasher>("DefaultHasher"),
        get_hasher_benchmarks::<AHasher>("AHasher"),
        get_hasher_benchmarks::<XxHash64>("XxHash64"),
    ];

    for hasher in &hashers {
        (hasher.sequential_insertion)(c, &test_data, hasher.name);
        (hasher.parallel_insertion)(c, &test_data, hasher.name);
        (hasher.lookup)(c, &test_data, hasher.name);
        (hasher.parallel_lookup)(c, &test_data, hasher.name);
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .with_plots()
        .sample_size(SAMPLE_SIZE);
    targets = bench_bloom_filters
}

criterion_main!(benches);
