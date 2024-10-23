use crate::{rocksdb_snapshot::SnapshotWithDBArc, DB};
use bonsai_trie::id::BasicId;
use std::{
    collections::{btree_map, BTreeMap},
    fmt,
    sync::{Arc, RwLock},
};

pub struct Snapshots {
    db: Arc<DB>,
    snapshots: RwLock<BTreeMap<BasicId, Arc<SnapshotWithDBArc<DB>>>>,
    max_saved_snapshots: Option<usize>,
}
impl fmt::Debug for Snapshots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} snapshots>", self.snapshots.read().expect("Poisoned lock").len())
    }
}

impl Snapshots {
    pub fn new(db: Arc<DB>, max_saved_snapshots: Option<usize>) -> Self {
        Self { db, snapshots: Default::default(), max_saved_snapshots }
    }

    pub fn create_new(&self, id: BasicId) {
        let mut snapshots = self.snapshots.write().expect("Poisoned lock");

        if let btree_map::Entry::Vacant(entry) = snapshots.entry(id) {
            log::debug!("Making snap at {id:?}");
            entry.insert(Arc::new(SnapshotWithDBArc::new(Arc::clone(&self.db))));

            if let Some(max_saved_snapshots) = self.max_saved_snapshots {
                if snapshots.len() > max_saved_snapshots {
                    snapshots.pop_first();
                }
            }
        }
    }

    pub fn get_closest(&self, id: BasicId) -> Option<(BasicId, Arc<SnapshotWithDBArc<DB>>)> {
        log::debug!("get closest {id:?} {self:?}");
        let snapshots = self.snapshots.read().expect("Poisoned lock");
        snapshots.range(..&id).next().map(|(id, snapshot)| (*id, Arc::clone(snapshot)))
    }
}
