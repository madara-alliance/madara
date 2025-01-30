use super::{
    pipeline::{P2pError, P2pPipelineController, P2pPipelineSteps},
    P2pPipelineArguments,
};
use crate::{import::BlockImporter, pipeline::PipelineController};
use futures::TryStreamExt;
use mc_db::{stream::BlockStreamConfig, MadaraBackend};
use mc_p2p::{P2pCommands, PeerId};
use mp_block::Header;
use std::{ops::Range, sync::Arc};

pub type TransactionsSync = PipelineController<P2pPipelineController<TransactionsSyncSteps>>;
pub fn transactions_pipeline(
    P2pPipelineArguments { backend, peer_set, p2p_commands, importer }: P2pPipelineArguments,
    parallelization: usize,
    batch_size: usize,
) -> TransactionsSync {
    PipelineController::new(
        P2pPipelineController::new(peer_set, TransactionsSyncSteps { backend, p2p_commands, importer }),
        parallelization,
        batch_size,
    )
}
pub struct TransactionsSyncSteps {
    backend: Arc<MadaraBackend>,
    p2p_commands: P2pCommands,
    importer: Arc<BlockImporter>,
}

impl P2pPipelineSteps for TransactionsSyncSteps {
    type InputItem = Header;
    type SequentialStepInput = Vec<Header>;
    type Output = Vec<Header>;

    async fn p2p_parallel_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Vec<Header>,
    ) -> Result<Self::SequentialStepInput, P2pError> {
        tracing::debug!("p2p transactions parallel step: {block_range:?}, peer_id: {peer_id}");
        let strm = self
            .p2p_commands
            .clone()
            .make_transactions_stream(
                peer_id,
                BlockStreamConfig::default().with_block_range(block_range.clone()),
                input.iter().map(|input| input.transaction_count as _).collect::<Vec<_>>(),
            )
            .await;
        tokio::pin!(strm);

        for (block_n, header) in block_range.zip(input.iter().cloned()) {
            let transactions = strm.try_next().await?.ok_or(P2pError::peer_error("Expected to receive item"))?;
            tracing::debug!("GOT STATE TRANSACTIONS FOR block_n={block_n}, {transactions:#?}");
            self.importer
                .run_in_rayon_pool(move |importer| {
                    importer.verify_transactions(
                        block_n,
                        &transactions,
                        &header,
                        /* allow_pre_v0_13_2 */ false,
                    )?;
                    importer.save_transactions(block_n, transactions)
                })
                .await?
        }

        Ok(input)
    }

    async fn p2p_sequential_step(
        self: Arc<Self>,
        peer_id: PeerId,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
    ) -> Result<Self::Output, P2pError> {
        tracing::debug!("p2p transactions sequential step: {block_range:?}, peer_id: {peer_id}");
        if let Some(block_n) = block_range.last() {
            self.backend.head_status().transactions.set(Some(block_n));
        }
        Ok(input)
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().transactions.get()
    }
}
