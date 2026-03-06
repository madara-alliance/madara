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
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::watch;

pub struct Batcher {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    l1_message_stream: BoxStream<'static, anyhow::Result<L1HandlerTransactionWithFee>>,
    ctx: ServiceContext,
    out: mpsc::Sender<BatchToExecute>,
    bypass_in: mpsc::Receiver<ValidatedTransaction>,
    mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
    batch_size: usize,
    replay_mode_enabled: bool,
}

impl Batcher {
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        l1_client: Arc<dyn SettlementClient>,
        ctx: ServiceContext,
        out: mpsc::Sender<BatchToExecute>,
        bypass_in: mpsc::Receiver<ValidatedTransaction>,
        mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
        replay_mode_enabled: bool,
    ) -> Self {
        Self {
            mempool,
            l1_message_stream: l1_client.create_message_to_l2_consumer(),
            ctx,
            out,
            bypass_in,
            mempool_intake_rx,
            batch_size: backend.chain_config().block_production_concurrency.batch_size,
            backend,
            replay_mode_enabled,
        }
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            // We use the permit API so that we don't have to remove transactions from the mempool until the last moment.
            // The buffer inside the channel is of size 1 - meaning we're preparing the next batch of transactions that will immediately be executed next, once
            // the worker has finished executing its current one.
            let Some(Ok(permit)) = self.ctx.run_until_cancelled(self.out.reserve()).await else {
                // Stop condition: service stopped (ctx), or batch sender closed.
                return anyhow::Ok(());
            };

            // We have 3 transactions streams:
            // * bypass inclusion (for admin rpc/chain bootstrapping purposes)
            // * l1 to l2 message transactions
            // * mempool transactions
            // and we want to fill in a batch with them, with some priority
            // this is a perfect candidate for the futures-rs stream select api :)

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
                    // L1HandlerTx timestamp is irrelevant
                    (btx, AdditionalTxInfo { declared_class, arrived_at: TxTimestamp::now() })
                })?)
            });

            // Note: this is not hoisted out of the loop, because we don't want to keep the lock around when waiting on the output channel reserve().
            let mempool_txs_stream: BoxStream<'static, anyhow::Result<_>> = {
                match *self.mempool_intake_rx.borrow() {
                    MempoolIntakeMode::Paused => stream::pending().boxed(),
                    MempoolIntakeMode::Running => stream::unfold(self.mempool.clone(), |mempool| async move {
                        let consumer = mempool.get_consumer().await;
                        Some((consumer, mempool))
                    })
                    .map(|c| {
                        stream::iter(c.map(|tx| {
                            tx.into_blockifier_for_sequencing()
                                .map(|(btx, ts, declared_class)| {
                                    (btx, AdditionalTxInfo { declared_class, arrived_at: ts })
                                })
                                .map_err(anyhow::Error::from)
                        }))
                    })
                    .flatten()
                    .boxed(),
                }
            };

            // merge all three streams :)
            // * all three streams are merged into one stream, allowing us to poll them all at once.
            // * this will always prioritise bypass_txs, then try and keep the balance betweeen l1_txs and mempool txs.

            // We then consume the merged stream using `try_ready_chunks`.
            // * `try_ready_chunks` is perfect here since:
            //   * if there is at least one ready item in the stream, it will return with a batch of all of these ready items, up
            //     to `batch_size`. This returns immediately and never waits.
            //   * if there are no ready items in the stream, it will wait until there is at least one.
            // This allows us to batch when possible, but never wait when we don't have to.
            // This means that when the congestion is very low (mempool empty & no pending l1 msg), when a
            // transaction arrives we can instantly pick it up and send it for execution, ensuring the lowest latency possible.

            let mut chunk_size = self.batch_size;
            if self.replay_mode_enabled {
                let head_state = self.backend.chain_head_state();
                let current_block_n = head_state
                    .external_preconfirmed_tip
                    .or(head_state.confirmed_tip.and_then(|block_n| block_n.checked_add(1)))
                    .unwrap_or(0);

                if let Some(remaining) = self.backend.replay_boundary_remaining_dispatch_capacity(current_block_n) {
                    if remaining == 0 {
                        tokio::select! {
                            _ = self.ctx.cancelled() => return anyhow::Ok(()),
                            res = self.mempool_intake_rx.changed() => {
                                if res.is_err() {
                                    return anyhow::Ok(());
                                }
                            }
                            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                        }
                        continue;
                    }

                    chunk_size = chunk_size.min(remaining as usize);
                }
            }

            let tx_stream = stream::select_with_strategy(
                bypass_txs_stream,
                stream::select(l1_txs_stream, mempool_txs_stream), // round-bobbin strategy
                |()| PollNext::Left, // always prioritise bypass_txs when there are ready items in multiple streams
            )
            .try_ready_chunks(chunk_size.max(1));

            tokio::pin!(tx_stream);

            let batch = tokio::select! {
                _ = self.ctx.cancelled() => {
                    // Stop condition: cancelled.
                    return anyhow::Ok(());
                }
                res = self.mempool_intake_rx.changed() => {
                    if res.is_err() {
                        return anyhow::Ok(());
                    }
                    continue;
                }
                Some(got) = tx_stream.next() => {
                    // got a batch :)
                    let got = got.context("Creating batch for block building")?;
                    tracing::debug!("Batcher got a batch of {}.", got.len());
                    got.into_iter().collect::<BatchToExecute>()
                }
                // Stop condition: tx_stream is empty.
                else => return anyhow::Ok(())
            };

            if !batch.is_empty() {
                tracing::debug!("Sending batch of {} transactions to the worker thread.", batch.len());
                if self.replay_mode_enabled {
                    let head_state = self.backend.chain_head_state();
                    let current_block_n = head_state
                        .external_preconfirmed_tip
                        .or(head_state.confirmed_tip.and_then(|block_n| block_n.checked_add(1)))
                        .unwrap_or(0);
                    if let Some(status) =
                        self.backend.replay_boundary_record_dispatched(current_block_n, batch.len() as u64)
                    {
                        if let Some(mismatch) = status.mismatch {
                            tracing::warn!(
                                "replay_boundary_mismatch_after_dispatch block_number={} expected_tx_count={} dispatched_tx_count={} executed_tx_count={} message={}",
                                current_block_n,
                                status.expected_tx_count,
                                status.dispatched_tx_count,
                                status.executed_tx_count,
                                mismatch
                            );
                        }
                    }
                }

                permit.send(batch);
            }
        }
    }
}
