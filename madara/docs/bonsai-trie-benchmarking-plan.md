# Bonsai Trie Storage & Revert Benchmarking Plan

## Executive Summary

This document outlines a benchmarking strategy to evaluate different storage and revert mechanisms for the Bonsai trie in Madara. The goal is to find the optimal trade-off between:
- **Storage size** (DB footprint, backup/copy time)
- **Revert performance** (time to restore historical state)
- **Operational complexity** (snapshot management, log retention)

---

## Baseline: Current Implementation

### What's Currently Stored

| Column | Content | Purpose |
|--------|---------|---------|
| `bonsai_*_trie` | Serialized `BinaryNode` and `EdgeNode` | Current trie structure |
| `bonsai_*_flat` | Leaf values (`Felt`) | Current leaf data |
| `bonsai_*_log` | `ChangeBatch` with old/new values for **both** intermediary nodes and leaves | Revert/historical access |

### Current Behavior

```
On commit(block_n):
  1. Compute new node hashes (CPU-bound)
  2. Serialize ALL modified nodes (intermediary + leaves)
  3. Store changes in trie log: { key: (old_value, new_value) }
  4. Optionally create RocksDB snapshot

On revert_to(target_block):
  1. For each block from current down to target+1:
     - Read ChangeBatch from log
     - Restore old_value for each key (intermediary + leaves)
  2. Clear in-memory cache

On get_transactional_state(block_n):
  1. Get closest snapshot
  2. Replay logs from snapshot to block_n
  3. Return read-only view
```

### Metrics to Establish Baseline

| Metric | How to Measure |
|--------|----------------|
| DB size (total) | `du -sh /path/to/db` |
| DB size (trie logs only) | `du -sh /path/to/db/*_log*` or RocksDB column stats |
| Revert time (10 blocks) | `time backend.revert_to(current - 10)` |
| Revert time (100 blocks) | `time backend.revert_to(current - 100)` |
| Revert time (1000 blocks) | `time backend.revert_to(current - 1000)` |
| Historical state access time | `time get_transactional_state(current - N)` |
| Snapshot copy time | `time cp -r /path/to/db /path/to/backup` |

---

## Scenario 1: Remove Intermediary Logs

### 1.1 Description

Store only **leaf value changes** in trie logs, not intermediary node changes.

**Current log entry:**
```rust
ChangeBatch {
    TrieKey::Trie(node_path) => Change { old_node_bytes, new_node_bytes },  // REMOVE
    TrieKey::Flat(leaf_key)  => Change { old_value, new_value },            // KEEP
}
```

**Modified log entry:**
```rust
ChangeBatch {
    TrieKey::Flat(leaf_key) => Change { old_value, new_value },  // ONLY LEAVES
}
```

### 1.2 Implementation Changes

```rust
// In key_value_db.rs, modify insert() to only track Flat keys:
pub(crate) fn insert(&mut self, key: &TrieKey, value: &[u8], batch: ...) {
    let old_value = self.db.insert(&key.into(), value, batch)?;

    // ONLY track leaf changes, not intermediary nodes
    if matches!(key, TrieKey::Flat(_)) {
        self.changes_store.current_changes.insert_in_place(
            key.clone(),
            Change { old_value, new_value: Some(value.into()) },
        );
    }
    Ok(())
}
```

### 1.3 Revert with Leaf-Only Logs

```rust
fn revert_to_leaf_only(target_id: u64, current_id: u64) {
    // Step 1: Restore leaf values from logs
    for id in (target_id + 1..=current_id).rev() {
        let changes = get_leaf_changes(id);  // Only Flat keys
        for (key, change) in changes {
            db.insert(&key, &change.old_value)?;
        }
    }

    // Step 2: Clear in-memory trie cache
    self.tries.reset_to_last_commit()?;

    // Step 3: RECOMPUTE all intermediary nodes
    // This is the expensive part!
    self.recompute_trie_from_leaves()?;
}

fn recompute_trie_from_leaves(&mut self) {
    // Option A: Full rebuild from all leaves
    // - Read all leaves from Flat column
    // - Rebuild trie structure
    // - Compute all hashes
    // - Write all nodes to Trie column

    // Option B: Incremental rebuild
    // - Track which leaves changed during revert
    // - Only recompute affected paths
    // - More complex but faster
}
```

### 1.4 Benchmark Tests

| Test | Description | Metrics |
|------|-------------|---------|
| 1.4.1 | Sync 10,000 blocks with leaf-only logs | DB size, sync time |
| 1.4.2 | Revert 10 blocks | Time, CPU usage |
| 1.4.3 | Revert 100 blocks | Time, CPU usage |
| 1.4.4 | Revert 1000 blocks | Time, CPU usage |
| 1.4.5 | Historical state access (proof generation) | Time per proof |

