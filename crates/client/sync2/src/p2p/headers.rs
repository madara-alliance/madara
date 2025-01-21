use super::{
    controller::{P2pError, P2pPipelineController, P2pPipelineSteps},
    peer_set::PeerSet,
    sync::Probe,
    P2pPipelineArguments,
};
use crate::{controller::PipelineController, import::BlockImporter};
use anyhow::Context;
use futures::{StreamExt, TryStreamExt};
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_block::{BlockHeaderWithSignatures, BlockId, Header};
use starknet_core::types::Felt;
use std::{ops::Range, sync::Arc};

pub struct P2pHeadersProbe {
    peer_set: Arc<PeerSet>,
    p2p_commands: P2pCommands,
    importer: Arc<BlockImporter>,
}
impl P2pHeadersProbe {
    pub fn new(peer_set: Arc<PeerSet>, p2p_commands: P2pCommands, importer: Arc<BlockImporter>) -> Self {
        Self { peer_set, p2p_commands, importer }
    }

    async fn block_exists_at(self: Arc<Self>, block_n: u64) -> anyhow::Result<bool> {
        let attempts = 5;

        tracing::debug!("testing block at {block_n}");

        // TODO: prefer a random peer instead of a scored one, try to avoid peer overlap
        for _peer in 0..attempts {
            let peer_guard = self.peer_set.clone().next_peer().await.context("Getting peer from peer set")?;
            match self.clone().block_exists_at_inner(*peer_guard, block_n).await {
                // Found it
                Ok(true) => return Ok(true),
                // Retry with another peer
                Ok(false) => continue,

                Err(P2pError::Peer(err)) => {
                    tracing::debug!(
                        "Retrying probing step (block_n={block_n:?}) due to peer error: {err:#} [peer_id={}]",
                        *peer_guard
                    );
                    peer_guard.error();
                }
                Err(P2pError::Internal(err)) => return Err(err.context("Peer to peer probing step")),
            }
        }
        Ok(false)
    }
    async fn block_exists_at_inner(self: Arc<Self>, peer_id: PeerId, block_n: u64) -> Result<bool, P2pError> {
        let strm = self
            .p2p_commands
            .clone()
            .make_headers_stream(peer_id, BlockStreamConfig::default().with_limit(1).with_start(block_n))
            .await;
        tokio::pin!(strm);
        let signed_header = match strm.next().await.transpose() {
            Ok(None) | Err(mc_p2p::SyncHandlerError::EndOfStream) => return Ok(false),
            Ok(Some(signed_header)) => signed_header,
            Err(err) => return Err(err.into()),
        };

        self.importer.verify_header(block_n, &signed_header)?;

        Ok(true)
    }
}

#[derive(Default, Debug, Clone)]
pub struct ProbeState {
    highest_known_block: Option<u64>,
}

const N_BATCHES_LOOKAHEAD: usize = 8;
impl Probe for P2pHeadersProbe {
    type State = ProbeState;

    fn highest_known_block_from_state(&self, state: &Self::State) -> Option<u64> {
        state.highest_known_block
    }

    async fn forward_probe(
        self: Arc<Self>,
        current_next_block: u64,
        batch_size: usize,
        mut state: Self::State,
    ) -> anyhow::Result<Self::State> {
        // current_block_n <= chain_head

        tracing::debug!("FORWARD PROBE {state:?} current_next_block={current_next_block}",);

        let checks = [
            // first test: 1 blocks away.
            // second test: batch_size - 1 blocks away.
            // third test: N_BATCHES_LOOKAHEAD * batch_size - 1 blocks away.
            current_next_block,
            current_next_block + (batch_size - 1) as u64,
            current_next_block + (N_BATCHES_LOOKAHEAD * batch_size - 1) as u64,
        ];

        state.highest_known_block = current_next_block.checked_sub(1);
        for (i, block_n) in checks.into_iter().enumerate() {
            tracing::debug!("PROBE CHECK {i} current_next_block={current_next_block} {block_n}");

            if !self.clone().block_exists_at(block_n).await? {
                return Ok(state);
            }
            state.highest_known_block = Some(block_n);
        }

        Ok(state)
    }
}

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
        let parent_block_hash = if let Some(block_n) = self.backend.head_status().headers.get() {
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

        Ok(input.into_iter().map(|h| h.header).collect())
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().headers.get()
    }
}
