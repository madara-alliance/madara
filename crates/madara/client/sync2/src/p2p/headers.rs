use super::{
    pipeline::{P2pError, P2pPipelineController, P2pPipelineSteps},
    P2pPipelineArguments,
};
use crate::{import::BlockImporter, pipeline::PipelineController};
use anyhow::Context;
use futures::{StreamExt, TryStreamExt};
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_block::{BlockHeaderWithSignatures, BlockId, Header};
use starknet_core::types::Felt;
use std::{ops::Range, sync::Arc};

pub type HeadersSync = PipelineController<P2pPipelineController<HeadersSyncSteps>>;
pub fn headers_pipeline(
    P2pPipelineArguments { backend, peer_set, p2p_commands, importer }: P2pPipelineArguments,
    parallelization: usize,
    batch_size: usize,
) -> HeadersSync {
    PipelineController::new(
        P2pPipelineController::new(peer_set, HeadersSyncSteps { backend, p2p_commands, importer }),
        parallelization,
        batch_size,
    )
}

pub struct HeadersSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
    importer: Arc<BlockImporter>,
}

impl P2pPipelineSteps for HeadersSyncSteps {
    type InputItem = ();
    type SequentialStepInput = Vec<BlockHeaderWithSignatures>;
    type Output = Vec<Header>;

    async fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        _input: Vec<Self::InputItem>,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        tracing::debug!("p2p headers parallel step: {block_range:?}, peer_id: {peer_id}");
        let mut previous_block_hash = None;
        let mut block_n = block_range.start;
        let limit = block_range.end.saturating_sub(block_range.start);
        let res: Vec<_> = self
            .p2p_commands
            .clone()
            .make_headers_stream(peer_id, BlockStreamConfig::default().with_block_range(block_range.clone()))
            .await
            .take(limit as _)
            .map(move |signed_header| {
                let signed_header = signed_header?;
                // verify parent hash for batch
                if let Some(latest_block_hash) = previous_block_hash {
                    if latest_block_hash != signed_header.header.parent_block_hash {
                        return Err(P2pError::peer_error(format!(
                            "Mismatched parent_block_hash: {:#x}, expected {:#x}",
                            signed_header.header.parent_block_hash, latest_block_hash
                        )));
                    }
                }

                self.importer.verify_header(block_n, &signed_header)?;

                previous_block_hash = Some(signed_header.block_hash);
                block_n += 1;

                Ok(signed_header)
            })
            .try_collect()
            .await?;

        if res.len() as u64 != limit {
            return Err(P2pError::peer_error(format!(
                "Unexpected returned batch len: {}, expected {}",
                res.len(),
                limit
            )));
        }
        Ok(res)
    }

    async fn p2p_sequential_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> Result<Self::Output, P2pError> {
        tracing::debug!("p2p headers sequential step: {block_range:?}, peer_id: {peer_id}");
        let Some(first_block) = input.first() else {
            return Ok(vec![]);
        };

        // verify first block_hash matches with db
        let parent_block_n = first_block.header.block_number.checked_sub(1);
        let parent_block_hash = if let Some(block_n) = parent_block_n {
            self.backend
                .get_block_hash(&BlockId::Number(block_n))
                .context("Getting latest block hash from database.")?
                .context("Mismatched headers / chain head number.")?
        } else {
            Felt::ZERO // genesis' parent block
        };

        if first_block.header.parent_block_hash != parent_block_hash {
            return Err(P2pError::peer_error(format!(
                "Mismatched parent_block_hash: {:#x}, expected {parent_block_hash:#x}",
                first_block.header.parent_block_hash
            )));
        }

        tracing::debug!("Storing headers for {block_range:?}, peer_id: {peer_id}");
        for header in input.iter().cloned() {
            self.importer.save_header(header.header.block_number, header)?;
        }

        self.backend.head_status().headers.set(block_range.last());
        self.backend.save_head_status_to_db().context("Saving head status to db")?;

        Ok(input.into_iter().map(|h| h.header).collect())
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}
