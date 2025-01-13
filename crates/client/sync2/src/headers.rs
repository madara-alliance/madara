// headers sync

use crate::{
    controller::PipelineController,
    p2p::{P2pError, P2pPipelineController, P2pPipelineSteps},
    peer_set::PeerSet,
};
use anyhow::Context;
use futures::{StreamExt, TryStreamExt};
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_block::{BlockHeaderWithSignatures, BlockId, BlockTag};
use mp_convert::ToFelt;
use starknet_core::types::Felt;
use std::{ops::Range, sync::Arc};

pub type HeadersSync = PipelineController<P2pPipelineController<HeadersSyncSteps>>;
pub fn headers_pipeline(
    backend: Arc<MadaraBackend>,
    peer_set: Arc<PeerSet>,
    p2p_commands: P2pCommands,
    parallelization: usize,
    batch_size: usize,
) -> HeadersSync {
    PipelineController::new(
        P2pPipelineController::new(peer_set, HeadersSyncSteps { backend, p2p_commands }),
        parallelization,
        batch_size,
    )
}

pub struct HeadersSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
}

impl P2pPipelineSteps for HeadersSyncSteps {
    type InputItem = ();
    type SequentialStepInput = Vec<BlockHeaderWithSignatures>;
    type Output = Vec<BlockHeaderWithSignatures>;

    async fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        _input: Vec<Self::InputItem>,
    ) -> Result<Self::SequentialStepInput, P2pError> {
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
                // TODO: verify signatures

                // verify parent hash for batch
                if let Some(latest_block_hash) = previous_block_hash {
                    if latest_block_hash != signed_header.header.parent_block_hash {
                        return Err(P2pError::peer_error(format!(
                            "Mismatched parent_hash: {}, expected {}",
                            signed_header.header.parent_block_hash, latest_block_hash
                        )));
                    }
                }
                previous_block_hash = Some(signed_header.block_hash);

                // verify block_number
                if block_n != signed_header.header.block_number {
                    return Err(P2pError::peer_error(format!(
                        "Mismatched block_number: {}, expected {}",
                        signed_header.header.block_number, block_n
                    )));
                }
                block_n += 1;

                // verify block_hash
                // TODO: pre_v0_13_2_override
                let block_hash = signed_header
                    .header
                    .compute_hash(self.backend.chain_config().chain_id.to_felt(), /* pre_v0_13_2_override */ true);
                if signed_header.block_hash != block_hash {
                    return Err(P2pError::peer_error(format!(
                        "Mismatched block_hash: {}, expected {}",
                        signed_header.block_hash, block_hash
                    )));
                }

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
        _peer_id: PeerId,
        _block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> Result<Self::Output, P2pError> {
        let Some(first_block) = input.first() else {
            return Ok(vec![]);
        };
        let first_block_hash = first_block.block_hash;

        // verify first block_hash matches with db
        let parent_block_hash = self
            .backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))
            .context("Getting latest block hash from database.")?
            .unwrap_or(/* No block in db, this is genesis' parent_block_hash */ Felt::ZERO);

        if first_block_hash != parent_block_hash {
            return Err(P2pError::peer_error(format!(
                "Mismatched block_hash: {first_block_hash}, expected {parent_block_hash}"
            )));
        }

        for header in input.iter().cloned() {
            self.backend.store_block_header(header).context("Storing block header")?;
        }

        Ok(input)
    }
}
