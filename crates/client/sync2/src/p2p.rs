use crate::{
    controller::{ApplyOutcome, PipelineSteps},
    import::{BlockImportError, BlockImporter, BlockValidationConfig},
    peer_set::PeerSet,
    util::AbortOnDrop,
};
use anyhow::Context;
use futures::Future;
use mc_db::MadaraBackend;
use mc_p2p::SyncHandlerError;
use mc_p2p::{P2pCommands, PeerId};
use std::{borrow::Cow, ops::Range, sync::Arc};

#[derive(Clone)]
pub struct P2pPipelineArguments {
    pub(crate) backend: Arc<MadaraBackend>,
    pub(crate) peer_set: Arc<PeerSet>,
    pub(crate) p2p_commands: P2pCommands,
    pub(crate) importer: Arc<BlockImporter>,
}

impl P2pPipelineArguments {
    pub fn new(backend: Arc<MadaraBackend>, p2p_commands: P2pCommands) -> Self {
        Self {
            importer: Arc::new(BlockImporter::new(backend.clone(), BlockValidationConfig::default())),
            backend,
            peer_set: Arc::new(PeerSet::new(p2p_commands.clone())),
            p2p_commands,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum P2pError {
    #[error("Internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
    #[error("Peer error: {0}")]
    Peer(Cow<'static, str>),
}

impl From<SyncHandlerError> for P2pError {
    fn from(value: SyncHandlerError) -> Self {
        match value {
            SyncHandlerError::Internal(err) => Self::Internal(err),
            SyncHandlerError::BadRequest(err) => Self::Peer(err),
            SyncHandlerError::EndOfStream => Self::peer_error("Stream ended unexpectedly"),
        }
    }
}

impl From<BlockImportError> for P2pError {
    fn from(value: BlockImportError) -> Self {
        if value.is_internal() {
            Self::Internal(anyhow::anyhow!(value))
        } else {
            Self::Peer(value.to_string().into())
        }
    }
}

impl P2pError {
    pub fn peer_error(err: impl Into<Cow<'static, str>>) -> Self {
        Self::Peer(err.into())
    }
}

pub trait P2pPipelineSteps: Send + Sync + 'static {
    type InputItem: Send + Sync + Clone;
    type SequentialStepInput: Send + Sync + 'static;
    type Output: Send + Sync + Clone;

    fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> impl Future<Output = Result<Self::SequentialStepInput, P2pError>> + Send;

    fn p2p_sequential_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_n_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> impl Future<Output = Result<Self::Output, P2pError>> + Send;

    fn starting_block_n(&self) -> Option<u64>;
}

pub struct P2pPipelineController<S: P2pPipelineSteps> {
    peer_set: Arc<PeerSet>,
    steps: Arc<S>,
}

impl<S: P2pPipelineSteps + Send + Sync + 'static> P2pPipelineController<S> {
    pub fn new(peer_set: Arc<PeerSet>, steps: S) -> Self {
        Self { peer_set, steps: Arc::new(steps) }
    }
}

// Note: we wrap the tasks in [`AbortOnDrop`] so that they can advance even when they are not polled.
// This may also allow the runtime to execute these functions other threads, but I am unsure how this
// would actually affect perf. The main reason is to make them advance even when the futures_unordered is not being polled.
impl<S: P2pPipelineSteps + Send + Sync + 'static> PipelineSteps for P2pPipelineController<S> {
    type InputItem = S::InputItem;
    type SequentialStepInput = (PeerId, S::SequentialStepInput);
    type Output = S::Output;

    async fn parallel_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> anyhow::Result<Self::SequentialStepInput> {
        AbortOnDrop::spawn(async move {loop {
            let peer_id = self.peer_set.next_peer().await.context("Getting peer from peer set")?;
            match self.steps.clone().p2p_parallel_step(peer_id, block_range.clone(), input.clone()).await {
                Ok(out) => return Ok((peer_id, out)),
                Err(P2pError::Peer(err)) => {
                    tracing::debug!("Retrying pipeline parallel step (block_n_range={block_range:?}) due to peer error: {err} [peer_id={peer_id}]");
                    self.peer_set.peer_operation_error(peer_id);
                }
                Err(P2pError::Internal(err)) => return Err(err.context("Peer to peer pipeline parallel step")),
            }
        }}).await
    }

    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        (peer_id, input): Self::SequentialStepInput,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>> {
        match self.steps.clone().p2p_sequential_step(peer_id, block_range.clone(), input).await {
            Ok(output) => {
                self.peer_set.peer_operation_success(peer_id);
                Ok(ApplyOutcome::Success(output))
            }
            Err(P2pError::Peer(err)) => {
                tracing::debug!("Retrying pipeline for block (block_n={block_range:?}) due to peer error during sequential step: {err} [peer_id={peer_id}]");
                self.peer_set.peer_operation_error(peer_id);
                Ok(ApplyOutcome::Retry)
            }
            Err(P2pError::Internal(err)) => Err(err.context("Peer to peer pipeline sequential step")),
        }
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.steps.starting_block_n()
    }
}
