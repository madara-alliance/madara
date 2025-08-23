use crate::{versions::user::v0_7_1::StarknetTraceRpcApiV0_7_1Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_block::BlockId;
use mp_rpc::v0_7_1::{
    BroadcastedTxn, SimulateTransactionsResult, SimulationFlag, TraceBlockTransactionsResult, TraceTransactionResult,
};
use simulate_transactions::simulate_transactions;
use starknet_types_core::felt::Felt;
use trace_block_transactions::trace_block_transactions;
use trace_transaction::trace_transaction;

pub(crate) mod simulate_transactions;
pub mod trace_block_transactions;
pub(crate) mod trace_transaction;

#[async_trait]
impl StarknetTraceRpcApiV0_7_1Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>> {
        Ok(simulate_transactions(self, block_id, transactions, simulation_flags).await?)
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TraceBlockTransactionsResult>> {
        Ok(trace_block_transactions(self, block_id).await?)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        Ok(trace_transaction(self, transaction_hash).await?)
    }
}
