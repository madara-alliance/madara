use crate::util::{AdditionalTxInfo, BatchToExecute};
use futures::{
    stream::{self, PollNext},
    StreamExt,
};
use mc_mempool::Mempool;
use mc_settlement_client::messages_to_l2_consumer::MessagesToL2Consumer;
use mp_chain_config::ChainConfig;
use mp_convert::ToFelt;
use mp_transactions::{validated::ValidatedMempoolTx, IntoBlockifierExt, L1HandlerTransactionWithFee};
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct Batcher {
    chain_config: Arc<ChainConfig>,
    mempool: Arc<Mempool>,
    message_from_l1_consumer: MessagesToL2Consumer,
    ctx: ServiceContext,
    out: mpsc::Sender<BatchToExecute>,
    bypass_in: mpsc::Receiver<ValidatedMempoolTx>,
    batch_size: usize,
}

impl Batcher {
    pub async fn run(mut self) -> anyhow::Result<()> {
        loop {
            // We use the permit API so that we don't have to remove transactions from the mempool until the last moment.
            // The buffer inside of the channel is of size 1 - meaning we're preparing the next batch of transactions that will immediately be executed next, once
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

            let bypass_txs_stream = stream::unfold(&mut self.bypass_in, |chan| async move {
                chan.recv().await.map(|tx| {
                    (
                        tx.into_blockifier()
                            .map(|(btx, _ts, declared_class)| (btx, AdditionalTxInfo { declared_class }))
                            .map_err(anyhow::Error::from),
                        chan,
                    )
                })
            });

            let (chain_id, sn_version) =
                (self.chain_config.chain_id.to_felt(), self.chain_config.latest_protocol_version);
            let l1_txs_stream = stream::unfold(&mut self.message_from_l1_consumer, |consumer| async move {
                consumer.consume_next_or_wait().await.map(|tx| {
                    (
                        tx.into_blockifier(chain_id, sn_version)
                            .map(|(btx, _class)| (btx, AdditionalTxInfo { declared_class }))
                            .map_err(anyhow::Error::from),
                        consumer,
                    )
                })
            });

            let mempool_txs_stream = stream::unfold(&self.mempool, |mempool| async move {
                let consumer = mempool.get_consumer_wait_for_ready_tx().await;
                Some((stream::iter(consumer.map(anyhow::Ok)), mempool))
            })
            .flatten();

            // merge all three streams :)

            // always prioritise bypass_txs, then try and keep the balance betweeen l1_txs and mempool txs.
            // try_ready_chunks will make sure if no item is in any of those streams, we wait, but
            // it will try to fill the batch to the maximum if there are ready items without waiting.
            let tx_stream = stream::select_with_strategy(
                bypass_txs_stream,
                stream::select(l1_txs_stream, mempool_txs_stream), // round-bobbin strategy
                |()| PollNext::Left,                               // always prioritise bypass_txs when possible.
            )
            .try_ready_chunks(self.batch_size);

            tokio::pin!(tx_stream);

            let batch = tokio::select! {
                _ = self.ctx.cancelled() => {
                    // Stop condition: cancelled.
                    return anyhow::Ok(());
                }
                Some(got) = tx_stream.next() => {
                    let got = got?;
                    got.into_iter().collect()
                }
                // Stop condition: tx_stream is empty.
                else => return anyhow::Ok(())
            };

            if !batch.is_empty() {
                tracing::debug!("Sending batch of {} transactions to the worker thread.", batch.len());

                permit.send(batch);
            }
        }
    }
}
