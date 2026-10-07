//! Durable root-keyed datasets with bounded memory and bounded cold proof reads.
use super::*;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataError, CommittedDataProvider, CommittedDataSet, CommittedDataWitness, MAX_COMMITTED_DATA_VALUES,
};
use std::{
    collections::{HashSet, VecDeque},
    sync::Mutex,
};
use tokio::sync::Semaphore;

const COMMITTED_ROOT_PUBLISHED_SELECTOR: Felt =
    Felt::from_hex_unchecked("0x0027df3eb1efc5b353e0523a2197b865bb34b8b3d881544a1b8303017ad72f0b");
const LIFECYCLE_SCAN_BLOCKS: u64 = 256;
const EVENT_PAGE_SIZE: usize = 1_024;
const PRUNE_ROOTS_PER_PASS: usize = 2;

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

    fn remove(&self, root: Felt) -> Result<()> {
        self.entries
            .lock()
            .map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?
            .retain(|tree| tree.root() != root);
        Ok(())
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Node-local execution switch; dataset imports and reads do not require activation.
    pub fn use_committed_data(&self) -> bool {
        self.config.use_committed_data
    }

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
        let staged_at_block = self.latest_confirmed_block_n().map_or(0, |block| block.saturating_add(1));
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let tree = Arc::new(CommittedDataSet::new(values)?);
            anyhow::ensure!(tree.root() == root, "Imported committed-data root mismatch");
            backend.db.write_committed_data_dataset(
                &tree,
                backend.config.committed_data_max_storage_bytes,
                staged_at_block,
            )?;
            backend.committed_data_cache.insert(tree)
        })
        .await?
    }
}

impl MadaraBackend<crate::rocksdb::RocksDBStorage> {
    pub fn committed_data_pruning_enabled(&self) -> bool {
        self.config.use_committed_data && !self.config.committed_data_oracle_addresses.is_empty()
    }

    fn scan_committed_data_publications(&self, oracle: Felt, start_block: u64, end_block: u64) -> Result<()> {
        let mut page_start = start_block;
        let mut skip_in_start_block = 0;
        loop {
            let events = self.db.get_events(crate::storage::EventFilter {
                start_block: page_start,
                start_event_index: skip_in_start_block,
                end_block,
                from_address: Some(oracle),
                keys_pattern: Some(vec![vec![COMMITTED_ROOT_PUBLISHED_SELECTOR]]),
                max_events: EVENT_PAGE_SIZE,
            })?;
            if events.is_empty() {
                break;
            }
            for event in &events {
                anyhow::ensure!(event.event.data.len() == 6, "Malformed CommittedRootPublished event");
                self.db.record_committed_data_publication(
                    oracle,
                    event.block_number,
                    event.event.data[0],
                    event.event.data[1],
                )?;
            }
            if events.len() < EVENT_PAGE_SIZE {
                break;
            }
            let last_block = events.last().expect("non-empty event page").block_number;
            let in_last_block = events.iter().rev().take_while(|event| event.block_number == last_block).count();
            skip_in_start_block =
                if last_block == page_start { skip_in_start_block + in_last_block } else { in_last_block };
            page_start = last_block;
        }
        self.db.write_committed_data_lifecycle_cursor(oracle, end_block.saturating_add(1))
    }

    /// Reads the canonical latest (including executed preconfirmed state) Oracle storage roots.
    /// This protects a previously retired root that is reused before its new publication settles.
    fn current_committed_data_roots(self: &Arc<Self>) -> Result<HashSet<Felt>> {
        let keys = [
            Felt::from(starknet_api::abi::abi_utils::get_storage_var_address("committed_price_root", &[])),
            Felt::from(starknet_api::abi::abi_utils::get_storage_var_address("committed_funding_root", &[])),
        ];
        let state = self.view_on_latest();
        let mut roots = HashSet::new();
        for oracle in &self.config.committed_data_oracle_addresses {
            for key in &keys {
                if let Some(root) = state.get_contract_storage(oracle, key)?.filter(|root| *root != Felt::ZERO) {
                    roots.insert(root);
                }
            }
        }
        Ok(roots)
    }