### 1.5 Expected Outcomes

| Metric | Expected Change | Reasoning |
|--------|-----------------|-----------|
| DB size | **-60% to -80%** | Intermediary nodes are ~50x leaf count |
| Revert (10 blocks) | **+200% to +500%** | Must recompute ~500-5000 node hashes |
| Revert (1000 blocks) | **+50% to +100%** | Hash computation amortized over more changes |
| Proof generation | **+100% to +300%** | Must rebuild trie for historical state |

---

## Scenario 2: Squashed Leaf Values

### 2.1 Description

When accessing historical state or reverting, **squash** all intermediate changes into a single diff per key.

**Current approach (per-block replay):**
```
Block 100→101: key_A: v0→v1
Block 101→102: key_A: v1→v2
Block 102→103: key_A: v2→v3
...
Block 199→200: key_A: v99→v100

To revert from 200 to 100: Apply 100 changes to key_A
```

**Squashed approach:**
```
Squashed diff (100→200): key_A: v0→v100

To revert from 200 to 100: Apply 1 change to key_A (restore v0)
```

### 2.2 Implementation

```rust
/// Squash changes across multiple blocks into a single diff
fn get_squashed_changes(
    from_block: u64,
    to_block: u64,
) -> Result<HashMap<TrieKey, Change>, Error> {
    let mut squashed: HashMap<TrieKey, Change> = HashMap::new();

    for block_id in from_block..to_block {
        let changes = ChangeBatch::deserialize(
            &BasicId::new(block_id),
            db.get_by_prefix(&DatabaseKey::TrieLog(&block_id.to_bytes()))?,
        );

        for (key, change) in changes.0 {
            // Only process leaf changes (if using Scenario 1)
            if !matches!(key, TrieKey::Flat(_)) {
                continue;
            }

            match squashed.entry(key) {
                Entry::Occupied(mut e) => {
                    // Keep ORIGINAL old_value, update to LATEST new_value
                    e.get_mut().new_value = change.new_value;
                }
                Entry::Vacant(e) => {
                    e.insert(change);
                }
            }
        }
    }

    // Remove no-ops where old == new after squashing
    squashed.retain(|_, c| c.old_value != c.new_value);

    Ok(squashed)
}

/// Revert using squashed diff
fn revert_to_squashed(target_block: u64, current_block: u64) {
    let squashed = get_squashed_changes(target_block, current_block)?;

    let mut batch = db.create_batch();
    for (key, change) in squashed {
        if let Some(old_value) = change.old_value {
            db.insert(&key.into(), &old_value, Some(&mut batch))?;
        } else {
            db.remove(&key.into(), Some(&mut batch))?;
        }
    }
    db.write_batch(batch)?;

    // If leaf-only: recompute trie
    self.recompute_trie_from_leaves()?;
}
```

### 2.3 Benchmark Tests

| Test | Description | Metrics |
|------|-------------|---------|
| 2.3.1 | Squash 10 blocks of changes | Time, memory usage |
| 2.3.2 | Squash 100 blocks of changes | Time, memory usage |
| 2.3.3 | Squash 1000 blocks of changes | Time, memory usage |
| 2.3.4 | Revert using squashed (10 blocks) | Total time vs baseline |
| 2.3.5 | Revert using squashed (100 blocks) | Total time vs baseline |
| 2.3.6 | Revert using squashed (1000 blocks) | Total time vs baseline |
| 2.3.7 | Unique keys ratio | `unique_keys / total_changes` |

### 2.4 Expected Outcomes

| Block Range | Est. Total Changes | Est. Unique Keys | Squash Ratio | DB Ops Reduction |
|-------------|-------------------|------------------|--------------|------------------|
| 10 blocks | 5,000 | 4,500 | 90% | 10% |
| 100 blocks | 50,000 | 20,000 | 40% | 60% |
| 1000 blocks | 500,000 | 80,000 | 16% | 84% |

**Key insight**: The longer the block range, the more benefit from squashing.

---

## Scenario 3: Snapshots Instead of Logs

### 3.1 Description

Instead of storing per-block change logs, maintain **full state snapshots** at regular intervals.

**Current (log-based):**
```
Block 1000: [snapshot] + logs for 1001, 1002, ..., 2000
Block 2000: [snapshot] + logs for 2001, 2002, ..., 3000
...
```

**Snapshot-based:**
```
Block 1000: [full snapshot]
Block 2000: [full snapshot]
Block 3000: [full snapshot]
...
No per-block logs!
```

### 3.2 Snapshot Types

