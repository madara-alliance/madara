//! Durable root-keyed datasets with bounded memory and bounded cold proof reads.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataBatchProof, CommittedDataError, CommittedDataProvider, CommittedDataSet, CommittedDataWitness,
    MAX_COMMITTED_DATA_VALUES,
};
use mp_gateway::committed_data::{CommittedDataUpdate, CommittedDataVersion, VALUES_PER_PAGE};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};
use tokio::sync::Semaphore;

/// Two imported trees at most; cold proof reads never wait on the import gate.
// ponytail: a two-entry FIFO needs only a short mutex, not an LRU framework.
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
    /// Node-local execution switch; dataset imports and reads do not require activation.
    pub fn use_committed_data(&self) -> bool {
        self.config.use_committed_data
    }

    /// Returns an authenticated proof for an imported root and present key.
    ///
    /// Missing roots/keys return `None`; corrupt records and
    /// mismatched proofs return an error. Cold reads have depth-bounded I/O and never rebuild
    /// a tree. This is synchronous storage work; async callers must use a blocking worker.
    pub fn committed_data_witness(&self, root: Felt, index: Felt) -> Result<Option<CommittedDataWitness>> {
        if let Some(tree) = self.committed_data_cache.get(root)? {
            return Ok(tree.witness(index));
        }
        let witness = self.db.get_committed_data_witness(root, index)?;
        if let Some(witness) = &witness {
            anyhow::ensure!(
                (witness.root, witness.key) == (root, index) && witness.verify(),
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
    /// Applies keyed upserts to an explicit retained base. Omission and zero never delete keys.
    /// Namespace labels are operational metadata, not application authorization or a latest head.
    /// Cold paths are authenticated before merging; all resulting nodes are durable before the
    /// version label is acknowledged. Rejected transaction submission may leave an unused version.
    pub async fn update_committed_data(
        self: &Arc<Self>,
        update: CommittedDataUpdate,
        root: Felt,
        entries: Vec<(Felt, Felt)>,
    ) -> Result<()> {
        let permit = Arc::clone(&self.committed_data_cache.import)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("Committed-data import busy; retry import later"))?;
        let imported_after_block = self.chain_head_state().confirmed_tip;
        let backend = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            // Validate the patch before loading the base; do not hash a separate patch trie.
            let mut patch = entries;
            anyhow::ensure!(
                (1..=MAX_COMMITTED_DATA_VALUES).contains(&patch.len()),
                "Invalid committed-data patch size"
            );
            patch.sort_unstable_by_key(|entry| entry.0);
            anyhow::ensure!(patch.windows(2).all(|pair| pair[0].0 != pair[1].0), "Duplicate committed-data patch key");
            let version =
                CommittedDataVersion { application_id: update.application_id, dataset_id: update.dataset_id, root };
            let mut values = BTreeMap::new();
            if let Some(base_root) = update.base_root {
                anyhow::ensure!(
                    backend.db.has_committed_data_version(CommittedDataVersion { root: base_root, ..version })?,
                    "Base root is not registered in this namespace"
                );
                if backend.db.patch_committed_data_dataset(base_root, root, patch.clone(), imported_after_block)? {
                    backend.db.write_committed_data_version(version, false)?;
                    return Ok(());
                }
                let base = match backend.committed_data_cache.get(base_root)? {
                    Some(base) => base,
                    None => {
                        let mut entries = Vec::new();
                        loop {
                            let (count, page) = backend
                                .db
                                .get_committed_data_page(base_root, entries.len() as u32)?
                                .ok_or_else(|| anyhow::anyhow!("Base dataset unavailable"))?;
                            anyhow::ensure!(
                                !page.is_empty() && page.len() <= VALUES_PER_PAGE,
                                "Invalid base dataset page"
                            );
                            entries.extend(page);
                            anyhow::ensure!(
                                entries.len() <= count as usize && count as usize <= MAX_COMMITTED_DATA_VALUES,
                                "Invalid base dataset count"
                            );
                            if entries.len() == count as usize {
                                break;
                            }
                        }
                        let base = Arc::new(CommittedDataSet::new(entries)?);
                        anyhow::ensure!(base.root() == base_root, "Stored base dataset root mismatch");
                        base
                    }
                };
                values.extend(base.entries().iter().copied());
            }
            values.extend(patch);
            // The same constructor is used by full imports and clients; no new hash format.
            let tree = Arc::new(CommittedDataSet::new(values.into_iter().collect())?);
            anyhow::ensure!(tree.root() == root, "Committed-data root does not match updated dataset");
            backend.db.write_committed_data_dataset(&tree, imported_after_block)?;
            backend.db.write_committed_data_version(version, update.base_root.is_none())?;
            backend.committed_data_cache.insert(tree)
        })
        .await?
    }

    /// Authenticates a dataset, durably writes its pages, then makes it available to execution.
    ///
    /// Rejects another admitted import before enqueuing blocking work. The permit stays with
    /// the worker if its async caller is cancelled; an accepted import may still complete.
    /// Success means a synchronous WAL write completed. Re-importing an identical dataset
    /// repairs its stored nodes. This never publishes a root on-chain.
    ///
    /// Returns an error for busy admission, invalid data/root or storage failure.
    pub async fn import_committed_data_snapshot(self: &Arc<Self>, root: Felt, values: Vec<(Felt, Felt)>) -> Result<()> {
        let permit = Arc::clone(&self.committed_data_cache.import)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("Committed-data import busy; retry import later"))?;
        let imported_after_block = self.chain_head_state().confirmed_tip;
        let backend = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let tree = Arc::new(CommittedDataSet::new(values)?);
            anyhow::ensure!(tree.root() == root, "Committed-data root does not match dataset");
            backend.db.write_committed_data_dataset(&tree, imported_after_block)?;
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
    fn batch(&self, root: Felt, keys: &[Felt]) -> std::result::Result<CommittedDataBatchProof, CommittedDataError> {
        let tree =
            self.0.committed_data_cache.get(root).map_err(|error| CommittedDataError::Provider(error.to_string()))?;
        match tree {
            Some(tree) => CommittedDataBatchProof::from_dataset(&tree, keys),
            None => CommittedDataBatchProof::new(root, keys, |key| {
                self.0
                    .committed_data_witness(root, key)
                    .map_err(|error| CommittedDataError::Provider(error.to_string()))
            }),
        }
    }

    fn value(&self, root: Felt, index: Felt) -> std::result::Result<Option<(Felt, usize)>, CommittedDataError> {
        let read = || -> Result<Option<(Felt, usize)>> {
            let value = if let Some(tree) = self.0.committed_data_cache.get(root)? {
                tree.value_and_depth(index)
            } else {
                self.0.committed_data_witness(root, index)?.map(|witness| (witness.value, witness.siblings.len()))
            };
            Ok(value)
        };
        read().map_err(|error| CommittedDataError::Provider(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rocksdb::RocksDBConfig;

    #[tokio::test]
    async fn committed_data_updates_retain_omitted_keys_history_and_namespaces_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
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
        let namespace = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
        let first = vec![(Felt::ONE, Felt::from(100_u32)), (Felt::TWO, Felt::from(200_u32))];
        // Roots from the independently implemented Python incremental vectors.
        let root = Felt::from_hex_unchecked("0x5a32e2ffd847d637b53eff4f469dcea2ef05175e9b5497282eb9bfbbe14ecda");
        {
            let backend = open();
            backend.update_committed_data(namespace, root, first).await.unwrap();
        }
        let backend = open(); // Exercise an authenticated cold base, not the import cache.
        let patch = vec![(Felt::TWO, Felt::from(250_u32)), (Felt::MAX, Felt::ZERO)];
        let next = Felt::from_hex_unchecked("0x6f7b24abc22284b8724b96d0a157b7096a1a289bb76bb0a2b2d1a175fe870b0");
        let update = CommittedDataUpdate { base_root: Some(root), ..namespace };
        for bad in [
            CommittedDataUpdate { application_id: Felt::THREE, ..update },
            CommittedDataUpdate { dataset_id: Felt::THREE, ..update },
            CommittedDataUpdate { base_root: Some(Felt::ZERO), ..update },
        ] {
            assert!(backend.update_committed_data(bad, next, patch.clone()).await.is_err());
        }
        assert!(backend.update_committed_data(update, next, vec![(Felt::TWO, Felt::ZERO); 2]).await.is_err());
        assert!(backend.update_committed_data(update, Felt::ZERO, patch.clone()).await.is_err());
        assert!(backend.db.get_committed_data_count(next).unwrap().is_none());
        // A mistaken initial request must not register a replacement root in this namespace.
        let replacement = vec![(Felt::TWO, Felt::from(250_u32))];
        let replacement_root = CommittedDataSet::new(replacement.clone()).unwrap().root();
        let error = backend.update_committed_data(namespace, replacement_root, replacement).await.unwrap_err();
        assert!(error.to_string().contains("already initialized"));
        assert!(!backend
            .db
            .has_committed_data_version(CommittedDataVersion {
                application_id: namespace.application_id,
                dataset_id: namespace.dataset_id,
                root: replacement_root,
            })
            .unwrap());
        // The first response might have been lost: resending the identical initialization is safe.
        backend
            .update_committed_data(
                namespace,
                root,
                vec![(Felt::ONE, Felt::from(100_u32)), (Felt::TWO, Felt::from(200_u32))],
            )
            .await
            .unwrap();
        backend.update_committed_data(update, next, patch.clone()).await.unwrap();
        backend.update_committed_data(update, next, patch).await.unwrap(); // Retry from the same base.
        assert_eq!(backend.committed_data_witness(next, Felt::ONE).unwrap().unwrap().value, Felt::from(100_u32));
        assert_eq!(backend.committed_data_witness(root, Felt::TWO).unwrap().unwrap().value, Felt::from(200_u32));
        assert_eq!(backend.committed_data_witness(next, Felt::MAX).unwrap().unwrap().value, Felt::ZERO);
        // A second application may reuse the same cryptographic map but has its own version label.
        backend
            .update_committed_data(
                CommittedDataUpdate { application_id: Felt::THREE, ..namespace },
                next,
                vec![(Felt::ONE, Felt::from(100_u32)), (Felt::TWO, Felt::from(250_u32)), (Felt::MAX, Felt::ZERO)],
            )
            .await
            .unwrap();
        let versions = backend.db.list_committed_data_versions(None).unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(backend.db.list_committed_data_versions(Some(versions[0])).unwrap(), versions[1..]);
        drop(backend);
        let backend = open();
        assert_eq!(backend.db.list_committed_data_versions(None).unwrap(), versions);
        for old in [root, next] {
            assert!(backend.committed_data_witness(old, Felt::ONE).unwrap().unwrap().verify());
        }
        // Reusing a historical base creates an explicit branch; it never rewrites other versions.
        backend.update_committed_data(update, root, vec![(Felt::TWO, Felt::from(200_u32))]).await.unwrap();
        assert_eq!(backend.db.list_committed_data_versions(None).unwrap(), versions);
    }

    #[tokio::test]
    async fn committed_data_updates_reject_corrupt_base_even_when_result_root_matches_corruption() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let context = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::ONE, base_root: None };
        let root = CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap().root();
        backend.update_committed_data(context, root, vec![(Felt::ZERO, Felt::TWO)]).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        let mut metadata_key = b"committed_data_mpt/".to_vec();
        metadata_key.extend_from_slice(&root.to_bytes_be());
        metadata_key.extend_from_slice(&[u8::MAX, 0, 0, 0, 0]);
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let metadata = backend.db.inner_db().get_cf(&cf, metadata_key).unwrap().unwrap();
        let id = u64::from_be_bytes(metadata[13..21].try_into().unwrap());
        let key = crate::persistent_committed_data::node_key(id);
        backend
            .db
            .inner_db()
            .put_cf(&cf, key, [vec![0], Felt::ZERO.to_bytes_be().to_vec(), Felt::ONE.to_bytes_be().to_vec()].concat())
            .unwrap();
        let corrupted = CommittedDataSet::new(vec![(Felt::ZERO, Felt::ONE), (Felt::ONE, Felt::THREE)]).unwrap().root();
        let error = backend
            .update_committed_data(
                CommittedDataUpdate { base_root: Some(root), ..context },
                corrupted,
                vec![(Felt::ONE, Felt::THREE)],
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("root"));
        assert!(backend.db.get_committed_data_count(corrupted).unwrap().is_none());
    }

    #[tokio::test]
    async fn committed_data_catalog_pages_existing_roots_and_values_without_schema_rewrite() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mut roots = Vec::new();
        for value in 0..65_u32 {
            let values: Vec<_> = vec![(Felt::ZERO, Felt::from(value))];
            let root = CommittedDataSet::new(values.clone()).unwrap().root();
            backend.import_committed_data_snapshot(root, values).await.unwrap();
            roots.push(root);
        }
        let mut versions = Vec::new();
        for (index, root) in roots.iter().copied().enumerate() {
            let version = CommittedDataVersion { application_id: Felt::from(index), dataset_id: Felt::ZERO, root };
            backend.db.write_committed_data_version(version, false).unwrap();
            versions.push(version);
        }
        assert_eq!(backend.db.list_committed_data_versions(None).unwrap(), versions[..64]);
        assert_eq!(backend.db.list_committed_data_versions(Some(versions[63])).unwrap(), versions[64..]);
        assert!(backend.db.list_committed_data_versions(Some(versions[64])).unwrap().is_empty());
        assert!(backend
            .db
            .write_committed_data_version(
                CommittedDataVersion { application_id: Felt::ZERO, dataset_id: Felt::ONE, root: Felt::ZERO },
                false
            )
            .is_err());
        roots.sort_unstable();
        assert_eq!(backend.db.list_committed_data_roots(None).unwrap(), roots[..64]);
        assert_eq!(backend.db.list_committed_data_roots(Some(roots[63])).unwrap(), roots[64..]);
        assert!(backend.db.list_committed_data_roots(Some(roots[64])).unwrap().is_empty());
        let values: Vec<_> = ((0..4097_u32).map(Felt::from).collect::<Vec<_>>())
            .into_iter()
            .enumerate()
            .map(|(i, value)| (Felt::from(i), value))
            .collect();
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
        let values: Vec<_> =
            vec![(Felt::ZERO, Felt::from(123_u32)), (Felt::ONE, Felt::from(456_u32)), (Felt::from(2_u32), Felt::MAX)];
        let tree = CommittedDataSet::new(values.clone()).unwrap();
        let root = tree.root();
        let keys = [Felt::TWO, Felt::ZERO, Felt::TWO];
        let batch = CommittedDataBatchProof::from_dataset(&tree, &keys).unwrap();
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
            let witness = backend.committed_data_witness(root, Felt::ONE).unwrap().unwrap();
            assert!(witness.verify());
            assert_eq!(witness.value, Felt::from(456_u32));
            assert_eq!(BackendCommittedData(Arc::clone(&backend)).batch(root, &keys).unwrap(), batch);
        }
        {
            let backend = open();
            // Cold reads must not need the import gate or reconstruct a whole tree.
            let importing = backend.committed_data_cache.import.try_acquire().unwrap();
            assert!(backend.committed_data_witness(root, Felt::from(2_u32)).unwrap().unwrap().verify());
            assert_eq!(BackendCommittedData(Arc::clone(&backend)).batch(root, &keys).unwrap(), batch);
            assert!(backend.committed_data_cache.get(root).unwrap().is_none());
            drop(importing);
            assert!(backend.committed_data_witness(root, Felt::from(3_u32)).unwrap().is_none());
            assert!(backend.committed_data_witness(root + Felt::ONE, Felt::ONE).unwrap().is_none());
        }
    }
    #[tokio::test]
    async fn committed_data_cold_read_rejects_a_canonical_but_unauthenticated_value() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let values: Vec<_> = vec![(Felt::ZERO, Felt::TWO)];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, values).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        let mut metadata_key = b"committed_data_mpt/".to_vec();
        metadata_key.extend_from_slice(&root.to_bytes_be());
        metadata_key.extend_from_slice(&[u8::MAX, 0, 0, 0, 0]);
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let metadata = backend.db.inner_db().get_cf(&cf, metadata_key).unwrap().unwrap();
        let id = u64::from_be_bytes(metadata[13..21].try_into().unwrap());
        let key = crate::persistent_committed_data::node_key(id);
        // Correct encoding and length must not be confused with authentication.
        backend
            .db
            .inner_db()
            .put_cf(&cf, key, [vec![0], Felt::ZERO.to_bytes_be().to_vec(), Felt::ONE.to_bytes_be().to_vec()].concat())
            .unwrap();
        let error = backend.committed_data_witness(root, Felt::ZERO).unwrap_err();
        assert!(error.to_string().contains("Persistent root mismatch"));
    }

    #[tokio::test]
    async fn committed_data_import_rejects_root_mismatch_without_writing() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let values: Vec<_> = vec![(Felt::ZERO, Felt::TWO)];
        let wrong_root = CommittedDataSet::new(values.clone()).unwrap().root() + Felt::ONE;
        assert!(backend.import_committed_data_snapshot(wrong_root, values).await.is_err());
        assert!(backend.committed_data_witness(wrong_root, Felt::ZERO).unwrap().is_none());
    }

    #[tokio::test]
    async fn committed_data_reimport_repairs_malformed_nodes() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let values: Vec<_> = vec![(Felt::ZERO, Felt::TWO)];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        let mut metadata_key = b"committed_data_mpt/".to_vec();
        metadata_key.extend_from_slice(&root.to_bytes_be());
        metadata_key.extend_from_slice(&[u8::MAX, 0, 0, 0, 0]);
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let metadata = backend.db.inner_db().get_cf(&cf, metadata_key).unwrap().unwrap();
        let id = u64::from_be_bytes(metadata[13..21].try_into().unwrap());
        let key = crate::persistent_committed_data::node_key(id);
        backend.db.inner_db().put_cf(&cf, key, [0xff]).unwrap();
        assert!(backend.committed_data_witness(root, Felt::ZERO).is_err());
        backend.import_committed_data_snapshot(root, values).await.unwrap();
        backend.committed_data_cache.entries.lock().unwrap().clear();
        assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
    }

    #[test]
    fn committed_data_import_rejects_a_second_request_before_blocking_work_starts() {
        tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap().block_on(
            async {
                let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
                let dataset = CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap();
                let (release, wait) = std::sync::mpsc::channel();
                let (started, ready) = tokio::sync::oneshot::channel();
                let blocker = tokio::task::spawn_blocking(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
                });
                ready.await.unwrap();
                let first = backend.import_committed_data_snapshot(dataset.root(), dataset.entries().to_vec());
                tokio::pin!(first);
                assert!(futures::poll!(&mut first).is_pending());
                // The first import is queued, not running. The second must fail without joining it.
                let second = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    backend.import_committed_data_snapshot(dataset.root(), dataset.entries().to_vec()),
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
    async fn committed_data_imports_accumulate_and_survive_reopen() {
        let directory = tempfile::TempDir::new().unwrap();
        let first = CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap();
        let second = CommittedDataSet::new(vec![(Felt::ZERO, Felt::from(3_u32))]).unwrap();
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
            backend.import_committed_data_snapshot(first.root(), first.entries().to_vec()).await.unwrap();
            backend.import_committed_data_snapshot(first.root(), first.entries().to_vec()).await.unwrap();
            backend.import_committed_data_snapshot(second.root(), second.entries().to_vec()).await.unwrap();
        }
        let backend = open();
        assert!(backend.committed_data_witness(first.root(), Felt::ZERO).unwrap().unwrap().verify());
        assert!(backend.committed_data_witness(second.root(), Felt::ZERO).unwrap().unwrap().verify());
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        assert!(backend.db.inner_db().get_cf(&cf, b"committed_data_pages_usage").unwrap().is_none());
    }

    #[tokio::test]
    async fn persistent_patch_is_atomic_and_reuses_unchanged_nodes() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let context = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::ONE, base_root: None };
        let mut values: Vec<_> = (0..1024u32).map(|k| (Felt::from(k), Felt::from(k + 10))).collect();
        let old = CommittedDataSet::new(values.clone()).unwrap();
        backend.update_committed_data(context, old.root(), values.clone()).await.unwrap();
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let allocated = || {
            let bytes = backend.db.inner_db().get_cf(&cf, crate::persistent_committed_data::NEXT_KEY).unwrap().unwrap();
            u64::from_be_bytes(bytes.try_into().unwrap())
        };
        let before = allocated();
        let update = CommittedDataUpdate { base_root: Some(old.root()), ..context };
        values[500].1 = Felt::ZERO;
        let new = CommittedDataSet::new(values.clone()).unwrap();
        // The rejected expected root must publish neither nodes nor a root/catalog entry.
        assert!(backend.update_committed_data(update, Felt::ONE, vec![values[500]]).await.is_err());
        assert_eq!(allocated(), before);
        assert!(backend.db.get_committed_data_count(Felt::ONE).unwrap().is_none());
        backend.update_committed_data(update, new.root(), vec![values[500]]).await.unwrap();
        assert!(allocated() > before && allocated() - before <= 11);
        let after = allocated();
        backend.update_committed_data(update, new.root(), vec![values[500]]).await.unwrap();
        // Retrying from the original base and updating the current root with identical values allocate nothing.
        backend
            .update_committed_data(
                CommittedDataUpdate { base_root: Some(new.root()), ..context },
                new.root(),
                vec![values[500]],
            )
            .await
            .unwrap();
        assert_eq!(allocated(), after);
        let no_op = allocated();
        backend.import_committed_data_snapshot(new.root(), values).await.unwrap();
        assert_eq!(allocated(), no_op);
        backend.committed_data_cache.entries.lock().unwrap().clear();
        assert_eq!(
            backend.committed_data_witness(old.root(), Felt::from(500)).unwrap().unwrap().value,
            Felt::from(510)
        );
        assert_eq!(backend.committed_data_witness(new.root(), Felt::from(500)).unwrap().unwrap().value, Felt::ZERO);
    }

    #[tokio::test]
    async fn committed_data_first_import_checkpoint_is_preserved_on_reimport() {
        let directory = tempfile::TempDir::new().unwrap();
        let backend = MadaraBackend::open_rocksdb(
            directory.path(),
            Arc::new(ChainConfig::madara_test()),
            MadaraBackendConfig::default(),
            RocksDBConfig::default(),
            Arc::new(NativeConfig::default()),
        )
        .unwrap();
        let cf = backend.db.inner_db().cf_handle("meta").unwrap();
        let dataset = CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap();
        backend.db.write_committed_data_dataset(&dataset, Some(42)).unwrap();
        backend.db.write_committed_data_dataset(&dataset, Some(100)).unwrap();
        let mut key = b"committed_data_mpt/".to_vec();
        key.extend_from_slice(&dataset.root().to_bytes_be());
        key.extend_from_slice(&[u8::MAX, 0, 0, 0, 0]);
        let metadata = backend.db.inner_db().get_cf(&cf, key).unwrap().unwrap();
        assert_eq!(metadata.len(), 21);
        assert_eq!(metadata[4], 1);
        assert_eq!(u64::from_be_bytes(metadata[5..13].try_into().unwrap()), 42);
    }

    #[test]
    fn committed_data_cache_is_bounded_and_hot_reads_do_not_take_import_lock() {
        let cache = SnapshotCache::default();
        let trees: Vec<_> =
            (0..3_u32).map(|i| Arc::new(CommittedDataSet::new(vec![(Felt::ZERO, Felt::from(i))]).unwrap())).collect();
        for tree in &trees {
            cache.insert(tree.clone()).unwrap();
        }
        let _building = cache.import.try_acquire().unwrap();
        assert!(cache.get(trees[0].root()).unwrap().is_none());
        assert!(cache.get(trees[2].root()).unwrap().is_some());
        assert_eq!(cache.entries.lock().unwrap().len(), 2);
    }
    #[tokio::test]
    async fn committed_data_concurrent_storage_writers_retain_both_roots() {
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
            CommittedDataSet::new(vec![(Felt::ZERO, Felt::TWO)]).unwrap(),
            CommittedDataSet::new(vec![(Felt::ZERO, Felt::from(3_u32))]).unwrap(),
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
                        backend.db.write_committed_data_dataset(dataset, None).is_ok()
                    })
                })
                .collect();
            handles.into_iter().map(|handle| usize::from(handle.join().unwrap())).sum::<usize>()
        });
        assert_eq!(successes, 2);
        for dataset in &datasets {
            assert!(backend.committed_data_witness(dataset.root(), Felt::ZERO).unwrap().unwrap().verify());
        }
    }
}
