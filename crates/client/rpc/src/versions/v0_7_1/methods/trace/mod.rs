pub(crate) mod simulate_transactions;
pub(crate) mod trace_block_transactions;
pub(crate) mod trace_transaction;

use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BlockId, BroadcastedTransaction, Felt, SimulatedTransaction, SimulationFlag, TransactionTraceWithHash,
};

use simulate_transactions::simulate_transactions;
use trace_block_transactions::trace_block_transactions;
use trace_transaction::trace_transaction;

use crate::{versions::v0_7_1::StarknetTraceRpcApiV0_7_1Server, Starknet};

#[async_trait]
impl StarknetTraceRpcApiV0_7_1Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTransaction>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulatedTransaction>> {
        Ok(simulate_transactions(self, block_id, transactions, simulation_flags).await?)
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TransactionTraceWithHash>> {
        Ok(trace_block_transactions(self, block_id).await?)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TransactionTraceWithHash> {
        Ok(trace_transaction(self, transaction_hash).await?)
    }
}
