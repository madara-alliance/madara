use crate::versions::user::v0_9_0::StarknetTraceRpcApiV0_9_0Server as V0_9_0Impl;
use crate::{versions::user::v0_10_0::StarknetTraceRpcApiV0_10_0Server, Starknet};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_0::{
    BlockId, BroadcastedTxn, SimulateTransactionsResult, SimulationFlag, TraceBlockTransactionsResult,
    TraceTransactionResult,
};
use starknet_types_core::felt::Felt;

// All trace types are re-exported from v0.9.0, and BlockId is a type alias to v0.9.0::BlockId,
// so we can delegate all trace functions to v0.9.0 implementations

#[async_trait]
impl StarknetTraceRpcApiV0_10_0Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>> {
        V0_9_0Impl::simulate_transactions(self, block_id, transactions, simulation_flags).await
    }

    async fn trace_block_transactions(&self, block_id: BlockId) -> RpcResult<Vec<TraceBlockTransactionsResult>> {
        V0_9_0Impl::trace_block_transactions(self, block_id).await
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        V0_9_0Impl::trace_transaction(self, transaction_hash).await
    }
}
