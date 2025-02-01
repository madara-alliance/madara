use super::{
    pipeline::{P2pError, P2pPipelineController, P2pPipelineSteps},
    P2pPipelineArguments,
};
use crate::{import::BlockImporter, pipeline::PipelineController};
use anyhow::Context;
use futures::TryStreamExt;
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_block::Header;
use std::{ops::Range, sync::Arc};

pub type EventsSync = PipelineController<P2pPipelineController<EventsSyncSteps>>;
pub fn events_pipeline(
    P2pPipelineArguments { backend, peer_set, p2p_commands, importer }: P2pPipelineArguments,
    parallelization: usize,
    batch_size: usize,
) -> EventsSync {
    PipelineController::new(
        P2pPipelineController::new(peer_set, EventsSyncSteps { backend, p2p_commands, importer }),
        parallelization,
        batch_size,
    )
}

pub struct EventsSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
    importer: Arc<BlockImporter>,
}

impl P2pPipelineSteps for EventsSyncSteps {
    type InputItem = Header;
    type SequentialStepInput = ();
    type Output = ();

    async fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        if input.iter().all(|i| i.event_count == 0) {
            return Ok(());
        }

        tracing::debug!("p2p events parallel step: {block_range:?}, peer_id: {peer_id}");
        let strm = self
            .p2p_commands
            .clone()
            .make_events_stream(
                peer_id,
                BlockStreamConfig::default().with_block_range(block_range.clone()),
                input.iter().map(|header| header.event_count as _).collect::<Vec<_>>(),
            )
            .await;
        tokio::pin!(strm);

        for (block_n, header) in block_range.zip(input) {
            let events = strm.try_next().await?.ok_or(P2pError::peer_error("Expected to receive item"))?;
            self.importer
                .run_in_rayon_pool(move |importer| {
                    importer.verify_events(block_n, &events, &header, /* allow_pre_v0_13_2 */ false)?;
                    importer.save_events(block_n, events)
                })
                .await?
        }

        Ok(())
    }

    async fn p2p_sequential_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        _input: Self::SequentialStepInput,
    ) -> Result<Self::Output, P2pError> {
        tracing::debug!("p2p events sequential step: {block_range:?}, peer_id: {peer_id}");
        if let Some(block_n) = block_range.last() {
            self.backend.head_status().events.set(Some(block_n));
            self.backend.save_head_status_to_db().context("Saving head status to db")?;
        }
        Ok(())
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}
