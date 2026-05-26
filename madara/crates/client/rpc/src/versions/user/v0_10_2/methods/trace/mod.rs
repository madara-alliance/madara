mod simulate_transactions;
mod trace_block_transactions;
mod trace_transaction;

use crate::versions::user::v0_10_2::StarknetTraceRpcApiV0_10_2Server;
use crate::Starknet;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::error::INVALID_PARAMS_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use mp_rpc::v0_10_0::{BlockId, BlockTag};
use mp_rpc::v0_10_2::{
    BroadcastedTxn, SimulateTransactionsResponse, SimulationFlag, TraceBlockTransactionsResponse, TraceFlag,
    TraceTransactionResult,
};
use starknet_types_core::felt::Felt;

// v0.10.2 trace API implementation
// Main changes from v0.10.0:
// - SimulationFlag now includes RETURN_INITIAL_READS
// - traceBlockTransactions now accepts optional trace_flags parameter
// - Results can include initial_reads when RETURN_INITIAL_READS flag is set

fn validate_trace_block_transactions_block_id(block_id: &BlockId) -> RpcResult<()> {
    if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
        return Err(ErrorObjectOwned::owned(INVALID_PARAMS_CODE, "Invalid params", None::<()>));
    }
    Ok(())
}

#[async_trait]
impl StarknetTraceRpcApiV0_10_2Server for Starknet {
    async fn simulate_transactions(
        &self,
        block_id: BlockId,
        transactions: Vec<BroadcastedTxn>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<SimulateTransactionsResponse> {
        Ok(simulate_transactions::simulate_transactions(self, block_id, transactions, simulation_flags).await?)
    }

    async fn trace_block_transactions(
        &self,
        block_id: BlockId,
        trace_flags: Option<Vec<TraceFlag>>,
    ) -> RpcResult<TraceBlockTransactionsResponse> {
        validate_trace_block_transactions_block_id(&block_id)?;
        Ok(trace_block_transactions::trace_block_transactions(self, block_id, trace_flags).await?)
    }

    async fn trace_transaction(&self, transaction_hash: Felt) -> RpcResult<TraceTransactionResult> {
        Ok(trace_transaction::trace_transaction(self, transaction_hash).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_block_transactions_rejects_pre_confirmed_tag() {
        let err = validate_trace_block_transactions_block_id(&BlockId::Tag(BlockTag::PreConfirmed))
            .expect_err("pre_confirmed should be rejected for traceBlockTransactions");
        assert_eq!(err.code(), INVALID_PARAMS_CODE);
        assert_eq!(err.message(), "Invalid params");
    }

    #[test]
    fn trace_block_transactions_accepts_latest_tag() {
        assert!(validate_trace_block_transactions_block_id(&BlockId::Tag(BlockTag::Latest)).is_ok());
    }
}
