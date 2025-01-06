use anyhow::Context;
use async_trait::async_trait;
use core::task;
use futures::channel::mpsc;
use futures::future::OptionFuture;
use futures::{stream, SinkExt, Stream};
use futures::{stream::FuturesOrdered, Future, StreamExt, TryStreamExt};
use mc_db::stream::{BlockStreamConfig, Direction};
use mc_db::MadaraBackend;
use mc_p2p::{P2pCommands, PeerId};
use mp_block::{BlockHeaderWithSignatures, BlockId, BlockTag, Header};
use mp_convert::ToFelt;
use starknet_core::types::Felt;
use std::iter;
use std::ops::Range;
use std::{borrow::Cow, marker::PhantomData, pin::Pin, sync::Arc};
use tokio::task::JoinHandle;

/// (peer_id, T) tuple
pub struct PeerWithData<T> {
    pub peer_id: PeerId,
    pub data: T,
}

pub struct AbortOnDrop<T>(JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort()
    }
}
impl<T> Future for AbortOnDrop<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> task::Poll<Self::Output> {
        // Panic: the task is never aborted, except on drop in which case it cannot be polled again.
        Pin::new(&mut self.get_mut().0).poll(cx).map(|r| r.expect("Join error"))
    }
}
impl<T> From<JoinHandle<T>> for AbortOnDrop<T> {
    fn from(value: JoinHandle<T>) -> Self {
        Self(value)
    }
}

pub struct PeerSet {
    // get_peers_mutex:
    // commands: P2pCommands,
}

impl PeerSet {
    /// Returns the next peer to use. If there is are no peers currently in the set,
    /// it will start a get random peers command.
    // TODO: keep track of the current number of request per peer, and avoid over-using a single peer.
    // TODO: if we really have to use a peer that just had a peer operation error, delay the next peer request a little
    // bit so that we don't spam that peer with requests.
    pub async fn next_peer(&self) -> anyhow::Result<PeerId> {
        todo!()
    }

    /// Signal that the peer did not follow the protocol correctly, sent bad data or timed out.
    /// We may want to avoid this peer in the future.
    pub fn peer_operation_error(&self, peer_id: PeerId) {}

    /// Signal that the operation with the peer was successful.
    ///
    // TODO: add a bandwidth argument to allow the peer set to score and avoid being drip-fed.
    pub fn peer_operation_success(&self, peer_id: PeerId) {}
}

/// I is the input to retry with.
pub enum ApplyOutcome<I> {
    Success,
    Retry(I),
}

#[derive(Debug, thiserror::Error)]
pub enum P2pError {
    #[error("Internal error: {0:#}")]
    Internal(#[from] anyhow::Error),
    #[error("Peer error: {0}")]
    Peer(Cow<'static, str>),
}

impl P2pError {
    pub fn peer_error(err: impl Into<Cow<'static, str>>) -> Self {
        Self::Peer(err.into())
    }
}

#[async_trait]
pub trait P2pPipelineSteps {
    type InputStream: Stream<Item = Self::ParallelStepInput> + Send + Sync + Unpin + 'static;
    type ParallelStepInput: Clone + Send + Sync + 'static;
    type SequentialStepInput: Send + Sync + 'static;
    async fn p2p_parallel_step(
        self: Arc<Self>,
        block_n_range: Range<u64>,
        peer_id: PeerId,
        input: Self::ParallelStepInput,
        block_n_processed: &mut u64,
    ) -> Result<Self::SequentialStepInput, P2pError>;
    fn has_sequential_step(&self) -> bool {
        false
    }
    async fn p2p_sequential_step(
        self: Arc<Self>,
        _block_n_range: Range<u64>,
        _peer_id: PeerId,
        _input: Self::SequentialStepInput,
    ) -> Result<(), P2pError> {
        Ok(())
    }
}

pub struct P2pPipelineController<S: P2pPipelineSteps + Send + Sync + 'static> {
    peer_set: Arc<PeerSet>,
    steps: Arc<S>,
}

impl<S: P2pPipelineSteps + Send + Sync + 'static> P2pPipelineController<S> {
    pub fn new(peer_set: Arc<PeerSet>, steps: S) -> Self {
        Self { peer_set, steps: Arc::new(steps) }
    }
}

