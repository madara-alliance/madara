//! Opt-in measurements on real RocksDB. This entire module is test-only.
//! The ignored measurement intentionally runs only through the workflow's measurement job;
//! ordinary correctness tests cover durability, pruning and reconstruction without timing assertions.
//! Synthetic wide felts and small integer movements exercise different compression behavior.
//! Neither distribution is a production workload or a capacity guarantee.
use super::*;
use crate::{MadaraBackend, MadaraBackendConfig, MadaraStorageRead, MadaraStorageWrite};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Instant};

const SYNTHETIC_VALUE_BITS: [u32; 8] = [8, 16, 24, 32, 40, 48, 56, 60];
/// Maximum synthetic movement is approximately 1/65536 of the preceding value.
const RELATIVE_CHANGE_DIVISOR: u64 = 65_536;
const MAX_MEASURED_VERSIONS: usize = 1000;
const PROOF_SAMPLE_KEYS: usize = 32;
const DEFAULT_ENTRIES: usize = 16_384;
const DEFAULT_UPDATES: usize = 1024;
const DEFAULT_VERSIONS: usize = 70;
const DEFAULT_RETAINED: usize = 8;
const DEFAULT_ROLLING_VERSIONS: usize = 16;
const PERMANENT_VALUES: &str = "permanent_values";
const PROOF_NODES: &str = "proof_nodes";
const REFERENCE_EDGES: &str = "reference_edges";
const ROOT_METADATA: &str = "root_metadata";
const LOGICAL_ROWS_AND_BYTES: &str = "logical_rows_and_bytes";
const PROC_STATUS_PATH: &str = "/proc/self/status";
const RSS_FIELD: &str = "VmRSS:";
const PEAK_RSS_FIELD: &str = "VmHWM:";

/// Explicit environment overrides keep expensive measurements out of default test runs.
fn parameter(name: &str, default: usize) -> usize {
    std::env::var(name).map(|s| s.parse().expect("integer benchmark parameter")).unwrap_or(default)
}
/// Linux-only process metrics: other platforms report null, never a misleading zero.
fn resident_kib(name: &str) -> Option<u64> {
    std::fs::read_to_string(PROC_STATUS_PATH).ok()?.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == name).then(|| words.next()?.parse().ok()).flatten()
    })
}
/// Logical key/value bytes and live SST sizes measure different things; neither includes all disk usage.
fn footprint(db: &RocksDBStorage) -> serde_json::Value {
    let cf = db.inner.get_column(meta::META_COLUMN);
    db.inner.db.flush_cf(&cf).unwrap();
    let mut buckets = BTreeMap::<String, (u64, u64)>::new();
    for row in db.inner.db.iterator_cf(&cf, IteratorMode::Start) {
        let (key, value) = row.unwrap();
        let name = if key.starts_with(HISTORY) || key.starts_with(archive::DICTIONARY) {
            PERMANENT_VALUES
        } else if key.starts_with(crate::persistent_committed_data::NODE_PREFIX) {
            PROOF_NODES
        } else if key.starts_with(super::super::proof_references::REFERENCES) {
            REFERENCE_EDGES
        } else if key.starts_with(b"cdh/") || key.starts_with(PREFIX) {
            ROOT_METADATA
        } else {
            continue;
        };
        let entry = buckets.entry(name.into()).or_default();
        entry.0 += 1;
        entry.1 += (key.len() + value.len()) as u64;
    }
    let files = db.inner.db.live_files().unwrap();
    let sst_bytes: usize = files.iter().map(|f| f.size).sum();
    let history_sst_bytes: usize =
        files.iter().filter(|f| f.column_family_name == meta::META_COLUMN.rocksdb_name).map(|f| f.size).sum();
    serde_json::json!({(LOGICAL_ROWS_AND_BYTES): buckets, "live_sst_bytes_all_columns": sst_bytes, "live_sst_bytes_history_column": history_sst_bytes,
        "rss_kib": resident_kib(RSS_FIELD), "peak_rss_kib": resident_kib(PEAK_RSS_FIELD)})
}
/// Deterministic high-entropy canonical felts; no external dataset or private records are embedded.
fn value(key: usize, tick: usize) -> Felt {
    let mut bytes: [u8; 32] = Sha256::digest(format!("value-{key}-{tick}")).into();
    bytes[0] &= 3; // Canonical high-entropy felt; do not model compression with tiny integers.
    Felt::from_bytes_be(&bytes)
}

