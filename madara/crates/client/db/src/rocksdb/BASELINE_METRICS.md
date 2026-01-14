# RocksDB Baseline Metrics Report

**Date**: 2026-01-14
**Pod**: `paradex-mainnet-madara-dev-0`
**Namespace**: `madara-full-node`
**Time Window**: Last 12 hours
**Data Source**: Victoria Metrics (`vmselect-central.karnot.xyz`)

---

## Important Notes

1. **Aggregated Metrics**: `db_level_files_count` and `db_num_immutable_memtables` are aggregated across all **27 column families**. Write stall thresholds apply per-CF, not to the sum.

2. **No Write Stalls**: `db_is_write_stopped` was consistently 0 during the baseline period.

3. **No Block Cache**: Block cache is not configured (0 MiB), affecting read performance.

---

## Key Metrics to Monitor After Changes

When evaluating RocksDB configuration changes, compare these metrics against the baseline:

| Category | Metric | Why It Matters |
|----------|--------|----------------|
| **Write Stalls** | `db_is_write_stopped` | Direct indicator of write stalls |
| **L0 Pressure** | `db_level_files_count{level="L0"}` | Triggers slowdown/stop at thresholds |
| **Compaction Backlog** | `db_pending_compaction_bytes` | Indicates compaction falling behind |
| **Block Production** | `close_block_total_duration_seconds` | End-to-end block production time |
| **Trie Operations** | `apply_to_global_trie_last_seconds` | Dominant operation in block close |
| **Memory Usage** | Container memory, table readers | Resource utilization |
| **Throughput** | Blocks per hour | Overall system throughput |

---

## 1. Write Performance (Latency)

### Close Block Duration (End-to-End)

| Percentile | Value | Notes |
|------------|-------|-------|
| **p50** | 5.02 sec | Median block close time |
| **p95** | 9.53 sec | 95th percentile |
| **p99** | 9.93 sec | 99th percentile |
| **Average** | 2.04 sec | Mean close time |

### Block Operation Breakdown

| Operation | Min | Avg | p95 | Max |
|-----------|-----|-----|-----|-----|
| `apply_to_global_trie` | 384 ms | **1,844 ms** | 4,044 ms | 14,367 ms |
| `contract_storage_trie_commit` | 258 ms | **467 ms** | 793 ms | 1,045 ms |
| `db_write_block_parts` | 8 ms | **12 ms** | 15 ms | 56 ms |
| `contract_trie_commit` | 2 ms | **9 ms** | 21 ms | 99 ms |
| `class_trie_commit` | 0 ms | 0 ms | 0 ms | 0 ms |
| `block_hash_compute` | 0 ms | 0 ms | 0 ms | 0 ms |

**Key Insight**: `apply_to_global_trie` dominates block close time (avg 1.8s, 90% of close time).

---

## 2. RocksDB Internal Metrics

### Write Stall Indicators

| Metric | Value | Threshold | Status |
|--------|-------|-----------|--------|
| `db_is_write_stopped` | 0 | 1 = stalled | ✅ Healthy |
| Stall events in 12h | 0 / 81 samples | - | ✅ No stalls |

### L0 File Count (Aggregated across 27 CFs)

| Statistic | Value | Per-CF Estimate |
|-----------|-------|-----------------|
| **Min** | 7 | ~0.3 |
| **Avg** | 29 | ~1.1 |
| **p50** | 22 | ~0.8 |
| **p95** | 70 | ~2.6 |
| **Max** | 146 | ~5.4 |

**Note**: Per-CF threshold is 20 (slowdown) / 36 (stop). Estimates show healthy per-CF counts.

### Immutable Memtables (Aggregated)

| Statistic | Value | Per-CF Estimate |
|-----------|-------|-----------------|
| **Min** | 0 | 0 |
| **Avg** | 3.24 | ~0.12 |
| **p95** | 18 | ~0.67 |
| **Max** | 19 | ~0.70 |

**Note**: Per-CF threshold is 5 (stall). Estimates show healthy per-CF counts.

### Pending Compaction

| Statistic | Value |
|-----------|-------|
| **Min** | 0 GiB |
| **Max** | 0 GiB |
| **Avg** | 0 GiB |

**Note**: Compaction is keeping up with writes.

---

## 3. LSM Tree Structure (Current Snapshot)