    /// Performs one bounded background pass and returns whether immediate follow-up work may remain.
    fn committed_data_pruning_pass(self: &Arc<Self>, observed_settled_tip: u64) -> Result<bool> {
        let Some(local_tip) = self.latest_confirmed_block_n() else {
            return Ok(true);
        };
        let mut more = local_tip < observed_settled_tip;
        let settled_tip = observed_settled_tip.min(local_tip);
        for oracle in &self.config.committed_data_oracle_addresses {
            let next_block = self.db.committed_data_lifecycle_cursor(*oracle)?;
            if next_block <= settled_tip {
                let end_block = settled_tip.min(next_block.saturating_add(LIFECYCLE_SCAN_BLOCKS - 1));
                self.scan_committed_data_publications(*oracle, next_block, end_block)?;
                more |= end_block < settled_tip;
            }
        }

        // The active-root guard is authoritative only after every configured Oracle has been
        // replayed through this settled tip. During initial catch-up a later publication could
        // reactivate a root that an earlier partial scan currently considers retired.
        if more {
            return Ok(true);
        }

        let Some(cutoff) = settled_tip.checked_sub(self.config.committed_data_retention_blocks) else {
            return Ok(more);
        };
        let current_roots = self.current_committed_data_roots()?;
        let deleted = self.db.prune_committed_data(settled_tip, cutoff, PRUNE_ROOTS_PER_PASS, &current_roots)?;
        for root in &deleted {
            self.committed_data_cache.remove(*root)?;
            tracing::info!(root = %root, cutoff, settled_tip, "Pruned retired committed-data dataset");
        }
        Ok(more || deleted.len() == PRUNE_ROOTS_PER_PASS)
    }