/// A synthetic integer workload with bounded changes or independent redraws.
fn integer_value(key: usize, tick: usize, previous: Option<Felt>, large_moves: bool) -> Felt {
    let bytes = value(key, tick).to_bytes_be();
    let random = u64::from_be_bytes(bytes[24..].try_into().unwrap());
    let amount = if let Some(previous) = previous.filter(|_| !large_moves) {
        let previous = u64::from_be_bytes(previous.to_bytes_be()[24..].try_into().unwrap());
        let change = 1 + random % (previous / RELATIVE_CHANGE_DIVISOR).max(1);
        if random & 1 == 0 {
            previous + change
        } else {
            previous.saturating_sub(change).max(1)
        }
    } else {
        // Deliberately cover multiple integer widths; wide-felt mode is tested separately.
        let bits = SYNTHETIC_VALUE_BITS[key % SYNTHETIC_VALUE_BITS.len()];
        (1u64 << (bits - 1)) | (random & ((1u64 << (bits - 1)) - 1))
    };
    let value = Felt::from(amount);
    // A random redraw can equal the preceding tick; keep the all-values-change workload exact.
    if Some(value) == previous {
        value + Felt::ONE
    } else {
        value
    }
}

fn compact(db: &RocksDBStorage) {
    let cf = db.inner.get_column(meta::META_COLUMN);
    db.inner.db.flush_cf(&cf).unwrap();
    db.inner.db.compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);
}

#[rstest::rstest]
#[case::small_movements(false)]
#[case::independent_redraws(true)]
fn committed_history_integer_workload_changes_every_value(#[case] large_moves: bool) {
    for key in 0..SYNTHETIC_VALUE_BITS.len() {
        let mut previous = integer_value(key, 0, None, false);
        for tick in 1..100 {
            let next = integer_value(key, tick, Some(previous), large_moves);
            assert_ne!(next, previous);
            assert_eq!(next, integer_value(key, tick, Some(previous), large_moves));
            previous = next;
        }
        let drawn = integer_value(key, 1, None, true);
        assert_eq!(integer_value(key, 1, Some(drawn), true), drawn + Felt::ONE);
    }
}