| Level | Files | Notes |
|-------|-------|-------|
| **L0** | 47 | Unsorted, triggers compaction |
| **L1** | 1 | First sorted level |
| **L2** | 11 | |
| **L3** | 127 | |
| **L4** | 242 | |
| **L5** | 351 | |
| **L6** | 3,401 | Cold storage |
| **Total** | 4,180 | Across all CFs |

---

## 4. Memory Usage

### RocksDB Memory

| Component | Min | Avg | Max |
|-----------|-----|-----|-----|
| **Table Readers** | 1.47 GiB | 1.89 GiB | 2.39 GiB |
| **Memtable Total** | 27 MiB | 52 MiB | 83 MiB |
| **Block Cache** | 0 MiB | 0 MiB | 0 MiB |

### Container Memory

| Statistic | Value |
|-----------|-------|
| **Min** | 4.79 GiB |
| **Avg** | 6.03 GiB |
| **Max** | 7.35 GiB |
| **Limit** | 8 GiB |

---

## 5. Storage & I/O

### Database Size

| Metric | Value |
|--------|-------|
| **Start (12h ago)** | 278.44 GiB |
| **End (now)** | 255.48 GiB |
| **Growth** | -22.96 GiB |

**Note**: Negative growth indicates compaction removed obsolete data.

### PVC Usage

| Metric | Value |
|--------|-------|
| **Used** | 256.51 GiB |
| **Capacity** | 491.08 GiB |
| **Utilization** | 52.2% |

### Disk I/O Throughput (Average)

| Direction | Throughput |
|-----------|------------|
| **Write** | 7.75 MiB/s |
| **Read** | 9.87 MiB/s |

---

## 6. Throughput

| Metric | Value |
|--------|-------|
| **Blocks in 12h** | 2,613 |
| **Blocks per Hour** | 217.75 |
| **Avg Block Time** | 2.04 sec |

---

## 7. Resource Utilization

### CPU

| Metric | Value |
|--------|-------|
| **Avg CPU Cores** | 0.66 |
| **CPU Limit** | 4 cores |
| **Utilization** | 16.5% |

---

## 8. Current Configuration (Defaults)

| Parameter | Value | CLI Flag |
|-----------|-------|----------|
| `max_write_buffer_number` | 5 | `--db-max-write-buffer-number` |
| `level_zero_slowdown_writes_trigger` | 20 | `--db-l0-slowdown-trigger` |
| `level_zero_stop_writes_trigger` | 36 | `--db-l0-stop-trigger` |
| `soft_pending_compaction_bytes` | 6 GiB | `--db-soft-pending-compaction-gib` |
| `hard_pending_compaction_bytes` | 12 GiB | `--db-hard-pending-compaction-gib` |
| `memtable_blocks_budget` | 1024 MiB | `--db-memtable-blocks-budget-mib` |
| `memtable_contracts_budget` | 128 MiB | `--db-memtable-contracts-budget-mib` |
| `prefix_bloom_filter_ratio` | 0.0 | `--db-memtable-prefix-bloom-filter-ratio` |

---

## Comparison Checklist

After applying new configuration, compare:

- [ ] **Write stalls**: `db_is_write_stopped` should remain 0
- [ ] **L0 files**: Should not increase significantly (aggregated)
- [ ] **Block close time**: p50/p95/p99 should not regress
- [ ] **Trie operations**: `apply_to_global_trie` timing
- [ ] **Memory usage**: Should stay within limits
- [ ] **Throughput**: Blocks per hour should not decrease
- [ ] **Disk I/O**: No significant increase in write amplification

---

## Recommended Changes (This PR)

| Parameter | Before | After | Rationale |
|-----------|--------|-------|-----------|
| `level_zero_slowdown_writes_trigger` | 20 | **24** | 20% more headroom |
| `level_zero_stop_writes_trigger` | 36 | **48** | Maintains 2x ratio |
| `soft_pending_compaction_bytes` | 6 GiB | **48 GiB** | Scale for 250+ GiB DBs |
| `hard_pending_compaction_bytes` | 12 GiB | **96 GiB** | Scale for 250+ GiB DBs |
| `max_write_buffer_number` | 5 | **6** | Extra buffer for bursts |
| `memtable_contracts_budget` | 128 MiB | **256 MiB** | Better state throughput |
| `prefix_bloom_filter_ratio` | 0.0 | **0.1** | Faster negative lookups |
