//! Opt-in measurements on the real RocksDB implementation, never a substitute storage model.
use super::*;
use crate::{MadaraBackend, MadaraBackendConfig, MadaraStorageRead, MadaraStorageWrite};
use sha2::{Digest, Sha256};
use std::time::Instant;

fn parameter(name: &str, default: usize) -> usize {
    std::env::var(name).map(|s| s.parse().expect("integer benchmark parameter")).unwrap_or(default)
}
fn resident_kib(name: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/self/status").ok()?.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == name).then(|| words.next()?.parse().ok()).flatten()
    })
}
fn footprint(db: &RocksDBStorage) -> serde_json::Value {
    let cf = db.inner.get_column(meta::META_COLUMN);
    db.inner.db.flush_cf(&cf).unwrap();
    let mut buckets = BTreeMap::<String, (u64, u64)>::new();
    for row in db.inner.db.iterator_cf(&cf, IteratorMode::Start) {
        let (key, value) = row.unwrap();
        let name = if key.starts_with(HISTORY) {
            "permanent_values"
        } else if key.starts_with(b"cdm/n/") {
            "proof_nodes"
        } else if key.starts_with(b"cdh/ref/") {
            "reference_edges"
        } else if key.starts_with(b"cdh/") || key.starts_with(PREFIX) {
            "root_metadata"
        } else {
            continue;
        };
        let entry = buckets.entry(name.into()).or_default();
        entry.0 += 1;
        entry.1 += (key.len() + value.len()) as u64;
    }
    let sst_bytes: usize = db.inner.db.live_files().unwrap().iter().map(|f| f.size).sum();
    serde_json::json!({"logical_rows_and_bytes": buckets, "live_sst_bytes_all_columns": sst_bytes,
        "rss_kib": resident_kib("VmRSS:"), "peak_rss_kib": resident_kib("VmHWM:")})
}
fn value(key: usize, tick: usize) -> Felt {
    let mut bytes: [u8; 32] = Sha256::digest(format!("price-{key}-{tick}")).into();
    bytes[0] &= 3; // Canonical high-entropy felt; do not model compression with tiny integers.
    Felt::from_bytes_be(&bytes)
}

fn price(key: usize, tick: usize, previous: Option<Felt>) -> Felt {
    let bytes = value(key, tick).to_bytes_be();
    let random = u64::from_be_bytes(bytes[24..].try_into().unwrap());
    let amount = if let Some(previous) = previous {
        let previous = u64::from_be_bytes(previous.to_bytes_be()[24..].try_into().unwrap());
        let change = 1 + random % (previous / 65536).max(1);
        if random & 1 == 0 {
            previous + change
        } else {
            previous.saturating_sub(change).max(1)
        }
    } else {
        // Mid-decile widths from 6,230,266 captured Oracle prices, not the private price records.
        let bits = [23, 27, 29, 30, 32, 33, 35, 36, 38, 41][key % 10];
        (1u64 << (bits - 1)) | (random & ((1u64 << (bits - 1)) - 1))
    };
    Felt::from(amount)
}

fn compact(db: &RocksDBStorage) {
    let cf = db.inner.get_column(meta::META_COLUMN);
    db.inner.db.flush_cf(&cf).unwrap();
    db.inner.db.compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);
}