#### 3.2.1 RocksDB Native Snapshots
- Lightweight sequence number reference
- Shares SST files with main DB
- Fast to create, minimal storage overhead
- **Limitation**: Cannot be copied/persisted independently

#### 3.2.2 RocksDB Checkpoints
- Full copy of DB at a point in time
- Uses hard links (fast creation, shared storage on same filesystem)
- Can be copied to external storage
- **Limitation**: Full DB size per checkpoint

#### 3.2.3 Incremental Backups
- Only stores changed SST files since last backup
- Requires base backup + incrementals to restore
- Most storage-efficient for periodic backups
- **Limitation**: Restoration requires replaying incrementals

### 3.3 Implementation

```rust
/// Snapshot-based state management
struct SnapshotManager {
    /// Path to checkpoint directory
    checkpoint_dir: PathBuf,
    /// Interval between full snapshots
    snapshot_interval: u64,
    /// Maximum snapshots to keep
    max_snapshots: usize,
    /// Index: block_n -> checkpoint path
    snapshots: BTreeMap<u64, PathBuf>,
}

impl SnapshotManager {
    /// Create checkpoint at current block
    fn create_checkpoint(&mut self, db: &DB, block_n: u64) -> Result<()> {
        if block_n % self.snapshot_interval != 0 {
            return Ok(());
        }

        let checkpoint_path = self.checkpoint_dir.join(format!("block_{}", block_n));

        // RocksDB checkpoint creation (uses hard links)
        let checkpoint = Checkpoint::new(db)?;
        checkpoint.create_checkpoint(&checkpoint_path)?;

        self.snapshots.insert(block_n, checkpoint_path);
        self.prune_old_snapshots()?;

        Ok(())
    }

    /// Revert by restoring snapshot
    fn revert_to_snapshot(&self, target_block: u64) -> Result<PathBuf> {
        // Find closest snapshot at or before target
        let (snap_block, snap_path) = self.snapshots
            .range(..=target_block)
            .next_back()
            .ok_or(Error::NoSnapshotAvailable)?;

        if *snap_block == target_block {
            // Exact match - just use this snapshot
            Ok(snap_path.clone())
        } else {
            // Need to replay from snapshot to target
            // This requires logs between snap_block and target_block
            Err(Error::NeedLogsForReplay {
                from: *snap_block,
                to: target_block
            })
        }
    }

    /// Fast revert: swap DB with snapshot
    fn swap_db_with_snapshot(
        &self,
        current_db_path: &Path,
        target_block: u64,
    ) -> Result<()> {
        let snap_path = self.get_snapshot_path(target_block)?;

        // 1. Close current DB
        // 2. Move current DB to backup
        // 3. Copy snapshot to DB path
        // 4. Reopen DB

        // This is MUCH faster than replaying logs for deep reverts
        Ok(())
    }
}
```

### 3.4 Hybrid Approach: Snapshots + Short-term Logs

```rust
/// Optimal configuration for different use cases
struct HybridConfig {
    /// Full checkpoint every N blocks
    checkpoint_interval: u64,      // e.g., 10,000

    /// Keep per-block logs for last M blocks (for reorgs)
    log_retention_blocks: u64,     // e.g., 1,000

    /// Maximum checkpoints to keep
    max_checkpoints: usize,        // e.g., 10 (100k blocks of history)
}

impl HybridConfig {
    /// Revert strategy based on target
    fn get_revert_strategy(&self, current: u64, target: u64) -> RevertStrategy {
        let distance = current - target;

        if distance <= self.log_retention_blocks {
            // Recent: use logs (fast, handles reorgs)
            RevertStrategy::ReplayLogs
        } else {
            // Deep: use checkpoint (fast for large jumps)
            RevertStrategy::RestoreCheckpoint
        }
    }
}

enum RevertStrategy {
    ReplayLogs,
    RestoreCheckpoint,
}
```

### 3.5 Benchmark Tests

| Test | Description | Metrics |
|------|-------------|---------|
| 3.5.1 | Create checkpoint | Time, disk space used |
| 3.5.2 | Checkpoint with hard links | Space on same FS |
| 3.5.3 | Checkpoint copy to external | Time, actual space |
| 3.5.4 | Restore from checkpoint (swap) | Time to become operational |
| 3.5.5 | Hybrid: revert 10 blocks (logs) | Time |
| 3.5.6 | Hybrid: revert 10,000 blocks (checkpoint) | Time |
| 3.5.7 | Storage: 10 checkpoints vs 100k blocks of logs | Total size |

### 3.6 Expected Outcomes

