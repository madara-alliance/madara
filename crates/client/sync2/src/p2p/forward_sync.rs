use super::peer_set::PeerSet;
use super::{
    classes::ClassesSync, events::EventsSync, headers::HeadersSync, state_diffs::StateDiffsSync,
    transactions::TransactionsSync,
};
use crate::import::BlockImporter;
use crate::p2p::pipeline::P2pError;
use crate::pipeline::{PipelineController, PipelineSteps};
use crate::sync::{ForwardPipeline, Probe, SyncController};
use crate::{apply_state::ApplyStateSync, p2p::P2pPipelineArguments};
use anyhow::Context;
use core::fmt;
use futures::StreamExt;
use mc_db::stream::BlockStreamConfig;
use mc_eth::state_update::L1HeadReceiver;
use mc_p2p::{P2pCommands, PeerId};
use std::iter;
use std::sync::Arc;

/// Pipeline order:
/// ```plaintext
///  ┌───────┐    ┌───────────┐     ┌───────┐           
///  │headers├─┬─►│state_diffs├──┬─►│classes│           
///  └───────┘ │  └───────────┘  │  └───────┘           
///            │                 │                      
///            │  ┌────────────┐ │  ┌──────────────────┐
///            ├─►│tx, receipts│ └─►│update_global_trie│
///            │  └────────────┘    └──────────────────┘
///            │                                        
///            │  ┌──────┐                              
///            └─►│events│                              
///               └──────┘                              
/// ```
/// State diffs, transactions with receipt, and events are checked against their corresponding commitments
/// in the header.
/// However, there is no commitment for classes: we instead check them against the state diff.
/// The update_global_trie step is a pipeline that only has a sequential part. It is separated from the state_diff step
/// because we want to apply big batches of state-diffs all at once in db, as an optimization. We want the batch size related
/// to that to be different from the batch size used to get state_diffs from p2p.
///
/// Headers are checked using the consensus signatures for forward sync.
///
/// ## Backwards mode
///
/// This is not implemented yet; however there will be a mode where we check the blocks
/// instead by going backwards from the latest block_hash verified on L1, and matching each earlier block with
/// the parent_hash of its successor. This mode won't need to check block signatures, but it can only help us
/// catch up with the latest block on L1. Syncing will switch in forward mode after that point, and consensus signatures
/// will be checked from that point on.
/// Until snap-sync is a thing, we also have to sync all state diffs in forward more.

pub struct ForwardSyncConfig {
    pub headers_parallelization: usize,
    pub headers_batch_size: usize,
    pub transactions_parallelization: usize,
    pub transactions_batch_size: usize,
    pub state_diffs_parallelization: usize,
    pub state_diffs_batch_size: usize,
    pub events_parallelization: usize,
    pub events_batch_size: usize,
    pub classes_parallelization: usize,
    pub classes_batch_size: usize,
    pub apply_state_parallelization: usize,
    pub apply_state_batch_size: usize,
}

impl Default for ForwardSyncConfig {
    fn default() -> Self {
        Self {
            headers_parallelization: 8,
            headers_batch_size: 8,
            transactions_parallelization: 8,
            transactions_batch_size: 4,
            state_diffs_parallelization: 8,
            state_diffs_batch_size: 4,
            events_parallelization: 8,
            events_batch_size: 4,
            classes_parallelization: 8,
            classes_batch_size: 4,
            apply_state_parallelization: 8,
            apply_state_batch_size: 4,
        }
    }
}

pub type P2pSync = SyncController<P2pForwardSync, P2pHeadersProbe>;
pub fn forward_sync(
    args: P2pPipelineArguments,
    l1_head_recv: L1HeadReceiver,
    stop_at_block_n: Option<u64>,
    config: ForwardSyncConfig,
) -> P2pSync {
    let probe = P2pHeadersProbe::new(args.peer_set.clone(), args.p2p_commands.clone(), args.importer.clone());
    SyncController::new(P2pForwardSync::new(args, config), l1_head_recv, stop_at_block_n, Some(probe.into()))
}

/// Events pipeline is currently always done after tx and receipts for now.
/// TODO: fix that when the db supports saving events separately.
pub struct P2pForwardSync {
    headers_pipeline: HeadersSync,
    transactions_pipeline: TransactionsSync,
    state_diffs_pipeline: StateDiffsSync,
    classes_pipeline: ClassesSync,
    events_pipeline: EventsSync,
    apply_state_pipeline: ApplyStateSync,
}

