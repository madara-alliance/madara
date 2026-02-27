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

    /// Get the closest snapshot at or before the provided `block_n`.
    ///
    /// Returns `None` when the only available persisted snapshot is newer than `block_n`.
    /// For an empty chain (`head_block_n == None`), this returns the current head snapshot.
    #[tracing::instrument(skip(self))]
    pub fn get_at_or_before(&self, block_n: u64) -> Option<(Option<u64>, SnapshotRef)> {
        let inner = self.inner.read().expect("Poisoned lock");

        if let Some((snapshot_block_n, snapshot)) = inner.historical.range(..=block_n).next_back() {
            return Some((Some(*snapshot_block_n), Arc::clone(snapshot)));
        }

        match inner.head_block_n {
            Some(head_block_n) if head_block_n <= block_n => Some((Some(head_block_n), Arc::clone(&inner.head))),
            Some(_) => None,
            None => Some((None, Arc::clone(&inner.head))),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rocksdb::{RocksDBConfig, RocksDBStorage};
    use rstest::rstest;

    fn create_test_storage(config: RocksDBConfig) -> (tempfile::TempDir, RocksDBStorage) {
        let temp_dir = tempfile::TempDir::with_prefix("snapshot-test").unwrap();
        let storage = RocksDBStorage::open(temp_dir.path(), config).unwrap();
        (temp_dir, storage)
    }

    /// When `max_kept_snapshots = Some(0)`, no snapshots should be created in historical.
    /// This verifies that snapshots are effectively disabled with the new default.
    #[test]
    fn test_snapshots_disabled_when_zero() {
        let config = RocksDBConfig::default();
        assert_eq!(config.max_kept_snapshots, Some(0), "Default config should disable snapshots");
        let (_temp_dir, storage) = create_test_storage(config);

        // Simulate adding blocks at snapshot intervals
        for block_n in 0..20 {
            storage.snapshots.set_new_head(block_n);
        }

        // No historical snapshots should be created when max_kept_snapshots is 0
        assert_eq!(
            storage.snapshots.inner.read().expect("Poisoned lock").historical.len(),
            0,
            "No historical snapshots should exist when max_kept_snapshots=0"
        );

        // get_closest should still return the head snapshot
        let (block_n, _snapshot) = storage.snapshots.get_closest(10);
        assert_eq!(block_n, Some(19), "Should return head block when snapshots are disabled");
    }

    /// When `max_kept_snapshots = Some(n)`, only n snapshots should be kept.
    /// Oldest snapshots are removed first (FIFO behavior).
    #[test]
    fn test_snapshots_limited_to_max() {
        let config = RocksDBConfig { max_kept_snapshots: Some(3), snapshot_interval: 5, ..Default::default() };
        let (_temp_dir, storage) = create_test_storage(config);

        // Add blocks 0-24 (snapshots at 0, 5, 10, 15, 20)
        for block_n in 0..25 {
            storage.snapshots.set_new_head(block_n);
        }

        // Should have exactly 3 snapshots (the limit)
        assert_eq!(
            storage.snapshots.inner.read().expect("Poisoned lock").historical.len(),
            3,
            "Should have exactly max_kept_snapshots historical snapshots"
        );

        // The oldest snapshots (0, 5) should have been removed, keeping 10, 15, 20
        let (block_n, _) = storage.snapshots.get_closest(0);
        // Since snapshot at 0 is removed, get_closest(0) should return the next available (10)
        assert_eq!(block_n, Some(10), "Oldest snapshots should be removed first");

        // Verify the kept snapshots
        let (block_n, _) = storage.snapshots.get_closest(10);
        assert_eq!(block_n, Some(10), "Snapshot at block 10 should exist");

        let (block_n, _) = storage.snapshots.get_closest(15);
        assert_eq!(block_n, Some(15), "Snapshot at block 15 should exist");

        let (block_n, _) = storage.snapshots.get_closest(20);
        assert_eq!(block_n, Some(20), "Snapshot at block 20 should exist");
    }

    /// When `max_kept_snapshots = None`, snapshots accumulate without limit.
    #[test]
    fn test_snapshots_unlimited_when_none() {
        let config = RocksDBConfig { max_kept_snapshots: None, snapshot_interval: 5, ..Default::default() };
        let (_temp_dir, storage) = create_test_storage(config);

        // Add blocks 0-49 (snapshots at 0, 5, 10, 15, 20, 25, 30, 35, 40, 45)
        for block_n in 0..50 {
            storage.snapshots.set_new_head(block_n);
        }

        // Should have 10 snapshots (one every 5 blocks from 0 to 45)
        assert_eq!(
            storage.snapshots.inner.read().expect("Poisoned lock").historical.len(),
            10,
            "All snapshots should be retained when max_kept_snapshots=None"
        );

        // All snapshots should be accessible
        for expected_block in (0..50).step_by(5) {
            let (block_n, _) = storage.snapshots.get_closest(expected_block);
            assert_eq!(block_n, Some(expected_block), "Snapshot at block {} should exist", expected_block);
        }
    }

    #[test]
    fn test_get_at_or_before_rejects_newer_only_head_snapshot() {
        let config = RocksDBConfig::default();
        let (_temp_dir, storage) = create_test_storage(config);

        storage.snapshots.set_new_head(10);
        assert!(
            storage.snapshots.get_at_or_before(5).is_none(),
            "newer head-only snapshot must not satisfy older snapshot base request"
        );
    }

    #[rstest]
    #[case(10, Some(10))]
    #[case(11, Some(10))]
    fn test_get_at_or_before_returns_head_for_equal_or_newer_request(
        #[case] requested_block_n: u64,
        #[case] expected_snapshot_n: Option<u64>,
    ) {
        let config = RocksDBConfig::default();
        let (_temp_dir, storage) = create_test_storage(config);

        storage.snapshots.set_new_head(10);
        let (snapshot_block_n, _) = storage
            .snapshots
            .get_at_or_before(requested_block_n)
            .expect("head should be returned for equal/newer request");
        assert_eq!(snapshot_block_n, expected_snapshot_n);
    }

    #[test]
    fn test_get_at_or_before_prefers_historical_snapshot_when_available() {
        let config = RocksDBConfig { max_kept_snapshots: Some(5), snapshot_interval: 5, ..Default::default() };
        let (_temp_dir, storage) = create_test_storage(config);

        for block_n in 0..=20 {
            storage.snapshots.set_new_head(block_n);
        }

        let (snapshot_block_n, _) = storage.snapshots.get_at_or_before(17).expect("historical snapshot should exist");
        assert_eq!(snapshot_block_n, Some(15));
    }
}
