use super::{
    classes::ClassesSync, events::EventsSync, headers::HeadersSync, state_diffs::StateDiffsSync,
    transactions::TransactionsSync,
};
use crate::import::BlockImporter;
use crate::metrics::SyncMetrics;
use crate::p2p::pipeline::P2pError;
use crate::probe::ThrottledRepeatedFuture;
use crate::sync::{ForwardPipeline, SyncController, SyncControllerConfig};
use crate::{apply_state::ApplyStateSync, p2p::P2pPipelineArguments};
use anyhow::Context;
use futures::TryStreamExt;
use mc_db::stream::BlockStreamConfig;
use mc_db::MadaraBackend;
use mc_p2p::{P2pCommands, PeerId};
use std::collections::HashSet;
use std::iter;
use std::sync::Arc;
use std::time::Duration;

/// Pipeline order:
/// ```plaintext
///  ┌───────┐    ┌───────────┐       ┌───────┐
///  │headers├─┬─►│state_diffs├────┬─►│classes│
///  └───────┘ │  └───────────┘    │  └───────┘
///            │                   │
///            │  ┌────────────┐   │  ┌──────────────────┐
///            └─►│tx, receipts├─┐ └─►│update_global_trie│
///               └────────────┘ │    └──────────────────┘
///                              │
///                              │  ┌──────┐
///                              └─►│events│
///                                 └──────┘
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
    pub disable_tries: bool,
}

impl Default for ForwardSyncConfig {
    fn default() -> Self {
        Self {
            headers_parallelization: 32,
            headers_batch_size: 8,
            transactions_parallelization: 32,
            transactions_batch_size: 4,
            state_diffs_parallelization: 32,
            state_diffs_batch_size: 4,
            events_parallelization: 32,
            events_batch_size: 4,
            classes_parallelization: 128,
            classes_batch_size: 1,
            apply_state_parallelization: 3,
            apply_state_batch_size: 5,
            disable_tries: false,
        }
    }
}

impl ForwardSyncConfig {
    #[allow(unused)]
    fn low() -> Self {
        Self {
            headers_parallelization: 1,
            headers_batch_size: 1,
            transactions_parallelization: 1,
            transactions_batch_size: 1,
            state_diffs_parallelization: 1,
            state_diffs_batch_size: 1,
            events_parallelization: 1,
            events_batch_size: 1,
            classes_parallelization: 1,
            classes_batch_size: 1,
            apply_state_parallelization: 1,
            apply_state_batch_size: 1,
            disable_tries: false,
        }
    }
    pub fn disable_tries(self, val: bool) -> Self {
        Self { disable_tries: val, ..self }
    }
}

pub type P2pSync = SyncController<P2pForwardSync>;
pub fn forward_sync(
    args: P2pPipelineArguments,
    controller_config: SyncControllerConfig,
    config: ForwardSyncConfig,
) -> P2pSync {
    let probe = Arc::new(P2pHeadersProbe::new(args.p2p_commands.clone(), args.importer.clone()));
    let probe = ThrottledRepeatedFuture::new(move |val| probe.clone().probe(val), Duration::from_secs(2));
    SyncController::new(P2pForwardSync::new(args, config), probe, controller_config, None)
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
    backend: Arc<MadaraBackend>,
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
            config.disable_tries,
        );

        Self {
            headers_pipeline,
            transactions_pipeline,
            state_diffs_pipeline,
            classes_pipeline,
            events_pipeline,
            apply_state_pipeline,
            backend: args.backend,
        }
    }

    fn pipeline_status(&self) -> PipelineStatus {
        PipelineStatus {
            headers: self.headers_pipeline.last_applied_block_n(),
            transactions: self.transactions_pipeline.last_applied_block_n(),
            events: self.events_pipeline.last_applied_block_n(),
            classes: self.classes_pipeline.last_applied_block_n(),
            apply_state: self.apply_state_pipeline.last_applied_block_n(),
        }
    }
}

#[derive(Clone)]
struct PipelineStatus {
    headers: Option<u64>,
    transactions: Option<u64>,
    events: Option<u64>,
    classes: Option<u64>,
    apply_state: Option<u64>,
}

impl PipelineStatus {
    pub fn min(&self) -> Option<u64> {
        self.headers.min(self.transactions).min(self.events).min(self.classes).min(self.apply_state)
    }
}

