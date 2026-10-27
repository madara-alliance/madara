//! Operational proof retention. Values never expire; age only selects the proof-serving index.
use std::time::Duration;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CommittedDataRetentionConfig {
    /// None retains all proof nodes. Otherwise prune strictly before settled height minus this buffer.
    pub proof_retention_secs: Option<u64>,
    /// Recent block distance for reusable in-memory nodes; zero disables reuse across operations.
    pub memory_blocks: u64,
    /// Estimated reusable-cache budget, split equally between decoded nodes and full imports.
    /// Excludes operation staging, cold reconstruction, RocksDB caches and other process memory.
    pub cache_bytes: usize,
    /// Maximum serialized dataset records waiting for external acknowledgement.
    pub pending_backup_bytes: u64,
}
impl Default for CommittedDataRetentionConfig {
    fn default() -> Self {
        Self {
            proof_retention_secs: None,
            memory_blocks: 7200,
            cache_bytes: 256 * 1024 * 1024,
            pending_backup_bytes: 512 * 1024 * 1024,
        }
    }
}

/// No settlement or insufficient history means no pruning, including block zero.
pub fn settled_retention_cutoff(settled: Option<u64>, buffer: Duration, block_time: Duration) -> Option<u64> {
    let block_ns = block_time.as_nanos();
    if block_ns == 0 {
        return None;
    }
    let blocks = u64::try_from(buffer.as_nanos().div_ceil(block_ns)).ok()?;
    settled?.checked_sub(blocks)
}

impl<D: crate::MadaraStorageRead> crate::MadaraBackend<D> {
    /// Wake the idle cleanup worker after durable ingestion. No work is queued while it is busy.
    pub fn request_committed_data_cleanup(&self) {
        if self.config.committed_data_retention.proof_retention_secs.is_some() {
            // ponytail: notify_waiters stores no permit, so repeated requests cannot queue sweeps.
            self.committed_data_cleanup.notify_waiters();
        }
    }

    /// Only the registered history service waits here; it owns cleanup and graceful shutdown.
    pub async fn wait_for_committed_data_cleanup(&self) {
        self.committed_data_cleanup.notified().await;
    }

    pub(crate) fn committed_data_proof_cutoff(&self) -> Option<u64> {
        let seconds = self.config.committed_data_retention.proof_retention_secs?;
        let head = self.chain_head_state().confirmed_tip.unwrap_or(0);
        let settled = self.latest_l1_confirmed_block_n().map(|height| height.min(head));
        settled_retention_cutoff(settled, Duration::from_secs(seconds), self.chain_config().block_time)
    }
}
impl<D: crate::MadaraStorage> crate::MadaraBackend<D> {
    /// One maintenance step, with settlement clamped to the locally available chain.
    pub fn maintain_committed_data_history(&self) -> anyhow::Result<bool> {
        if self.config.committed_data_retention.proof_retention_secs.is_none() {
            return Ok(false);
        }
        self.db.maintain_committed_data_history(self.committed_data_proof_cutoff())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MadaraBackend, MadaraBackendConfig};
    use futures::FutureExt;
    use std::sync::Arc;

    fn backend(retention: Option<u64>) -> Arc<MadaraBackend> {
        MadaraBackend::open_for_testing_with_config(
            mp_chain_config::ChainConfig::madara_test().into(),
            MadaraBackendConfig {
                committed_data_retention: CommittedDataRetentionConfig {
                    proof_retention_secs: retention,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn committed_data_cleanup_drops_busy_triggers_and_rearms_without_a_queue() {
        let backend = backend(Some(0));
        for _ in 0..10 {
            backend.request_committed_data_cleanup();
        }
        assert!(backend.wait_for_committed_data_cleanup().now_or_never().is_none());
        let mut waiting = Box::pin(backend.wait_for_committed_data_cleanup());
        assert!(futures::poll!(&mut waiting).is_pending());
        backend.request_committed_data_cleanup();
        assert!(futures::poll!(&mut waiting).is_ready());
        drop(waiting);
        // Model the worker inside its sweep, with no notification future registered.
        for _ in 0..10 {
            backend.request_committed_data_cleanup();
        }
        let mut next = Box::pin(backend.wait_for_committed_data_cleanup());
        assert!(futures::poll!(&mut next).is_pending(), "Busy triggers must not queue another sweep");
        backend.request_committed_data_cleanup();
        assert!(futures::poll!(&mut next).is_ready());
    }

    #[tokio::test]
    async fn committed_data_cleanup_is_disabled_without_retention() {
        let backend = backend(None);
        let mut waiting = Box::pin(backend.wait_for_committed_data_cleanup());
        assert!(futures::poll!(&mut waiting).is_pending());
        backend.request_committed_data_cleanup();
        assert!(futures::poll!(&mut waiting).is_pending());
    }

    #[tokio::test]
    async fn committed_data_cleanup_waits_for_successful_durable_journal() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        use mp_convert::Felt;
        use mp_gateway::committed_data::CommittedDataUpdate;
        let backend = backend(Some(0));
        let values = vec![(Felt::ONE, Felt::TWO)];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        let mut waiting = Box::pin(backend.wait_for_committed_data_cleanup());
        assert!(futures::poll!(&mut waiting).is_pending());
        backend.import_committed_data_snapshot(root, values.clone()).await.unwrap();
        assert!(futures::poll!(&mut waiting).is_pending(), "Import must not race its recovery journal");
        let missing_base =
            CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: Some(Felt::THREE) };
        assert!(backend.journal_committed_data(Some(missing_base), root, values.clone()).await.is_err());
        assert!(futures::poll!(&mut waiting).is_pending());
        let id = backend.journal_committed_data(None, root, values).await.unwrap();
        assert_eq!(backend.recovery_reference(None, root).unwrap(), id);
        assert!(futures::poll!(&mut waiting).is_ready());
    }

    #[test]
    fn settlement_cutoff_rounds_up_and_never_underflows() {
        let block = Duration::from_millis(1500);
        assert_eq!(settled_retention_cutoff(None, Duration::ZERO, block), None);
        assert_eq!(settled_retention_cutoff(Some(100), Duration::from_secs(10), block), Some(93));
        assert_eq!(settled_retention_cutoff(Some(6), Duration::from_secs(10), block), None);
        assert_eq!(settled_retention_cutoff(Some(7), Duration::from_secs(10), block), Some(0));
        assert_eq!(settled_retention_cutoff(Some(100), Duration::ZERO, Duration::ZERO), None);
    }
}
