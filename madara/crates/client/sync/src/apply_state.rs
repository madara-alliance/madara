use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
};
use anyhow::Context;
use mc_db::MadaraBackend;
use mp_state_update::StateDiff;
use std::{ops::Range, sync::Arc};

pub type ApplyStateSync = PipelineController<ApplyStateSteps>;
pub fn apply_state_pipeline(
    _backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    starting_block_n: u64,
    parallelization: usize,
    batch_size: usize,
    disable_tries: bool,
) -> ApplyStateSync {
    PipelineController::new(ApplyStateSteps { importer, disable_tries }, parallelization, batch_size, starting_block_n)
}
pub struct ApplyStateSteps {
    importer: Arc<BlockImporter>,
    disable_tries: bool,
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
        tracing::debug!("Apply state sequential step {block_range:?}");

        let block_range_ = block_range.clone();
        // Importer is in charge of setting the head status.
        self.importer
            .run_in_rayon_pool_global(move |importer| importer.apply_to_global_trie(block_range_, input))
            .await
            .with_context(|| format!("Applying global trie step for block_range={block_range:?}"))?;
        Ok(ApplyOutcome::Success(()))
    }
}
