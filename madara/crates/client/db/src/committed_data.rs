//! Durable root-keyed datasets with bounded memory and serialized cold reconstruction.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataError, CommittedDataProvider, CommittedDataSet, CommittedDataWitness, MAX_COMMITTED_DATA_VALUES,
};
use std::collections::VecDeque;

/// Two full trees at most; cold reconstruction never holds the hot-cache mutex.
#[derive(Debug, Default)]
pub(crate) struct SnapshotCache {
    entries: Mutex<VecDeque<Arc<CommittedDataSet>>>,
    reconstruction: Mutex<()>,
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
    /// Loads and authenticates a dataset before serving any value from it.
    pub fn committed_data_snapshot(&self, root: Felt, publisher: Felt) -> Result<Option<Arc<CommittedDataSet>>> {
        if let Some(tree) = self.committed_data_cache.get(root, publisher)? {
            return Ok(Some(tree));
        }
        let _build = self
            .committed_data_cache
            .reconstruction
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data reconstruction lock poisoned"))?;
        // Another reader may have completed the same reconstruction while we waited.
        if let Some(tree) = self.committed_data_cache.get(root, publisher)? {
            return Ok(Some(tree));
        }
        let Some(values) = self.db.get_committed_data_values(root, publisher)? else {
            return Ok(None);
        };
        let tree = Arc::new(CommittedDataSet::new(publisher, values)?);
        anyhow::ensure!(tree.root() == root, "Stored committed-data root mismatch");
        self.committed_data_cache.insert(Arc::clone(&tree))?;
        Ok(Some(tree))
    }

    pub fn committed_data_witness(
        &self,
        root: Felt,
        publisher: Felt,
        index: u32,
    ) -> Result<Option<CommittedDataWitness>> {
        anyhow::ensure!((index as usize) < MAX_COMMITTED_DATA_VALUES, "Committed-data index out of range");
        Ok(self.committed_data_snapshot(root, publisher)?.and_then(|tree| tree.witness(index)))
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
            .reconstruction
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Committed-data reconstruction busy; retry import later"))?;
        let tree = Arc::new(CommittedDataSet::new(publisher, values)?);
        anyhow::ensure!(tree.root() == root, "Imported committed-data root mismatch");
        self.db.write_committed_data_values(
            root,
            publisher,
            tree.values(),
            self.chain_config().committed_data_max_storage_bytes,
        )?;
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
        self.0
            .committed_data_snapshot(root, publisher)
            .map(|tree| tree.and_then(|tree| tree.value(index)))
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
            assert!(backend.committed_data_witness(root, publisher, 2).unwrap().unwrap().verify());
            assert!(backend.committed_data_witness(root, publisher, 3).unwrap().is_none());
            assert!(backend.committed_data_witness(root, publisher + Felt::ONE, 1).unwrap().is_none());
            assert!(backend.committed_data_witness(root + Felt::ONE, publisher, 1).unwrap().is_none());
            // Simulate corrupt private data at rest; a cold read must not trust it.
            backend.committed_data_cache.entries.lock().unwrap().clear();
            let cf = backend.db.inner_db().cf_handle("meta").unwrap();
            let mut key = b"committed_data_snapshot_v1/".to_vec();
            key.extend_from_slice(&root.to_bytes_be());
            key.extend_from_slice(&publisher.to_bytes_be());
            backend.db.inner_db().put_cf(&cf, key, [0xff]).unwrap();
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
        // One record fits, two do not (record includes publisher/root key and one felt).
        config.committed_data_max_storage_bytes = 150;
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
            assert!(backend.committed_data_snapshot(second.root(), publisher).unwrap().is_none());
        }
        let backend = open();
        assert!(backend.import_committed_data_snapshot(second.root(), publisher, second.values().to_vec()).is_err());
        assert!(backend.committed_data_witness(first.root(), publisher, 0).unwrap().unwrap().verify());
    }

    #[test]
    fn committed_data_cache_is_bounded_and_hot_reads_do_not_take_reconstruction_lock() {
        let cache = SnapshotCache::default();
        let trees: Vec<_> =
            (0..3_u32).map(|i| Arc::new(CommittedDataSet::new(Felt::ONE, vec![Felt::from(i)]).unwrap())).collect();
        for tree in &trees {
            cache.insert(tree.clone()).unwrap();
        }
        let _building = cache.reconstruction.lock().unwrap();
        assert!(cache.get(trees[0].root(), Felt::ONE).unwrap().is_none());
        assert!(cache.get(trees[2].root(), Felt::ONE).unwrap().is_some());
        assert_eq!(cache.entries.lock().unwrap().len(), 2);
    }
}