impl ForwardPipeline for P2pForwardSync {
    async fn run(
        &mut self,
        target_height: u64,
        _probe_height: Option<u64>,
        metrics: &mut SyncMetrics,
    ) -> anyhow::Result<()> {
        let mut done = false;
        while !done {
            tracing::trace!("stop_block={target_height:?}, hl={}", self.headers_pipeline.is_empty());

            while self.headers_pipeline.can_schedule_more()
                && self.headers_pipeline.next_input_block_n() <= target_height
            {
                let next_input_block_n = self.headers_pipeline.next_input_block_n();
                self.headers_pipeline.push(next_input_block_n..next_input_block_n + 1, iter::once(()));
            }

            // Min applied block
            let start_next_block = self.pipeline_status().min().map(|n| n + 1).unwrap_or(0);

            // We poll the consumers first. This seems to help bring the blocks/s higher.
            // Poll order being important makes me worry that we're not polled enough.
            // We would need to bring our own `FuturesOrdered` replacement to ensure that
            // we can `poll` the inner `FuturesUnordered` so that it can make progress even
            // when we are not trying to get a next element.
            // We can emulate this behaviour by wrapping all of the futures in `tokio::spawn`,
            // but it seems there is not much of a perf gain vs this order of polling.
            tokio::select! {
                Some(res) = self.apply_state_pipeline.next() => {
                    res?;
                }
                Some(res) = self.classes_pipeline.next() => {
                    res?;
                }
                Some(res) = self.state_diffs_pipeline.next(),
                    if self.classes_pipeline.can_schedule_more() && self.apply_state_pipeline.can_schedule_more() =>
                {
                    let (range, state_diffs) = res?;
                    self.classes_pipeline.push(range.clone(), state_diffs.iter().map(|s| s.all_declared_classes()));
                    self.apply_state_pipeline.push(range, state_diffs);
                }
                Some(res) = self.events_pipeline.next() => {
                    res?;
                }
                Some(res) = self.transactions_pipeline.next(), if self.events_pipeline.can_schedule_more() => {
                    let (range, headers) = res?;
                    self.events_pipeline.push(range, headers);
                }
                Some(res) = self.headers_pipeline.next(), if self.transactions_pipeline.can_schedule_more() && self.state_diffs_pipeline.can_schedule_more() => {
                    let (range, headers) = res?;
                    self.transactions_pipeline.push(range.clone(), headers.iter().cloned());
                    self.state_diffs_pipeline.push(range, headers);
                }
                // all pipelines are empty, we're done :)
                else => done = true
            }

            let new_next_block = self.pipeline_status().min().map(|n| n + 1).unwrap_or(0);
            for block_n in start_next_block..new_next_block {
                // Notify of a new full block here.
                self.backend.on_block(block_n).await?;
                metrics.update(block_n, &self.backend).context("Updating metrics")?;
            }
        }
        Ok(())
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

    fn show_status(&self) {
        tracing::info!(
            "📥 Headers: {} | Txs: {} | StateDiffs: {} | Classes: {} | Events: {} | State: {}",
            self.headers_pipeline.status(),
            self.transactions_pipeline.status(),
            self.state_diffs_pipeline.status(),
            self.classes_pipeline.status(),
            self.events_pipeline.status(),
            self.apply_state_pipeline.status(),
        );
    }

    fn latest_block(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}

struct P2pHeadersProbe {
    p2p_commands: P2pCommands,
    importer: Arc<BlockImporter>,
}
impl P2pHeadersProbe {
    pub fn new(p2p_commands: P2pCommands, importer: Arc<BlockImporter>) -> Self {
        Self { p2p_commands, importer }
    }

    async fn block_exists_at(self: Arc<Self>, block_n: u64, peers: &HashSet<PeerId>) -> anyhow::Result<bool> {
        let max_attempts = 4;

        for peer_id in peers.iter().take(max_attempts) {
            tracing::debug!("Probe exists_at checking {peer_id}, {block_n}");
            match self.clone().block_exists_at_inner(*peer_id, block_n).await {
                // Found it
                Ok(true) => return Ok(true),
                // Retry with another peer
                Ok(false) => continue,

                Err(P2pError::Peer(err)) => {
                    tracing::debug!(
                        "Retrying probing step (block_n={block_n:?}) due to peer error: {err:#} [peer_id={peer_id}]"
                    );
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
        let signed_header = match strm.try_next().await {
            Ok(None) | Err(mc_p2p::SyncHandlerError::EndOfStream) => return Ok(false),
            Ok(Some(signed_header)) => signed_header,
            Err(err) => return Err(err.into()),
        };

        self.importer.ctx().verify_header(block_n, &signed_header)?;

        Ok(true)
    }
    async fn probe(self: Arc<Self>, mut highest_known_block: Option<u64>) -> anyhow::Result<Option<u64>> {
        let current_next_block = highest_known_block.map(|n| n + 1).unwrap_or(0);
        tracing::debug!("Forward probe current_next_block={current_next_block}",);

        // powers of two
        let checks = iter::successors(Some(1u64), |n| n.checked_mul(2)).map(|n| current_next_block + n - 1);

        let mut peers = self.p2p_commands.clone().get_random_peers().await;
        peers.remove(&self.p2p_commands.peer_id()); // remove ourselves
        tracing::debug!("Probe got {} peers", peers.len());

        for (i, block_n) in checks.into_iter().enumerate() {
            tracing::debug!("Probe check {i} current_next_block={current_next_block} {block_n}");

            if !self.clone().block_exists_at(block_n, &peers).await? {
                return Ok(highest_known_block);
            }
            highest_known_block = Some(block_n);
        }

        Ok(highest_known_block)
    }
}
