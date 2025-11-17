use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
};
use anyhow::Context;
use mc_db::MadaraBackend;
use mp_state_update::StateDiff;
use std::{ops::Range, sync::Arc};
use tokio::sync::Mutex;
use crate::pipeline::PipelineStatus;
use crate::sync_utils::compress_state_diff;

// TODO(heemankv, 2025-10-26): Should be driven from env for hardware-based customisations
/// Batch size for snap sync state accumulation before flushing to the global trie.
///
/// During snap sync, state diffs are accumulated in memory rather than computing
/// the trie after every block. After this many blocks have been processed, the
/// accumulated state diffs are squashed and applied to the global trie in a
/// single operation; then the in-memory accumulator is cleared.
///
/// This represents a memory-performance trade-off:
/// - Larger values: Fewer trie computations (better performance), higher memory usage
/// - Smaller values: More frequent trie updates (lower performance), less memory usage
///
/// The value of 1000 blocks has been empirically tested to provide good performance
/// on modern hardware without excessive memory consumption. It typically results in
/// trie updates every few minutes during active syncing.
///
/// Not exposed as a CLI option to maintain simplicity (KISS principle). If adjustment
/// is needed for specific hardware constraints, modify this constant directly.
const APPLY_STATE_SNAP_BATCH_SIZE: u64 = 1000;

pub type ApplyStateSync = PipelineController<ApplyStateSteps>;
pub fn apply_state_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    starting_block_n: u64,
    parallelization: usize,
    batch_size: usize,
    disable_tries: bool,
    snap_sync: bool,
) -> ApplyStateSync {
    PipelineController::new(
        ApplyStateSteps {
            importer,
            backend,
            disable_tries,
            snap_sync,
            state_diff_map: Mutex::new(Default::default()),
            target_block: Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
        },
        parallelization,
        batch_size,
        starting_block_n
    )
}

pub struct ApplyStateSteps {
    importer: Arc<BlockImporter>,
    pub(crate) backend: Arc<MadaraBackend>,
    disable_tries: bool,
    snap_sync: bool,
    state_diff_map: Mutex<crate::sync_utils::StateDiffMap>,
    target_block: Arc<std::sync::atomic::AtomicU64>,
}

impl PipelineController<ApplyStateSteps> {

    pub fn trie_state_status(&self) -> PipelineStatus {
        PipelineStatus {
            jobs: self.queue_len(),
            applying: self.is_applying(),
            latest_applied: self.steps.backend.get_latest_applied_trie_update().ok().flatten(),
        }
    }
}

impl ApplyStateSteps {
    pub async fn sync_each_block(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: <ApplyStateSteps as PipelineSteps>::SequentialStepInput
    ) -> anyhow::Result<ApplyOutcome<()>> {
        let block_range_clone = block_range.clone();
        self.importer
            .run_in_rayon_pool_global(move |importer| importer.apply_to_global_trie(block_range_clone, input))
            .await
            .with_context(|| format!("Applying global trie step for block_range={block_range:?}"))?;
        Ok(ApplyOutcome::Success(()))
    }

    pub async fn sync_snap(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: <ApplyStateSteps as PipelineSteps>::SequentialStepInput
    ) -> anyhow::Result<ApplyOutcome<()>> {
        let current_first_block = self.backend.get_latest_applied_trie_update()?.map(|n| n + 1).unwrap_or(0);
        let latest_block = block_range.end;

        // Accumulate state diffs
        {
            let already_imported_count = current_first_block.saturating_sub(block_range.start);
            let state_diffs = input.iter().skip(already_imported_count as _);

            let mut state_diff_map = self.state_diff_map.lock().await;
            for single_contract_state_diff in state_diffs {
                state_diff_map.apply_state_diff(&single_contract_state_diff);
            }
        }

        let target_block = self.target_block.load(std::sync::atomic::Ordering::Relaxed);

        // Apply if we've accumulated enough blocks or reached target
        if latest_block >= (current_first_block + APPLY_STATE_SNAP_BATCH_SIZE) || latest_block >= target_block {
            self.clone()
                .apply_accumulated_diffs(current_first_block, latest_block)
                .await
                .with_context(|| format!("Applying snap sync trie for block_range={block_range:?}"))?;
        }

        Ok(ApplyOutcome::Success(()))
    }


