# RocksDB Baseline Metrics Report

**Capture Date**: 2026-01-15
**Time Window**: 2026-01-15 02:20:52 IST to 2026-01-15 14:20:52 IST (12 hours)
**Pod**: `paradex-mainnet-madara-dev-0`
**Namespace**: `madara-full-node`
**Image**: `ghcr.io/madara-alliance/madara:manual-bf4872f`
**Data Source**: Victoria Metrics (`vmselect-central.karnot.xyz`)

---

## Time Reference

For future comparisons, use data **after** this timestamp:
- **Unix Timestamp**: `1768467052`
- **ISO 8601**: `2026-01-15T14:20:52+05:30`

---

## Key Metrics Summary

| Category | Metric | Value | Status |
|----------|--------|-------|--------|
| **Write Stalls** | `db_is_write_stopped` max | 0 | ✅ Healthy |
| **Throughput** | Blocks per Hour | 470.7 | ✅ |
| **Close Block** | Average | 1.33 sec | ✅ |
| **Close Block** | p95 | 9.5 sec | ✅ |
| **Memory** | Container avg | 4.16 GiB | ✅ |

---

## 1. Write Performance

### Close Block Duration

| Percentile | Value |
|------------|-------|
| **p50** | 5.00 sec |
| **p95** | 9.50 sec |
| **p99** | 9.90 sec |
| **Average** | 1.33 sec |

### Block Operation Breakdown

| Operation | Min | Avg | p95 | Max |
|-----------|-----|-----|-----|-----|
| `apply_to_global_trie` | 373 ms | **1,241 ms** | 2,656 ms | 4,925 ms |
| `contract_storage_trie_commit` | 219 ms | **372 ms** | 533 ms | 1,317 ms |
| `db_write_block_parts` | 6 ms | **11 ms** | 14 ms | 19 ms |
| `contract_trie_commit` | - | **7 ms** | - | - |
| `class_trie_commit` | - | **0.6 ms** | - | - |

**Key Insight**: `apply_to_global_trie` dominates block close time (avg 1.24s, ~93% of close time).

---

## 2. RocksDB Internal Metrics

### Write Stall Indicators

| Metric | Value | Status |
|--------|-------|--------|
| `db_is_write_stopped` max | 0 | ✅ No stalls |
| `db_pending_compaction_bytes` max | 0 GiB | ✅ Compaction keeping up |

### L0 File Count (Aggregated across all CFs)

| Statistic | Value | Per-CF Estimate (27 CFs) |
|-----------|-------|--------------------------|
| **Min** | 7 | ~0.3 |
| **Avg** | 27 | ~1.0 |
| **p95** | 62 | ~2.3 |
| **Max** | 112 | ~4.1 |

**Note**: Per-CF threshold is 24 (slowdown) / 48 (stop). Estimates show healthy per-CF counts.

### Immutable Memtables (Aggregated)

| Statistic | Value | Per-CF Estimate |
|-----------|-------|-----------------|
| **Min** | 0 | 0 |
| **Avg** | 2.42 | ~0.09 |
| **p95** | 18 | ~0.67 |
| **Max** | 20 | ~0.74 |

---

## 3. Memory Usage

### RocksDB Memory

| Component | Min | Avg | Max |
|-----------|-----|-----|-----|
| **Table Readers** | 0.85 GiB | 1.06 GiB | 1.24 GiB |
| **Memtable Total** | 91 MiB | 123 MiB | 197 MiB |

### Container Memory

| Statistic | Value |
|-----------|-------|
| **Min** | 2.86 GiB |
| **Avg** | 4.16 GiB |
| **Max** | 6.22 GiB |
| **Limit** | 8 GiB |
| **Utilization (avg)** | 52% |

---

## 4. Throughput

| Metric | Value |
|--------|-------|
| **Blocks in 12h** | 5,648 |
| **Blocks per Hour** | 470.7 |
| **Avg Block Time** | 1.33 sec |

