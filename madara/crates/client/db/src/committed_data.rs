//! Durable root-keyed datasets with bounded memory and bounded cold proof reads.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataError, CommittedDataProvider, CommittedDataSet, CommittedDataWitness, MAX_COMMITTED_DATA_VALUES,
};
use std::collections::VecDeque;
use tokio::sync::Semaphore;

/// Two imported trees at most; cold proof reads never wait on the import gate.
#[derive(Debug)]
pub(crate) struct SnapshotCache {
    entries: Mutex<VecDeque<Arc<CommittedDataSet>>>,
    import: Arc<Semaphore>,
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self { entries: Mutex::new(VecDeque::new()), import: Arc::new(Semaphore::new(1)) }
    }
}

impl SnapshotCache {
    /// Returns a shared immutable tree without hashing or waiting for imports.
    fn get(&self, root: Felt) -> Result<Option<Arc<CommittedDataSet>>> {
        let cache = self.entries.lock().map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?;
        Ok(cache.iter().find(|tree| tree.root() == root).cloned())
    }

    /// Retains two distinct roots in insertion order; duplicate imports do not reorder them.
    fn insert(&self, tree: Arc<CommittedDataSet>) -> Result<()> {
        let mut cache = self.entries.lock().map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?;
        if !cache.iter().any(|old| old.root() == tree.root()) {
            if cache.len() == 2 {
                cache.pop_front();
            }
            cache.push_back(tree);
        }
        Ok(())
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Returns an authenticated proof for an imported root and occupied index.
    ///
    /// Missing roots/unused leaves return `None`; out-of-range indices, corrupt records and
    /// mismatched proofs return an error. Cold reads have fixed-height I/O and never rebuild
    /// a tree. This is synchronous storage work; async callers must use a blocking worker.
    pub fn committed_data_witness(&self, root: Felt, index: u32) -> Result<Option<CommittedDataWitness>> {
        anyhow::ensure!((index as usize) < MAX_COMMITTED_DATA_VALUES, "Committed-data index out of range");
        if let Some(tree) = self.committed_data_cache.get(root)? {
            return Ok(tree.witness(index));
        }
        let witness = self.db.get_committed_data_witness(root, index)?;
        if let Some(witness) = &witness {
            anyhow::ensure!(
                (witness.root, witness.index) == (root, index) && witness.verify(),
                "Stored committed-data witness mismatch"
            );
        }
        Ok(witness)
    }

    /// Creates the shared execution provider; cold values are authenticated before returning.
    /// The Cairo OS independently verifies witnesses during proving.
    pub fn committed_data_provider(self: &Arc<Self>) -> Arc<dyn CommittedDataProvider> {
        Arc::new(BackendCommittedData(Arc::clone(self)))
    }
}

impl<D: MadaraStorage> MadaraBackend<D> {
    /// Authenticates a dataset, durably writes its pages, then makes it available to execution.
    ///
    /// Rejects another admitted import before enqueuing blocking work. The permit stays with
    /// the worker if its async caller is cancelled; an accepted import may still complete.
    /// Success means a synchronous WAL write completed. Re-importing an identical dataset
    /// repairs its pages without charging quota twice. This never publishes a root on-chain.
    ///
    /// Returns an error for busy admission, invalid data/root, exhausted quota or storage failure.
    pub async fn import_committed_data_snapshot(self: &Arc<Self>, root: Felt, values: Vec<Felt>) -> Result<()> {
        let permit = Arc::clone(&self.committed_data_cache.import)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("Committed-data import busy; retry import later"))?;
        let backend = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let tree = Arc::new(CommittedDataSet::new(values)?);
            anyhow::ensure!(tree.root() == root, "Imported committed-data root mismatch");
            backend.db.write_committed_data_dataset(&tree, backend.chain_config().committed_data_max_storage_bytes)?;
            backend.committed_data_cache.insert(tree)
        })
        .await?
    }
}

