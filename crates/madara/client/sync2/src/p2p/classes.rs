use super::{
    pipeline::{P2pError, P2pPipelineController, P2pPipelineSteps},
    P2pPipelineArguments,
};
use crate::{import::BlockImporter, pipeline::PipelineController};
use anyhow::Context;
use futures::TryStreamExt;
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_state_update::DeclaredClassCompiledClass;
use starknet_core::types::Felt;
use std::{collections::HashMap, ops::Range, sync::Arc};

pub type ClassesSync = PipelineController<P2pPipelineController<ClassesSyncSteps>>;
pub fn classes_pipeline(
    P2pPipelineArguments { backend, peer_set, p2p_commands, importer }: P2pPipelineArguments,
    parallelization: usize,
    batch_size: usize,
) -> ClassesSync {
    PipelineController::new(
        P2pPipelineController::new(peer_set, ClassesSyncSteps { backend, p2p_commands, importer }),
        parallelization,
        batch_size,
    )
}
pub struct ClassesSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
    importer: Arc<BlockImporter>,
}

impl P2pPipelineSteps for ClassesSyncSteps {
    /// All declared classes, extracted from state diff.
    type InputItem = HashMap<Felt, DeclaredClassCompiledClass>;
    type SequentialStepInput = ();
    type Output = ();

    async fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        if input.iter().all(|i| i.is_empty()) {
            return Ok(());
        }

        tracing::debug!("p2p classes parallel step: {block_range:?}, peer_id: {peer_id}");
        let strm = self
            .p2p_commands
            .clone()
            .make_classes_stream(
                peer_id,
                BlockStreamConfig::default().with_block_range(block_range.clone()),
                input.iter(),
            )
            .await;
        tokio::pin!(strm);

        for (block_n, check_against) in block_range.zip(input.iter().cloned()) {
            let classes = strm.try_next().await?.ok_or(P2pError::peer_error("Expected to receive item"))?;

            self.importer
                .run_in_rayon_pool(move |importer| {
                    let classes = importer.verify_compile_classes(block_n, classes, &check_against)?;
                    importer.save_classes(block_n, classes)
                })
                .await
        }

        Ok(())
    }

    async fn p2p_sequential_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        _input: Self::SequentialStepInput,
    ) -> Result<Self::Output, P2pError> {
        tracing::debug!("p2p classes sequential step: {block_range:?}, peer_id: {peer_id}");
        if let Some(block_n) = block_range.last() {
            self.backend.head_status().classes.set(Some(block_n));
            self.backend.save_head_status_to_db().context("Saving head status to db")?;
        }
        Ok(())
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}
