# RocksDB Baseline Metrics Report

**Date**: 2026-01-14
**Pod**: `paradex-mainnet-madara-dev-0`
**Namespace**: `madara-full-node`
**Time Window**: Last 12 hours
**Data Source**: Victoria Metrics (`vmselect-central.karnot.xyz`)

---

## Executive Summary

The current RocksDB configuration is causing significant write pressure:
- **61% of the time** L0 files exceeded the slowdown threshold (20 files)
- **22% of the time** L0 files exceeded the stop threshold (36 files)
- L0 files peaked at **146 files** (4x the stop threshold)
- **No block cache configured** - all reads go to disk

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

### Threshold Violations

| Condition | Count | Percentage |
|-----------|-------|------------|
| L0 > 20 (slowdown threshold) | 52/85 samples | **61%** |
| L0 > 36 (stop threshold) | 19/85 samples | **22%** |
| L0 > 48 (proposed new stop) | 10/85 samples | 12% |

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

**Note**: The `db_is_write_stopped` metric shows 0, but the high L0 file counts indicate write throttling (slowdowns) were occurring frequently.

---

## Key Findings

1. **L0 File Pressure is Critical**: L0 files exceeded the stop threshold 22% of the time, indicating frequent write stalls or severe throttling.

2. **Current Thresholds Too Aggressive**: The default 20/36 thresholds were tuned for ~20 GiB databases, but this production DB is ~270 GiB (13x larger).

3. **No Block Cache**: All reads go directly to disk, wasting the table readers memory (~2 GiB).

4. **Memtable Pressure**: Immutable memtables reached 19 at peak, far exceeding the max_write_buffer_number of 5.

5. **Compaction Bytes OK**: Pending compaction bytes stayed at 0, suggesting the byte-based limits are appropriate.

---

## Recommended Changes

See the accompanying configuration changes in this PR for optimized values based on this baseline analysis.
