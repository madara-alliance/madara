use crate::metrics::BlockProductionMetrics;
use crate::util::{AdditionalTxInfo, BatchToExecute};
use crate::MempoolIntakeMode;
use anyhow::Context;
use futures::{
    stream::{self, BoxStream, PollNext},
    StreamExt, TryStreamExt,
};
use mc_db::MadaraBackend;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_convert::ToFelt;
use mp_transactions::{
    validated::{TxTimestamp, ValidatedTransaction},
    L1HandlerTransactionWithFee,
};
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time::Instant;

pub struct Batcher {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    metrics: Arc<BlockProductionMetrics>,
    l1_message_stream: BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>>,
    ctx: ServiceContext,
    out: mpsc::Sender<BatchToExecute>,
    bypass_in: mpsc::Receiver<ValidatedTransaction>,
    mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
    batch_size: usize,
}

enum BatcherStep {
    Batch(BatchToExecute),
    RebuildStreams,
    Stop,
}

impl Batcher {
    /// Wires the three transaction sources into the executor batch output.
    /// The resulting task owns source prioritization and applies channel backpressure.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
        ctx: ServiceContext,
        out: mpsc::Sender<BatchToExecute>,
        bypass_in: mpsc::Receiver<ValidatedTransaction>,
        mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
    ) -> Self {
        Self {
            mempool,
            metrics,
            l1_message_stream: l1_client.create_message_to_l2_consumer(),
            ctx,
            out,
            bypass_in,
            mempool_intake_rx,
            batch_size: backend.chain_config().block_production_concurrency.batch_size,
            backend,
        }
    }

    /// Reserves owned output capacity without consuming transactions from any source.
    /// Waiting time is recorded as executor-channel backpressure for the batcher.
    async fn reserve_output(&mut self) -> Option<mpsc::OwnedPermit<BatchToExecute>> {
        let permit_wait_started = Instant::now();
        let permit = self.ctx.run_until_cancelled(self.out.clone().reserve_owned()).await?.ok()?;
        self.metrics.batcher_output_backpressure_duration.record(permit_wait_started.elapsed().as_secs_f64(), &[]);
        Some(permit)
    }

    /// Builds one prioritized ready batch from bypass, L1-message, and mempool streams.
    /// Intake-mode changes request a stream rebuild without sending an empty batch.
    async fn next_batch(&mut self) -> anyhow::Result<BatcherStep> {
        let (chain_id, sn_version) =
            (self.backend.chain_config().chain_id.to_felt(), self.backend.chain_config().latest_protocol_version);
        let bypass_txs_stream =
            stream::unfold(&mut self.bypass_in, |chan| async move { chan.recv().await.map(|tx| (tx, chan)) }).map(
                |tx| {
                    tx.into_blockifier_for_sequencing()
                        .map(|(btx, ts, declared_class)| (btx, AdditionalTxInfo { declared_class, arrived_at: ts }))
                        .map_err(anyhow::Error::from)
                },
            );
        let l1_txs_stream = self.l1_message_stream.as_mut().map(|res| {
            Ok(res?.into_blockifier(chain_id, sn_version).map(|(btx, declared_class)| {
                (btx, AdditionalTxInfo { declared_class, arrived_at: TxTimestamp::now() })
            })?)
        });
        let mempool_txs_stream: BoxStream<'static, anyhow::Result<_>> = match *self.mempool_intake_rx.borrow() {
            MempoolIntakeMode::Paused => stream::pending().boxed(),
            MempoolIntakeMode::Running => stream::unfold(self.mempool.clone(), |mempool| async move {
                let consumer = mempool.get_consumer().await;
                Some((consumer, mempool))
            })
            .map(|consumer| {
                stream::iter(consumer.map(|tx| {
                    tx.into_blockifier_for_sequencing()
                        .map(|(btx, ts, declared_class)| (btx, AdditionalTxInfo { declared_class, arrived_at: ts }))
                        .map_err(anyhow::Error::from)
                }))
            })
            .flatten()
            .boxed(),
        };
        let tx_stream =
            stream::select_with_strategy(bypass_txs_stream, stream::select(l1_txs_stream, mempool_txs_stream), |()| {
                PollNext::Left
            })
            .try_ready_chunks(self.batch_size.max(1));
        tokio::pin!(tx_stream);

        let batch_wait_started = Instant::now();
        let step = tokio::select! {
            _ = self.ctx.cancelled() => BatcherStep::Stop,
            result = self.mempool_intake_rx.changed() => {
                if result.is_err() { BatcherStep::Stop } else { BatcherStep::RebuildStreams }
            }
            Some(batch) = tx_stream.next() => {
                let batch = batch.context("Creating batch for block building")?;
                tracing::debug!("Batcher got a batch of {}.", batch.len());
                BatcherStep::Batch(batch.into_iter().collect())
            }
            else => BatcherStep::Stop,
        };
        if matches!(&step, BatcherStep::Batch(batch) if !batch.is_empty()) {
            self.metrics.batcher_batch_wait_duration.record(batch_wait_started.elapsed().as_secs_f64(), &[]);
        }
        Ok(step)
    }

    /// Repeatedly reserves executor capacity, builds the next prioritized batch, and sends it.
    /// Cancellation, source closure, or intake-control closure ends the task cleanly.
    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            let Some(permit) = self.reserve_output().await else {
                return Ok(());
            };
            match self.next_batch().await? {
                BatcherStep::Batch(batch) if !batch.is_empty() => {
                    tracing::debug!("Sending batch of {} transactions to the worker thread.", batch.len());
                    permit.send(batch);
                }
                BatcherStep::Batch(_) | BatcherStep::RebuildStreams => continue,
                BatcherStep::Stop => return Ok(()),
            }
        }
    }
}