impl P2pForwardSync {
    pub fn new(args: P2pPipelineArguments, config: ForwardSyncConfig) -> Self {
        let headers_pipeline =
            super::headers::headers_pipeline(args.clone(), config.headers_parallelization, config.headers_batch_size);
        let transactions_pipeline = super::transactions::transactions_pipeline(
            args.clone(),
            config.transactions_parallelization,
            config.transactions_batch_size,
        );
        let state_diffs_pipeline = super::state_diffs::state_diffs_pipeline(
            args.clone(),
            config.state_diffs_parallelization,
            config.state_diffs_batch_size,
        );
        let classes_pipeline =
            super::classes::classes_pipeline(args.clone(), config.classes_parallelization, config.classes_batch_size);
        let events_pipeline =
            super::events::events_pipeline(args.clone(), config.events_parallelization, config.events_batch_size);
        let apply_state_pipeline = crate::apply_state::apply_state_pipeline(
            args.backend.clone(),
            args.importer.clone(),
            config.apply_state_parallelization,
            config.apply_state_batch_size,
        );

        Self {
            headers_pipeline,
            transactions_pipeline,
            state_diffs_pipeline,
            classes_pipeline,
            events_pipeline,
            apply_state_pipeline,
        }
    }
}

impl ForwardPipeline for P2pForwardSync {
    async fn run(&mut self, target_height: u64) -> anyhow::Result<()> {
        loop {
            tracing::debug!("stop_block={target_height:?}, hl={}", self.headers_pipeline.is_empty());

            while self.headers_pipeline.can_schedule_more()
                && self.headers_pipeline.next_input_block_n() <= target_height
            {
                self.headers_pipeline.push(iter::once(()));
            }

            tokio::select! {
                Some(res) = self.headers_pipeline.next(), if self.transactions_pipeline.can_schedule_more() && self.state_diffs_pipeline.can_schedule_more() => {
                    let (_range, headers) = res?;
                    self.transactions_pipeline.push(headers.iter().cloned());
                    self.state_diffs_pipeline.push(headers);
                }
                Some(res) = self.transactions_pipeline.next(), if self.events_pipeline.can_schedule_more() => {
                    let (_range, headers) = res?;
                    self.events_pipeline.push(headers);
                }
                Some(res) = self.events_pipeline.next() => {
                    res?;
                }
                Some(res) = self.state_diffs_pipeline.next(), if self.classes_pipeline.can_schedule_more() => {
                    let (_range, state_diffs) = res?;
                    self.classes_pipeline.push(state_diffs.iter().map(|s| s.all_declared_classes()));
                    self.apply_state_pipeline.push(state_diffs);
                }
                Some(res) = self.classes_pipeline.next() => {
                    res?;
                }
                Some(res) = self.apply_state_pipeline.next() => {
                    res?;
                }
                // all pipelines are empty, we're done :)
                else => break Ok(())
            }
        }
    }

    fn next_input_block_n(&self) -> u64 {
        self.headers_pipeline.next_input_block_n()
    }

    fn is_empty(&self) -> bool {
        self.headers_pipeline.is_empty()
            && self.transactions_pipeline.is_empty()
            && self.state_diffs_pipeline.is_empty()
            && self.classes_pipeline.is_empty()
            && self.events_pipeline.is_empty()
            && self.apply_state_pipeline.is_empty()
    }

    fn input_batch_size(&self) -> usize {
        self.headers_pipeline.input_batch_size()
    }

    fn show_status(&self, target_height: Option<u64>) {
        struct DisplayFromFn<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result>(F);
        impl<F: Fn(&mut fmt::Formatter<'_>) -> fmt::Result> fmt::Display for DisplayFromFn<F> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                (self.0)(f)
            }
        }
        fn show_pipeline<S: PipelineSteps>(
            f: &mut fmt::Formatter<'_>,
            name: &str,
            pipeline: &PipelineController<S>,
            target_height: Option<u64>,
        ) -> fmt::Result {
            let last_applied_block_n = pipeline.last_applied_block_n();
            write!(f, "{name}: ")?;

            if let Some(last_applied_block_n) = last_applied_block_n {
                write!(f, "{last_applied_block_n}")?;
            } else {
                write!(f, "N")?;
            }

            if let Some(target_height) = target_height {
                write!(f, "/{target_height}")?;
            } else {
                write!(f, "/?")?;
            }

            write!(f, " [{}", pipeline.queue_len())?;
            if pipeline.is_applying() {
                write!(f, "+")?;
            }
            write!(f, "]")?;

            Ok(())
        }

        tracing::info!(
            "{}",
            DisplayFromFn(move |f| {
                show_pipeline(f, "Headers", &self.headers_pipeline, target_height)?;
                write!(f, " | ")?;
                show_pipeline(f, "Txs", &self.transactions_pipeline, target_height)?;
                write!(f, " | ")?;
                show_pipeline(f, "StateDiffs", &self.state_diffs_pipeline, target_height)?;
                write!(f, " | ")?;
                show_pipeline(f, "Classes", &self.classes_pipeline, target_height)?;
                write!(f, " | ")?;
                show_pipeline(f, "Events", &self.events_pipeline, target_height)?;
                write!(f, " | ")?;
                show_pipeline(f, "State", &self.apply_state_pipeline, target_height)?;
                Ok(())
            })
        );
    }
}

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
