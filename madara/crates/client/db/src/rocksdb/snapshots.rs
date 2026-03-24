use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, RwLock},
};

use crate::rocksdb::{rocksdb_snapshot::SnapshotWithDBArc, RocksDBStorageInner};

pub type SnapshotRef = Arc<SnapshotWithDBArc>;

struct SnapshotsInner {
    exact: BTreeMap<u64, SnapshotRef>,
    historical: BTreeMap<u64, SnapshotRef>,
    empty_base: Option<SnapshotRef>,
    /// Current snapshot for the latest block.
    head: SnapshotRef,
    head_block_n: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotInventory {
    pub head_block_n: Option<u64>,
    pub exact_count: usize,
    pub historical_count: usize,
    pub oldest_exact: Option<u64>,
    pub newest_exact: Option<u64>,
    pub oldest_historical: Option<u64>,
    pub newest_historical: Option<u64>,
    pub has_empty_base: bool,
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
        let inner = self.inner.read().expect("Poisoned lock");
        write!(f, "<{} exact snapshots, {} interval snapshots>", inner.exact.len(), inner.historical.len())
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
        tracing::debug!(
            "initialized_db_snapshots head_block_n={head_block_n:?} max_kept_snapshots={max_kept_snapshots:?} snapshot_interval={snapshot_interval}"
        );
        Self {
            db,
            inner: SnapshotsInner {
                exact: Default::default(),
                historical: Default::default(),
                empty_base: head_block_n.is_none().then_some(Arc::clone(&head)),
                head,
                head_block_n,
            }
            .into(),
            max_kept_snapshots,
            snapshot_interval,
        }
    }

    fn save_interval_snapshot_locked(&self, inner: &mut SnapshotsInner, block_n: u64, snapshot: SnapshotRef) {
        tracing::debug!("Saving interval snapshot at {block_n:?}");
        inner.historical.insert(block_n, snapshot);

        let pruned_snapshot_block = if self.max_kept_snapshots.is_some_and(|n| inner.historical.len() > n) {
            inner.historical.pop_first().map(|(pruned_block_n, _)| pruned_block_n)
        } else {
            None
        };
        tracing::debug!(
            "db_snapshot_saved block_number={} historical_count={} exact_count={} oldest_snapshot={:?} newest_snapshot={:?} oldest_exact={:?} newest_exact={:?} pruned_snapshot={pruned_snapshot_block:?} max_kept_snapshots={:?} snapshot_interval={}",
            block_n,
            inner.historical.len(),
            inner.exact.len(),
            inner.historical.keys().next().copied(),
            inner.historical.keys().next_back().copied(),
            inner.exact.keys().next().copied(),
            inner.exact.keys().next_back().copied(),
            self.max_kept_snapshots,
            self.snapshot_interval
        );
    }

    /// Called when a new block has been added in the database. This will make a snapshot
    /// on top of the new block, and it will store that snapshot in the snapshot history every
    /// `snapshot_interval` blocks.
    #[tracing::instrument(skip(self))]
    pub fn set_new_head(&self, block_n: u64) {
        let snapshot = Arc::new(SnapshotWithDBArc::new(Arc::clone(&self.db)));

        let mut inner = self.inner.write().expect("Poisoned lock");

        if self.max_kept_snapshots != Some(0) && self.snapshot_interval != 0 && block_n % self.snapshot_interval == 0 {
            self.save_interval_snapshot_locked(&mut inner, block_n, Arc::clone(&snapshot));
        }

        // Update head snapshot
        inner.head = snapshot;
        inner.head_block_n = Some(block_n);
    }

    #[tracing::instrument(skip(self))]
    pub fn pin_head(&self, block_n: u64) {
        let mut inner = self.inner.write().expect("Poisoned lock");
        if inner.head_block_n != Some(block_n) {
            tracing::warn!(
                "db_snapshot_pin_skipped requested_block={} head_block_n={:?} exact_count={} historical_count={}",
                block_n,
                inner.head_block_n,
                inner.exact.len(),
                inner.historical.len()
            );
            return;
        }

        let head = Arc::clone(&inner.head);
        inner.exact.insert(block_n, head);
        tracing::debug!(
            "db_snapshot_pinned block_number={} exact_count={} oldest_exact={:?} newest_exact={:?} historical_count={} oldest_snapshot={:?} newest_snapshot={:?}",
            block_n,
            inner.exact.len(),
            inner.exact.keys().next().copied(),
            inner.exact.keys().next_back().copied(),
            inner.historical.len(),
            inner.historical.keys().next().copied(),
            inner.historical.keys().next_back().copied()
        );
    }

