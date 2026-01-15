mod simulate_transactions;

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
        // Check if RETURN_INITIAL_READS flag is set
        let return_initial_reads = trace_flags
            .as_ref()
            .map(|flags| flags.iter().any(|f| matches!(f, TraceFlag::ReturnInitialReads)))
            .unwrap_or(false);

        // Call v0.9.0 implementation
        let results = V0_9_0Impl::trace_block_transactions(self, block_id).await?;

        // Convert results to v0.10.1 format
        let v0_10_1_results: Vec<TraceBlockTransactionsResult> = results
            .into_iter()
            .map(|result| TraceBlockTransactionsResult {
                trace_root: result.trace_root,
                transaction_hash: result.transaction_hash,
                // TODO: Populate initial_reads when RETURN_INITIAL_READS is requested
                // This requires integration with the execution layer's get_initial_reads()
                initial_reads: if return_initial_reads { Some(Default::default()) } else { None },
            })
            .collect();

        Ok(v0_10_1_results)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        V0_9_0Impl::trace_transaction(self, transaction_hash).await
    }
}