#[test]
#[ignore = "Opt-in real storage/RSS measurement; parameterized workload, not a capacity SLA"]
fn committed_history_measure_storage_and_memory() {
    let entries = parameter("CD_MEASURE_ENTRIES", DEFAULT_ENTRIES);
    let updates = parameter("CD_MEASURE_UPDATES", DEFAULT_UPDATES);
    let versions = parameter("CD_MEASURE_VERSIONS", DEFAULT_VERSIONS);
    let retained = parameter("CD_MEASURE_RETAINED", DEFAULT_RETAINED);
    let rolling_versions = parameter("CD_MEASURE_ROLLING_VERSIONS", DEFAULT_ROLLING_VERSIONS);
    let large_moves = std::env::var("CD_MEASURE_LARGE_MOVES").is_ok_and(|v| v == "true");
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let integer_values = std::env::var("CD_MEASURE_INTEGER_VALUES").is_ok_and(|value| value == "true");
    assert!(entries > 0 && entries <= MAX_COMMITTED_DATA_VALUES);
    assert!(
        updates > 0 && updates <= entries && retained > 0 && retained < versions && versions <= MAX_MEASURED_VERSIONS
    );
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
    let baseline = footprint(db);
    let mut values: Vec<_> = (0..entries)
        .map(|key| {
            (value(key, usize::MAX), if integer_values { integer_value(key, 0, None, false) } else { value(key, 0) })
        })
        .collect();
    let tree = CommittedDataSet::new(values.clone()).unwrap();
    let mut root = tree.root();
    let started = Instant::now();
    db.write_committed_data_dataset(&tree, Some(0)).unwrap();
    let initial_ms = started.elapsed().as_secs_f64() * 1000.;
    drop(tree);
    let namespace = mp_gateway::committed_data::CommittedDataUpdate {
        application_id: Felt::ONE,
        dataset_id: Felt::TWO,
        base_root: None,
    };
    // Register the initial checkpoint through the same backend + outbox path as the private RPC.
    runtime.block_on(backend.update_committed_data(namespace, root, values.clone())).unwrap();
    let journal = runtime.block_on(backend.journal_committed_data(Some(namespace), root, values.clone())).unwrap();
    db.mark_recovery_exported(&journal).unwrap(); // Simulated exporter ACK, never a Mongo durability claim.
    let initial = footprint(db);
    let mut roots = vec![root];
    let mut import_ms = Vec::new();
    let mut journal_ms = Vec::new();
    let mut acknowledged_ms = Vec::new();
    for tick in 1..=versions {
        let patch: Vec<_> = (0..updates)
            .map(|i| {
                let key = (tick * updates + i) % entries;
                let previous = values[key].1;
                values[key].1 = if integer_values {
                    integer_value(key, tick, Some(previous), large_moves)
                } else {
                    value(key, tick)
                };
                assert_ne!(values[key].1, previous, "Every benchmark value must change");
                values[key]
            })
            .collect();
        // Independent full construction is outside the timed incremental import, but within RSS.
        let next = CommittedDataSet::new(values.clone()).unwrap().root();
        let started = Instant::now();
        let update = mp_gateway::committed_data::CommittedDataUpdate { base_root: Some(root), ..namespace };
        backend.chain_head_state.send_modify(|state| state.confirmed_tip = Some(tick as u64));
        backend.check_recovery_capacity(Some(update), next, patch.len()).unwrap();
        runtime.block_on(backend.update_committed_data(update, next, patch.clone())).unwrap();
        import_ms.push(started.elapsed().as_secs_f64() * 1000.);
        let journal_started = Instant::now();
        let journal = runtime.block_on(backend.journal_committed_data(Some(update), next, patch)).unwrap();
        journal_ms.push(journal_started.elapsed().as_secs_f64() * 1000.);
        acknowledged_ms.push(started.elapsed().as_secs_f64() * 1000.);
        db.mark_recovery_exported(&journal).unwrap();
        root = next;
        roots.push(root);
    }
    let before_compaction = footprint(db);
    compact(db);
    let before = footprint(db);
    let target = roots[versions - retained - 1];
    let keys: Vec<_> = values.iter().take(PROOF_SAMPLE_KEYS).map(|(key, _)| *key).collect();
    let started = Instant::now();
    let warm = db.get_committed_data_batch(target, &keys).unwrap();
    let warm_ms = started.elapsed().as_secs_f64() * 1000.;
    let started = Instant::now();
    let cutoff = (versions - retained + 2) as u64;
    let mut pruned = 0;
    while db.maintain_history(Some(cutoff)).unwrap() {
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
        before[LOGICAL_ROWS_AND_BYTES][PERMANENT_VALUES],
        after_delete[LOGICAL_ROWS_AND_BYTES][PERMANENT_VALUES]
    );
    compact(db);
    let after_compaction = footprint(db);
    let latest = db.get_committed_data_batch(root, &keys).unwrap();
    let mut remaining = 0;
    while db.maintain_history(Some(u64::MAX)).unwrap() {
        remaining += 1;
    }
    assert_eq!(remaining, retained);
    assert_eq!(db.get_committed_data_batch(root, &keys).unwrap(), latest);
    compact(db);
    let values_only = footprint(db);
    assert!(values_only[LOGICAL_ROWS_AND_BYTES].get(PROOF_NODES).is_none());
    assert!(values_only[LOGICAL_ROWS_AND_BYTES].get(REFERENCE_EDGES).is_none());
    assert_eq!(before[LOGICAL_ROWS_AND_BYTES][PERMANENT_VALUES], values_only[LOGICAL_ROWS_AND_BYTES][PERMANENT_VALUES]);
    // Separately measure repeated imports while reclaiming obsolete proof roots every tick.
    // Serialize maintenance after import: cycle time includes its entire writer-lock cost.
    db.write_committed_data_dataset(&CommittedDataSet::new(values.clone()).unwrap(), Some(versions as u64)).unwrap();
    let mut rolling_ack_ms = Vec::new();
    let mut rolling_cycle_ms = Vec::new();
    for tick in versions + 1..=versions + rolling_versions {
        let patch: Vec<_> = (0..updates)
            .map(|i| {
                let key = (tick * updates + i) % entries;
                let previous = values[key].1;
                values[key].1 = if integer_values {
                    integer_value(key, tick, Some(previous), large_moves)
                } else {
                    value(key, tick)
                };
                assert_ne!(values[key].1, previous);
                values[key]
            })
            .collect();
        let next = CommittedDataSet::new(values.clone()).unwrap().root();
        let update = mp_gateway::committed_data::CommittedDataUpdate { base_root: Some(root), ..namespace };
        backend.chain_head_state.send_modify(|state| state.confirmed_tip = Some(tick as u64));
        let started = Instant::now();
        backend.check_recovery_capacity(Some(update), next, patch.len()).unwrap();
        runtime.block_on(backend.update_committed_data(update, next, patch.clone())).unwrap();
        let journal = runtime.block_on(backend.journal_committed_data(Some(update), next, patch)).unwrap();
        rolling_ack_ms.push(started.elapsed().as_secs_f64() * 1000.);
        db.mark_recovery_exported(&journal).unwrap();
        while db.maintain_history(Some((tick + 2 - retained) as u64)).unwrap() {}
        rolling_cycle_ms.push(started.elapsed().as_secs_f64() * 1000.);
        root = next;
    }
    assert_eq!(
        db.get_committed_data_batch(root, &keys).unwrap(),
        blockifier::execution::syscalls::committed_data::CommittedDataBatchProof::from_dataset(
            &CommittedDataSet::new(values).unwrap(),
            &keys
        )
        .unwrap()
    );
    while db.maintain_history(Some(u64::MAX)).unwrap() {}
    compact(db);
    let final_values_only = footprint(db);
    let report = serde_json::json!({"entries": entries, "updates_per_version": updates, "versions": versions + 1,
        "retained_versions": retained, "integer_values": integer_values, "large_moves": large_moves, "all_submitted_values_changed": true, "cache_budget_bytes": cache_bytes,
        "baseline": baseline, "initial": initial, "before_prune_uncompacted": before_compaction,
        "before_prune": before, "after_delete": after_delete, "after_compaction": after_compaction,
        "values_only_after_compaction": values_only, "final_values_only_after_compaction": final_values_only, "rolling_versions": rolling_versions,
        "rolling_ack_ms": rolling_ack_ms, "rolling_cycle_ms": rolling_cycle_ms, "journal_ms": journal_ms, "acknowledged_ms": acknowledged_ms, "final_peak_rss_kib": resident_kib(PEAK_RSS_FIELD),
        "initial_import_ms": initial_ms, "incremental_import_ms": import_ms, "prune_total_ms": prune_ms,
        "retained_batch_ms": warm_ms, "reconstructed_batch_ms": cold_ms,
        "limitations": "Synthetic single-process functional workload. Price mode uses captured value-width deciles and synthetic bounded movements, not captured transactions. RSS includes independent reference construction and allocator retention. Live SST bytes exclude WAL and obsolete files. Timed ACK includes backend import, namespace fsync and recovery journal fsync; excludes RPC parsing/signatures/network/account transaction validation. Export ACK is simulated, no Mongo durability measurement. Rolling cycle also includes pruning. No production capacity claim."});
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if let Ok(path) = std::env::var("CD_MEASURE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