    #[tracing::instrument(skip(self))]
    pub fn rewind_to(&self, block_n: u64) {
        let snapshot = Arc::new(SnapshotWithDBArc::new(Arc::clone(&self.db)));
        let mut inner = self.inner.write().expect("Poisoned lock");
        inner.exact.retain(|saved_block_n, _| *saved_block_n <= block_n);
        inner.historical.retain(|saved_block_n, _| *saved_block_n <= block_n);
        inner.head = snapshot;
        inner.head_block_n = Some(block_n);
        tracing::debug!(
            "db_snapshots_rewound target_block={} exact_count={} historical_count={} oldest_exact={:?} newest_exact={:?} oldest_snapshot={:?} newest_snapshot={:?}",
            block_n,
            inner.exact.len(),
            inner.historical.len(),
            inner.exact.keys().next().copied(),
            inner.exact.keys().next_back().copied(),
            inner.historical.keys().next().copied(),
            inner.historical.keys().next_back().copied()
        );
    }

    #[tracing::instrument(skip(self))]
    pub fn get_exact(&self, block_n: Option<u64>) -> Option<SnapshotRef> {
        let inner = self.inner.read().expect("Poisoned lock");
        match block_n {
            None => inner.empty_base.as_ref().map(Arc::clone),
            Some(block_n) if inner.head_block_n == Some(block_n) => Some(Arc::clone(&inner.head)),
            Some(block_n) => inner.exact.get(&block_n).or_else(|| inner.historical.get(&block_n)).map(Arc::clone),
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn get_floor(&self, max_block_n: Option<u64>) -> Option<(Option<u64>, SnapshotRef)> {
        let inner = self.inner.read().expect("Poisoned lock");
        match max_block_n {
            None => inner.empty_base.as_ref().map(|snapshot| (None, Arc::clone(snapshot))),
            Some(max_block_n) => {
                if inner.head_block_n.is_some_and(|head_block_n| head_block_n <= max_block_n) {
                    return Some((inner.head_block_n, Arc::clone(&inner.head)));
                }

                let exact =
                    inner.exact.range(..=max_block_n).next_back().map(|(block_n, snapshot)| (*block_n, snapshot));
                let historical =
                    inner.historical.range(..=max_block_n).next_back().map(|(block_n, snapshot)| (*block_n, snapshot));

                match (exact, historical) {
                    (Some((exact_block_n, exact_snapshot)), Some((historical_block_n, historical_snapshot))) => {
                        if exact_block_n >= historical_block_n {
                            Some((Some(exact_block_n), Arc::clone(exact_snapshot)))
                        } else {
                            Some((Some(historical_block_n), Arc::clone(historical_snapshot)))
                        }
                    }
                    (Some((block_n, snapshot)), None) | (None, Some((block_n, snapshot))) => {
                        Some((Some(block_n), Arc::clone(snapshot)))
                    }
                    (None, None) => inner.empty_base.as_ref().map(|snapshot| (None, Arc::clone(snapshot))),
                }
            }
        }
    }

    /// Return the latest snapshot floor that is known to be a durable trie base.
    ///
    /// In parallel-merkle mode, only pinned exact snapshots and the empty-base snapshot
    /// are safe as root-computation bases. Head/historical snapshots may reflect block
    /// part writes for non-boundary blocks whose trie overlay has not been flushed yet.
    #[tracing::instrument(skip(self))]
    pub fn get_durable_floor(&self, max_block_n: Option<u64>) -> Option<(Option<u64>, SnapshotRef)> {
        let inner = self.inner.read().expect("Poisoned lock");
        match max_block_n {
            None => inner.empty_base.as_ref().map(|snapshot| (None, Arc::clone(snapshot))),
            Some(max_block_n) => inner
                .exact
                .range(..=max_block_n)
                .next_back()
                .map(|(block_n, snapshot)| (Some(*block_n), Arc::clone(snapshot)))
                .or_else(|| inner.empty_base.as_ref().map(|snapshot| (None, Arc::clone(snapshot)))),
        }
    }

    pub fn inventory(&self) -> SnapshotInventory {
        let inner = self.inner.read().expect("Poisoned lock");
        SnapshotInventory {
            head_block_n: inner.head_block_n,
            exact_count: inner.exact.len(),
            historical_count: inner.historical.len(),
            oldest_exact: inner.exact.keys().next().copied(),
            newest_exact: inner.exact.keys().next_back().copied(),
            oldest_historical: inner.historical.keys().next().copied(),
            newest_historical: inner.historical.keys().next_back().copied(),
            has_empty_base: inner.empty_base.is_some(),
        }
    }

    /// Get the closest snapshot that had been made at or after the provided `block_n`.
    /// Also returns the block_n, which can be null if no block is in database in that snapshot.
    #[tracing::instrument(skip(self))]
    pub fn get_closest(&self, block_n: u64) -> (Option<u64>, SnapshotRef) {
        tracing::debug!("get closest {block_n:?} {self:?}");
        let inner = self.inner.read().expect("Poisoned lock");
        let historical_count = inner.historical.len();
        let oldest_snapshot = inner.historical.keys().next().copied();
        let newest_snapshot = inner.historical.keys().next_back().copied();
        // We want the closest snapshot that is younger than this block_n.
        let (selected_block_n, snapshot_ref, source) = inner
            .historical
            .range(&block_n..)
            .next()
            .map(|(block_n, snapshot)| (Some(*block_n), Arc::clone(snapshot), "historical"))
            // If none was found, this means that we are asking for a block that's between the last snapshot and the current latest block, or
            // snapshots are disabled. In these cases we want to return the snapshot for the current latest block.
            .unwrap_or_else(|| (inner.head_block_n, Arc::clone(&inner.head), "head"));
        if selected_block_n.is_some_and(|selected| selected > block_n) {
            tracing::debug!(
                "db_snapshot_selected_after_requested requested_block={} selected_snapshot_block={selected_block_n:?} source={} head_block_n={:?} historical_count={} oldest_snapshot={oldest_snapshot:?} newest_snapshot={newest_snapshot:?} max_kept_snapshots={:?} snapshot_interval={}",
                block_n,
                source,
                inner.head_block_n,
                historical_count,
                self.max_kept_snapshots,
                self.snapshot_interval
            );
        }
        (selected_block_n, snapshot_ref)
    }
}

#[cfg(test)]
mod tests {
    use crate::rocksdb::{RocksDBConfig, RocksDBStorage};

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
        assert!(storage.snapshots.get_exact(None).is_some(), "Empty-base snapshot should be preserved");

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
    fn test_exact_snapshot_survives_head_advance_when_pinned() {
        let config = RocksDBConfig::default();
        let (_temp_dir, storage) = create_test_storage(config);

        storage.snapshots.set_new_head(0);
        storage.snapshots.pin_head(0);
        storage.snapshots.set_new_head(1);

        assert!(storage.snapshots.get_exact(Some(0)).is_some(), "Pinned exact snapshot should survive head advance");
        assert_eq!(storage.snapshots.inventory().exact_count, 1);
    }

    #[test]
    fn test_rewind_prunes_future_exact_and_interval_snapshots() {
        let config = RocksDBConfig { max_kept_snapshots: Some(5), snapshot_interval: 1, ..Default::default() };
        let (_temp_dir, storage) = create_test_storage(config);

        for block_n in 0..=4 {
            storage.snapshots.set_new_head(block_n);
        }
        storage.snapshots.pin_head(4);
        storage.snapshots.set_new_head(5);
        storage.snapshots.pin_head(5);

        storage.snapshots.rewind_to(4);

        assert!(storage.snapshots.get_exact(Some(5)).is_none(), "Future exact snapshot should be pruned");
        assert!(storage.snapshots.get_exact(Some(4)).is_some(), "Target exact snapshot should remain available");
        let inventory = storage.snapshots.inventory();
        assert_eq!(inventory.head_block_n, Some(4));
        assert_eq!(inventory.newest_exact, Some(4));
        assert_eq!(inventory.newest_historical, Some(4));
    }

    #[test]
    fn test_durable_floor_ignores_non_exact_head_and_historical_snapshots() {
        let config = RocksDBConfig { max_kept_snapshots: Some(5), snapshot_interval: 1, ..Default::default() };
        let (_temp_dir, storage) = create_test_storage(config);

        storage.snapshots.set_new_head(670723);
        storage.snapshots.pin_head(670723);
        storage.snapshots.set_new_head(670724);
        storage.snapshots.pin_head(670724);
        storage.snapshots.set_new_head(670725);

        let (block_n, _) = storage.snapshots.get_floor(Some(670725)).expect("any floor snapshot");
        assert_eq!(block_n, Some(670725), "generic floor lookup should still see the head snapshot");

        let (block_n, _) = storage.snapshots.get_durable_floor(Some(670725)).expect("durable floor snapshot");
        assert_eq!(block_n, Some(670724), "durable floor must ignore non-exact head snapshots");
    }
}