---

## 5. Resource Utilization

### CPU

| Metric | Value |
|--------|-------|
| **Avg CPU Cores** | 1.35 |
| **CPU Limit** | 4 cores |
| **Utilization** | 33.8% |

---

## 6. Current Configuration

| Parameter | Value | CLI Flag |
|-----------|-------|----------|
| `level_zero_slowdown_writes_trigger` | 24 | `--db-l0-slowdown-trigger` |
| `level_zero_stop_writes_trigger` | 48 | `--db-l0-stop-trigger` |
| `soft_pending_compaction_bytes` | 48 GiB | `--db-soft-pending-compaction-gib` |
| `hard_pending_compaction_bytes` | 96 GiB | `--db-hard-pending-compaction-gib` |
| `max_write_buffer_number` | 6 | `--db-max-write-buffer-number` |
| `memtable_blocks_budget` | 1024 MiB | `--db-memtable-blocks-budget-mib` |
| `memtable_contracts_budget` | 256 MiB | `--db-memtable-contracts-budget-mib` |

---

## 7. Comparison with Previous Baseline

| Metric | Previous (Jan 14) | Current (Jan 15) | Change |
|--------|-------------------|------------------|--------|
| **Throughput** | 217.75 blocks/hr | 470.7 blocks/hr | **+116%** ✅ |
| **Close Block avg** | 2.04 sec | 1.33 sec | **-35%** ✅ |
| **apply_to_global_trie avg** | 1,844 ms | 1,241 ms | **-33%** ✅ |
| **apply_to_global_trie p95** | 4,044 ms | 2,656 ms | **-34%** ✅ |
| **L0 Files max** | 146 | 112 | **-23%** ✅ |
| **Table Readers avg** | 1.89 GiB | 1.06 GiB | **-44%** ✅ |
| **Container Memory avg** | 6.03 GiB | 4.16 GiB | **-31%** ✅ |
| **Write Stalls** | 0 | 0 | ✅ |

---

## 8. Baseline Values for Scripts

Copy these values to `compare_baseline.sh`:

```bash
# Write Performance
BASELINE_CLOSE_BLOCK_P50=5.00
BASELINE_CLOSE_BLOCK_P95=9.50
BASELINE_CLOSE_BLOCK_P99=9.90
BASELINE_CLOSE_BLOCK_AVG=1.33

# Block Operation Breakdown (milliseconds)
BASELINE_APPLY_TRIE_AVG=1241
BASELINE_APPLY_TRIE_P95=2656
BASELINE_CONTRACT_STORAGE_TRIE_AVG=372
BASELINE_DB_WRITE_PARTS_AVG=11

# RocksDB Internals
BASELINE_L0_FILES_AVG=27
BASELINE_L0_FILES_MAX=112
BASELINE_IMMUTABLE_MEMTABLES_AVG=2.42
BASELINE_IMMUTABLE_MEMTABLES_MAX=20
BASELINE_PENDING_COMPACTION_MAX=0

# Memory (GiB)
BASELINE_TABLE_READERS_AVG=1.06
BASELINE_CONTAINER_MEMORY_AVG=4.16

# Throughput
BASELINE_BLOCKS_PER_HOUR=470.7

# I/O (MiB/s) - not available in this capture
BASELINE_DISK_WRITE_AVG=0
BASELINE_DISK_READ_AVG=0
```

---

## Comparison Checklist

When comparing future metrics against this baseline:

- [ ] **Write stalls**: `db_is_write_stopped` should remain 0
- [ ] **L0 files max**: Should stay below 112 (aggregated)
- [ ] **Block close time**: p95 should stay around 9.5s or lower
- [ ] **Trie operations**: `apply_to_global_trie` p95 should stay around 2.7s or lower
- [ ] **Memory usage**: Container memory should stay below 6.2 GiB
- [ ] **Throughput**: Should maintain ~470 blocks/hr or higher
