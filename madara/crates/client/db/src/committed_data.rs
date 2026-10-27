//! Durable root-keyed datasets with bounded memory and bounded cold proof reads.
#[cfg(test)]
mod tests;

use super::*;
use blockifier::execution::syscalls::committed_data::{
    CommittedDataBatchProof, CommittedDataError, CommittedDataProvider, CommittedDataSet, CommittedDataWitness,
};
use mp_gateway::committed_data::{CommittedDataUpdate, CommittedDataVersion};
use std::{collections::VecDeque, sync::Mutex};
use tokio::sync::Semaphore;

/// Two imported trees at most; cold proof reads never wait on the import gate.
// ponytail: a two-entry FIFO needs only a short mutex, not an LRU framework.
#[derive(Debug)]
pub(crate) struct SnapshotCache {
    entries: Mutex<VecDeque<(Arc<CommittedDataSet>, u64)>>,
    budget: usize,
    blocks: u64,
    import: Arc<Semaphore>,
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self::new(&Default::default())
    }
}

impl SnapshotCache {
    pub(crate) fn new(config: &crate::committed_data_retention::CommittedDataRetentionConfig) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            import: Arc::new(Semaphore::new(1)),
            budget: config.cache_bytes / 2,
            blocks: config.memory_blocks,
        }
    }
    // Conservative allocation estimate for entries, branches, Arc and vector bookkeeping.
    fn weight(tree: &CommittedDataSet) -> usize {
        256 + tree.entries().len() * 256
    }

    /// Returns a shared immutable tree without hashing or waiting for imports.
    fn get(&self, root: Felt, block: u64) -> Result<Option<Arc<CommittedDataSet>>> {
        let mut cache = self.entries.lock().map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?;
        cache.retain(|(_, imported)| block.saturating_sub(*imported) < self.blocks);
        Ok(cache.iter().find(|(tree, _)| tree.root() == root).map(|(tree, _)| Arc::clone(tree)))
    }

    /// Retains two distinct roots in insertion order; duplicate imports do not reorder them.
    fn insert(&self, tree: Arc<CommittedDataSet>, block: u64) -> Result<()> {
        let mut cache = self.entries.lock().map_err(|_| anyhow::anyhow!("Committed-data cache poisoned"))?;
        cache.retain(|(_, imported)| block.saturating_sub(*imported) < self.blocks);
        let weight = Self::weight(&tree);
        if self.blocks == 0 || weight > self.budget {
            return Ok(());
        }
        if !cache.iter().any(|(old, _)| old.root() == tree.root()) {
            while cache.len() >= 2
                || cache.iter().map(|(tree, _)| Self::weight(tree)).sum::<usize>() + weight > self.budget
            {
                cache.pop_front();
            }
            cache.push_back((tree, block));
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
    /// mismatched proofs return an error. Retained proofs use depth-bounded I/O; pruned
    /// proofs rebuild from permanent values under a separate concurrency gate. This is synchronous storage work; async callers must use a blocking worker.
    pub fn committed_data_witness(&self, root: Felt, index: Felt) -> Result<Option<CommittedDataWitness>> {
        if let Some(tree) = self.committed_data_cache.get(root, self.chain_head_state().confirmed_tip.unwrap_or(0))? {
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
        self.committed_data_provider_at(self.chain_head_state().confirmed_tip.unwrap_or(0).saturating_add(1))
    }

    pub fn committed_data_provider_at(self: &Arc<Self>, block: u64) -> Arc<dyn CommittedDataProvider> {
        Arc::new(BackendCommittedData(Arc::clone(self), block, Mutex::new(std::collections::HashSet::new())))
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
            let version =
                CommittedDataVersion { application_id: update.application_id, dataset_id: update.dataset_id, root };
            if let Some(base_root) = update.base_root {
                anyhow::ensure!(
                    backend.db.has_committed_data_version(CommittedDataVersion { root: base_root, ..version })?,
                    "Base root is not registered in this namespace"
                );
                backend.db.patch_committed_data_dataset(base_root, root, entries, imported_after_block)?;
                return backend.db.write_committed_data_version(version, false);
            }
            // ponytail: authenticate initialization in the persistent tree instead of hashing
            // the entire map twice. Snapshot imports still populate their separate read cache.
            backend.db.import_committed_data_values(root, entries, imported_after_block)?;
            backend.db.write_committed_data_version(version, true)
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
            backend.committed_data_cache.insert(tree, imported_after_block.unwrap_or(0))
        })
        .await?
    }
}

struct BackendCommittedData<D>(Arc<MadaraBackend<D>>, u64, Mutex<std::collections::HashSet<Felt>>);
impl<D: MadaraStorageRead> BackendCommittedData<D> {
    fn retain(&self, root: Felt) -> std::result::Result<(), CommittedDataError> {
        if self.0.committed_data_proof_cutoff().is_some_and(|cutoff| self.1 < cutoff) {
            return Ok(());
        }
        let record = || -> Result<()> {
            let mut seen = self.2.lock().map_err(|_| anyhow::anyhow!("Committed-data use lock poisoned"))?;
            if !seen.contains(&root) && self.0.db.get_committed_data_count(root)?.is_some() {
                self.0.db.note_committed_data_use(root, self.1)?;
                seen.insert(root);
            }
            Ok(())
        };
        record().map_err(|e| CommittedDataError::Provider(e.to_string()))
    }
}
impl<D> std::fmt::Debug for BackendCommittedData<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BackendCommittedData")
    }
}
impl<D: MadaraStorageRead> CommittedDataProvider for BackendCommittedData<D> {
    fn batch(&self, root: Felt, keys: &[Felt]) -> std::result::Result<CommittedDataBatchProof, CommittedDataError> {
        self.retain(root)?;
        let tree = self
            .0
            .committed_data_cache
            .get(root, self.0.chain_head_state().confirmed_tip.unwrap_or(0))
            .map_err(|error| CommittedDataError::Provider(error.to_string()))?;
        match tree {
            Some(tree) => CommittedDataBatchProof::from_dataset(&tree, keys),
            None => self
                .0
                .db
                .get_committed_data_batch(root, keys)
                .map_err(|error| CommittedDataError::Provider(error.to_string())),
        }
    }

    fn value(&self, root: Felt, index: Felt) -> std::result::Result<Option<(Felt, usize)>, CommittedDataError> {
        self.retain(root)?;
        let read = || -> Result<Option<(Felt, usize)>> {
            let value = if let Some(tree) =
                self.0.committed_data_cache.get(root, self.0.chain_head_state().confirmed_tip.unwrap_or(0))?
            {
                tree.value_and_depth(index)
            } else {
                self.0.committed_data_witness(root, index)?.map(|witness| (witness.value, witness.siblings.len()))
            };
            Ok(value)
        };
        read().map_err(|error| CommittedDataError::Provider(error.to_string()))
    }
}
