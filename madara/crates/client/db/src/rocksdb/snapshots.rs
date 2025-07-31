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
    pub fn new(db: Arc<RocksDBStorageInner>, head_block_n: Option<u64>, max_kept_snapshots: Option<usize>, snapshot_interval: u64) -> Self {
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
