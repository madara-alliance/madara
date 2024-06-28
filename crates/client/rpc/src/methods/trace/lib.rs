use jsonrpsee::core::{async_trait, RpcResult};
use starknet_core::types::{
    BlockId, BroadcastedTransaction, Felt, SimulatedTransaction, SimulationFlag, TransactionTraceWithHash,
};

use super::simulate_transactions::simulate_transactions;
use super::trace_block_transactions::trace_block_transactions;
use super::trace_transaction::trace_transaction;
use crate::{Starknet, StarknetTraceRpcApiServer};

#[async_trait]
impl StarknetTraceRpcApiServer for Starknet {
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