struct BackendCommittedData<D>(Arc<MadaraBackend<D>>);
impl<D> std::fmt::Debug for BackendCommittedData<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BackendCommittedData")
    }
}
impl<D: MadaraStorageRead> CommittedDataProvider for BackendCommittedData<D> {
    fn value(&self, root: Felt, index: u32) -> std::result::Result<Option<Felt>, CommittedDataError> {
        if let Some(tree) =
            self.0.committed_data_cache.get(root).map_err(|error| CommittedDataError::Provider(error.to_string()))?
        {
            return Ok(tree.value(index));
        }
        self.0
            .committed_data_witness(root, index)
            .map(|witness| witness.map(|w| w.value))
            .map_err(|error| CommittedDataError::Provider(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::RocksDBConfig;

    #[tokio::test]
    async fn committed_data_catalog_pages_existing_roots_and_values_without_schema_rewrite() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mut roots = Vec::new();
        for value in 0..65_u32 {
            let values = vec![Felt::from(value)];
            let root = CommittedDataSet::new(values.clone()).unwrap().root();
            backend.import_committed_data_snapshot(root, values).await.unwrap();
            roots.push(root);
        }
        roots.sort_unstable();
        assert_eq!(backend.db.list_committed_data_roots(None).unwrap(), roots[..64]);
        assert_eq!(backend.db.list_committed_data_roots(Some(roots[63])).unwrap(), roots[64..]);
        assert!(backend.db.list_committed_data_roots(Some(roots[64])).unwrap().is_empty());
        let values: Vec<_> = (0..4097_u32).map(Felt::from).collect();
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
        assert_eq!(backend.db.get_committed_data_page(root, 0).unwrap(), Some((4097, values[..4096].to_vec())));
        assert_eq!(backend.db.get_committed_data_page(root, 4096).unwrap(), Some((4097, values[4096..].to_vec())));
        assert!(backend.db.get_committed_data_page(root, 1).is_err());
        assert!(backend.db.get_committed_data_page(root, 8192).is_err());
    }

    #[tokio::test]
    async fn committed_data_snapshot_persists_and_authenticates_after_reopen() {
        let directory = tempfile::TempDir::new().unwrap();
        let values = vec![Felt::from(123_u32), Felt::from(456_u32), Felt::MAX];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
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
            backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
            let witness = backend.committed_data_witness(root, 1).unwrap().unwrap();
            assert!(witness.verify());
            assert_eq!(witness.value, Felt::from(456_u32));
        }
        {
            let backend = open();
            // Cold reads must not need the import gate or reconstruct a whole tree.
            let importing = backend.committed_data_cache.import.try_acquire().unwrap();
            assert!(backend.committed_data_witness(root, 2).unwrap().unwrap().verify());
            drop(importing);
            assert!(backend.committed_data_witness(root, 3).unwrap().is_none());
            assert!(backend.committed_data_witness(root + Felt::ONE, 1).unwrap().is_none());
        }
    }
    #[tokio::test]
    async fn committed_data_cold_read_rejects_a_canonical_but_unauthenticated_value() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let values = vec![Felt::TWO];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, values).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let mut key = b"committed_data_pages_v2/".to_vec();
        key.extend_from_slice(&root.to_bytes_be());
        key.extend_from_slice(&[0, 0, 0, 0, 0]);
        // Correct encoding and length must not be confused with authentication.
        backend.db.inner_db().put_cf(&cf, key, Felt::ONE.to_bytes_be()).unwrap();
        let error = backend.committed_data_witness(root, 0).unwrap_err();
        assert!(error.to_string().contains("Stored committed-data witness mismatch"));
    }

    #[tokio::test]
    async fn committed_data_import_rejects_root_mismatch_without_writing() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let values = vec![Felt::TWO];
        let wrong_root = CommittedDataSet::new(values.clone()).unwrap().root() + Felt::ONE;
        assert!(backend.import_committed_data_snapshot(wrong_root, values).await.is_err());
        assert!(backend.committed_data_witness(wrong_root, 0).unwrap().is_none());
    }

    #[tokio::test]
    async fn committed_data_reimport_repairs_malformed_pages() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let values = vec![Felt::TWO];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let mut key = b"committed_data_pages_v2/".to_vec();
        key.extend_from_slice(&root.to_bytes_be());
        key.extend_from_slice(&[0, 0, 0, 0, 0]);
        backend.db.inner_db().put_cf(&cf, key, [0xff]).unwrap();
        assert!(backend.committed_data_witness(root, 0).is_err());
        backend.import_committed_data_snapshot(root, values).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        assert!(backend.committed_data_witness(root, 0).unwrap().unwrap().verify());
    }

    #[test]
    fn committed_data_import_rejects_a_second_request_before_blocking_work_starts() {
        tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap().block_on(
            async {
                let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
                let dataset = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
                let (release, wait) = std::sync::mpsc::channel();
                let (started, ready) = tokio::sync::oneshot::channel();
                let blocker = tokio::task::spawn_blocking(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
                });
                ready.await.unwrap();
                let first = backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec());
                tokio::pin!(first);
                assert!(futures::poll!(&mut first).is_pending());
                // The first import is queued, not running. The second must fail without joining it.
                let second = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec()),
                )
                .await;
                release.send(()).unwrap();
                blocker.await.unwrap();
                first.await.unwrap();
                assert!(second
                    .expect("busy import must not wait for a blocking worker")
                    .unwrap_err()
                    .to_string()
                    .contains("import busy"));
            },
        );
    }

    #[tokio::test]
    async fn committed_data_quota_is_durable_and_duplicate_imports_are_free() {
        let directory = tempfile::TempDir::new().unwrap();
        let first = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
        let second = CommittedDataSet::new(vec![Felt::from(3_u32)]).unwrap();
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
            backend.import_committed_data_snapshot(first.root(), first.values().to_vec()).await.unwrap();
            backend.import_committed_data_snapshot(first.root(), first.values().to_vec()).await.unwrap();
            assert!(backend.import_committed_data_snapshot(second.root(), second.values().to_vec()).await.is_err());
            assert!(backend.committed_data_witness(second.root(), 0).unwrap().is_none());
        }
        let backend = open();
        assert!(backend.import_committed_data_snapshot(second.root(), second.values().to_vec()).await.is_err());
        assert!(backend.committed_data_witness(first.root(), 0).unwrap().unwrap().verify());
    }

    #[tokio::test]
    async fn committed_data_legacy_storage_usage_still_counts_toward_quota() {
        let directory = tempfile::TempDir::new().unwrap();
        let mut config = ChainConfig::madara_test();
        config.committed_data_max_storage_bytes = 3000;
        let backend = MadaraBackend::open_rocksdb(
            directory.path(),
            Arc::new(config),
            MadaraBackendConfig::default(),
            RocksDBConfig::default(),
            Arc::new(NativeConfig::default()),
        )
        .unwrap();
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        // Simulate retained v1 records already consuming the quota before migration.
        backend.db.inner_db().put_cf(&cf, b"committed_data_pages_usage_v1", 3000_u64.to_be_bytes()).unwrap();
        let dataset = CommittedDataSet::new(vec![Felt::TWO]).unwrap();
        assert!(backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec()).await.is_err());
        assert!(backend.committed_data_witness(dataset.root(), 0).unwrap().is_none());
    }

    #[test]
    fn committed_data_cache_is_bounded_and_hot_reads_do_not_take_import_lock() {
        let cache = SnapshotCache::default();
        let trees: Vec<_> = (0..3_u32).map(|i| Arc::new(CommittedDataSet::new(vec![Felt::from(i)]).unwrap())).collect();
        for tree in &trees {
            cache.insert(tree.clone()).unwrap();
        }
        let _building = cache.import.try_acquire().unwrap();
        assert!(cache.get(trees[0].root()).unwrap().is_none());
        assert!(cache.get(trees[2].root()).unwrap().is_some());
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
        let datasets =
            [CommittedDataSet::new(vec![Felt::TWO]).unwrap(), CommittedDataSet::new(vec![Felt::from(3_u32)]).unwrap()];
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
