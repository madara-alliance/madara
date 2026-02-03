use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, RwLock},
};

use crate::rocksdb::{rocksdb_snapshot::SnapshotWithDBArc, RocksDBStorageInner};

pub type SnapshotRef = Arc<SnapshotWithDBArc>;

struct SnapshotsInner {
    historical: BTreeMap<u64, SnapshotRef>,
    /// Current snapshot for the latest block.
    head: SnapshotRef,
    head_block_n: Option<u64>,
}

/// This struct holds the snapshots. To avoid holding the lock the entire time the snapshot is used, it's behind
/// an Arc. Getting a snapshot only holds the lock for the time of cloning the Arc.
pub struct Snapshots {
    inner: RwLock<SnapshotsInner>,
    db: Arc<RocksDBStorageInner>,
    max_kept_snapshots: Option<usize>,
    snapshot_interval: u64,
}
impl fmt::Debug for Snapshots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} snapshots>", self.inner.read().expect("Poisoned lock").historical.len())
    }
}

impl Snapshots {
    pub fn new(
        db: Arc<RocksDBStorageInner>,
        head_block_n: Option<u64>,
        max_kept_snapshots: Option<usize>,
        snapshot_interval: u64,
    ) -> Self {
        let head = Arc::new(SnapshotWithDBArc::new(Arc::clone(&db)));
        Self {
            db,
            inner: SnapshotsInner { historical: Default::default(), head, head_block_n }.into(),
            max_kept_snapshots,
            snapshot_interval,
        }
    }

    /// Called when a new block has been added in the database. This will make a snapshot
    /// on top of the new block, and it will store that snapshot in the snapshot history every
    /// `snapshot_interval` blocks.
    #[tracing::instrument(skip(self))]
    pub fn set_new_head(&self, block_n: u64) {
        let snapshot = Arc::new(SnapshotWithDBArc::new(Arc::clone(&self.db)));

        let mut inner = self.inner.write().expect("Poisoned lock");

        if self.max_kept_snapshots != Some(0) && self.snapshot_interval != 0 && block_n % self.snapshot_interval == 0 {
            tracing::debug!("Saving snapshot at {block_n:?}");
            inner.historical.insert(block_n, Arc::clone(&snapshot));

            // remove the oldest snapshot
            if self.max_kept_snapshots.is_some_and(|n| inner.historical.len() > n) {
                inner.historical.pop_first();
            }
        }

        // Update head snapshot
        inner.head = snapshot;
        inner.head_block_n = Some(block_n);
    }

    /// Get the closest snapshot that had been made at or after the provided `block_n`.
    /// Also returns the block_n, which can be null if no block is in database in that snapshot.
    #[tracing::instrument(skip(self))]
    pub fn get_closest(&self, block_n: u64) -> (Option<u64>, SnapshotRef) {
        tracing::debug!("get closest {block_n:?} {self:?}");
        let inner = self.inner.read().expect("Poisoned lock");
        // We want the closest snapshot that is younger than this block_n.
        inner
            .historical
            .range(&block_n..)
            .next()
            .map(|(block_n, snapshot)| (Some(*block_n), Arc::clone(snapshot)))
            // If none was found, this means that we are asking for a block that's between the last snapshot and the current latest block, or
            // snapshots are disabled. In these cases we want to return the snapshot for the current latest block.
            .unwrap_or_else(|| (inner.head_block_n, Arc::clone(&inner.head)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::{column::ALL_COLUMNS, options::rocksdb_global_options, RocksDBConfig};
    use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, WriteOptions};

    /// Helper to create a test RocksDBStorageInner with a temporary directory.
    fn create_test_db() -> (tempfile::TempDir, Arc<RocksDBStorageInner>) {
        let temp_dir = tempfile::TempDir::with_prefix("snapshot-test").unwrap();
        let config = RocksDBConfig::default();
        let opts = rocksdb_global_options(&config).unwrap();
        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(
            &opts,
            temp_dir.path(),
            ALL_COLUMNS.iter().map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(&config))),
        )
        .unwrap();
        let writeopts = WriteOptions::default();
        let inner = Arc::new(RocksDBStorageInner { db, writeopts, config });
        (temp_dir, inner)
    }