impl<S: P2pPipelineSteps + Send + Sync + 'static> PipelineSteps for P2pPipelineController<S> {
    // We need to keep the parallel step input for retry logic.
    type SequentialStepInput = PeerWithData<(S::SequentialStepInput, S::ParallelStepInput)>;
    type InputStream = S::InputStream;

    fn parallel_step(
        &self,
        block_n_range: Range<u64>,
        input: <S::InputStream as Stream>::Item,
    ) -> AbortOnDrop<anyhow::Result<Self::SequentialStepInput>> {
        let peer_set = Arc::clone(&self.peer_set);
        let steps = Arc::clone(&self.steps);
        tokio::spawn(async move {
            loop {
                let peer_id = peer_set.next_peer().await.context("Getting peer from peer set")?;
                let block_n_processed = 0;
                match Arc::clone(&steps).p2p_parallel_step(block_n_range.clone(), peer_id, input.clone(), &mut block_n_processed).await {
                    Ok(out) => return Ok(PeerWithData { peer_id, data: (out, input) }),
                    Err(P2pError::Peer(err)) => {
                        tracing::debug!("Retrying pipeline parallel step (block_n_range={block_n_range:?}) due to peer error: {err} [peer_id={peer_id}]");
                        peer_set.peer_operation_error(peer_id);
                    }
                    Err(P2pError::Internal(err)) => {
                        return Err(err.context("Peer to peer pipeline parallel step"))
                    }
                }
            }
        })
        .into()
    }

    fn sequential_step(
        &self,
        block_n_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> Option<AbortOnDrop<anyhow::Result<ApplyOutcome<<Self::InputStream as Stream>::Item>>>> {
        let peer_set = Arc::clone(&self.peer_set);
        if !self.steps.has_sequential_step() {
            return None;
        }
        let steps: Arc<S> = Arc::clone(&self.steps);
        Some(tokio::spawn(async move {
            let peer_id = peer_set.next_peer().await.context("Getting peer from peer set")?;
            let (input, retry_input) = input.data;
            match Arc::clone(&steps).p2p_sequential_step(block_n_range.clone(), peer_id, input).await {
                Ok(()) => {
                    peer_set.peer_operation_success(peer_id);
                    Ok(ApplyOutcome::Success)
                },
                Err(P2pError::Peer(err)) => {
                        tracing::debug!("Retrying all pipeline for block (block_n={block_n_range:?}) due to peer error during sequential step: {err} [peer_id={peer_id}]");
                    peer_set.peer_operation_error(peer_id);
                    Ok(ApplyOutcome::Retry(retry_input))
                }
                Err(P2pError::Internal(err)) => {
                    Err(err.context("Peer to peer pipeline sequential step"))
                }
            }
        })
        .into())
    }
}

pub trait PipelineSteps {
    type SequentialStepInput: Send + Sync;
    type InputStream: Stream + Send + Sync + Unpin;

    fn parallel_step(
        &self,
        block_n_range: Range<u64>,
        input: <Self::InputStream as Stream>::Item,
    ) -> AbortOnDrop<anyhow::Result<Self::SequentialStepInput>>;
    fn sequential_step(
        &self,
        _block_n_range: Range<u64>,
        _input: Self::SequentialStepInput,
    ) -> Option<AbortOnDrop<anyhow::Result<ApplyOutcome<<Self::InputStream as Stream>::Item>>>> {
        None
    }
}

pub struct SyncStatus {
    // pub height: u64,
}

pub struct PipelineController<S: PipelineSteps> {
    steps: S,
    input: S::InputStream,
    queue: FuturesOrdered<AbortOnDrop<anyhow::Result<S::SequentialStepInput>>>,
    parallelization: usize,
    batch_size: usize,
    /// next block that will be returned by the queue
    next_block_n: u64,
    applying: Option<AbortOnDrop<anyhow::Result<ApplyOutcome<<S::InputStream as Stream>::Item>>>>,
}

