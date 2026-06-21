use crate::versions::user::v0_10_2::StarknetTraceRpcApiV0_10_2Server as V0_10_2Impl;
use crate::versions::user::v0_10_3::StarknetTraceRpcApiV0_10_3Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_convert::Felt;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_3::{
    BroadcastedTxn, SimulateTransactionsResponse, SimulationFlag, TraceBlockTransactionsResponse, TraceFlag,
    TraceTransactionResult,
};

// v0.10.3 has no semantic changes to the trace API: delegate to v0.10.2.
#[async_trait]
impl StarknetTraceRpcApiV0_10_3Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<SimulateTransactionsResponse> {
        V0_10_2Impl::simulate_transactions(self, block_id, transactions, simulation_flags).await
    }

    async fn trace_block_transactions(
        &self,
        block_id: BlockId,
        trace_flags: Option<Vec<TraceFlag>>,
    ) -> RpcResult<TraceBlockTransactionsResponse> {
        V0_10_2Impl::trace_block_transactions(self, block_id, trace_flags).await
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        V0_10_2Impl::trace_transaction(self, transaction_hash).await
    }
}