    /// Low-priority pruning worker. All RocksDB work runs on the blocking pool, errors only delay
    /// pruning, and bounded passes yield between batches so sequencing never waits for retention.
    pub async fn run_committed_data_pruner(self: Arc<Self>, mut ctx: mp_utils::service::ServiceContext) -> Result<()> {
        if !self.committed_data_pruning_enabled() {
            return Ok(());
        }
        tracing::info!(
            retention_blocks = self.config.committed_data_retention_blocks,
            oracle_contracts = self.config.committed_data_oracle_addresses.len(),
            "Committed-data pruning enabled"
        );
        let mut settled = self.watch_l1_confirmed();
        loop {
            let Some(settled_tip) = *settled.current() else {
                if ctx.run_until_cancelled(settled.recv()).await.is_none() {
                    return Ok(());
                }
                continue;
            };
            let backend = Arc::clone(&self);
            let pass = tokio::task::spawn_blocking(move || backend.committed_data_pruning_pass(settled_tip)).await;
            match pass {
                Ok(Ok(true)) => {
                    if ctx
                        .run_until_cancelled(tokio::time::sleep(std::time::Duration::from_millis(250)))
                        .await
                        .is_none()
                    {
                        return Ok(());
                    }
                }
                Ok(Ok(false)) => {
                    if ctx.run_until_cancelled(settled.recv()).await.is_none() {
                        return Ok(());
                    }
                }
                Ok(Err(error)) => {
                    tracing::error!(%error, "Committed-data pruning pass failed; sequencing is unaffected");
                    if ctx.run_until_cancelled(tokio::time::sleep(std::time::Duration::from_secs(5))).await.is_none() {
                        return Ok(());
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "Committed-data pruning worker failed; sequencing is unaffected");
                    if ctx.run_until_cancelled(tokio::time::sleep(std::time::Duration::from_secs(5))).await.is_none() {
                        return Ok(());
                    }
                }
            }
        }
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

    #[test]
    fn committed_root_event_selector_matches_contract_abi() {
        assert_eq!(COMMITTED_ROOT_PUBLISHED_SELECTOR, starknet_core::utils::starknet_keccak(b"CommittedRootPublished"));
    }

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
        // One paged dataset fits, two do not (including metadata and keys).
        let open = || {
            MadaraBackend::open_rocksdb(
                directory.path(),
                Arc::new(ChainConfig::madara_test()),
                MadaraBackendConfig { committed_data_max_storage_bytes: 3000, ..Default::default() },
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
        let backend = MadaraBackend::open_rocksdb(
            directory.path(),
            Arc::new(ChainConfig::madara_test()),
            MadaraBackendConfig { committed_data_max_storage_bytes: 3000, ..Default::default() },
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

    #[tokio::test]
    async fn committed_data_pruning_requires_retirement_and_the_settlement_buffer() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let oracle = Felt::from(99_u64);
        let old_price = CommittedDataSet::new(vec![Felt::from(1_u64)]).unwrap();
        let old_funding = CommittedDataSet::new(vec![Felt::from(2_u64)]).unwrap();
        let new_price = CommittedDataSet::new(vec![Felt::from(3_u64)]).unwrap();
        let new_funding = CommittedDataSet::new(vec![Felt::from(4_u64)]).unwrap();
        for dataset in [&old_price, &old_funding, &new_price, &new_funding] {
            backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec()).await.unwrap();
        }

        backend.db.record_committed_data_publication(oracle, 11, old_price.root(), old_funding.root()).unwrap();
        assert!(backend.db.prune_committed_data(u64::MAX, u64::MAX, 8, &HashSet::new()).unwrap().is_empty());

        backend.db.record_committed_data_publication(oracle, 6_012, new_price.root(), new_funding.root()).unwrap();
        assert!(backend.db.prune_committed_data(6_011, 6_011, 8, &HashSet::new()).unwrap().is_empty());
        let current_roots = HashSet::from([old_price.root(), old_funding.root()]);
        assert!(backend.db.prune_committed_data(6_012, 6_012, 8, &current_roots).unwrap().is_empty());
        let mut deleted = backend.db.prune_committed_data(6_012, 6_012, 8, &HashSet::new()).unwrap();
        deleted.sort_unstable();
        let mut expected = vec![old_price.root(), old_funding.root()];
        expected.sort_unstable();
        assert_eq!(deleted, expected);
        assert!(backend.db.get_committed_data_count(old_price.root()).unwrap().is_none());
        assert!(backend.db.get_committed_data_count(old_funding.root()).unwrap().is_none());
        assert!(backend.db.get_committed_data_count(new_price.root()).unwrap().is_some());
        assert!(backend.db.get_committed_data_count(new_funding.root()).unwrap().is_some());
        assert!(backend.db.prune_committed_data(u64::MAX, u64::MAX, 8, &HashSet::new()).unwrap().is_empty());
    }

    #[tokio::test]
    async fn committed_data_pruning_keeps_shared_active_and_reimported_roots() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let first_oracle = Felt::from(77_u64);
        let second_oracle = Felt::from(78_u64);
        let shared = CommittedDataSet::new(vec![Felt::from(10_u64)]).unwrap();
        let replacement = CommittedDataSet::new(vec![Felt::from(11_u64)]).unwrap();
        for dataset in [&shared, &replacement] {
            backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec()).await.unwrap();
        }
        // Replay the first Oracle farther into history before the second Oracle starts. Retirement
        // must remain the global maximum block even though per-Oracle catch-up is out of order.
        backend.db.record_committed_data_publication(first_oracle, 150, shared.root(), shared.root()).unwrap();
        backend
            .db
            .record_committed_data_publication(first_oracle, 250, replacement.root(), replacement.root())
            .unwrap();
        backend.db.record_committed_data_publication(second_oracle, 100, shared.root(), replacement.root()).unwrap();
        assert!(backend.db.prune_committed_data(u64::MAX, u64::MAX, 8, &HashSet::new()).unwrap().is_empty());

        backend
            .db
            .record_committed_data_publication(second_oracle, 200, replacement.root(), replacement.root())
            .unwrap();
        assert!(backend.db.prune_committed_data(249, 249, 8, &HashSet::new()).unwrap().is_empty());
        backend.db.write_committed_data_dataset(&shared, DEFAULT_COMMITTED_DATA_STORAGE_BYTES, 1_000).unwrap();
        assert!(backend.db.prune_committed_data(999, 250, 8, &HashSet::new()).unwrap().is_empty());
        assert!(backend.db.get_committed_data_count(shared.root()).unwrap().is_some());
        assert_eq!(backend.db.prune_committed_data(1_000, 250, 8, &HashSet::new()).unwrap(), vec![shared.root()]);
    }