impl<S: PipelineSteps> PipelineController<S> {
    pub fn new(input: S::InputStream, steps: S, parallelization: usize, batch_size: usize) -> Self {
        Self { input, steps, queue: Default::default(), parallelization, batch_size, next_block_n: 0, applying: None }
    }

    /// The returned future can be dropped and this function can be re-called at any moment.
    pub async fn run(&mut self, target_height: u64) -> anyhow::Result<()> {
        loop {
            // Add futures at the back of the queue if we have room.
            let extra_room = usize::saturating_sub(self.parallelization, self.queue.len());
            let blocks_remaining = u64::saturating_sub(target_height + 1, self.next_block_n);
            let futures_to_add = u64::min(extra_room as _, blocks_remaining / self.batch_size as u64);

            // `next_block_n` is the number of the next block that will be returned by the `self.queue`.
            // batch_size = 16, paralellization = 3, queue_len = 2, next_block_n = 16, target_height = 72
            //
            // BLOCK 0      | BLOCK 16 | BLOCK 32 | BLOCK 48 | BLOCK 64 | BLOCK 72...
            //  *processed* |  *queued*           |  *not yet queued*   |
            //              ^                     ^                     ^
            //            next_block_n          queue_tail            target_height

            let queue_tail = self.next_block_n + self.queue.len() as u64;
            let not_yet_queued_len = target_height.saturating_sub(queue_tail);
            let n_batches_to_queue = not_yet_queued_len / self.batch_size as u64;
            let last_batch_len = not_yet_queued_len % self.batch_size as u64;

            tokio::select! {
                Some(input) = self.input.next(), if self.queue.len() < self.parallelization && n_batches_to_queue > 0 => {
                    let block_range_len = if n_batches_to_queue == 1 {
                        last_batch_len
                    } else {
                        self.batch_size as u64
                    };
                    let block_range = queue_tail..(queue_tail + block_range_len);

                    self.queue.push_back(self.steps.parallel_step(block_range, input));
                },
                Some(outcome) = OptionFuture::from(self.applying.as_mut()) => {
                    match outcome? {
                        ApplyOutcome::Success => {
                            self.applying = None;
                            self.next_block_n += self.batch_size as u64;
                        }
                        ApplyOutcome::Retry(input) => self.queue.push_front(self.steps.parallel_step(self.next_block_n, input)),
                    }
                }
                Some(res) = self.queue.next() => {
                    self.applying = self.steps.sequential_step(self.next_block_n, res?);
                }
                else => break,
            };
        }
        Ok(())
    }
}

// headers sync

pub type HeadersSync = PipelineController<P2pPipelineController<HeadersSyncSteps>>;
pub fn headers_pipeline(
    backend: Arc<MadaraBackend>,
    peer_set: Arc<PeerSet>,
    p2p_commands: P2pCommands,
    parallelization: usize,
    batch_size: usize,
    header_senders: HeaderSenders,
) -> HeadersSync {
    PipelineController::new(
        stream::repeat(()),
        P2pPipelineController::new(peer_set, HeadersSyncSteps { backend, p2p_commands, header_senders }),
        parallelization,
        batch_size,
    )
}

pub struct HeadersSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
    header_senders: HeaderSenders,
}

#[async_trait]
impl P2pPipelineSteps for HeadersSyncSteps {
    type SequentialStepInput = Vec<BlockHeaderWithSignatures>;
    type ParallelStepInput = ();
    /// Noop infinite stream (no input)
    type InputStream = stream::Repeat<()>;

    async fn p2p_parallel_step(
        self: Arc<Self>,
        block_n_range: Range<u64>,
        peer_id: PeerId,
        _input: Self::ParallelStepInput,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        let mut previous_block_hash = None;
        let mut block_n = block_n_range.start;
        let limit = block_n_range.end.saturating_sub(block_n_range.start);
        let res: Vec<_> = self
            .p2p_commands
            .clone()
            .make_headers_stream(
                peer_id,
                BlockStreamConfig { start: block_n_range.start, limit: Some(limit), ..Default::default() },
            )
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

        if res.len() != limit as _ {
            return Err(P2pError::peer_error(format!(
                "Unexpected returned batch len: {}, expected {}",
                res.len(),
                limit
            )));
        }
        Ok(res)
    }