    /// Flushes accumulated state diffs to the global trie when transitioning from snap sync
    /// to block-by-block sync mode.
    async fn flush_accumulated_state_diffs(
        self: Arc<Self>,
        up_to_block: u64,
    ) -> anyhow::Result<()> {
        let current_first_block = self.backend.get_latest_applied_trie_update()?.map(|n| n + 1).unwrap_or(0);
        self.apply_accumulated_diffs(current_first_block, up_to_block).await
    }

    /// Main sync function that decides whether to use snap sync or block-by-block sync
    /// based on the distance to the target block.
    pub async fn sync(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: <ApplyStateSteps as PipelineSteps>::SequentialStepInput
    ) -> anyhow::Result<ApplyOutcome<()>> {
        let target_block = self.target_block.load(std::sync::atomic::Ordering::Relaxed);
        let distance_to_target = target_block.saturating_sub(block_range.start);
        // Use snap sync if:
        // 1. Snap sync is enabled, AND
        // 2. We're far enough from the target (distance >= APPLY_STATE_SNAP_BATCH_SIZE)
        let should_use_snap_sync = self.snap_sync && distance_to_target >= APPLY_STATE_SNAP_BATCH_SIZE;
        if should_use_snap_sync {
            self.sync_snap(block_range, input).await
        } else {
            // Before switching to block-by-block sync, we need to flush any accumulated state diffs
            // from snap sync to avoid losing data or computing incorrect state roots
            if self.snap_sync {
                let has_accumulated_diffs = !self.state_diff_map.lock().await
                    .to_raw_state_diff()
                    .is_empty();

                if has_accumulated_diffs {
                    self.clone().flush_accumulated_state_diffs(block_range.start).await?;
                }
            }
            self.sync_each_block(block_range, input).await
        }
    }

    async fn apply_accumulated_diffs(
        self: Arc<Self>,
        current_first_block: u64,
        latest_block: u64,
    ) -> anyhow::Result<()> {
        // Lock to read and prepare state_diff
        let state_diff = {
            let state_diff_map = self.state_diff_map.lock().await;
            let mut state_diff = state_diff_map.to_raw_state_diff();
            state_diff.sort();
            state_diff
        };

        let pre_range_block_check = if current_first_block == 0 {
            None
        } else {
            Some(current_first_block.saturating_sub(1))
        };

        let accumulated_state_diff = compress_state_diff(
            state_diff,
            pre_range_block_check,
            self.backend.clone()
        ).await?;

        // Move the trie computation to rayon pool
        let backend = self.backend.clone();

        self.importer
            .run_in_rayon_pool_global(move |_| {
                let global_state_root = backend
                    .write_access()
                    .apply_to_global_trie(current_first_block, vec![accumulated_state_diff].iter())?;

                backend.write_latest_applied_trie_update(&latest_block.checked_sub(1))?;

                // Update snap sync marker - this path is only taken during snap sync mode
                backend.write_snap_sync_latest_block(&latest_block.checked_sub(1))?;

                tracing::info!("Global State Root till block {:?} is {:?}", &latest_block.checked_sub(1), global_state_root);
                Ok::<(), anyhow::Error>(())
            })
            .await?;

        // Clear the in-memory state_diff_map
        let mut state_diff_map = self.state_diff_map.lock().await;
        *state_diff_map = crate::sync_utils::StateDiffMap::default();

        Ok(())
    }

    pub fn set_target_block(&self, target: u64) {
        self.target_block.store(target, std::sync::atomic::Ordering::Relaxed);
    }
}

impl PipelineSteps for ApplyStateSteps {
    type InputItem = StateDiff;
    type SequentialStepInput = Vec<StateDiff>;
    type Output = ();

    async fn parallel_step(
        self: Arc<Self>,
        _block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> anyhow::Result<Self::SequentialStepInput> {
        Ok(input)
    }

    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>> {
        if self.disable_tries {
            return Ok(ApplyOutcome::Success(()));
        }

        self.sync(block_range, input).await
    }
}
