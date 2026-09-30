//! Durable root-keyed datasets with bounded memory and bounded cold proof reads.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataError, CommittedDataProvider, CommittedDataSet, CommittedDataWitness, MAX_COMMITTED_DATA_VALUES,
};
use std::collections::VecDeque;

/// Two imported trees at most; cold proof reads never wait on the import gate.
#[derive(Debug, Default)]
pub(crate) struct SnapshotCache {
    entries: Mutex<VecDeque<Arc<CommittedDataSet>>>,
    import: Mutex<()>,
}

impl SnapshotCache {
    fn get(&self, root: Felt, publisher: Felt) -> Result<Option<Arc<CommittedDataSet>>> {
        let cache = self.entries.lock().map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?;
        Ok(cache.iter().find(|tree| tree.root() == root && tree.publisher() == publisher).cloned())
    }

    fn insert(&self, tree: Arc<CommittedDataSet>) -> Result<()> {
        let mut cache = self.entries.lock().map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?;
        if !cache.iter().any(|old| old.root() == tree.root() && old.publisher() == tree.publisher()) {
            if cache.len() == 2 {
                cache.pop_front();
            }
            cache.push_back(tree);
        }
        Ok(())
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Reads only a fixed-height proof on cache misses, never rehashing a whole dataset.
    pub fn committed_data_witness(
        &self,
        root: Felt,
        publisher: Felt,
        index: u32,
    ) -> Result<Option<CommittedDataWitness>> {
        anyhow::ensure!((index as usize) < MAX_COMMITTED_DATA_VALUES, "Committed-data index out of range");
        if let Some(tree) = self.committed_data_cache.get(root, publisher)? {
            return Ok(tree.witness(index));
        }
        let witness = self.db.get_committed_data_witness(root, publisher, index)?;
        if let Some(witness) = &witness {
            anyhow::ensure!(
                (witness.root, witness.publisher, witness.index) == (root, publisher, index) && witness.verify(),
                "Stored committed-data witness mismatch"
            );
        }
        Ok(witness)
    }

    pub fn committed_data_provider(self: &Arc<Self>) -> Arc<dyn CommittedDataProvider> {
        Arc::new(BackendCommittedData(Arc::clone(self)))
    }
}

impl<D: MadaraStorage> MadaraBackend<D> {
    /// Authenticates and durably stores immutable data. Does not publish a root on-chain.
    /// Rejects concurrent imports instead of accumulating unbounded hashing tasks.
    pub fn import_committed_data_snapshot(&self, root: Felt, publisher: Felt, values: Vec<Felt>) -> Result<()> {
        let _build = self
            .committed_data_cache
            .import
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Committed-data import busy; retry import later"))?;
        let tree = Arc::new(CommittedDataSet::new(publisher, values)?);
        anyhow::ensure!(tree.root() == root, "Imported committed-data root mismatch");
        self.db.write_committed_data_dataset(&tree, self.chain_config().committed_data_max_storage_bytes)?;
        self.committed_data_cache.insert(tree)
    }
}

struct BackendCommittedData<D>(Arc<MadaraBackend<D>>);
impl<D> std::fmt::Debug for BackendCommittedData<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BackendCommittedData")
    }
}
impl<D: MadaraStorageRead> CommittedDataProvider for BackendCommittedData<D> {
    fn value(&self, root: Felt, publisher: Felt, index: u32) -> std::result::Result<Option<Felt>, CommittedDataError> {
        if let Some(tree) = self
            .0
            .committed_data_cache
            .get(root, publisher)
            .map_err(|error| CommittedDataError::Provider(error.to_string()))?
        {
            return Ok(tree.value(index));
        }
        self.0
            .committed_data_witness(root, publisher, index)
            .map(|witness| witness.map(|w| w.value))
            .map_err(|error| CommittedDataError::Provider(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::RocksDBConfig;

    #[tokio::test]
    async fn committed_data_snapshot_persists_and_authenticates_after_reopen() {
        let directory = tempfile::TempDir::new().unwrap();
        let publisher = Felt::from(12345_u32);
        let values = vec![Felt::from(123_u32), Felt::from(456_u32), Felt::MAX];
        let root = CommittedDataSet::new(publisher, values.clone()).unwrap().root();
        let open = || {
            MadaraBackend::open_rocksdb(
                directory.path(),
                Arc::new(ChainConfig::madara_test()),
                MadaraBackendConfig::default(),
                RocksDBConfig::default(),
                Arc::new(NativeConfig::default()),
            )
            .unwrap()
        };
        {
            let backend = open();
            assert!(backend.import_committed_data_snapshot(root + Felt::ONE, publisher, values.clone()).is_err());
            backend.import_committed_data_snapshot(root, publisher, values.clone()).unwrap();
            backend.import_committed_data_snapshot(root, publisher, values.clone()).unwrap();
            let witness = backend.committed_data_witness(root, publisher, 1).unwrap().unwrap();
            assert!(witness.verify());
            assert_eq!(witness.value, Felt::from(456_u32));
        }
        {
            let backend = open();
            // Cold reads must not need the import gate or reconstruct a whole tree.
            let importing = backend.committed_data_cache.import.lock().unwrap();
            assert!(backend.committed_data_witness(root, publisher, 2).unwrap().unwrap().verify());
            drop(importing);
            assert!(backend.committed_data_witness(root, publisher, 3).unwrap().is_none());
            assert!(backend.committed_data_witness(root, publisher + Felt::ONE, 1).unwrap().is_none());
            assert!(backend.committed_data_witness(root + Felt::ONE, publisher, 1).unwrap().is_none());
            // Simulate corrupt private data at rest; a cold read must not trust it.
            backend.committed_data_cache.entries.lock().unwrap().clear();
            let cf = backend.db.inner_db().cf_handle("meta").unwrap();
            let mut key = b"committed_data_pages_v1/".to_vec();
            key.extend_from_slice(&root.to_bytes_be());
            key.extend_from_slice(&publisher.to_bytes_be());
            key.extend_from_slice(&[0, 0, 0, 0, 0]);
            backend.db.inner_db().put_cf(&cf, &key, [0xff]).unwrap();
            assert!(backend.committed_data_witness(root, publisher, 0).is_err());
            backend.import_committed_data_snapshot(root, publisher, values.clone()).unwrap();
            backend.committed_data_cache.entries.lock().unwrap().clear();
            assert!(backend.committed_data_witness(root, publisher, 0).unwrap().unwrap().verify());
            backend.db.inner_db().delete_cf(&cf, key).unwrap();
            assert!(backend.committed_data_witness(root, publisher, 0).is_err());
        }
    }
    #[tokio::test]
    async fn committed_data_quota_is_durable_and_duplicate_imports_are_free() {
        let directory = tempfile::TempDir::new().unwrap();
        let publisher = Felt::ONE;
        let first = CommittedDataSet::new(publisher, vec![Felt::TWO]).unwrap();
        let second = CommittedDataSet::new(publisher, vec![Felt::from(3_u32)]).unwrap();
        let mut config = ChainConfig::madara_test();
        // One paged dataset fits, two do not (including metadata and keys).
        config.committed_data_max_storage_bytes = 3000;
        let open = || {
            MadaraBackend::open_rocksdb(
                directory.path(),
                Arc::new(config.clone()),
                MadaraBackendConfig::default(),
                RocksDBConfig::default(),
                Arc::new(NativeConfig::default()),
            )
            .unwrap()
        };
        {
            let backend = open();
            backend.import_committed_data_snapshot(first.root(), publisher, first.values().to_vec()).unwrap();
            backend.import_committed_data_snapshot(first.root(), publisher, first.values().to_vec()).unwrap();
            assert!(backend
                .import_committed_data_snapshot(second.root(), publisher, second.values().to_vec())
                .is_err());
            assert!(backend.committed_data_witness(second.root(), publisher, 0).unwrap().is_none());
        }
        let backend = open();
        assert!(backend.import_committed_data_snapshot(second.root(), publisher, second.values().to_vec()).is_err());
        assert!(backend.committed_data_witness(first.root(), publisher, 0).unwrap().unwrap().verify());
    }

    #[test]
    fn committed_data_cache_is_bounded_and_hot_reads_do_not_take_import_lock() {
        let cache = SnapshotCache::default();
        let trees: Vec<_> =
            (0..3_u32).map(|i| Arc::new(CommittedDataSet::new(Felt::ONE, vec![Felt::from(i)]).unwrap())).collect();
        for tree in &trees {
            cache.insert(tree.clone()).unwrap();
        }
        let _building = cache.import.lock().unwrap();
        assert!(cache.get(trees[0].root(), Felt::ONE).unwrap().is_none());
        assert!(cache.get(trees[2].root(), Felt::ONE).unwrap().is_some());
        assert_eq!(cache.entries.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn committed_data_concurrent_storage_writers_cannot_bypass_quota() {
        let directory = tempfile::TempDir::new().unwrap();
        let backend = MadaraBackend::open_rocksdb(
            directory.path(),
            Arc::new(ChainConfig::madara_test()),
            MadaraBackendConfig::default(),
            RocksDBConfig::default(),
            Arc::new(NativeConfig::default()),
        )
        .unwrap();
        let datasets = [
            CommittedDataSet::new(Felt::ONE, vec![Felt::TWO]).unwrap(),
            CommittedDataSet::new(Felt::ONE, vec![Felt::from(3_u32)]).unwrap(),
        ];
        let barrier = std::sync::Barrier::new(2);
        let successes = std::thread::scope(|scope| {
            let handles: Vec<_> = datasets
                .iter()
                .map(|dataset| {
                    let backend = &backend;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        backend.db.write_committed_data_dataset(dataset, 3000).is_ok()
                    })
                })
                .collect();
            handles.into_iter().map(|handle| usize::from(handle.join().unwrap())).sum::<usize>()
        });
        assert_eq!(successes, 1);
    }
}
