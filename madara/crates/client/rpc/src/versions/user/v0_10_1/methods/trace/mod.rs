mod simulate_transactions;
mod trace_block_transactions;

use crate::versions::user::v0_10_1::StarknetTraceRpcApiV0_10_1Server;
use crate::versions::user::v0_9_0::StarknetTraceRpcApiV0_9_0Server as V0_9_0Impl;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_1::{
    BroadcastedTxn, SimulateTransactionsResult, SimulationFlag, TraceBlockTransactionsResult, TraceFlag,
    TraceTransactionResult,
};
use starknet_types_core::felt::Felt;

// v0.10.1 trace API implementation
// Main changes from v0.10.0:
// - SimulationFlag now includes RETURN_INITIAL_READS
// - traceBlockTransactions now accepts optional trace_flags parameter
// - Results can include initial_reads when RETURN_INITIAL_READS flag is set

#[async_trait]
impl StarknetTraceRpcApiV0_10_1Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<Vec<SimulateTransactionsResult>> {
        Ok(simulate_transactions::simulate_transactions(self, block_id, transactions, simulation_flags).await?)
    }

    async fn trace_block_transactions(
        &self,
        block_id: BlockId,
        trace_flags: Option<Vec<TraceFlag>>,
    ) -> RpcResult<Vec<TraceBlockTransactionsResult>> {
        Ok(trace_block_transactions::trace_block_transactions(self, block_id, trace_flags).await?)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        V0_9_0Impl::trace_transaction(self, transaction_hash).await
    }
}
