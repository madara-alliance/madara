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
        println!("block_range={:?}", block_range);

        let current_first_block = self.backend.get_latest_applied_trie_update()?.map(|n| n + 1).unwrap_or(0);
        let latest_block = block_range.end;
        println!("length of  state_diffs = {:?}", input.len());

        // Lock the mutex and apply state diffs
        {
            let already_imported_count = current_first_block.saturating_sub(block_range.start);
            let state_diffs = input.iter().skip(already_imported_count as _);

            println!("length of  state_diffs = {:?}", state_diffs.len());
            let mut state_diff_map = self.state_diff_map.lock().await;
            for single_contract_state_diff in state_diffs {
                state_diff_map.apply_state_diff(&single_contract_state_diff);
            }
        }

        println!("Current first block: {}", current_first_block);
        println!("Latest block: {}", latest_block);

        let target_block = self.target_block.load(std::sync::atomic::Ordering::Relaxed);

        if latest_block >= (current_first_block + 1000) || latest_block >= target_block {
            println!("End of state triggered, applying state diff");

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
                    // Apply the accumulated state diff to calculate the global state root
                    let global_state_root = backend
                        .write_access()
                        .apply_to_global_trie(current_first_block, vec![accumulated_state_diff].iter())?;

                    backend.write_latest_applied_trie_update(&latest_block.checked_sub(1))?;

                    println!("Global State Root till block {:?} is {:?}", latest_block.checked_sub(1), global_state_root);

                    Ok::<(), anyhow::Error>(())
                })
                .await
                .with_context(|| format!("Applying snap sync trie for block_range={block_range:?}"))?;

            // Clear the in-memory state_diff_map
            let mut state_diff_map = self.state_diff_map.lock().await;
            *state_diff_map = crate::sync_utils::StateDiffMap::default();
        }
        Ok(ApplyOutcome::Success(()))
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
        if self.snap_sync {
            self.sync_snap(block_range, input).await
        } else {
            self.sync_each_block(block_range, input).await
        }
    }
}
