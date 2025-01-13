use crate::{
    controller::PipelineController,
    p2p::{P2pError, P2pPipelineController, P2pPipelineSteps},
    peer_set::PeerSet,
};
use anyhow::Context;
use futures::StreamExt;
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_block::BlockHeaderWithSignatures;
use std::{ops::Range, sync::Arc};

pub type TransactionsSync = PipelineController<P2pPipelineController<TransactionsSyncSteps>>;
pub fn transactions_pipeline(
    backend: Arc<MadaraBackend>,
    peer_set: Arc<PeerSet>,
    p2p_commands: P2pCommands,
    parallelization: usize,
    batch_size: usize,
) -> TransactionsSync {
    PipelineController::new(
        P2pPipelineController::new(peer_set, TransactionsSyncSteps { backend, p2p_commands }),
        parallelization,
        batch_size,
    )
}
pub struct TransactionsSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
}

impl P2pPipelineSteps for TransactionsSyncSteps {
    type InputItem = BlockHeaderWithSignatures;
    type SequentialStepInput = ();
    type Output = ();

    async fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        tracing::debug!("p2p transactions parallel step: {block_range:?}, peer_id: {peer_id}");
        let strm = self
            .p2p_commands
            .clone()
            .make_transactions_stream(peer_id, BlockStreamConfig::default().with_block_range(block_range.clone()))
            .await;
        tokio::pin!(strm);

        for (block_n, signed_header) in block_range.zip(input) {
            let mut transactions = Vec::with_capacity(signed_header.header.transaction_count as _);
            for _index in 0..signed_header.header.transaction_count {
                let received = strm.next().await.ok_or(P2pError::peer_error("Expected to receive item"))?;
                transactions.push(received);
            }

            // compute merkle root for transactions tree.

            tracing::debug!("storing transactions for {block_n:?}, peer_id: {peer_id}");

            self.backend.store_transactions(block_n, transactions).context("Storing transactions to database")?;
        }

        Ok(())
    }

    async fn p2p_sequential_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        _input: Self::SequentialStepInput,
    ) -> Result<Self::Output, P2pError> {
        tracing::debug!("p2p transactions sequential step: {block_range:?}, peer_id: {peer_id}");
        if let Some(block_n) = block_range.last() {
            self.backend.head_status().transactions.set(Some(block_n));
        }
        Ok(())
    }
}