    #[tokio::test]
    async fn historical_lifecycle_replay_does_not_clear_a_newer_staged_import() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let first_oracle = Felt::from(79_u64);
        let second_oracle = Felt::from(80_u64);
        let staged = CommittedDataSet::new(vec![Felt::from(30_u64)]).unwrap();
        let replacement = CommittedDataSet::new(vec![Felt::from(31_u64)]).unwrap();
        for dataset in [&staged, &replacement] {
            backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec()).await.unwrap();
        }
        backend.db.record_committed_data_publication(first_oracle, 150, staged.root(), replacement.root()).unwrap();
        backend
            .db
            .record_committed_data_publication(first_oracle, 250, replacement.root(), replacement.root())
            .unwrap();

        backend.db.write_committed_data_dataset(&staged, DEFAULT_COMMITTED_DATA_STORAGE_BYTES, 1_000).unwrap();
        backend.db.record_committed_data_publication(second_oracle, 100, staged.root(), replacement.root()).unwrap();
        backend
            .db
            .record_committed_data_publication(second_oracle, 200, replacement.root(), replacement.root())
            .unwrap();

        assert!(backend.db.prune_committed_data(999, 999, 8, &HashSet::new()).unwrap().is_empty());
        assert!(backend.db.get_committed_data_count(staged.root()).unwrap().is_some());
        assert_eq!(backend.db.prune_committed_data(1_000, 1_000, 8, &HashSet::new()).unwrap(), vec![staged.root()]);
    }

    #[tokio::test]
    async fn committed_data_pruning_does_not_scan_past_the_local_chain() {
        let oracle = Felt::from(88_u64);
        let backend = MadaraBackend::open_for_testing_with_config(
            Arc::new(ChainConfig::madara_test()),
            MadaraBackendConfig {
                committed_data_retention_blocks: 1_000,
                committed_data_oracle_addresses: vec![oracle],
                ..Default::default()
            },
        );
        let old = CommittedDataSet::new(vec![Felt::from(20_u64)]).unwrap();
        let new = CommittedDataSet::new(vec![Felt::from(21_u64)]).unwrap();
        for dataset in [&old, &new] {
            backend.import_committed_data_snapshot(dataset.root(), dataset.values().to_vec()).await.unwrap();
        }
        backend.db.record_committed_data_publication(oracle, 0, old.root(), old.root()).unwrap();
        backend.db.record_committed_data_publication(oracle, 1, new.root(), new.root()).unwrap();

        assert!(backend.committed_data_pruning_pass(2_000).unwrap());
        assert!(backend.db.get_committed_data_count(old.root()).unwrap().is_some());

        assert_eq!(backend.db.committed_data_lifecycle_cursor(oracle).unwrap(), 0);
        assert!(backend.db.get_committed_data_count(new.root()).unwrap().is_some());
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
                        backend.db.write_committed_data_dataset(dataset, 3000, 0).is_ok()
                    })
                })
                .collect();
            handles.into_iter().map(|handle| usize::from(handle.join().unwrap())).sum::<usize>()
        });
        assert_eq!(successes, 1);
    }
}