    /// When `max_kept_snapshots = Some(0)`, no snapshots should be created in historical.
    /// This verifies that snapshots are effectively disabled with the new default.
    #[test]
    fn test_snapshots_disabled_when_zero() {
        let (_temp_dir, db) = create_test_db();

        // Create snapshots with max_kept_snapshots = Some(0) (disabled)
        let snapshots = Snapshots::new(db, None, Some(0), 5);

        // Simulate adding blocks at snapshot intervals
        for block_n in 0..20 {
            snapshots.set_new_head(block_n);
        }

        // No historical snapshots should be created when max_kept_snapshots is 0
        assert_eq!(
            snapshots.inner.read().expect("Poisoned lock").historical.len(),
            0,
            "No historical snapshots should exist when max_kept_snapshots=0"
        );

        // get_closest should still return the head snapshot
        let (block_n, _snapshot) = snapshots.get_closest(10);
        assert_eq!(block_n, Some(19), "Should return head block when snapshots are disabled");
    }

    /// When `max_kept_snapshots = Some(n)`, only n snapshots should be kept.
    /// Oldest snapshots are removed first (FIFO behavior).
    #[test]
    fn test_snapshots_limited_to_max() {
        let (_temp_dir, db) = create_test_db();

        // Create snapshots with max_kept_snapshots = Some(3), interval = 5
        let snapshots = Snapshots::new(db, None, Some(3), 5);

        // Add blocks 0-24 (snapshots at 0, 5, 10, 15, 20)
        for block_n in 0..25 {
            snapshots.set_new_head(block_n);
        }

        // Should have exactly 3 snapshots (the limit)
        assert_eq!(
            snapshots.inner.read().expect("Poisoned lock").historical.len(),
            3,
            "Should have exactly max_kept_snapshots historical snapshots"
        );

        // The oldest snapshots (0, 5) should have been removed, keeping 10, 15, 20
        let (block_n, _) = snapshots.get_closest(0);
        // Since snapshot at 0 is removed, get_closest(0) should return the next available (10)
        assert_eq!(block_n, Some(10), "Oldest snapshots should be removed first");

        // Verify the kept snapshots
        let (block_n, _) = snapshots.get_closest(10);
        assert_eq!(block_n, Some(10), "Snapshot at block 10 should exist");

        let (block_n, _) = snapshots.get_closest(15);
        assert_eq!(block_n, Some(15), "Snapshot at block 15 should exist");

        let (block_n, _) = snapshots.get_closest(20);
        assert_eq!(block_n, Some(20), "Snapshot at block 20 should exist");
    }

    /// When `max_kept_snapshots = None`, snapshots accumulate without limit.
    #[test]
    fn test_snapshots_unlimited_when_none() {
        let (_temp_dir, db) = create_test_db();

        // Create snapshots with max_kept_snapshots = None (unlimited), interval = 5
        let snapshots = Snapshots::new(db, None, None, 5);

        // Add blocks 0-49 (snapshots at 0, 5, 10, 15, 20, 25, 30, 35, 40, 45)
        for block_n in 0..50 {
            snapshots.set_new_head(block_n);
        }

        // Should have 10 snapshots (one every 5 blocks from 0 to 45)
        assert_eq!(
            snapshots.inner.read().expect("Poisoned lock").historical.len(),
            10,
            "All snapshots should be retained when max_kept_snapshots=None"
        );

        // All snapshots should be accessible
        for expected_block in (0..50).step_by(5) {
            let (block_n, _) = snapshots.get_closest(expected_block);
            assert_eq!(block_n, Some(expected_block), "Snapshot at block {} should exist", expected_block);
        }
    }

    /// Verify snapshot_interval behavior: snapshots are only created at interval boundaries.
    #[test]
    fn test_snapshot_interval_behavior() {
        let (_temp_dir, db) = create_test_db();

        // Create snapshots with max_kept_snapshots = None, interval = 10
        let snapshots = Snapshots::new(db, None, None, 10);

        // Add blocks 0-35
        for block_n in 0..36 {
            snapshots.set_new_head(block_n);
        }

        // Should have 4 snapshots (at 0, 10, 20, 30)
        assert_eq!(
            snapshots.inner.read().expect("Poisoned lock").historical.len(),
            4,
            "Snapshots should only be created at interval boundaries"
        );

        // Verify snapshots at exact intervals
        let (block_n, _) = snapshots.get_closest(0);
        assert_eq!(block_n, Some(0));

        let (block_n, _) = snapshots.get_closest(10);
        assert_eq!(block_n, Some(10));

        // Block 15 should return snapshot at 20 (closest >= 15)
        let (block_n, _) = snapshots.get_closest(15);
        assert_eq!(block_n, Some(20), "get_closest should return the next available snapshot");
    }
}