#[test]
#[ignore = "Opt-in real storage/RSS measurement; parameterized workload, not a capacity SLA"]
fn committed_history_measure_storage_and_memory() {
    let entries = parameter("CD_MEASURE_ENTRIES", 16384);
    let updates = parameter("CD_MEASURE_UPDATES", 1024);
    let versions = parameter("CD_MEASURE_VERSIONS", 70);
    let retained = parameter("CD_MEASURE_RETAINED", 8);
    let price_values = std::env::var("CD_MEASURE_PRICE_VALUES").is_ok_and(|value| value == "true");
    assert!(entries > 0 && entries <= MAX_COMMITTED_DATA_VALUES);
    assert!(updates > 0 && updates <= entries && retained > 0 && retained < versions && versions <= 1000);
    let dir = tempfile::tempdir().unwrap();
    let config = MadaraBackendConfig::default();
    let cache_bytes = config.committed_data_retention.cache_bytes;
    let backend = MadaraBackend::open_rocksdb(
        dir.path(),
        mp_chain_config::ChainConfig::madara_test().into(),
        config,
        RocksDBConfig::default(),
        Default::default(),
    )
    .unwrap();
    let db = &backend.db;
    assert!(db.maintain_history(None, 0).unwrap()); // Empty legacy scan.
    let baseline = footprint(db);
    let mut values: Vec<_> = (0..entries)
        .map(|key| (value(key, usize::MAX), if price_values { price(key, 0, None) } else { value(key, 0) }))
        .collect();
    let tree = CommittedDataSet::new(values.clone()).unwrap();
    let mut root = tree.root();
    let started = Instant::now();
    db.write_committed_data_dataset(&tree, Some(0)).unwrap();
    let initial_ms = started.elapsed().as_secs_f64() * 1000.;
    drop(tree);
    let initial = footprint(db);
    let mut roots = vec![root];
    let mut import_ms = Vec::new();
    for tick in 1..=versions {
        let patch: Vec<_> = (0..updates)
            .map(|i| {
                let key = (tick * updates + i) % entries;
                values[key].1 = if price_values { price(key, tick, Some(values[key].1)) } else { value(key, tick) };
                values[key]
            })
            .collect();
        // Independent full construction is outside the timed incremental import, but within RSS.
        let next = CommittedDataSet::new(values.clone()).unwrap().root();
        let started = Instant::now();
        db.patch_committed_data_dataset(root, next, patch, Some(tick as u64)).unwrap();
        import_ms.push(started.elapsed().as_secs_f64() * 1000.);
        root = next;
        roots.push(root);
    }
    let before_compaction = footprint(db);
    compact(db);
    let before = footprint(db);
    let target = roots[versions - retained - 1];
    let keys: Vec<_> = values.iter().take(32).map(|(key, _)| *key).collect();
    let started = Instant::now();
    let warm = db.get_committed_data_batch(target, &keys).unwrap();
    let warm_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let cutoff = (versions - retained + 2) as u64;
    let mut pruned = 0;
    while db.maintain_history(Some(cutoff), 0).unwrap() {
        pruned += 1;
    }
    let prune_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(pruned, versions + 1 - retained);
    let after_delete = footprint(db);
    let started = Instant::now();
    let cold = db.get_committed_data_batch(target, &keys).unwrap();
    let cold_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(cold, warm);
    assert_eq!(
        before["logical_rows_and_bytes"]["permanent_values"],
        after_delete["logical_rows_and_bytes"]["permanent_values"]
    );
    compact(db);
    let after_compaction = footprint(db);
    let latest = db.get_committed_data_batch(root, &keys).unwrap();
    let mut remaining = 0;
    while db.maintain_history(Some(u64::MAX), 0).unwrap() {
        remaining += 1;
    }
    assert_eq!(remaining, retained);
    assert_eq!(db.get_committed_data_batch(root, &keys).unwrap(), latest);
    compact(db);
    let values_only = footprint(db);
    assert!(values_only["logical_rows_and_bytes"].get("proof_nodes").is_none());
    assert!(values_only["logical_rows_and_bytes"].get("reference_edges").is_none());
    assert_eq!(
        before["logical_rows_and_bytes"]["permanent_values"],
        values_only["logical_rows_and_bytes"]["permanent_values"]
    );
    let report = serde_json::json!({"entries": entries, "updates_per_version": updates, "versions": versions + 1,
        "retained_versions": retained, "price_values": price_values, "cache_budget_bytes": cache_bytes,
        "baseline": baseline, "initial": initial, "before_prune_uncompacted": before_compaction,
        "before_prune": before, "after_delete": after_delete, "after_compaction": after_compaction,
        "values_only_after_compaction": values_only,
        "initial_import_ms": initial_ms, "incremental_import_ms": import_ms, "prune_total_ms": prune_ms,
        "retained_batch_ms": warm_ms, "reconstructed_batch_ms": cold_ms,
        "limitations": "Synthetic single-process functional workload. Price mode uses captured price-width deciles and synthetic bounded movements, not captured transactions. RSS includes independent reference construction and allocator retention. Live SST bytes exclude WAL and obsolete files. No throughput or production capacity claim."});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Ok(path) = std::env::var("CD_MEASURE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
