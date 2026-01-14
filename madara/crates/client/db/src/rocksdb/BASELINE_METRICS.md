# RocksDB Baseline Metrics Report

**Date**: 2026-01-14
**Pod**: `paradex-mainnet-madara-dev-0`
**Namespace**: `madara-full-node`
**Time Window**: Last 12 hours
**Data Source**: Victoria Metrics (`vmselect-central.karnot.xyz`)

---

## Executive Summary

**Important**: The `db_level_files_count` metric is **aggregated across all 27 column families**.
Write stall thresholds are applied **per column family**, not to the aggregated total.

Corrected analysis:
- L0 peaked at 146 **total across 27 CFs** = ~5.4 files per CF average (below 20 threshold)
- `db_is_write_stopped` was always 0, confirming no write stalls occurred
- **No block cache configured** - all reads go to disk

The current configuration is adequate for preventing write stalls, but the new defaults
provide additional headroom for larger databases and burst write scenarios.

---

## Environment

| Parameter | Value |
|-----------|-------|
| Pod CPU | 4 cores |
| Pod Memory | 8 GiB |
| Database Size | ~255-278 GiB |
| Blocks Produced (12h) | 5,945 |

---

## Current Configuration (Defaults)

| Parameter | Current Value | CLI Flag |
|-----------|---------------|----------|
| `max_write_buffer_number` | 5 | `--db-max-write-buffer-number` |
| `level_zero_slowdown_writes_trigger` | 20 | `--db-l0-slowdown-trigger` |
| `level_zero_stop_writes_trigger` | 36 | `--db-l0-stop-trigger` |
| `soft_pending_compaction_bytes` | 6 GiB | `--db-soft-pending-compaction-gib` |
| `hard_pending_compaction_bytes` | 12 GiB | `--db-hard-pending-compaction-gib` |
| `memtable_blocks_budget` | 1024 MiB | `--db-memtable-blocks-budget-mib` |
| `memtable_contracts_budget` | 128 MiB | `--db-memtable-contracts-budget-mib` |
| `memtable_other_budget` | 128 MiB | `--db-memtable-other-budget-mib` |
| `prefix_bloom_filter_ratio` | 0.0 | `--db-memtable-prefix-bloom-filter-ratio` |

---

## L0 File Count Analysis

| Metric | Value |
|--------|-------|
| **Minimum** | 7 |
| **Maximum** | 146 |
| **Average** | 29 |
| **p50** | 22 |
| **p90** | 58 |
| **p95** | 70 |
| **p99** | 146 |

### Threshold Analysis (Aggregated vs Per-CF)

**Note**: These are aggregated counts across 27 column families.
Per-CF thresholds (20 slowdown, 36 stop) apply individually, not to the sum.

| Condition (Aggregated) | Count | Avg Per CF |
|------------------------|-------|------------|
| L0 > 20 (aggregated) | 52/85 samples | ~0.8-5.4 per CF |
| L0 > 36 (aggregated) | 19/85 samples | ~1.3-5.4 per CF |

**Actual per-CF estimate**: 146 max ÷ 27 CFs = ~5.4 L0 files per CF (well below 20 threshold)

---

## LSM Tree Level Distribution (12h)

| Level | Min Files | Max Files | Avg Files |
|-------|-----------|-----------|-----------|
| L0 | 7 | 146 | 29 |
| L1 | 0 | 17 | 4 |
| L2 | 0 | 30 | 13 |
| L3 | 68 | 129 | 108 |
| L4 | 232 | 359 | 295 |
| L5 | 284 | 573 | 449 |
| L6 | 3,401 | 3,526 | 3,458 |

---

## Memtable Metrics

| Metric | Value |
|--------|-------|
| **Immutable Memtables - Min** | 0 |
| **Immutable Memtables - Max** | 19 |
| **Immutable Memtables - Avg** | 3.09 |
| **Samples >= 4 (near stall)** | 15/85 (17%) |
| **Samples >= 5 (at stall)** | 15/85 (17%) |
| **Memtable Size - Min** | 27 MiB |
| **Memtable Size - Max** | 83 MiB |
| **Memtable Size - Avg** | 52 MiB |

---

## Memory Usage

| Component | Min | Max | Avg |
|-----------|-----|-----|-----|
| Table Readers | 1.07 GiB | 2.39 GiB | 1.9 GiB |
| Block Cache | 0 MiB | 0 MiB | 0 MiB |

**Note**: Block cache is not configured, which significantly impacts read performance.

---

## Pending Compaction

| Metric | Value |
|--------|-------|
| Min | 0 GiB |
| Max | 0 GiB |
| Avg | 0 GiB |

Compaction is keeping up with write load in terms of bytes, but L0 file count is the bottleneck.

---

## Block Production Performance

| Metric | Value |
|--------|-------|
| **Blocks Closed** | 5,945 |
| **Total Close Block Time** | 8,751 seconds |
| **Avg Close Block Time** | 1.47 seconds |
| **p50 Close Block Duration** | 5.02 seconds |
| **p95 Close Block Duration** | 9.53 seconds |
| **p99 Close Block Duration** | 9.93 seconds |

### Write Operations

| Metric | Value |
|--------|-------|
| Total Block Writes | 5,945 |
| Total Write Time | 72.3 seconds |
| Avg Write Time | 12.2 ms |

### Global Trie Merklization

| Metric | Value |
|--------|-------|
| Total Operations | 5,945 |
| Total Time | 7,676 seconds |
| Avg Time per Operation | 1.29 seconds |

---

## Write Stall Status

| Metric | Value |
|--------|-------|
| `db_is_write_stopped` Max | 0 |
| Detected Write Stops | 0 |

**Note**: The `db_is_write_stopped` metric shows 0, confirming no write stalls occurred.
The high aggregated L0 counts are the sum across 27 column families, not per-CF values.

---

## Key Findings

1. **No Write Stalls Detected**: `db_is_write_stopped` was consistently 0, confirming the system is healthy.

2. **Metrics Are Aggregated**: The `db_level_files_count` metric sums across all 27 column families.
   - L0 = 146 aggregated ≈ 5.4 per CF (well below 20 threshold)
   - Immutable memtables = 19 aggregated ≈ 0.7 per CF (well below 5 threshold)

3. **No Block Cache**: All reads go directly to disk. Adding block cache would improve read performance.

4. **Compaction Keeping Up**: Pending compaction bytes stayed at 0, indicating healthy compaction.

5. **Headroom for Growth**: Current config works but new defaults provide buffer for larger databases.

---

## Recommended Changes

See the accompanying configuration changes in this PR for optimized values based on this baseline analysis.