| Metric | Log-based | Checkpoint-based | Hybrid |
|--------|-----------|------------------|--------|
| Storage (100k blocks) | 50-100 GB | 10-20 GB × N checkpoints | 20-40 GB |
| Revert 10 blocks | 100ms | N/A (no logs) | 100ms (logs) |
| Revert 10,000 blocks | 10-60s | 1-5s (swap) | 1-5s (checkpoint) |
| Revert 100,000 blocks | 60-300s | 1-5s (swap) | 1-5s (checkpoint) |
| Backup/copy time | Hours | Minutes per checkpoint | Minutes |

---

## Test Environment Setup

### Hardware Requirements
```yaml
Recommended:
  CPU: 8+ cores (for parallel hash computation benchmarks)
  RAM: 32+ GB (for in-memory squashing tests)
  Storage: NVMe SSD, 500+ GB free

Minimum:
  CPU: 4 cores
  RAM: 16 GB
  Storage: SSD, 200+ GB free
```

### Software Setup
```bash
# Clone and build
git clone <madara-repo>
cd madara
cargo build --release -p mc-db

# Sync test data (or use existing synced node)
./target/release/madara --network sepolia --db-path /path/to/test-db

# Or use snapshot for reproducibility
# Download pre-synced DB snapshot at specific block
```

### Test Data Requirements
```yaml
Minimum blocks synced: 100,000
Recommended: 500,000+
Network: Sepolia (smaller state) or Mainnet (realistic load)
```

---

## Benchmark Execution Script

```bash
#!/bin/bash
# benchmark_bonsai.sh

DB_PATH="/path/to/test-db"
RESULTS_DIR="./benchmark_results"
CURRENT_BLOCK=$(get_current_block $DB_PATH)

mkdir -p $RESULTS_DIR

echo "=== Baseline Measurements ==="
measure_db_size $DB_PATH > $RESULTS_DIR/baseline_size.txt
measure_column_sizes $DB_PATH > $RESULTS_DIR/baseline_columns.txt

echo "=== Scenario 1: Leaf-only Logs ==="
# Requires code modification - run with feature flag
cargo test --release -p mc-db --features leaf-only-logs bench_revert_10
cargo test --release -p mc-db --features leaf-only-logs bench_revert_100
cargo test --release -p mc-db --features leaf-only-logs bench_revert_1000

echo "=== Scenario 2: Squashed Diffs ==="
cargo test --release -p mc-db bench_squash_10
cargo test --release -p mc-db bench_squash_100
cargo test --release -p mc-db bench_squash_1000

echo "=== Scenario 3: Checkpoints ==="
cargo test --release -p mc-db bench_checkpoint_create
cargo test --release -p mc-db bench_checkpoint_restore
cargo test --release -p mc-db bench_hybrid_revert

echo "=== Results ==="
cat $RESULTS_DIR/*.txt
```

---

## Success Criteria

| Scenario | Acceptable | Good | Excellent |
|----------|------------|------|-----------|
| **Scenario 1: Size reduction** | >30% | >50% | >70% |
| **Scenario 1: Revert slowdown** | <500% | <300% | <200% |
| **Scenario 2: Squash speedup** | >20% | >50% | >70% |
| **Scenario 3: Deep revert speedup** | >500% | >1000% | >5000% |
| **Scenario 3: Storage efficiency** | Similar | -30% | -50% |

---

## Questions for Clarification

1. **Test network**: Should we benchmark on Sepolia (smaller, faster) or Mainnet (realistic)?

2. **Reorg depth priority**: What's the typical reorg depth we need to handle quickly?
   - < 10 blocks: Most common (network hiccups)
   - 10-100 blocks: Rare (consensus issues)
   - 100+ blocks: Very rare (catastrophic)

3. **Historical access patterns**: How often do we need proofs for very old blocks?
   - Recent (< 1000 blocks): Common (RPC queries)
   - Historical (1000-10000 blocks): Occasional
   - Deep history (> 10000 blocks): Rare

4. **Backup requirements**:
   - How often do operators backup the DB?
   - Is backup time a critical factor?

5. **Memory constraints**: What's the maximum memory budget for squashing operations?

---

## Next Steps

1. [ ] Set up isolated test environment with synced DB
2. [ ] Establish baseline measurements
3. [ ] Implement Scenario 1 (leaf-only) behind feature flag
4. [ ] Run Scenario 1 benchmarks
5. [ ] Implement Scenario 2 (squashing) as utility function
6. [ ] Run Scenario 2 benchmarks
7. [ ] Implement Scenario 3 (checkpoints) infrastructure
8. [ ] Run Scenario 3 benchmarks
9. [ ] Analyze results and make recommendations
10. [ ] Document findings and propose implementation plan

---

*Document version: 1.0*
*Created: 2025-02-01*
*Author: Claude Code*