    fn has_sequential_step(&self) -> bool {
        true
    }

    async fn p2p_sequential_step(
        self: Arc<Self>,
        block_n_range: Range<u64>,
        peer_id: PeerId,
        input: Self::SequentialStepInput,
    ) -> Result<(), P2pError> {
        // verify first block_hash matches with db
        let parent_block_hash = self
            .backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))
            .context("Getting latest block hash from database.")?
            .unwrap_or(/* No block in db, this is genesis' parent_block_hash */ Felt::ZERO);

        let senders = self.header_senders.clone();
        for header in input {
            // we could parallelize that?
            self.backend.store_block_header(header.clone()).context("Storing block header")?;
            senders.send(header).await;
        }

        // store into db
        Ok(())
    }
}

pub type TransactionsSync = PipelineController<P2pPipelineController<TransactionsSyncSteps>>;
pub fn transactions_pipeline(
    backend: Arc<MadaraBackend>,
    peer_set: Arc<PeerSet>,
    p2p_commands: P2pCommands,
    parallelization: usize,
    batch_size: usize,
    headers: mpsc::Receiver<BlockHeaderWithSignatures>,
) -> TransactionsSync {
    PipelineController::new(
        headers,
        P2pPipelineController::new(peer_set, TransactionsSyncSteps { batch_size, backend, p2p_commands }),
        parallelization,
        batch_size,
    )
}
pub struct TransactionsSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
    batch_size: usize,
}

#[async_trait]
impl P2pPipelineSteps for TransactionsSyncSteps {
    type SequentialStepInput = ();
    type ParallelStepInput = BlockHeaderWithSignatures;
    type InputStream = mpsc::Receiver<BlockHeaderWithSignatures>;

    async fn p2p_parallel_step(
        self: Arc<Self>,
        block_n_range: Range<u64>,
        peer_id: PeerId,
        input: Vec<BlockHeaderWithSignatures>,
        block_n_processed: &mut u64,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        let limit = block_n_range.end.saturating_sub(block_n_range.start);

        let mut strm = self
            .p2p_commands
            .clone()
            .make_transactions_stream(
                peer_id,
                BlockStreamConfig { start: block_n_range.start, limit: Some(limit), ..Default::default() },
            )
            .await;
        tokio::pin!(strm);

        for (block_n, signed_header) in block_n_range.zip(input) {
            let mut transactions = Vec::with_capacity(signed_header.header.transaction_count as _);
            for _index in 0..signed_header.header.transaction_count {
                let received = strm.next().await.ok_or(P2pError::peer_error("Expected to receive item"))?;
                transactions.push(received);
            }

            
            // save transactions for block_n
            *block_n_processed += 1;
        }

        Ok(())
    }
}

pub async fn forward_sync(backend: Arc<MadaraBackend>) {
    let target_height = 999999;

    let (senders, receivers) =
        HeaderSenders::new(/* allowed lag between header tip and the other pipelines */ 100);

    tokio::join!(
        tokio::spawn(headers_pipeline(backend, peer_set, p2p_commands, 100, 16, senders).run(target_height)).into(),
        tokio::spawn(
            transactions_pipeline(backend, peer_set, p2p_commands, 100, 16, receivers.transactions).run(target_height)
        )
        .into(),
    );
}

pub struct HeaderReceivers {
    pub transactions: mpsc::Receiver<BlockHeaderWithSignatures>,
}

/// Implement backpressure so that we avoid having a huge gap between the header pipeline and the other pipelines.
pub struct HeaderSenders {
    transactions: mpsc::Sender<BlockHeaderWithSignatures>,
}

impl HeaderSenders {
    pub fn new(buffer: usize) -> (Self, HeaderReceivers) {
        let (s1, r1) = mpsc::channel(buffer);

        (Self { transactions: s1 }, HeaderReceivers { transactions: r1 })
    }

    pub async fn send(&mut self, header: BlockHeaderWithSignatures) {
        let (_res1,) = tokio::join!(self.transactions.send(header),);
    }
}
